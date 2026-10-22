//! 系统字体探测与装载。
//!
//! ## 为什么必须有这个模块
//!
//! egui 自带的字体只有 `Ubuntu-Light` + `NotoEmoji` + `emoji-icon-font`，
//! **完全不含 CJK 字形**。如果不额外装字体，界面上的所有中文都会渲染成「豆腐块」。
//!
//! 同时，导出图片时文字标注需要用同一个字体文件做 CPU 栅格化（见 [`crate::flatten`]），
//! 所以这里返回的不只是「已装进 egui」，还包括原始字节与 TTC 内的 face 索引，
//! 保证**预览与导出用的是同一个字形**。
//!
//! ## 为什么按路径探测而不是打包字体
//!
//! 一个能覆盖中日韩的字体动辄 10-20 MB，直接违背本项目「小体积」的目标。
//! Windows 全平台自带 Microsoft YaHei / SimHei，直接复用系统字体是零成本方案；
//! 找不到时降级为拉丁字体并给出警告，而不是崩溃。

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 已装载的字体。
///
/// `bytes` 是 `&'static [u8]` 而不是 `Vec<u8>`/`Arc<Vec<u8>>`，这是**刻意的**：
/// epaint 的 `FontData` 内部是 `Cow<'static, [u8]>`，`FontData::from_owned` 会**再拷一份**
/// 整个字体文件。微软雅黑 `msyh.ttc` 有 18.8 MB，两份就是 37 MB 常驻内存 ——
/// 对一个"内存占用要小"的工具来说不可接受。
///
/// 字体是进程级常量，`Box::leak` 一次即可让 egui 与 CPU 栅格化（[`crate::flatten`]）
/// 共用同一份字节，零拷贝。代价：**在设置里换字体会让旧字体文件的字节留在进程里**
/// （每次换字体最多泄漏一个字体文件的大小，实测换字体是低频操作）。
#[derive(Clone, Debug)]
pub struct LoadedFont {
    /// 字体文件原始字节（进程生命周期内常驻，见上面的说明）。
    pub bytes: &'static [u8],
    /// TrueType Collection 内的 face 索引（普通 `.ttf`/`.otf` 为 0）。
    pub index: u32,
    /// 人类可读名称，显示在设置界面。
    pub name: String,
    /// 来源路径。
    pub path: PathBuf,
    /// 是否具备 CJK 字形（用于决定要不要提示用户）。
    pub has_cjk: bool,
}

impl LoadedFont {
    /// 传给 [`crate::flatten::flatten`] 的引用形式（零拷贝）。
    #[must_use]
    pub fn font_ref(&self) -> crate::flatten::FontRef<'_> {
        crate::flatten::FontRef {
            bytes: self.bytes,
            index: self.index,
        }
    }
}

/// egui 里这份字体的键名。
pub const EGUI_FONT_KEY: &str = "cuter-system";

/// 在 egui 中装载该字体，并把它插到 `Proportional` / `Monospace` 家族的**最前面**
/// （前面的优先；egui 自带的 emoji 字体仍在后面兜底）。
pub fn install_into_egui(ctx: &egui::Context, font: &LoadedFont) {
    let mut definitions = egui::FontDefinitions::default();

    // 直接构造 `FontData` 走 `Cow::Borrowed`：**不要**用 `from_owned`，
    // 那会把整个字体文件再拷一份（见 `LoadedFont` 的文档）。
    // `tweak` 取默认值即可：它只影响竖直方向的微调，我们没有定制需求。
    let data = egui::FontData {
        font: std::borrow::Cow::Borrowed(font.bytes),
        index: font.index,
        tweak: egui::FontTweak::default(),
    };
    definitions
        .font_data
        .insert(EGUI_FONT_KEY.to_owned(), Arc::new(data));

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        definitions
            .families
            .entry(family)
            .or_default()
            .insert(0, EGUI_FONT_KEY.to_owned());
    }

    ctx.set_fonts(definitions);
    // 已经排好版的那一帧用的是旧（无 CJK）字体表，`set_fonts` 之后必须再画一帧，
    // 否则用户会看见一帧全方框（"概率性出现文字变成豆腐块"）。
    ctx.request_repaint();
}

/// 一个候选字体。
#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: PathBuf,
    pub index: u32,
    pub name: &'static str,
    pub has_cjk: bool,
}

/// 系统字体目录（Windows 会同时看全局与用户级安装目录）。
#[must_use]
pub fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(windows)]
    {
        if let Ok(windir) = std::env::var("WINDIR") {
            dirs.push(PathBuf::from(windir).join("Fonts"));
        } else {
            dirs.push(PathBuf::from(r"C:\Windows\Fonts"));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/System/Library/Fonts"));
        dirs.push(PathBuf::from("/Library/Fonts"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        dirs.push(PathBuf::from("/usr/share/fonts"));
        dirs.push(PathBuf::from("/usr/local/share/fonts"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(&home).join(".local/share/fonts"));
            dirs.push(PathBuf::from(&home).join(".fonts"));
        }
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

/// 按优先级排列的候选字体文件名。
///
/// 顺序即优先级：越靠前越「能显示中文且好看」。中日韩字体在前，纯拉丁字体垫底。
const CANDIDATE_FILES: &[(&str, u32, &str, bool)] = &[
    // ---- 中文（简体）----
    ("msyh.ttc", 0, "Microsoft YaHei（微软雅黑）", true),
    ("msyh.ttf", 0, "Microsoft YaHei（微软雅黑）", true),
    ("msyhl.ttc", 0, "Microsoft YaHei Light", true),
    ("simhei.ttf", 0, "SimHei（黑体）", true),
    ("Deng.ttf", 0, "DengXian（等线）", true),
    ("simsun.ttc", 0, "SimSun（宋体）", true),
    ("simkai.ttf", 0, "KaiTi（楷体）", true),
    ("simfang.ttf", 0, "FangSong（仿宋）", true),
    // ---- 中文（繁体 / 日 / 韩）----
    ("msjh.ttc", 0, "Microsoft JhengHei（正黑体）", true),
    ("YuGothM.ttc", 0, "Yu Gothic（游ゴシック）", true),
    ("meiryo.ttc", 0, "Meiryo（メイリオ）", true),
    ("malgun.ttf", 0, "Malgun Gothic（맑은 고딕）", true),
    // ---- 开源 CJK（用户自行安装时命中）----
    ("NotoSansCJK-Regular.ttc", 0, "Noto Sans CJK", true),
    ("NotoSansSC-Regular.otf", 0, "Noto Sans SC", true),
    ("SourceHanSansSC-Regular.otf", 0, "Source Han Sans SC", true),
    ("wqy-microhei.ttc", 0, "WenQuanYi Micro Hei（文泉驿微米黑）", true),
    ("PingFang.ttc", 0, "PingFang SC（苹方）", true),
    // ---- 拉丁兜底 ----
    ("segoeui.ttf", 0, "Segoe UI", false),
    ("arial.ttf", 0, "Arial", false),
    ("tahoma.ttf", 0, "Tahoma", false),
    ("DejaVuSans.ttf", 0, "DejaVu Sans", false),
    ("Verdana.ttf", 0, "Verdana", false),
];

/// 展开全部候选（已过滤掉不存在的路径）。
#[must_use]
pub fn candidates() -> Vec<Candidate> {
    let dirs = font_dirs();
    let mut out = Vec::new();
    for (file, index, name, has_cjk) in CANDIDATE_FILES {
        for dir in &dirs {
            let path = dir.join(file);
            if path.is_file() {
                out.push(Candidate {
                    path,
                    index: *index,
                    name,
                    has_cjk: *has_cjk,
                });
            }
        }
    }
    out
}

/// 字体文件的魔数校验：避免把损坏文件或占位文件读进来。
#[must_use]
fn looks_like_font(bytes: &[u8]) -> bool {
    if bytes.len() < 12 {
        return false;
    }
    matches!(
        &bytes[0..4],
        [0x00, 0x01, 0x00, 0x00] // TrueType
            | b"true"             // Apple TrueType
            | b"OTTO"             // CFF / OpenType
            | b"ttcf"             // TrueType Collection
            | b"wOFF"             // Web Open Font Format
    )
}

/// 装载系统字体。
///
/// `preferred` 可以是一个**文件路径**，也可以是一个**候选名称关键字**
/// （例如 `"simhei"`、`"YaHei"`），来自 `config.annotate.text_font`。
/// 传入 `None` 或匹配失败时按 [`CANDIDATE_FILES`] 的优先级回退。
///
/// 全部候选都失败时返回 `None`，调用方应当提示用户「中文可能无法显示」。
#[must_use]
pub fn load_system_font(preferred: Option<&str>) -> Option<LoadedFont> {
    let candidates = candidates();

    if let Some(pref) = preferred.map(str::trim).filter(|s| !s.is_empty())
        && let Some(font) = try_load_preferred(pref) {
            return Some(font);
        }

    for candidate in &candidates {
        if let Some(font) = try_load(candidate) {
            log::info!(
                "已装载系统字体：{}（{}，CJK={}）",
                font.name,
                font.path.display(),
                font.has_cjk
            );
            if !font.has_cjk {
                log::warn!(
                    "未找到中文字体，界面与文字标注中的中文可能显示为方框；\
                     可在设置中指定字体文件，或安装「微软雅黑 / 黑体」"
                );
            }
            return Some(font);
        }
    }

    log::error!("系统里找不到任何可用字体，文字将无法渲染");
    None
}

/// 尝试把 `preferred` 当作路径或名称关键字解析。
fn try_load_preferred(preferred: &str) -> Option<LoadedFont> {
    // 1) 直接当路径
    let path = Path::new(preferred);
    if path.is_file() {
        let candidate = Candidate {
            path: path.to_path_buf(),
            index: 0,
            name: "自定义字体",
            has_cjk: true, // 用户显式指定的，按用户的判断来
        };
        if let Some(font) = try_load(&candidate) {
            log::info!("使用设置中指定的字体：{}", path.display());
            return Some(font);
        }
    }

    // 2) 当名称关键字模糊匹配
    let needle = preferred.to_ascii_lowercase();
    for candidate in candidates() {
        let file_name = candidate
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if (file_name.contains(&needle) || candidate.name.to_ascii_lowercase().contains(&needle))
            && let Some(font) = try_load(&candidate) {
                log::info!("按关键字「{preferred}」匹配到字体：{}", font.path.display());
                return Some(font);
            }
    }

    log::warn!("设置里指定的字体「{preferred}」没找到，回退到自动探测");
    None
}

fn try_load(candidate: &Candidate) -> Option<LoadedFont> {
    let bytes = match std::fs::read(&candidate.path) {
        Ok(bytes) => bytes,
        Err(err) => {
            log::debug!("读取字体失败 {}：{err}", candidate.path.display());
            return None;
        }
    };
    if !looks_like_font(&bytes) {
        log::debug!("{} 不是有效的字体文件", candidate.path.display());
        return None;
    }
    Some(LoadedFont {
        // 见 `LoadedFont` 的文档：leak 一次，egui 与导出共用同一份字节，避免两份 18.8 MB
        bytes: Box::leak(bytes.into_boxed_slice()),
        index: candidate.index,
        name: candidate.name.to_owned(),
        path: candidate.path.clone(),
        has_cjk: candidate.has_cjk,
    })
}

/// 便捷封装：探测 + 装进 egui，一步到位。返回装载结果供导出复用。
pub fn setup(ctx: &egui::Context, preferred: Option<&str>) -> Option<LoadedFont> {
    let font = load_system_font(preferred)?;
    install_into_egui(ctx, &font);
    Some(font)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_number_check_accepts_known_font_headers() {
        assert!(looks_like_font(&[0x00, 0x01, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(looks_like_font(b"ttcf__________"));
        assert!(looks_like_font(b"OTTO__________"));
        assert!(!looks_like_font(b"not a font!!"));
        assert!(!looks_like_font(b"short"));
        assert!(!looks_like_font(&[]));
    }

    #[test]
    fn candidates_point_at_existing_files() {
        for candidate in candidates() {
            assert!(
                candidate.path.is_file(),
                "候选路径应当是真实存在的文件：{}",
                candidate.path.display()
            );
        }
    }

    #[test]
    fn load_returns_a_usable_font_when_any_font_exists() {
        let Some(font) = load_system_font(None) else {
            // 无字体环境（极端精简容器）不算失败
            return;
        };
        assert!(looks_like_font(font.bytes));
        assert!(!font.name.is_empty());
        assert_eq!(
            font.font_ref().bytes.len(),
            font.bytes.len(),
            "FontRef 必须借用同一份字节，避免导出时二次拷贝"
        );
        // 借用同一份内存而不是各自持有一份：这是「内存占用」指标的关键不变量
        assert!(
            std::ptr::eq(font.font_ref().bytes, font.bytes),
            "FontRef 与 LoadedFont 必须指向同一个缓冲区"
        );
    }

    #[test]
    fn unknown_preferred_name_falls_back_instead_of_failing() {
        // 故意给一个不存在的名字：应当回退到自动探测，而不是返回 None
        let fallback = load_system_font(Some("ThisFontDoesNotExist-zzz"));
        let auto = load_system_font(None);
        assert_eq!(fallback.is_some(), auto.is_some());
    }

    #[test]
    fn preferred_path_is_used_verbatim() {
        let Some(first) = load_system_font(None) else {
            return;
        };
        let path = first.path.to_string_lossy().to_string();
        let again = load_system_font(Some(&path)).expect("显式路径必须能装载");
        assert_eq!(again.path, first.path);
    }
}
