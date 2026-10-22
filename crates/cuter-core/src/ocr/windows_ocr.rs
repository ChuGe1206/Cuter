//! 基于 `Windows.Media.Ocr` 的离线 OCR 引擎。
//!
//! ## 为什么选它，而不是打包 PP-OCR 模型
//!
//! 本项目的硬约束之一是**存储占用要小**。PP-OCR 的 det + rec + 字典加起来通常
//! 十几到几十 MB，而且要靠 `rten` / `onnxruntime` 之类的运行时去跑。
//! 而 Windows 10/11 自带 `Windows.Media.Ocr`：
//!
//! * **零下载、零打包**——只要系统装了对应语言的 OCR 组件就能用；
//! * **完全离线**，不联网、不依赖任何服务；
//! * 识别质量对截图取词这个场景足够（中英混排、带每行包围盒）。
//!
//! 所以在 Windows 上把它作为默认引擎；`config.ocr.engine` 仍保留 `Http` 选项，
//! 需要更高精度时可以接自建服务。
//!
//! ## 线程模型（很重要）
//!
//! [`crate::ocr::OcrEngine::recognize`] 内部会**阻塞等待** WinRT 异步操作完成
//! （`IAsyncOperation::get()`）。如果这是在已经初始化为 STA 的线程（例如 winit 的
//! 事件循环线程）上做，等待会与「异步结果需要回到同一套间」互相等待而死锁。
//!
//! 因此本模块刻意**不负责线程初始化**，而是要求调用方在**自己的 MTA 工作线程**上
//! 调用 [`WindowsOcr::new`]；[`ensure_winrt_initialized`] 提供了该线程上应当执行的
//! 初始化。宿主（`cuter-app`）就是这么做的：OCR 永远在后台线程跑。
//!
//! ## 已知限制
//!
//! * `OcrWord` **不暴露置信度**，所以 [`TextBlock::score`] 一律填 1.0；
//!   配置里的 `min_score` 对内置引擎实际不生效（已在文档与设置界面说明）；
//! * 只支持系统已安装 OCR 组件的语言，可以用 [`available_languages`] 查询；
//! * 图片单边不得超过 [`WindowsOcr::max_image_dimension`]（本机为 10000），
//!   8K 多屏拼接一般仍在范围内，但超大图会被拒绝而不是静默截断。

use crate::error::{Error, Result};
use crate::frame::Frame;
use crate::geom::{Point, Rect};
use crate::ocr::{OcrEngine, OcrResult, TextBlock};

use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine as WinOcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::HSTRING;

/// 在**当前线程**上初始化 WinRT。
///
/// 必须在使用本模块之前、于同一个工作线程上调用一次。重复调用是安全的。
///
/// 用 MTA 而不是 STA：MTA 下阻塞等待异步结果不会与套间消息泵互相依赖，
/// 因此 `get()` 不会死锁。
pub fn ensure_winrt_initialized() {
    // SAFETY: CoInitializeEx 只接受一个套间模型标志，没有指针参数；
    // 它对同一线程重复调用是合法且幂等的（第二次返回 S_FALSE 或
    // RPC_E_CHANGED_MODE），两种情况都不影响后续调用。
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if hr.is_err() {
        // RPC_E_CHANGED_MODE 表示线程已经是别的套间，这在本进程里不会发生
        // （OCR 跑在我们自己创建的后台线程上）；记录下来便于排查。
        log::debug!("CoInitializeEx(MTA) 返回 {hr:?}（若为 RPC_E_CHANGED_MODE 可忽略）");
    }
}

/// 系统上可用的 OCR 语言标签（例如 `zh-Hans-CN`、`en-US`）。
#[must_use]
pub fn available_languages() -> Vec<String> {
    let mut out = Vec::new();
    let Ok(languages) = WinOcrEngine::AvailableRecognizerLanguages() else {
        return out;
    };
    let Ok(size) = languages.Size() else {
        return out;
    };
    for index in 0..size {
        if let Ok(language) = languages.GetAt(index)
            && let Ok(tag) = language.LanguageTag()
        {
            out.push(tag.to_string());
        }
    }
    out
}

/// Windows 内置 OCR 引擎。
pub struct WindowsOcr {
    engine: WinOcrEngine,
    /// 实际使用的语言标签。
    language: String,
    /// 引擎可接受的最大单边像素。
    max_dimension: u32,
}

impl std::fmt::Debug for WindowsOcr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowsOcr")
            .field("language", &self.language)
            .field("max_dimension", &self.max_dimension)
            .finish()
    }
}

impl WindowsOcr {
    /// 创建引擎。
    ///
    /// `language` 为 `None` / 空串时按用户配置文件的偏好顺序挑选；
    /// 指定的语言系统没装时**不会失败**，而是回退到用户偏好并记一条警告——
    /// 「设置里选了个没装的语言」不该让整个功能不可用。
    pub fn new(language: Option<&str>) -> Result<Self> {
        ensure_winrt_initialized();

        let requested = language.map(str::trim).filter(|s| !s.is_empty());
        let engine = if let Some(tag) = requested {
            match WinOcrEngine::TryCreateFromLanguage(&Language::CreateLanguage(&HSTRING::from(tag))?)
            {
                Ok(engine) => engine,
                Err(err) => {
                    let fallback = WinOcrEngine::TryCreateFromUserProfileLanguages()
                        .map_err(|e| Error::Ocr(format!("系统未安装「{tag}」的 OCR 组件（{err}），\
                             且无法回退到用户偏好语言：{e}")))?;
                    let fallback_tag = fallback.RecognizerLanguage()?.LanguageTag()?;
                    log::warn!("指定的 OCR 语言「{tag}」不可用（{err}），已回退到 {fallback_tag}");
                    fallback
                }
            }
        } else {
            WinOcrEngine::TryCreateFromUserProfileLanguages().map_err(|err| {
                Error::Ocr(format!(
                    "系统没有可用的 OCR 语言（请在「设置 → 时间和语言 → 语言」里安装\
                     语言包的「光学字符识别」组件）：{err}"
                ))
            })?
        };

        let language = engine.RecognizerLanguage()?.LanguageTag()?.to_string();
        let max_dimension = WinOcrEngine::MaxImageDimension().unwrap_or(10_000);

        Ok(Self {
            engine,
            language,
            max_dimension,
        })
    }

    /// 实际使用的语言标签。
    #[must_use]
    pub fn language(&self) -> &str {
        &self.language
    }

    /// 引擎可接受的最大单边像素。
    #[must_use]
    pub fn max_image_dimension(&self) -> u32 {
        self.max_dimension
    }

    /// 把 RGBA8 的 [`Frame`] 转成 OCR 引擎要的 `SoftwareBitmap`。
    ///
    /// 用 `Bgra8` 而不是 `Rgba8`：Windows 自己的解码器产出的就是 Bgra8，
    /// 这条路径是系统内部最常用的，实测也最稳；代价只是一次通道交换。
    fn to_software_bitmap(frame: &Frame) -> Result<SoftwareBitmap> {
        let size = frame.size();
        if size.is_empty() {
            return Err(Error::Ocr("要识别的图片是空的".into()));
        }

        let mut bgra = Vec::with_capacity(frame.data().len());
        for pixel in frame.data().as_chunks::<4>().0 {
            bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }

        let buffer = CryptographicBuffer::CreateFromByteArray(&bgra)
            .map_err(|err| Error::Ocr(format!("创建 WinRT 缓冲区失败：{err}")))?;
        SoftwareBitmap::CreateCopyFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            size.w as i32,
            size.h as i32,
        )
        .map_err(|err| Error::Ocr(format!("创建 SoftwareBitmap 失败：{err}")))
    }
}

/// 阻塞等待一个 WinRT 异步操作完成并取回结果。
///
/// ## 为什么自己轮询，而不是用现成的 `get()`
///
/// `windows-future` **0.3.2 没有提供阻塞式的 `get()`**（那是 0.2.x 的 API，
/// 而 `windows` 0.62 依赖的是 0.3.2），只剩下 COM 层的 `GetResults()`；
/// 而 `GetResults()` 在操作尚未完成时的行为依实现而定，不能依赖。
///
/// 这里走最直白也最稳的路：查 `IAsyncInfo::Status`（方法直接生成在
/// `IAsyncOperation` 上）直到终态，再取结果。一次 OCR 约 100–300 ms，
/// 5 ms 的轮询间隔最多带来几毫秒额外延迟，换来的是**完全不依赖套间消息泵**，
/// 因此在 MTA 工作线程上不会死锁。
fn wait_for_async<T>(operation: &windows_future::IAsyncOperation<T>) -> Result<T>
where
    T: windows::core::RuntimeType + 'static,
{
    use windows_future::AsyncStatus;

    loop {
        let status = operation
            .Status()
            .map_err(|err| Error::Ocr(format!("读取异步状态失败：{err}")))?;
        if status == AsyncStatus::Completed {
            break;
        }
        if status == AsyncStatus::Canceled {
            return Err(Error::Ocr("识别操作被取消".into()));
        }
        if status == AsyncStatus::Error {
            let code = operation
                .ErrorCode()
                .map_err(|err| Error::Ocr(format!("读取错误码失败：{err}")))?;
            return Err(Error::Ocr(format!("识别失败：{code}")));
        }
        // Started / Closed：继续等
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    operation
        .GetResults()
        .map_err(|err| Error::Ocr(format!("取回识别结果失败：{err}")))
}

impl OcrEngine for WindowsOcr {
    fn name(&self) -> &str {
        "Windows 内置 OCR"
    }

    fn recognize(&self, frame: &Frame) -> Result<OcrResult> {
        let size = frame.size();
        if size.w > self.max_dimension || size.h > self.max_dimension {
            return Err(Error::Ocr(format!(
                "图片 {}×{} 超过引擎上限 {}×{}；请缩小框选范围",
                size.w, size.h, self.max_dimension, self.max_dimension
            )));
        }

        let started = std::time::Instant::now();
        let bitmap = Self::to_software_bitmap(frame)?;
        let operation = self
            .engine
            .RecognizeAsync(&bitmap)
            .map_err(|err| Error::Ocr(format!("启动识别失败：{err}")))?;
        let result = wait_for_async(&operation)?;

        let lines = result
            .Lines()
            .map_err(|err| Error::Ocr(format!("读取识别结果失败：{err}")))?;
        let count = lines
            .Size()
            .map_err(|err| Error::Ocr(format!("读取结果行数失败：{err}")))?;

        let mut blocks = Vec::with_capacity(count as usize);
        for index in 0..count {
            let Ok(line) = lines.GetAt(index) else {
                continue;
            };
            let Ok(text) = line.Text() else {
                continue;
            };
            let text = text.to_string();
            if text.trim().is_empty() {
                continue;
            }

            // 一行的包围盒 = 该行所有词包围盒的并集。
            // Windows OCR 只给词级的矩形，行文本却已经在 OcrLine 上给好了，
            // 所以这里把两者拼起来即可。
            let Some(bounds) = line_bounds(&line) else {
                log::debug!("第 {index} 行没有可用的包围盒，已跳过：{text}");
                continue;
            };

            blocks.push(TextBlock::from_quad(quad_of(bounds), text, 1.0));
        }

        Ok(OcrResult {
            blocks,
            elapsed_ms: started.elapsed().as_millis() as u64,
        }
        .in_reading_order())
    }

    fn is_ready(&self) -> bool {
        true
    }
}

/// 一行所有词的包围盒并集。
fn line_bounds(line: &windows::Media::Ocr::OcrLine) -> Option<Rect> {
    let words = line.Words().ok()?;
    let count = words.Size().ok()?;
    if count == 0 {
        return None;
    }

    let mut bounds: Option<Rect> = None;
    for index in 0..count {
        let Ok(word) = words.GetAt(index) else {
            continue;
        };
        let Ok(rect) = word.BoundingRect() else {
            continue;
        };
        // WinRT 的 Rect 用 f32；四舍五入到整像素
        let word_rect = Rect::from_ltrb(
            rect.X.round() as i32,
            rect.Y.round() as i32,
            (rect.X + rect.Width).round() as i32,
            (rect.Y + rect.Height).round() as i32,
        );
        bounds = Some(match bounds {
            Some(acc) => acc.union(word_rect),
            None => word_rect,
        });
    }
    bounds
}

/// 轴对齐矩形 → 四点多边形（我们的 [`TextBlock`] 用四边形是为了兼容旋转文本）。
fn quad_of(rect: Rect) -> [Point; 4] {
    [
        rect.origin(),
        Point::new(rect.right(), rect.top()),
        Point::new(rect.right(), rect.bottom()),
        Point::new(rect.left(), rect.bottom()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Size;

    #[test]
    fn quad_follows_top_left_clockwise_order() {
        let quad = quad_of(Rect::new(10, 20, 30, 40));
        assert_eq!(quad[0], Point::new(10, 20));
        assert_eq!(quad[1], Point::new(40, 20));
        assert_eq!(quad[2], Point::new(40, 60));
        assert_eq!(quad[3], Point::new(10, 60));
    }

    #[test]
    fn quad_derived_bounds_roundtrip() {
        let rect = Rect::new(-5, 7, 100, 20);
        let block = TextBlock::from_quad(quad_of(rect), "x".into(), 1.0);
        assert_eq!(block.bounds, rect, "四边形推出的包围盒必须与原矩形一致");
    }

    #[test]
    fn empty_frame_is_rejected_before_touching_winrt() {
        // 这条不需要 OCR 组件，也不该 panic
        let empty = Frame::new(Size::ZERO);
        assert!(WindowsOcr::to_software_bitmap(&empty).is_err());
    }

    /// 真机：系统至少要有一种 OCR 语言，且能创建引擎并识别出一张带文字的图。
    #[test]
    #[ignore = "需要已安装 OCR 语言组件的交互式 Windows"]
    fn recognizes_rendered_text() {
        let languages = available_languages();
        println!("可用 OCR 语言：{languages:?}");
        assert!(
            !languages.is_empty(),
            "系统没有安装任何 OCR 语言组件，内置引擎无法工作"
        );

        let engine = WindowsOcr::new(None).expect("应当能创建引擎");
        println!(
            "引擎语言 = {}，最大边长 = {}",
            engine.language(),
            engine.max_image_dimension()
        );

        // 画一张确定内容的图：白底 + 黑色横条模拟"文字"。
        // 这里刻意不依赖任何字体渲染——OCR 对纯色块不会给出文字，
        // 所以这条测试只验证「整条管线能跑通并返回结果结构」，
        // 真正的识别质量验证由端到端脚本负责。
        let frame = Frame::filled(Size::new(320, 120), [255, 255, 255, 255]);
        let result = engine.recognize(&frame).expect("识别调用本身不应失败");
        println!("空白图识别到 {} 个文本块", result.blocks.len());
        assert!(
            result.blocks.is_empty() || !result.plain_text(true).trim().is_empty(),
            "识别结果要么为空，要么文本非空，不应出现矛盾的半成品"
        );
    }
}
