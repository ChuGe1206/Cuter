//! 配置持久化。
//!
//! 全部设置项集中在 `%APPDATA%\Cuter\config.toml`（其它平台为
//! `dirs::config_dir()/Cuter/config.toml`）。设计要点：
//!
//! * 每个字段都有 `Default`，因此配置文件可以只写用户改过的项；
//! * 解析失败不会让程序崩溃——坏文件被重命名为 `config.toml.bak` 后回退默认值，
//!   这与「升级后配置不兼容直接报错」的常见做法正好相反；
//! * 不依赖 `chrono`：本地时间直接走 Win32 `GetLocalTime`，避免为格式化一个
//!   文件名而引入时区数据库。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::geom::Rect;
use crate::scroll::MatchOptions;

/// 应用级配置根。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub hotkeys: HotkeyConfig,
    pub capture: CaptureConfig,
    pub annotate: AnnotateConfig,
    pub save: SaveConfig,
    pub pin: PinConfig,
    pub ocr: OcrConfig,
    pub scroll: ScrollConfig,
    pub history: HistoryConfig,
}

/// 便携模式的标记文件名。放在 exe 旁边即进入便携模式。
pub const PORTABLE_MARKER: &str = "cuter.portable";

impl Config {
    /// 配置文件路径。
    #[must_use]
    pub fn path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    /// 配置目录。
    ///
    /// **便携模式**下就是 exe 所在目录（见 [`Config::is_portable`]），否则是
    /// `%APPDATA%\Cuter`。日志与数据目录都从它派生，所以"便携"是一处开关、全局生效。
    #[must_use]
    pub fn config_dir() -> PathBuf {
        if let Some(dir) = exe_dir().filter(|dir| portable_root(dir).is_some()) {
            return dir;
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Cuter")
    }

    /// 是否处于**便携模式**：exe 旁边有 [`PORTABLE_MARKER`] 这个空文件。
    ///
    /// 为什么用标记文件而不是"exe 旁边可写就便携"：可写性会在 U 盘写保护、
    /// 程序装在 `Program Files`、目录只读等情况下悄悄变化，而用户最容易被坑的正是这里——
    /// 明明想"带着配置走"，配置却写回了 `%APPDATA%`。标记文件是用户自己放的，
    /// 语义确定，也方便打包脚本一键生成。
    #[must_use]
    pub fn is_portable() -> bool {
        exe_dir().is_some_and(|dir| portable_root(&dir).is_some())
    }

    /// 数据目录（缩略图缓存、截图历史、OCR 模型等）。便携模式下同样跟着 exe 走。
    ///
    /// ⚠️ 用 [`dirs::data_local_dir`]（`%LOCALAPPDATA%`）而**不是** `dirs::data_dir`：
    /// 后者在 Windows 上是 **Roaming**，域账户/多机同步的漫游配置文件会把
    /// 几十 MB 的截图历史一起同步走——那是缓存，不该漫游。实测踩过：
    /// 历史最早写进了 `%APPDATA%\Cuter\history`，探针按 `%LOCALAPPDATA%` 找，一条都找不到。
    #[must_use]
    pub fn data_dir() -> PathBuf {
        if Self::is_portable() {
            return Self::config_dir().join("data");
        }
        dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Cuter")
    }

    /// 读取配置。**永不失败**：任何异常都会记录日志并回退到默认值。
    ///
    /// 读坏的那份会被改名成 `config.toml.broken-<时间戳>` 留在原处（**带时间戳**，
    /// 因为早期版本用的是固定名 `config.toml.bak`：第二次读坏就把第一次的备份盖掉了，
    /// 而那份备份往往才是用户唯一能救回来的东西）。
    ///
    /// 回退这件事必须让用户知道——设置"莫名其妙全变回默认"是最让人怀疑程序在坑他的瞬间，
    /// 所以 [`Self::last_recovery`] 会把这次回退（连同备份路径）记下来，交给界面展示。
    #[must_use]
    pub fn load() -> Self {
        let (config, recovery) = Self::load_with_recovery();
        if let Some(recovery) = recovery {
            let _ = RECOVERY.set(recovery);
        }
        config
    }

    /// 同 [`Self::load`]，但把"这次是不是回退了"直接交给调用方（测试用这条，避免全局状态）。
    #[must_use]
    pub fn load_with_recovery() -> (Self, Option<Recovery>) {
        Self::load_from_with_recovery(&Self::path())
    }

    /// [`Self::load_with_recovery`] 的"任意路径"版本：测试要能指向临时文件。
    ///
    /// **文件不存在不等于读坏**：首次运行本来就没有配置文件，这时不该说"配置读不出来"
    /// （那会把正常的第一印象变成一次莫须有的报错）。
    #[must_use]
    pub fn load_from_with_recovery(path: &Path) -> (Self, Option<Recovery>) {
        if !path.exists() {
            return (Self::default(), None);
        }
        match Self::load_from(path) {
            Ok(cfg) => (cfg, None),
            Err(err) => {
                let mut backup = None;
                if path.exists() {
                    log::warn!("读取配置失败（{err}），已回退默认值：{}", path.display());
                    let target = broken_backup_path(path);
                    match std::fs::rename(path, &target) {
                        Ok(()) => {
                            log::warn!("损坏的配置已备份到：{}", target.display());
                            backup = Some(target);
                        }
                        Err(e) => log::warn!("备份损坏配置失败：{e}"),
                    }
                }
                (
                    Self::default(),
                    Some(Recovery {
                        error: err.to_string(),
                        backup,
                    }),
                )
            }
        }
    }

    /// 最近一次"配置读坏了、已回退默认值"的记录（进程内只保留第一次）。
    #[must_use]
    pub fn last_recovery() -> Option<&'static Recovery> {
        RECOVERY.get()
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        let cfg: Self = toml::from_str(&text)?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let text = toml::to_string_pretty(self)?;
        write_atomically(path, text.as_bytes()).map_err(|e| Error::io(path, e))?;
        Ok(())
    }

    /// 修正越界数值（手动编辑配置文件后仍能正常启动）。
    pub fn sanitize(&mut self) {
        self.annotate.clamp();
        self.pin.clamp();
        self.ocr.clamp();
        self.scroll.clamp();
        self.history.clamp();
        self.capture.delay_ms = self.capture.delay_ms.min(30_000);
        if self.save.format.is_empty() {
            self.save.format = "png".into();
        }
        if self.save.filename_pattern.trim().is_empty() {
            self.save.filename_pattern = SaveConfig::default().filename_pattern;
        }
        self.save.quality = self.save.quality.clamp(1, 100);
        // 时间戳只增不减；手改配置写成负数会让节流永远命中，等于关掉了自动检查。
        self.general.last_update_check = self.general.last_update_check.max(0);
        self.general.skipped_version = self.general.skipped_version.trim().to_owned();
    }
}

/// exe 所在目录（取不到时为 `None`）。
#[must_use]
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

/// 目录里有便携标记文件时返回该目录。抽成纯函数是为了能无头测试。
#[must_use]
fn portable_root(dir: &Path) -> Option<PathBuf> {
    dir.join(PORTABLE_MARKER)
        .is_file()
        .then(|| dir.to_path_buf())
}

/// 原子写入：先写同目录下的临时文件，落盘后再 `rename` 覆盖目标。
///
/// 为什么不能直接 `fs::write`：配置是**每次退出**、每次改设置都会重写的。直接覆写意味着
/// "写到一半断电 / 被杀进程"会留下半个文件——虽然 [`Config::load`] 会把损坏的配置备份并
/// 回退默认值（程序仍然起得来），但用户所有设置会一起丢。`rename` 在同一个卷上是原子的
/// （Windows 上底层是带 `REPLACE_EXISTING` 的 `MoveFileEx`）。
///
/// `sync_all` 不能省：不把数据真的刷到盘上就 `rename`，断电后可能得到一个"文件名对、
/// 内容为空"的配置。
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);

    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();

    if result.is_err() {
        // 失败时别把临时文件留在用户目录里；删不掉也无所谓。
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

// ---------------------------------------------------------------------------
// general
// ---------------------------------------------------------------------------
/// 通用设置。
///
/// **这里只放已经真正生效的项**：`check_update` 曾经作为占位字段存在、后来被删掉
/// （属于「配置里写着、程序里没有」的假开关）；到 0.1.47 真正做出了更新检查，
/// 它才带着 `update_endpoint` 一起回到这张表里。
/// `show_tray_notifications` 同样是先占位、后在托盘气泡那一轮真正实现。
/// 以后再想加开关，先看 `docs/roadmap.md` 里有没有对应条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub theme: ThemeKind,
    /// 界面语言：`auto`（跟随系统）/ `zh-CN` / `en-US`。
    ///
    /// 中文原文就是翻译表的键，查不到就原样显示中文，所以这个开关**永远不会**把界面
    /// 变成空白或乱码；认不出来的值一律按 `auto` 处理。
    pub language: String,
    /// 启动后只驻留托盘、不显示窗口。
    pub start_in_tray: bool,
    /// 关闭主窗口时退到托盘而不是退出。
    pub close_to_tray: bool,
    /// 开机自启：在 `HKCU\...\Run` 里写一条指向本 exe 的命令（见 `cuter-app` 的 `desktop`）。
    pub autostart: bool,
    /// 托盘气泡通知：启动驻留托盘时打个招呼（告诉用户热键是什么），
    /// 以及每次成功保存后提示落在哪里。走系统通知，通知失败只记日志、绝不影响截图。
    pub show_tray_notifications: bool,
    /// 启动时检查有没有新版本（只查、只提示、只帮你打开下载页，不做静默替换）。
    pub check_update: bool,
    /// 版本清单的地址：一份 `{"version","url","notes"}` 的 JSON。
    ///
    /// 本项目没有官方发布渠道，所以这里**默认留空**——留空就等于关掉检查，
    /// 不会往任何服务器发请求。想用就填自己的地址（`https://` 走系统 WinHTTP，
    /// `http://` 走内置的极简客户端；两者都不引入新依赖）。
    pub update_endpoint: String,
    /// 用户点过「跳过这个版本」的版本号；自动检查不再为它提示。
    ///
    /// 只记一个版本，不记历史：下一个版本出来时用户自然会再看到提示，
    /// 而记一堆「永远不想看到」的版本，反而会在用户换机器后继续沉默。
    pub skipped_version: String,
    /// 上次**成功**检查更新的时间（Unix 秒，0 表示从没查过）。
    ///
    /// 启动时的自动检查靠它做节流（见 `cuter-app` 的 `start_update_check`）：
    /// 一天最多查一次，不做那种每次开机都联网的程序。手点「立即检查」不受它限制。
    pub last_update_check: i64,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            theme: ThemeKind::Auto,
            language: "auto".to_owned(),
            start_in_tray: true,
            close_to_tray: true,
            // 默认**不**自启：不请自来地往注册表里写东西，是用户最反感的行为之一。
            autostart: false,
            show_tray_notifications: true,
            check_update: true,
            // 没有官方发布渠道：地址留空=不检查（发了请求也是发给我们不认识的人）。
            update_endpoint: String::new(),
            skipped_version: String::new(),
            last_update_check: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeKind {
    Auto,
    Dark,
    Light,
}

impl ThemeKind {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "跟随系统",
            Self::Dark => "深色",
            Self::Light => "浅色",
        }
    }
}

// ---------------------------------------------------------------------------
// hotkeys
// ---------------------------------------------------------------------------

/// 全局热键。空字符串表示不绑定。
///
/// 语法：`修饰键+修饰键+主键`，修饰键支持 `Ctrl`/`Control`/`Alt`/`Shift`/`Win`/`Super`，
/// 主键为字母、数字或 `F1`-`F24`、`PrintScreen` 等。例如 `Alt+C`、`Ctrl+Shift+A`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// 框选截图。
    pub capture: String,
    /// 直接截取鼠标所在显示器全屏。
    pub capture_monitor: String,
    /// 截取整个虚拟桌面。
    pub capture_desktop: String,
    /// 滚动截屏（长截图）。
    pub scroll_capture: String,
    /// 把剪贴板里的图片贴到屏幕上。
    pub pin_from_clipboard: String,
    /// 对剪贴板 / 上次截图做 OCR。
    pub ocr: String,
    /// 从剪贴板贴图并立刻 OCR。
    pub ocr_clipboard: String,
    /// 隐藏 / 显示屏幕上所有贴图（开会、演示时用）。
    pub toggle_pins: String,
    /// 切换到下一个贴图分组。
    pub next_pin_group: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            // 沿用常见截图工具的肌肉记忆，但避开 Win+Shift+S
            capture: "Alt+C".into(),
            capture_monitor: "Alt+X".into(),
            capture_desktop: String::new(),
            scroll_capture: "Alt+S".into(),
            pin_from_clipboard: "Alt+V".into(),
            ocr: "Alt+O".into(),
            ocr_clipboard: String::new(),
            // ⚠️ **不要用别人占着的键位**：Shift+F3 / Ctrl+F3 常被常驻的截图类程序
            // 拿去做「隐藏所有贴图」「切换贴图组」，这类程序在用户机器上常常就在跑，
            // `RegisterHotKey` 会直接失败（实测日志：`HotKey already registered`），
            // 于是功能"看起来实现了、按下去没反应"。
            // 用 Alt+H（hide）与 Alt+G（group），和本程序已有的 Alt+C/X/S/V/O 也不冲突。
            toggle_pins: "Alt+H".into(),
            next_pin_group: "Alt+G".into(),
        }
    }
}

impl HotkeyConfig {
    /// 逐项遍历（用于注册/注销），返回 `(设置项名, 组合键字符串)`。
    #[must_use]
    pub fn entries(&self) -> [(&'static str, &str); 9] {
        [
            ("capture", &self.capture),
            ("capture_monitor", &self.capture_monitor),
            ("capture_desktop", &self.capture_desktop),
            ("scroll_capture", &self.scroll_capture),
            ("pin_from_clipboard", &self.pin_from_clipboard),
            ("ocr", &self.ocr),
            ("ocr_clipboard", &self.ocr_clipboard),
            ("toggle_pins", &self.toggle_pins),
            ("next_pin_group", &self.next_pin_group),
        ]
    }

    pub fn set(&mut self, key: &str, value: String) {
        match key {
            "capture" => self.capture = value,
            "capture_monitor" => self.capture_monitor = value,
            "capture_desktop" => self.capture_desktop = value,
            "scroll_capture" => self.scroll_capture = value,
            "pin_from_clipboard" => self.pin_from_clipboard = value,
            "ocr" => self.ocr = value,
            "ocr_clipboard" => self.ocr_clipboard = value,
            "toggle_pins" => self.toggle_pins = value,
            "next_pin_group" => self.next_pin_group = value,
            _ => log::warn!("未知热键项：{key}"),
        }
    }
}

// ---------------------------------------------------------------------------
// capture
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureConfig {
    /// 捕获后端：`gdi`（默认，兼容性最好）或 `dxgi`（桌面复制，更快）。
    ///
    /// 默认 GDI 是有意的：DXGI 依赖桌面复制，RDP 会话没有、被别的录屏程序独占时也没有。
    /// 选 `dxgi` 时拿不到复制通道会自动回退 GDI，并写一条日志。
    pub backend: crate::capture::CaptureBackend,
    /// 把鼠标指针画进位图。
    pub include_cursor: bool,
    /// 按下热键后延迟若干毫秒再捕获（用于等菜单收起）。
    pub delay_ms: u32,
    /// 记住上次框选区域，下次按方向键可快速复原。
    pub remember_last_region: bool,
    pub last_region: Option<Rect>,
    /// 遮罩上显示放大镜 + 取色。
    pub show_magnifier: bool,
    /// 框选时自动吸附到窗口/控件边界。
    pub auto_detect_windows: bool,
    /// 框选时进一步吸附到**单个控件**（按钮 / 输入框 / 列表项），走系统 UI Automation。
    ///
    /// 比窗口级更精确，但每次探测要跨进程调 COM（实测 5~8 ms），所以遮罩里做了节流
    /// （悬停时最多 150 ms 问一次）。老机器 / 远程桌面上如果觉得卡，可以在这里关掉。
    pub snap_controls: bool,
    /// 捕获前隐藏 Cuter 自己的窗口（贴图、主窗口）。
    pub hide_own_windows: bool,
    /// 遮罩出现时播放提示音。
    pub play_sound: bool,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            backend: crate::capture::CaptureBackend::Gdi,
            include_cursor: true,
            delay_ms: 0,
            remember_last_region: true,
            last_region: None,
            show_magnifier: true,
            auto_detect_windows: true,
            snap_controls: true,
            hide_own_windows: true,
            play_sound: false,
        }
    }
}

// ---------------------------------------------------------------------------
// annotate
// ---------------------------------------------------------------------------

/// 标注工具种类（配置层定义，UI 层负责渲染）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// 无工具：纯框选 / 移动选区。
    Select,
    Rectangle,
    Ellipse,
    Arrow,
    Pen,
    Highlighter,
    Mosaic,
    Blur,
    Text,
    /// 序号标记（①②③…）。
    Counter,
    Laser,
}

impl ToolKind {
    /// 工具条上的显示顺序。
    pub const ALL: [Self; 10] = [
        Self::Select,
        Self::Rectangle,
        Self::Ellipse,
        Self::Arrow,
        Self::Pen,
        Self::Highlighter,
        Self::Mosaic,
        Self::Blur,
        Self::Text,
        Self::Counter,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "选择",
            Self::Rectangle => "矩形",
            Self::Ellipse => "椭圆",
            Self::Arrow => "箭头",
            Self::Pen => "画笔",
            Self::Highlighter => "荧光笔",
            Self::Mosaic => "马赛克",
            Self::Blur => "模糊",
            Self::Text => "文字",
            Self::Counter => "序号",
            Self::Laser => "激光笔",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnnotateConfig {
    pub default_tool: ToolKind,
    /// RGB 描边颜色，范围 0-255。
    pub stroke_color: [u8; 3],
    pub stroke_width: f32,
    /// 矩形/椭圆是否填充半透明底色。
    pub fill_shapes: bool,
    pub font_size: f32,
    pub mosaic_block: u32,
    pub blur_radius: u32,
    /// 文字标注使用系统字体还是内置等宽字体。
    pub text_font: String,
    /// 撤回栈上限。
    pub undo_limit: usize,
}

impl Default for AnnotateConfig {
    fn default() -> Self {
        Self {
            // **默认是「选择」，不是「矩形」**。
            //
            // 之前默认矩形，后果是：遮罩一出来工具条上高亮的就是矩形，用户第一次拖动
            // 仍然会拉出选区（那时还没有选区），但**单击高亮的窗口什么都不发生**
            // （只是画了一个 1 像素的矩形）——这正是用户反馈的第 3 条。
            // 截图工具的第一步永远是"选哪里"，所以默认回到「选择」；
            // 想"选完立刻画框"的用户在设置里把默认工具改成矩形即可。
            default_tool: ToolKind::Select,
            stroke_color: [255, 59, 48],
            stroke_width: 3.0,
            fill_shapes: false,
            font_size: 20.0,
            mosaic_block: 12,
            blur_radius: 10,
            text_font: "Microsoft YaHei".into(),
            undo_limit: 100,
        }
    }
}

impl AnnotateConfig {
    /// 夹取到合法范围。遮罩里改完样式后写回配置也要过这一道，
    /// 免得滑块/手改配置留下越界值。
    pub fn clamp(&mut self) {
        self.stroke_width = self.stroke_width.clamp(1.0, 64.0);
        self.font_size = self.font_size.clamp(8.0, 200.0);
        self.mosaic_block = self.mosaic_block.clamp(2, 128);
        self.blur_radius = self.blur_radius.clamp(1, 64);
        self.undo_limit = self.undo_limit.clamp(1, 1000);
    }
}

// ---------------------------------------------------------------------------
// save
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SaveConfig {
    /// 空字符串表示使用系统「图片」目录下的 `Cuter`。
    pub dir: String,
    /// `png` / `jpg` / `webp` / `bmp`。
    pub format: String,
    /// JPEG 质量。
    pub quality: u8,
    /// 文件名模板，支持 `{date}` `{time}` `{datetime}` `{unix}` `{seq}` `{w}` `{h}`
/// `{monitor}`（显示器序号）`{app}`（按热键时前台程序名）。
    pub filename_pattern: String,
    /// 完成截图后自动复制到剪贴板。
    pub auto_copy: bool,
    /// 完成截图后自动保存（不弹保存框）。
    pub auto_save: bool,
    /// 截图后是否弹出保存路径选择框。
    pub ask_path_on_save: bool,
    /// 保存后在资源管理器中高亮该文件。
    pub reveal_after_save: bool,
}

impl Default for SaveConfig {
    fn default() -> Self {
        Self {
            dir: String::new(),
            format: "png".into(),
            quality: 92,
            filename_pattern: "Cuter_{datetime}".into(),
            auto_copy: true,
            auto_save: false,
            ask_path_on_save: false,
            reveal_after_save: false,
        }
    }
}

impl SaveConfig {
    /// 展开后的保存目录（自动创建）。
    ///
    /// 便携模式下默认落在 exe 旁边的 `Screenshots`：既然用户希望"什么都跟着程序走"，
    /// 就不该把截图悄悄写进系统「图片」目录。
    #[must_use]
    pub fn resolved_dir(&self) -> PathBuf {
        let raw = self.dir.trim();
        if raw.is_empty() {
            if Config::is_portable() {
                return Config::config_dir().join("Screenshots");
            }
            return dirs::picture_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("Cuter");
        }
        PathBuf::from(expand_env(raw))
    }

    /// 校验通过的图片格式，**并把 `quality` 应用到有损格式上**。
    ///
    /// 这里的 `with_quality` 不是装饰：`ImageFormat::from_extension` 只能给出
    /// 一个默认质量（92），若不在出口处统一覆盖，设置界面里的质量滑块就成了一枚
    /// 「改了没反应」的假开关。
    #[must_use]
    pub fn image_format(&self) -> crate::frame::ImageFormat {
        crate::frame::ImageFormat::from_extension(&self.format)
            .unwrap_or(crate::frame::ImageFormat::Png)
            .with_quality(self.quality)
    }
}

/// 展开 `%VAR%`（Windows）与 `~`（其它平台）前缀。
#[must_use]
pub fn expand_env(input: &str) -> String {
    let mut out = input.to_owned();
    if let Some(rest) = out.strip_prefix('~') {
        let home = dirs::home_dir().unwrap_or_default();
        out = format!("{}{}", home.display(), rest);
    }
    // 逐个替换 %VAR%
    while let Some(start) = out.find('%') {
        let Some(rel_end) = out[start + 1..].find('%') else {
            break;
        };
        let end = start + 1 + rel_end;
        let name = out[start + 1..end].to_owned();
        let value = std::env::var(&name).unwrap_or_default();
        out.replace_range(start..=end, &value);
        if value.is_empty() && name.is_empty() {
            break;
        }
    }
    out
}

/// 渲染文件名时用得上的上下文。
///
/// 单独一个结构体而不是继续加参数：占位符每多一个就多一个参数，
/// 位置参数多了以后调用点会变成一串看不懂的实参。
#[derive(Debug, Clone)]
pub struct NameContext {
    pub now: LocalTime,
    /// 本次进程内的第几张（1 开始）。
    pub seq: u64,
    pub size: crate::geom::Size,
    /// 截图所在显示器序号（1 开始；单独一个虚拟桌面的 GDI 后端下永远是 1）。
    pub monitor: u32,
    /// 按下热键时前台窗口的进程名（不含扩展名）；取不到就是空串。
    pub app: String,
}

impl NameContext {
    /// 只有时间与尺寸的最小上下文（无界面路径、测试用）。
    #[must_use]
    pub fn minimal(now: LocalTime, size: crate::geom::Size) -> Self {
        Self {
            now,
            seq: 0,
            size,
            monitor: 1,
            app: String::new(),
        }
    }
}

/// 按模板渲染文件名（不含扩展名）。
#[must_use]
pub fn render_filename(pattern: &str, context: &NameContext) -> String {
    let now = context.now;
    let size = context.size;
    let date = format!("{:04}{:02}{:02}", now.year, now.month, now.day);
    let time = format!("{:02}{:02}{:02}", now.hour, now.minute, now.second);
    let mut out = pattern
        .replace("{date}", &date)
        .replace("{time}", &time)
        .replace("{datetime}", &format!("{date}_{time}"))
        .replace("{unix}", &now.unix.to_string())
        .replace("{seq}", &context.seq.to_string())
        .replace("{w}", &size.w.to_string())
        .replace("{h}", &size.h.to_string())
        .replace("{monitor}", &context.monitor.to_string())
        .replace("{app}", &context.app);
    // 清理文件名非法字符与尾部空白/点
    out = out
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let trimmed = out.trim().trim_end_matches('.').to_owned();
    if trimmed.is_empty() {
        format!("Cuter_{date}_{time}")
    } else {
        trimmed
    }
}

/// 本地时间（避免引入时区依赖）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    /// Unix 时间戳（秒）。
    pub unix: u64,
}

impl LocalTime {
    /// 取当前本地时间。
    #[must_use]
    pub fn now() -> Self {
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::SYSTEMTIME;
            use windows::Win32::System::SystemInformation::GetLocalTime;
            let st: SYSTEMTIME = unsafe { GetLocalTime() };
            Self {
                year: st.wYear,
                month: st.wMonth,
                day: st.wDay,
                hour: st.wHour,
                minute: st.wMinute,
                second: st.wSecond,
                unix,
            }
        }
        #[cfg(not(windows))]
        {
            // 无系统 API 时的退化实现：用 UTC 日期（时间戳仍然正确）
            let days = unix / 86_400;
            let (y, m, d) = civil_from_days(days as i64);
            let secs = unix % 86_400;
            Self {
                year: y as u16,
                month: m as u16,
                day: d as u16,
                hour: (secs / 3600) as u16,
                minute: ((secs % 3600) / 60) as u16,
                second: (secs % 60) as u16,
                unix,
            }
        }
    }
}

/// Howard Hinnant 的 civil_from_days 算法（非 Windows 平台回退用）。
#[cfg(not(windows))]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------------
// pin（贴图）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PinConfig {
    /// 初始不透明度 0.1-1.0。
    pub opacity: f32,
    /// 鼠标穿透（点击穿过贴图）。
    pub click_through: bool,
    pub always_on_top: bool,
    /// 显示 1px 描边，便于区分贴图与背景。
    pub border: bool,
    pub shadow: bool,
    /// 图片本身也裁成圆角（阴影早就是圆角了，图片还是直角会露出来）。
    /// 半径按短边自适应：短边的 6%，夹在 2~24 像素之间。
    pub rounded_corners: bool,
    /// 滚轮缩放、中键还原。
    pub wheel_zoom: bool,
    /// 滚轮缩放时固定窗口大小（只放大内容）——同类工具里常见的做法。
    pub zoom_within_window: bool,
    pub initial_scale: f32,
    /// 贴图纹理总量预算（MB）。超过时**提醒**用户（不降质、也不拒绝新贴图）：
    /// 4K 截图一张的纹理就是 33 MB，十几张就是几百 MB，用户通常不知道自己占了这么多。
    pub texture_budget_mb: u32,
    /// 贴图分组数量（1-9）。分组是"几套互不干扰的贴图"：
    /// `Ctrl+F3` 切到下一组时，上一组的贴图会整体隐藏（不销毁）。
    pub groups: u32,
    /// 当前在第几组（0 开始）。**持久化**：下次启动新建的贴图落在同一组里。
    pub active_group: u32,
}

impl Default for PinConfig {
    fn default() -> Self {
        Self {
            opacity: 0.95,
            click_through: false,
            always_on_top: true,
            border: true,
            shadow: true,
            rounded_corners: true,
            wheel_zoom: true,
            zoom_within_window: false,
            initial_scale: 1.0,
            texture_budget_mb: 256,
            groups: 2,
            active_group: 0,
        }
    }
}

impl PinConfig {
    pub fn clamp(&mut self) {
        self.opacity = self.opacity.clamp(0.1, 1.0);
        self.initial_scale = self.initial_scale.clamp(0.05, 16.0);
        self.groups = self.groups.clamp(1, 9);
        // 下限 32 MB（再小一张 4K 就超了，提醒会变成噪音），上限 4 GB
        self.texture_budget_mb = self.texture_budget_mb.clamp(32, 4_096);
        // 组号必须落在组数之内，否则切组会切到一片空白
        self.active_group = self.active_group.min(self.groups - 1);
    }
}

// ---------------------------------------------------------------------------
// ocr
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrEngine {
    /// 关闭识别功能。
    Disabled,
    /// 内置离线引擎：Windows 上用系统自带的 `Windows.Media.Ocr`。
    ///
    /// 选它而不是打包 PP-OCR 模型，是因为本项目要求存储占用小——
    /// 系统 OCR 零下载、零打包、完全离线，模型由系统语言包提供。
    Builtin,
    /// 调用外部 HTTP 接口（自建 PaddleOCR 服务）：把框选的图 POST 到 [`OcrConfig::http_endpoint`]。
    Http,
}

impl OcrEngine {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "关闭",
            // 下拉框只放得下十来个字：这里只写名字，白话解释交给下面那句说明。
            // 以前这句是「内置离线（系统自带，零下载）」，直接把下拉框撑成整行宽。
            Self::Builtin => "内置离线",
            Self::Http => "在线接口",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OcrConfig {
    pub engine: OcrEngine,
    /// 语言代码，如 `ch` / `en` / `japan`。
    pub languages: Vec<String>,
    /// 截图完成后立即 OCR（「框选即识别」）。
    pub auto_after_capture: bool,
    /// 段落模式（按行合并，保留换行）。
    pub paragraph: bool,
    /// 低于该置信度的结果丢弃。
    pub min_score: f32,
    /// 在线接口地址。
    pub http_endpoint: String,
    /// 识别结果自动复制到剪贴板。
    pub auto_copy: bool,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            // 默认开启：系统自带的 OCR 零成本、离线可用，「开箱即用的框选取词」
            // 本来就是这类工具最核心的体验，不该让用户先去设置里找开关。
            // 系统没装 OCR 语言组件时会在设置界面给出明确提示，而不是静默失败。
            engine: OcrEngine::Builtin,
            // 留空表示"跟随系统偏好语言"，比写死 ch 更符合实际
            languages: Vec::new(),
            auto_after_capture: false,
            paragraph: true,
            min_score: 0.3,
            http_endpoint: "http://127.0.0.1:8866/ocr".into(),
            auto_copy: true,
        }
    }
}

impl OcrConfig {
    fn clamp(&mut self) {
        self.min_score = self.min_score.clamp(0.0, 1.0);
    }
}

/// 滚动截屏（长截图）参数。
///
/// 这里的每一项都在 `cuter-app` 的滚动采集线程里有真实作用点：
/// 没有实现的能力一律不给开关，避免"设置了却没反应"。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScrollConfig {
    /// 两次抓帧之间的间隔（毫秒）。太小会白烧 CPU，太大会漏掉快速滚动。
    pub interval_ms: u32,
    /// 拼接结果的高度上限，超过后停止追加（防止内存被无限长图吃满）。
    pub max_height: u32,
    /// 小于该位移视为"没有滚动"（像素）。
    pub min_offset: u32,
    /// 最大搜索位移（像素）。越大越能跟上大幅滚动，代价是耗时线性增长。
    pub max_offset: u32,
    /// 自动滚动：由程序自己合成滚轮事件，用户不用手滚。
    ///
    /// **默认关**：它会把光标移到选区中央再发滚轮，部分程序还会忽略合成滚轮。
    /// 面板上按 `A` 可以随时开关（这里只是新会话的初始值）。
    pub auto_scroll: bool,
    /// 自动滚动时，连续多少帧画面没有新增内容就停下（到底了 / 目标程序忽略了滚轮）。
    pub auto_stall_frames: u32,
}

impl Default for ScrollConfig {
    fn default() -> Self {
        let options = MatchOptions::default();
        Self {
            // 250ms 大约是"人眼感觉流畅"与"CPU 占用可忽略"的折中
            interval_ms: 250,
            // 20000 行 4 字节 RGBA 约 160 MB（按 1920 宽算），已经是可接受的上限
            max_height: 20_000,
            min_offset: options.min_offset,
            max_offset: options.max_offset,
            auto_scroll: false,
            auto_stall_frames: 3,
        }
    }
}

impl ScrollConfig {
    /// 夹取到合法范围。设置界面在用户拖动数值后会立刻调用它——否则把
    /// `max_offset` 拖到小于 `min_offset` 就会让匹配器永远搜不到位移。
    pub fn clamp(&mut self) {
        self.interval_ms = self.interval_ms.clamp(60, 5_000);
        self.max_height = self.max_height.clamp(200, 100_000);
        self.min_offset = self.min_offset.clamp(1, 64);
        self.max_offset = self.max_offset.clamp(self.min_offset + 1, 8_000);
    }

    /// 转成算法层参数。`sample_columns` 等调参项不进配置：它们对用户没有可理解的
    /// 语义，暴露出去只会变成"乱调一下就坏了"。
    #[must_use]
    pub fn match_options(&self) -> MatchOptions {
        MatchOptions {
            max_offset: self.max_offset,
            min_offset: self.min_offset,
            ..MatchOptions::default()
        }
    }
}

// ---------------------------------------------------------------------------
// history（截图历史）
// ---------------------------------------------------------------------------

/// 截图历史。
///
/// 历史**把原图存到磁盘**（`Config::data_dir()/history`），理由与代价写在
/// [`crate::history`] 的模块文档里。这里只有两个旋钮：
///
/// * `limit`：保留几张。`0` = 关闭历史（既不写盘也不显示入口）；
/// * `thumb_width`：面板里缩略图的最大宽度。它只影响"看起来多大"，
///   不影响"再复制一次"拿到的像素——那永远是原图。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    /// 保留最近多少张（0 = 关闭）。
    pub limit: usize,
    /// 缩略图最大宽度（像素）。
    pub thumb_width: u32,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            // 30 张、1920×1080 的 PNG 大约 10 MB 上下，是"有用"与"别吓到用户"的折中；
            // 面板里会把这个数报出来，觉得大就调小或清空。
            limit: 30,
            thumb_width: 240,
        }
    }
}

impl HistoryConfig {
    /// 夹取到合法范围。上限 500 是防手抖：500 张 4K PNG 已经是 GB 级。
    pub fn clamp(&mut self) {
        self.limit = self.limit.min(500);
        self.thumb_width = self.thumb_width.clamp(64, 1024);
    }

    /// 是否启用（`limit == 0` 就是关闭）。
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.limit > 0
    }

    /// 历史目录。**便携模式下跟着 exe 走**（`data_dir` 已经处理了这一点）。
    #[must_use]
    pub fn dir(&self) -> PathBuf {
        Config::data_dir().join("history")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Size;
    #[test]
    fn default_roundtrips_through_toml() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let cfg: Config = toml::from_str("[hotkeys]\ncapture = \"Ctrl+Alt+Q\"\n").unwrap();
        assert_eq!(cfg.hotkeys.capture, "Ctrl+Alt+Q");
        assert_eq!(cfg.hotkeys.ocr, HotkeyConfig::default().ocr);
        assert_eq!(cfg.annotate.stroke_width, 3.0);
    }

    #[test]
    fn sanitize_clamps_out_of_range_values() {
        let mut cfg = Config::default();
        cfg.annotate.stroke_width = 999.0;
        cfg.pin.opacity = 5.0;
        cfg.ocr.min_score = -1.0;
        cfg.save.filename_pattern = "  ".into();
        cfg.sanitize();
        assert_eq!(cfg.annotate.stroke_width, 64.0);
        assert_eq!(cfg.pin.opacity, 1.0);
        assert_eq!(cfg.ocr.min_score, 0.0);
        assert_eq!(cfg.save.filename_pattern, SaveConfig::default().filename_pattern);
    }

    #[test]
    fn filename_pattern_expands_placeholders() {
        let now = LocalTime {
            year: 2026,
            month: 3,
            day: 7,
            hour: 9,
            minute: 5,
            second: 4,
            unix: 1_772_000_000,
        };
        let context = NameContext {
            now,
            seq: 12,
            size: Size::new(800, 600),
            monitor: 2,
            app: "Code".to_owned(),
        };
        let name = render_filename("Cuter_{date}_{time}_{w}x{h}_{seq}", &context);
        assert_eq!(name, "Cuter_20260307_090504_800x600_12");
    }

    #[test]
    fn filename_pattern_strips_illegal_chars() {
        let now = LocalTime::now();
        let context = NameContext::minimal(now, Size::new(1, 1));
        let name = render_filename("a/b:c*d?", &context);
        assert!(!name.contains(['/', ':', '*', '?']));
    }

    /// 热键表必须与 `HotkeyCommand::ALL` 一一对应：两边数量不一致时，
    /// 设置界面里会少一项（用户根本看不到那个功能可以绑键），或者绑了不生效。
    /// 这里的 9 与 `cuter-app` 的 `HotkeyCommand::ALL` 是同一条约定。
    #[test]
    fn hotkey_entries_and_set_are_consistent() {
        let mut hk = HotkeyConfig::default();
        assert_eq!(hk.entries().len(), 9);
        hk.set("capture", "Ctrl+1".into());
        assert_eq!(hk.capture, "Ctrl+1");
        assert!(hk.entries().iter().any(|(_, v)| *v == "Ctrl+1"));
        // 后加的两项都要有默认键位，不然用户不知道该按什么。
        // **别改成 Shift+F3 / Ctrl+F3**：这两个组合常被常驻程序占着，用户机器上会注册失败。
        assert_eq!(hk.toggle_pins, "Alt+H");
        assert_eq!(hk.next_pin_group, "Alt+G");
        assert!(hk.entries().iter().any(|(k, _)| *k == "toggle_pins"));
        assert!(hk.entries().iter().any(|(k, _)| *k == "next_pin_group"));
    }

    /// 贴图分组：组数被夹住、当前组永远落在组数以内。
    #[test]
    fn pin_groups_are_clamped_consistently() {
        let mut pin = PinConfig {
            groups: 99,
            active_group: 50,
            ..PinConfig::default()
        };
        pin.clamp();
        assert_eq!(pin.groups, 9, "组数上限是 9");
        assert_eq!(pin.active_group, 8, "当前组不能被夹到组数之外");

        let mut single = PinConfig {
            groups: 1,
            active_group: 5,
            ..PinConfig::default()
        };
        single.clamp();
        assert_eq!(single.groups, 1);
        assert_eq!(single.active_group, 0, "只有一组时当前组必须是 0");

        // 手改成 0 组也要能起来（否则组号取模会除零）
        let mut zero = PinConfig {
            groups: 0,
            ..PinConfig::default()
        };
        zero.clamp();
        assert_eq!(zero.groups, 1);
    }

    #[test]
    fn expand_env_handles_missing_variables() {
        // 不存在的变量应被替换为空串而不是 panic
        let out = expand_env("%CUTER_DEFINITELY_MISSING_VAR%\\shots");
        assert!(out.ends_with("\\shots"));
        assert!(!out.contains('%'));
    }

    #[test]
    fn scroll_config_clamps_and_feeds_the_matcher() {
        let mut cfg = ScrollConfig {
            interval_ms: 0,
            max_height: 1,
            min_offset: 0,
            max_offset: 0,
            auto_scroll: false,
            auto_stall_frames: 0,
        };
        cfg.clamp();
        assert_eq!(cfg.interval_ms, 60);
        assert_eq!(cfg.max_height, 200);
        assert_eq!(cfg.min_offset, 1);
        // max_offset 必须严格大于 min_offset，否则匹配器搜不到任何位移
        assert!(cfg.max_offset > cfg.min_offset);

        let options = cfg.match_options();
        assert_eq!(options.min_offset, cfg.min_offset);
        assert_eq!(options.max_offset, cfg.max_offset);
        // 调参项不进配置，保持算法默认值
        assert_eq!(options.sample_columns, MatchOptions::default().sample_columns);
    }

    #[test]
    fn scroll_config_roundtrips_through_toml() {
        let mut cfg = Config::default();
        cfg.scroll.max_height = 12_345;
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(text.contains("12345"));
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.scroll, cfg.scroll);
    }

    #[test]
    fn save_format_falls_back_to_png() {
        let unknown = SaveConfig {
            format: "tiff".into(),
            ..SaveConfig::default()
        };
        assert_eq!(unknown.image_format(), crate::frame::ImageFormat::Png);

        let jpeg = SaveConfig {
            format: "jpg".into(),
            ..SaveConfig::default()
        };
        assert!(matches!(
            jpeg.image_format(),
            crate::frame::ImageFormat::Jpeg { .. }
        ));
    }

    /// `save.quality` 必须真的出现在导出的格式里，否则设置界面的滑块就是假开关。
    #[test]
    fn configured_quality_reaches_the_image_format() {
        let cfg = SaveConfig {
            format: "jpg".into(),
            quality: 61,
            ..SaveConfig::default()
        };
        assert!(matches!(
            cfg.image_format(),
            crate::frame::ImageFormat::Jpeg { quality: 61 }
        ));

        let png = SaveConfig {
            format: "png".into(),
            quality: 61,
            ..SaveConfig::default()
        };
        assert!(matches!(png.image_format(), crate::frame::ImageFormat::Png));
    }

    #[test]
    fn sanitize_clamps_jpeg_quality() {
        let mut cfg = Config::default();
        cfg.save.quality = 0;
        cfg.sanitize();
        assert_eq!(cfg.save.quality, 1);
        cfg.save.quality = 200;
        cfg.sanitize();
        assert_eq!(cfg.save.quality, 100);
    }

    /// 删除占位字段后，老配置文件里的未知键必须被安静忽略。    ///
    /// 这条测试守的是「升级不炸」——`docs/HANDOFF.md` 承诺过配置文件永不导致启动失败，
    /// 而删字段正好是唯一可能违反它的改动。
    /// 配置读坏时：回退默认值、原文件被改名成带时间戳的备份、并且有可展示的记录。
    #[test]
    fn a_broken_config_is_backed_up_and_reported() {
        let dir = std::env::temp_dir().join(format!("cuter-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("config.toml");
        std::fs::write(&path, "this is not = valid toml [").expect("write");

        let (config, recovery) = Config::load_from_with_recovery(&path);
        assert_eq!(config, Config::default(), "a broken file must fall back to defaults");

        let recovery = recovery.expect("the caller must be told");
        assert!(!recovery.error.is_empty(), "the error text is what we log");
        let backup = recovery.backup.expect("the broken file should be kept");
        assert!(backup.exists(), "backup missing: {}", backup.display());
        let name = backup.file_name().expect("name").to_string_lossy().into_owned();
        assert!(
            name.starts_with("config.toml.broken-"),
            "the backup must be timestamped so a second failure cannot overwrite the first: {name}"
        );
        assert!(!path.exists(), "the broken file should have been moved away");
        // 备份内容就是原来那份坏文件。
        let kept = std::fs::read_to_string(&backup).expect("read backup");
        assert!(kept.contains("not = valid toml"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_config_is_not_a_recovery() {
        let dir = std::env::temp_dir().join(format!("cuter-cfg-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("config.toml");

        let (config, recovery) = Config::load_from_with_recovery(&path);
        assert_eq!(config, Config::default());
        assert!(recovery.is_none(), "a first run is not a recovery: there was nothing to lose");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_general_keys_are_ignored() {
        let cfg: Config = toml::from_str(
            "[general]\ntheme = \"dark\"\nlanguage = \"en-US\"\ncheck_update = false\n\
             show_tray_notifications = false\nclose_to_tray = false\n",
        )
        .unwrap();
        assert_eq!(cfg.general.theme, ThemeKind::Dark);
        assert!(!cfg.general.close_to_tray);
        assert!(!cfg.general.autostart, "老配置里没有这个字段时必须是关闭");
    }

    /// 0.1.69：更新检查的节流与跳过名单。
    #[test]
    fn update_check_state_defaults_and_survives_old_configs() {
        let cfg = Config::default();
        assert_eq!(cfg.general.last_update_check, 0, "默认从没查过");
        assert_eq!(cfg.general.skipped_version, "", "默认没跳过任何版本");

        // 老配置里没有这两个字段：必须按默认值补齐，不能读失败
        let back: Config = toml::from_str(
            "[general]\ntheme = \"dark\"\ncheck_update = true\n\
             update_endpoint = \"https://example.com/v.json\"\n",
        )
        .unwrap();
        assert_eq!(back.general.last_update_check, 0);
        assert!(back.general.skipped_version.is_empty());
    }

    #[test]
    fn sanitize_clamps_the_update_check_stamp() {
        let mut cfg = Config::default();
        cfg.general.last_update_check = -5;
        cfg.general.skipped_version = "  1.2.3  ".to_owned();
        cfg.sanitize();
        assert_eq!(cfg.general.last_update_check, 0, "负数会被当成永远查过");
        assert_eq!(cfg.general.skipped_version, "1.2.3");
    }

    /// 保存必须是原子的：不许留下半个文件，也不许留下临时文件。
    #[test]
    fn save_is_atomic_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!(
            "cuter-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        let mut cfg = Config::default();
        cfg.save_to(&path).expect("第一次保存");
        cfg.save.quality = 61;
        cfg.save_to(&path).expect("第二次保存（覆盖）");

        let back = Config::load_from(&path).expect("读回来的内容必须是完整的 TOML");
        assert_eq!(back.save.quality, 61);

        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "不该留下临时文件：{leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 便携模式的判据是一个**标记文件**，不是"目录能不能写"。
    #[test]
    fn portable_mode_is_decided_by_the_marker_file() {
        let dir = std::env::temp_dir().join(format!(
            "cuter-portable-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(portable_root(&dir).is_none(), "没有标记文件就不是便携模式");

        std::fs::write(dir.join(PORTABLE_MARKER), b"").unwrap();
        assert_eq!(portable_root(&dir).as_deref(), Some(dir.as_path()));

        // 目录不存在时也不能 panic
        let missing = dir.join("nope");
        assert!(portable_root(&missing).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 默认保存目录必须是可预期的（便携模式下跟着 exe，否则进系统「图片」目录）。
    #[test]
    fn default_save_dir_is_predictable() {
        let dir = SaveConfig::default().resolved_dir();
        assert!(!dir.as_os_str().is_empty());
        assert!(
            dir.ends_with("Cuter") || dir.ends_with("Screenshots"),
            "默认目录应当是可预期的：{}",
            dir.display()
        );
    }

    /// 历史的旋钮：默认开着、上限被夹住、`limit = 0` 就是关闭。
    #[test]
    fn history_config_clamps_and_switches_off() {
        let mut cfg = HistoryConfig {
            limit: 100_000,
            thumb_width: 5,
        };
        cfg.clamp();
        assert_eq!(cfg.limit, 500);
        assert_eq!(cfg.thumb_width, 64);
        assert!(cfg.enabled());

        cfg.limit = 0;
        assert!(!cfg.enabled(), "limit = 0 表示关闭历史");

        let default = HistoryConfig::default();
        assert!(default.enabled(), "默认应当开着历史");
        assert!(default.dir().ends_with("history"));
    }

    /// 缓存目录必须在 **Local** 而不是 Roaming：漫游配置文件不该同步截图历史。
    #[test]
    fn data_dir_is_local_not_roaming() {
        let dir = Config::data_dir();
        assert!(
            dir.ends_with("Cuter"),
            "数据目录应当以 Cuter 结尾：{}",
            dir.display()
        );
        // 测试进程旁边没有便携标记，所以应当是系统目录而不是 exe 旁边
        assert!(
            !dir.starts_with(crate::config::exe_dir().unwrap_or_default()),
            "非便携模式下数据目录不该落在 exe 旁边：{}",
            dir.display()
        );
    }

    #[test]
    fn template_supports_monitor_and_app() {
        let context = NameContext {
            now: LocalTime {
                year: 2026,
                month: 10,
                day: 4,
                hour: 9,
                minute: 8,
                second: 7,
                unix: 0,
            },
            seq: 3,
            size: Size::new(800, 600),
            monitor: 2,
            app: "Code".to_owned(),
        };
        assert_eq!(
            render_filename("{app}_{monitor}_{seq}", &context),
            "Code_2_3"
        );
        // 取不到前台程序名时就是空串，不该留下 `{app}` 字面量
        let without = NameContext {
            app: String::new(),
            ..context.clone()
        };
        assert_eq!(render_filename("{app}-{w}", &without), "-800");
        // `{app}` 里的非法字符也要被清掉（进程名不会，但模板是用户写的）
        let odd = NameContext {
            app: "a/b:c".to_owned(),
            ..context
        };
        assert_eq!(render_filename("{app}", &odd), "a_b_c");
    }}

/// 一次"配置读坏了、已回退默认值"的记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    /// 原始错误（TOML 解析错误里通常带行列号）。
    pub error: String,
    /// 损坏文件被备份到哪（改名失败时为 `None`）。
    pub backup: Option<PathBuf>,
}

/// 进程内第一条回退记录：界面启动时读它，给用户一句解释。
static RECOVERY: std::sync::OnceLock<Recovery> = std::sync::OnceLock::new();

/// `config.toml` → `config.toml.broken-20261018-183000`。
///
/// 用 `SystemTime` 手写时间戳，不引入时间库（这个项目对依赖很抠）。
fn broken_backup_path(path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 只求"同一天里不撞名"，所以直接用秒数：`config.toml.broken-1760790000`
    let name = format!(
        "{}.broken-{stamp}",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "config.toml".to_owned())
    );
    path.with_file_name(name)
}
