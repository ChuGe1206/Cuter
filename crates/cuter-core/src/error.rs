//! 统一错误类型。

use std::path::PathBuf;

/// cuter-core 的错误类型。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("当前平台不支持该功能：{0}")]
    Unsupported(&'static str),

    #[error("Win32 调用 {api} 失败（错误码 {code}）")]
    Win32 { api: &'static str, code: u32 },

    #[error("没有找到可用的显示器")]
    NoMonitor,

    #[error("显示器不存在：{0}")]
    MonitorNotFound(u64),

    #[error("区域为空或完全落在屏幕之外：{0:?}")]
    EmptyRegion(crate::geom::Rect),

    #[error("位图尺寸非法：{w}x{h}（期望缓冲区 {expected} 字节，实际 {actual} 字节）")]
    BadBufferSize {
        w: u32,
        h: u32,
        expected: usize,
        actual: usize,
    },

    #[error("不支持的图片格式：{0}")]
    UnsupportedFormat(String),

    #[error("无法识别的图片数据")]
    UnknownImageFormat,

    #[error("文件 IO 失败 {path}：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("图片编解码失败：{0}")]
    Image(#[from] image::ImageError),

    #[error("配置解析失败：{0}")]
    ConfigParse(#[from] toml::de::Error),

    #[error("配置序列化失败：{0}")]
    ConfigSerialize(#[from] toml::ser::Error),

    #[error("OCR 失败：{0}")]
    Ocr(String),

    /// 参数/状态不合法（不是 IO 也不是编解码问题）。
    #[error("{0}")]
    Invalid(String),

    /// 当前环境不满足条件，**换一条实现通常就行**（调用方应当回退）。
    ///
    /// 典型场景：RDP 会话 / 被别的录屏程序独占时拿不到 DXGI 桌面复制、
    /// 桌面复制遇到 `ACCESS_LOST`、冷启动时一直 `WAIT_TIMEOUT`。
    /// 与 [`Error::Unsupported`] 的区别：那个是"这个平台/功能根本没实现"，这个只是"这会儿不行"。
    #[error("当前环境不可用：{0}")]
    Unavailable(String),

    /// WinRT（`Windows.*` 命名空间）调用失败。
    ///
    /// 单独一个变体是为了让 `?` 能直接作用在 WinRT 调用上，同时保留原始 HRESULT——
    /// 排查「OCR 语言组件没装」这类问题时，`0x80070002` 之类的错误码是有用信息，
    /// 丢掉它只会让人一头雾水。
    #[cfg(windows)]
    #[error("WinRT 调用失败：{0}")]
    WinRt(#[from] windows::core::Error),
}

impl Error {
    /// 把 `std::io::Error` 附加路径上下文。
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// 便捷别名。
pub type Result<T> = std::result::Result<T, Error>;
