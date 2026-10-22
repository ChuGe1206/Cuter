//! 日志：控制台 + 落盘（按大小滚动）。
//!
//! ## 为什么必须落盘
//!
//! 这是一个**常驻托盘、平时没有窗口**的程序：用户说"它没反应"时，控制台早就被
//! [`crate::detach_orphan_console`] 丢掉了，我们手里什么都没有。所以日志同时写两份：
//! 一份照旧给 stderr（`tools/probe/*.ps1` 会 grep 它以做断言），
//! 一份写进 `%APPDATA%\Cuter\logs\cuter.log`，用户可以直接发给我们。
//!
//! ## 设计要点
//!
//! * **格式完全一致**：文件那一份只是把 env_logger 的 writer 换掉，
//!   所以控制台与文件里的时间戳、级别、模块路径都一样，对照着看不需要翻译；
//! * **按大小滚动**，最多 [`KEEP_FILES`] 份，绝不无限增长；
//! * **任何一步失败都只降级**（退回"只有控制台"），绝不让程序因为写不了日志起不来；
//! * `File` 不做缓冲，每条日志都直接落盘——崩溃时最后一行必须还在。

use cuter_ui::{fill, tr};

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use cuter_core::config::Config;

/// 单个日志文件的上限；超过就在**启动时**滚动一次。
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
/// 保留的文件数：`cuter.log`、`cuter.log.1`、`cuter.log.2`。
const KEEP_FILES: u32 = 3;

/// 实际生效的日志文件路径（写不进去时为 `None`）。
static LOG_FILE: OnceLock<Option<PathBuf>> = OnceLock::new();
/// 是否允许在 panic 时弹窗（只在 GUI 路径下打开；CLI 路径靠 stderr）。
static PANIC_DIALOG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 日志目录：`%APPDATA%\Cuter\logs`。
#[must_use]
pub fn log_dir() -> PathBuf {
    Config::config_dir().join("logs")
}

/// 主日志文件。
#[must_use]
pub fn log_file_path() -> PathBuf {
    log_dir().join("cuter.log")
}

/// 初始化日志。返回实际生效的日志文件路径（写不进去时为 `None`）。
///
/// 幂等：重复调用只会重新设置 `log` 的全局 logger（第二次会失败并被忽略），
/// 这样可以安全地在测试里调用。
pub fn init() -> Option<PathBuf> {
    let filter = std::env::var("CUTER_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .unwrap_or_else(|_| "info".to_owned());

    let console = LoggerBuilder::new(&filter).build();

    let file = open_log_file();
    let path = file.as_ref().map(|(path, _)| path.clone());
    let combined: Box<dyn log::Log> = match file {
        Some((_, file)) => {
            let file_logger = LoggerBuilder::new(&filter).pipe(Box::new(file)).build();
            Box::new(MultiLogger {
                console: Box::new(console),
                file: Some(Box::new(file_logger)),
            })
        }
        None => Box::new(console),
    };

    // 过滤在 logger 的 `enabled()` 里真正生效；这里的 max_level 只是别让宏提前短路。
    if log::set_boxed_logger(combined).is_ok() {
        log::set_max_level(log::LevelFilter::Trace);
    }
    let _ = LOG_FILE.set(path.clone());
    path
}

/// panic 时是否弹一个提示框（GUI 路径下打开；CLI 路径下不要弹）。
pub fn enable_panic_dialog() {
    PANIC_DIALOG.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// 装一个 panic hook：把 panic 写进日志，GUI 模式下再弹一个明确的提示。
///
/// 没有它的话，一个常驻托盘的程序 panic 之后就是"闪一下消失"：用户既不知道发生了什么，
/// 我们也没有任何现场。**这是"出错有据可查"的底线。**
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let where_ = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "位置未知".to_owned());
        log::error!("程序内部错误（{where_}）：{info}");
        match log_file() {
            Some(path) => log::error!("详细日志：{}", path.display()),
            None => log::error!("日志没有落盘，上面的内容就是全部现场"),
        }
        log::logger().flush();

        if PANIC_DIALOG.load(std::sync::atomic::Ordering::Relaxed) {
            let tail = match log_file() {
                Some(path) => fill!(tr!("\n\n详细日志已经写到：\n{}"), path.display()),
                None => String::new(),
            };
            show_error_dialog(&fill!(tr!("Cuter 遇到了内部错误，需要重启。\n\n{}{}"), info, tail));
        }
        previous(info);
    }));
}

/// 日志文件路径（`init` 之后可用）。
#[must_use]
pub fn log_file() -> Option<PathBuf> {
    LOG_FILE.get().cloned().flatten()
}

// ---------------------------------------------------------------------------
// 内部实现
// ---------------------------------------------------------------------------

/// 一层薄封装，免得上面重复写一遍 env_logger 的构建链。
struct LoggerBuilder {
    builder: env_logger::Builder,
}

impl LoggerBuilder {
    fn new(filter: &str) -> Self {
        let mut builder = env_logger::Builder::new();
        builder.parse_filters(filter);
        builder.format_timestamp_millis();
        // 行首带进程号：日志是**追加**写的，同一个文件里可能落进相邻两次启动的内容
        // （便携模式下更是如此）。没有 pid 时，"同一句话出现两遍"分不清是"同一条被写了
        // 两次"还是"两个进程各写了一次"——见 HANDOFF 第 88 节。
        //
        // `CUTER_LOG_SEQ=1` 时再加 `seq=`（每次**格式化**自增）和 `tid=`（线程号）：
        // 排查"同一句写两遍"时，两条的 seq 相差 1 说明是两次格式化（同一记录走两个
        // 输出端，或两条记录），相差 2 说明中间那条（控制台那份）也落进了同一个文件。
        let seq_mode = std::env::var_os("CUTER_LOG_SEQ").is_some();
        builder.format(move |buf, record| {
            use std::io::Write as _;
            static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            if seq_mode {
                let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                writeln!(
                    buf,
                    "[{} {:>5} {} pid={} seq={} tid={:?}] {}",
                    buf.timestamp_millis(),
                    record.level(),
                    record.target(),
                    std::process::id(),
                    seq,
                    std::thread::current().id(),
                    record.args()
                )
            } else {
                writeln!(
                    buf,
                    "[{} {:>5} {} pid={}] {}",
                    buf.timestamp_millis(),
                    record.level(),
                    record.target(),
                    std::process::id(),
                    record.args()
                )
            }
        });
        builder.target(env_logger::Target::Stderr);
        Self { builder }
    }

    fn pipe(mut self, writer: Box<dyn std::io::Write + Send + 'static>) -> Self {
        self.builder.target(env_logger::Target::Pipe(writer));
        self
    }

    fn build(mut self) -> env_logger::Logger {
        self.builder.build()
    }
}

/// 同时往两个 logger 里写（控制台 + 文件）。
struct MultiLogger {
    console: Box<dyn log::Log>,
    file: Option<Box<dyn log::Log>>,
}

impl log::Log for MultiLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.console.enabled(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.console.enabled(record.metadata()) {
            return;
        }
        self.console.log(record);
        if let Some(file) = &self.file {
            file.log(record);
        }
    }

    fn flush(&self) {
        self.console.flush();
        if let Some(file) = &self.file {
            file.flush();
        }
    }
}

/// 打开（必要时先滚动）日志文件。
fn open_log_file() -> Option<(PathBuf, File)> {
    let dir = log_dir();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("无法创建日志目录 {}：{err}", dir.display());
        return None;
    }
    let path = log_file_path();
    if let Err(err) = rotate_in(&dir, MAX_LOG_BYTES, KEEP_FILES) {
        // 滚动失败不致命：大不了这次继续往同一个文件里追加。
        eprintln!("日志滚动失败（{err}），继续追加到 {} ", path.display());
    }
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => Some((path, file)),
        Err(err) => {
            eprintln!("无法写入日志文件 {}：{err}", path.display());
            None
        }
    }
}

/// 超过 `max_bytes` 就把 `cuter.log` 依次后移，最多保留 `keep` 份。
///
/// 单独抽出来是为了能无头测试（用临时目录 + 很小的阈值）。
fn rotate_in(dir: &Path, max_bytes: u64, keep: u32) -> std::io::Result<()> {
    let base = dir.join("cuter.log");
    let size = std::fs::metadata(&base).map(|m| m.len()).unwrap_or(0);
    if size < max_bytes || keep == 0 {
        return Ok(());
    }
    // 最老的一份直接删：`keep` 份包含 `cuter.log` 本身。
    let _ = std::fs::remove_file(dir.join(format!("cuter.log.{}", keep - 1)));
    for index in (1..keep.saturating_sub(1)).rev() {
        let from = dir.join(format!("cuter.log.{index}"));
        if from.exists() {
            let _ = std::fs::rename(&from, dir.join(format!("cuter.log.{}", index + 1)));
        }
    }
    std::fs::rename(&base, dir.join("cuter.log.1"))
}

#[cfg(windows)]
fn show_error_dialog(message: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::HSTRING;

    let text = HSTRING::from(message);
    let caption = HSTRING::from("Cuter 内部错误");
    // SAFETY: 只是弹一个模态提示框；两个字符串在本调用期间都活着。
    unsafe {
        MessageBoxW(None, &text, &caption, MB_OK | MB_ICONERROR);
    }
}

#[cfg(not(windows))]
fn show_error_dialog(message: &str) {
    eprintln!("{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cuter-log-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        dir
    }

    #[test]
    fn rotate_is_a_noop_below_the_threshold() {
        let dir = temp_dir("small");
        std::fs::write(dir.join("cuter.log"), b"tiny").unwrap();
        rotate_in(&dir, 1024, 3).unwrap();
        assert!(dir.join("cuter.log").exists());
        assert!(!dir.join("cuter.log.1").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotate_shifts_files_and_drops_the_oldest() {
        let dir = temp_dir("shift");
        std::fs::write(dir.join("cuter.log"), vec![b'a'; 64]).unwrap();
        std::fs::write(dir.join("cuter.log.1"), b"one").unwrap();
        std::fs::write(dir.join("cuter.log.2"), b"two").unwrap();

        // keep = 3 ⇒ 允许 cuter.log / .1 / .2；最老的 .2 会被删掉，.1 变成 .2。
        rotate_in(&dir, 16, 3).unwrap();

        assert!(!dir.join("cuter.log").exists(), "主文件应当被移走");
        assert_eq!(std::fs::read(dir.join("cuter.log.1")).unwrap(), vec![b'a'; 64]);
        assert_eq!(std::fs::read(dir.join("cuter.log.2")).unwrap(), b"one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotate_creates_the_first_backup_even_without_history() {
        let dir = temp_dir("fresh");
        std::fs::write(dir.join("cuter.log"), vec![b'x'; 32]).unwrap();
        rotate_in(&dir, 8, 3).unwrap();
        assert!(dir.join("cuter.log.1").exists());
        assert!(!dir.join("cuter.log.2").exists(), "没有历史时不该凭空造出 .2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_file_path_lives_next_to_the_config() {
        assert_eq!(log_dir(), Config::config_dir().join("logs"));
        assert_eq!(log_file_path().file_name().unwrap(), "cuter.log");
    }

    /// hook 只负责"留现场"，绝不能把 panic 吃掉（否则 `catch_unwind`、
    /// 测试框架、以及 `panic = "abort"` 之外的所有恢复逻辑都会失效）。
    #[test]
    fn panic_hook_does_not_swallow_panics() {
        install_panic_hook();
        let caught = std::panic::catch_unwind(|| panic!("故意 panic：验证 hook 不吞异常"));
        assert!(caught.is_err(), "panic 必须继续传播");
    }
}
