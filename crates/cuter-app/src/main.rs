//! Cuter 命令行入口。
//!
//! 除了托盘常驻模式，还提供一组**无界面**子命令：
//!
//! ```text
//! cuter                          启动托盘 + 设置窗口
//! cuter --capture                启动并立刻进入框选
//! cuter --full [--out a.png]     直接截取整个虚拟桌面
//! cuter --monitor [--out a.png]  截取鼠标所在显示器
//! cuter --region 100,100,800,600 截取指定区域
//! cuter --list-monitors          打印显示器信息
//! cuter --copy                   把结果放进剪贴板
//! ```
//!
//! 无界面子命令不只是「顺带的功能」：它是**唯一能在没有交互桌面的环境里验证捕获
//! 链路是否正确**的手段，因此所有捕获相关的能力都从这里暴露一份。

mod app;
mod desktop;
mod icon;
mod logging;
mod save_dialog;
mod single_instance;



#[macro_use]
extern crate cuter_ui;

use cuter_ui::tr;
use std::process::ExitCode;

use cuter_core::capture::desktop_capture;
use cuter_core::config::Config;
use cuter_core::geom::Rect;

use crate::app::{CaptureRequest, CuterApp, LaunchOptions};
use crate::single_instance::{Acquire, Activation, Guard};

/// 解析后的命令行参数。
#[derive(Debug, Default)]
struct Cli {
    help: bool,
    version: bool,
    capture: bool,
    /// 启动后只驻留托盘（开机自启用的就是它：不能让开机蹦出一个设置窗口）。
    tray: bool,
    full: bool,
    monitor: bool,
    list_monitors: bool,
    region: Option<String>,
    out: Option<String>,
    copy: bool,
    pin: bool,
    /// 启动后打开截图历史面板（`cuter --history`）。
    history: bool,
    /// 启动后打开自绘的托盘菜单面板（`cuter --tray-menu`）。
    ///
    /// 这个菜单平时只能右键托盘图标打开，而托盘图标在通知区域、合成点击很不可靠。
    /// 这条入口让探针能直接核对它的版面，自己也不用去点通知区域。
    tray_menu: bool,
    /// 启动后直接打开「设置 → 关于」页（`cuter --about`）。
    ///
    /// 和 `--tray-menu` 同一个理由：更新检查那一栏就在这一页，而"先打开设置、再点左边
    /// 导航里的第十二项"这种合成点击不可靠；有了它，探针能确定性地核对这一页的版面。
    about: bool,
    /// 转交给已在跑的实例：隐藏 / 显示当前组的贴图（`cuter --toggle-pins`）。
    ///
    /// 与 `--capture` 同一条路（单实例转交），所以可以绑到别的工具的快捷键上，
    /// 也**不依赖键盘注入**——探针就是靠它做确定性验证的。
    toggle_pins: bool,
    /// 转交给已在跑的实例：切到下一个贴图分组（`cuter --next-pin-group`）。
    next_pin_group: bool,
    /// 转交给已在跑的实例：取消所有贴图的鼠标穿透（`cuter --clear-pin-click-through`）。
    ///
    /// 和托盘菜单里那一项走的是**同一个处理函数**。开穿透的贴图收不到任何鼠标消息，
    /// 自己既不能右键也不能拖，只能靠窗口外的出口把它救回来；除了托盘菜单，这条命令行
    /// 出口让"救回来"这件事可以被脚本/探针确定性地验证。
    clear_pin_click_through: bool,
    /// 隐藏的自检开关：故意 panic，用来验证"崩溃现场落盘 + panic hook"这条链路。
    self_test_panic: bool,
    /// 无界面地打开/关闭开机自启（安装器与卸载器调用的就是它）。
    ///
    /// 为什么不直接写注册表就完事：自启是"配置 + 注册表"两处状态，必须由**同一个作用点**
    /// （`desktop::reconcile_autostart`）来写；否则装完第一次启动，启动时的对账就会
    /// 按配置里的旧值把它改回去。
    set_autostart: Option<bool>,
}

impl Cli {
    /// 是否走无界面路径。
    fn headless(&self) -> bool {
        self.full || self.monitor || self.region.is_some() || self.list_monitors
    }
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut cli = Cli::default();
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => cli.help = true,
            "-V" | "--version" => cli.version = true,
            "-c" | "--capture" => cli.capture = true,
            "--tray" => cli.tray = true,
            "--full" | "--desktop" => cli.full = true,
            "-m" | "--monitor" => cli.monitor = true,
            "--list-monitors" => cli.list_monitors = true,
            "--copy" => cli.copy = true,
            "-p" | "--pin" => cli.pin = true,
            "--history" => cli.history = true,
            "--tray-menu" => cli.tray_menu = true,
            "--about" => cli.about = true,
            "--toggle-pins" => cli.toggle_pins = true,
            "--next-pin-group" => cli.next_pin_group = true,
            "--clear-pin-click-through" => cli.clear_pin_click_through = true,
            // 安装器/卸载器用：无界面地改开机自启（写配置 + 注册表），然后自己退出。
            "--enable-autostart" => cli.set_autostart = Some(true),
            "--disable-autostart" => cli.set_autostart = Some(false),
            // 只给开发者/CI 用：验证 panic 也会留下现场。不出现在 --help 里。
            "--self-test-panic" => cli.self_test_panic = true,
            "-o" | "--out" => {
                cli.out = Some(
                    args.next()
                        .ok_or_else(|| fill!(tr!("{} 缺少参数：需要给出输出文件路径"), arg))?,
                );
            }
            "-r" | "--region" => {
                cli.region = Some(
                    args.next()
                        .ok_or_else(|| fill!(tr!("{} 缺少参数：需要 x,y,w,h"), arg))?,
                );
            }
            other if other.starts_with("--out=") => {
                cli.out = Some(other["--out=".len()..].to_owned());
            }
            other if other.starts_with("--region=") => {
                cli.region = Some(other["--region=".len()..].to_owned());
            }
            other => return Err(fill!(tr!("未知参数：{}（用 --help 查看用法）"), other)),
        }
    }

    let modes = [cli.full, cli.monitor, cli.region.is_some(), cli.capture]
        .into_iter()
        .filter(|chosen| *chosen)
        .count();
    if modes > 1 {
        return Err("--full / --monitor / --region / --capture 只能选一个".to_owned());
    }
    Ok(cli)
}

fn print_help() {
    println!(
        "Cuter {} —— 纯 Rust 的截图 / 标注 / 贴图工具

用法：
  cuter [选项]

启动方式：
  -c, --capture            启动后立即进入框选截图
      --tray               启动后只驻留托盘，不显示窗口（开机自启用的就是它）
      --full               截取整个虚拟桌面（无界面）
  -m, --monitor            截取鼠标所在显示器（无界面）
  -r, --region x,y,w,h     截取指定区域（无界面，物理像素）
      --list-monitors      打印显示器列表后退出

输出：
  -o, --out <路径>         保存到指定文件（扩展名决定格式：png/jpg/webp/bmp）
      --copy               把结果复制到系统剪贴板

贴图：
  -p, --pin                启动后把剪贴板里的图片贴到鼠标位置

历史：
      --history            打开截图历史面板（最近几张，可再复制 / 再贴图）

托盘菜单：
      --tray-menu          直接打开托盘右键菜单（平时要右键通知区域里的托盘图标）

设置窗口：
      --about              直接打开「设置 → 关于」页（更新检查就在这一页）

贴图组（转交给已在跑的实例，可绑到别的工具的快捷键上）：
      --toggle-pins        隐藏 / 显示当前组的贴图（同一组内的贴图一起动）
      --clear-pin-click-through
                           取消所有贴图的鼠标穿透（开了穿透的贴图收不到鼠标消息，
                           这是托盘菜单之外的第二个出口，方便脚本救回来）
      --next-pin-group     切到下一个贴图分组

其它：
  -h, --help               显示本帮助
  -V, --version            显示版本
      --enable-autostart   无界面地打开开机自启（安装器用；改配置与注册表后退出）
      --disable-autostart  无界面地关闭开机自启（卸载器用）

示例：
  cuter --full --out shot.png
  cuter --region 0,0,1920,1080 --copy
  cuter --capture
  cuter --pin              （先把图片复制到剪贴板）

配置文件：{}
",
        cuter_core::VERSION,
        Config::path().display()
    );
}

/// 解析 `x,y,w,h`。
fn parse_region(text: &str) -> Result<Rect, String> {
    let parts: Vec<&str> = text.split([',', 'x', 'X']).collect();
    if parts.len() != 4 {
        return Err(fill!(tr!("区域格式应为 x,y,w,h，实际为「{}」"), text));
    }
    let mut values = [0i32; 4];
    for (index, part) in parts.iter().enumerate() {
        values[index] = part
            .trim()
            .parse::<i32>()
            .map_err(|_| fill!(tr!("区域里的「{}」不是整数"), part))?;
    }
    let (x, y, w, h) = (values[0], values[1], values[2], values[3]);
    if w <= 0 || h <= 0 {
        return Err(fill!(tr!("区域的宽高必须为正数，实际为 {}×{}"), w, h));
    }
    Ok(Rect::new(x, y, w as u32, h as u32))
}

/// 把「只属于我们自己的」控制台窗口丢掉。
///
/// 程序刻意保持**控制台子系统**，因为 `cuter.exe --full --out x.png` 这类无界面
/// 用法必须满足三件事：cmd / PowerShell **会等它跑完**、stdout 能被重定向、退出码
/// 能传出去（`tools/probe/*.ps1` 整条链路都依赖这些）。GUI 子系统做不到前两条。
///
/// 代价是：双击（或 `Start-Process`、计划任务）启动时，Windows 会**先**给进程建一个
/// 独立的控制台窗口再让它开始跑 —— 用户看到的是一个黑框一闪，像没做完的命令行工具。
///
/// 判据是 `GetConsoleProcessList` 返回的**共享该控制台的进程数**：
///
/// * `== 1`：这个控制台是专门给我们开的（双击 / 新进程组），立刻 `FreeConsole` 丢掉；
/// * `>= 2`：它是 cmd / PowerShell 的控制台，**必须保留**，否则 CLI 输出会消失。
///
/// 返回 0（调用失败）时什么都不做：宁可留一个控制台窗口，也不能把输出弄丢。
#[cfg(windows)]
fn detach_orphan_console() {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{
        CONSOLE_MODE, FreeConsole, GetConsoleMode, GetConsoleProcessList, GetStdHandle,
        STD_ERROR_HANDLE, SetStdHandle,
    };

    let mut owners = [0u32; 4];
    // SAFETY: `owners` 是长度为 4 的可写数组，函数最多写入 4 个元素并返回
    // 「共享此控制台的进程总数」；进程数超过 4 时返回值大于 4，我们据此走保留分支。
    let count = unsafe { GetConsoleProcessList(&mut owners) };
    if count != 1 {
        return;
    }

    // 先看清 stderr 到底指向哪儿。重定向到文件或管道时（探针、脚本都这么启动我们），
    // 那个句柄不是控制台的，`FreeConsole` 也不会关它——此时把槽清掉，只会把
    // 调用方要读的那份 stderr 弄丢。`GetConsoleMode` 成功即说明这是控制台句柄。
    // SAFETY: 两个调用都只读句柄，失败时返回错误，不影响内存安全。
    let stderr_on_console = unsafe {
        let mut mode = CONSOLE_MODE::default();
        match GetStdHandle(STD_ERROR_HANDLE) {
            Ok(handle) => !handle.is_invalid() && GetConsoleMode(handle, &mut mode).is_ok(),
            Err(_) => false,
        }
    };

    // SAFETY: 无参数、无前置条件；即便失败也只是留下一个多余的控制台窗口。
    let _ = unsafe { FreeConsole() };

    if !stderr_on_console {
        return;
    }
    // `FreeConsole` 关掉了三个标准句柄，但**进程环境块里存的还是那三个数值**。
    // 之后任何 `CreateFile` 都可能拿到同一个数值（Windows 分配句柄时优先用最小的空槽），
    // 于是「写 stderr」会写进那个文件——日志里"同一句话连着出现两遍"就是它：
    // 一份来自控制台日志器、一份来自文件日志器，两份都落进了 cuter.log。
    //
    // 只清 stderr：那是日志唯一的控制台出口。stdout/stdin 故意留着不动——
    // `println!` 在写失败时会 panic，清掉 stdout 会让 `cuter --version` 这类
    // 命令行路径在一个"独占控制台"的父进程下直接崩掉，而它们本来就不该受影响。
    let none = HANDLE::default();
    // SAFETY: `SetStdHandle` 只写进程环境块里的一个数值。
    unsafe {
        let _ = SetStdHandle(STD_ERROR_HANDLE, none);
    }
}

#[cfg(not(windows))]
fn detach_orphan_console() {}

#[cfg(windows)]
fn setup_dpi_awareness() {
    cuter_core::capture::ensure_dpi_awareness();
}

#[cfg(not(windows))]
fn setup_dpi_awareness() {}

/// 把控制台输入输出切到 UTF-8。
///
/// Windows 控制台默认代码页是 GBK(936)，而 Rust 写出去的日志与 `println!` 都是
/// UTF-8 字节，于是所有中文都会变成「鎸夊叧閿瓧」这样的乱码。这个坑只有在真正
/// 往控制台打印中文时才会暴露（重定向到文件时反而是对的），所以必须显式切换。
#[cfg(windows)]
fn setup_console_utf8() {
    use windows::Win32::System::Console::{SetConsoleCP, SetConsoleOutputCP};
    const CP_UTF8: u32 = 65001;
    // SAFETY: 两个调用只接受一个 u32 代码页编号，失败时返回错误，不影响内存安全。
    unsafe {
        let _ = SetConsoleOutputCP(CP_UTF8);
        let _ = SetConsoleCP(CP_UTF8);
    }
}

#[cfg(not(windows))]
fn setup_console_utf8() {}

fn init_logging() {
    // 控制台 + 落盘（`%APPDATA%\Cuter\logs\cuter.log`，按大小滚动），见 `logging` 模块。
    // 文档（HANDOFF 6.6）里让用户用 `CUTER_LOG=debug`，两个变量都认（在 `logging::init` 里）。
    logging::init();
    // panic 也要有据可查：写日志 + （GUI 模式下）弹一个说明框。
    logging::install_panic_hook();
}

/// 无界面路径上的捕获后端：**跟随 `capture.backend` 配置**，拿不到就回退 GDI。
///
/// 与 GUI 的差别：无界面是"一次性的"，没有后台预热的机会，所以这里直接建。
/// DXGI 建一次要上秒（实测 1.76 s），因此无界面路径**默认仍然用 GDI**
/// （配置里写 dxgi 才会付出这个代价）——脚本化截图通常更在意"别卡半天"。
fn headless_backend() -> Result<Box<dyn cuter_core::capture::ScreenCapture>, String> {
    let config = Config::load();
    if config.capture.backend == cuter_core::capture::CaptureBackend::Gdi {
        return desktop_capture().map_err(|e| fill!(tr!("捕获后端不可用：{}"), e));
    }
    let start = std::time::Instant::now();
    match cuter_core::capture::desktop_capture_with(config.capture.backend) {
        Ok(backend) => {
            log::info!(
                "无界面捕获使用 {}（建立耗时 {} ms）",
                backend.backend_name(),
                start.elapsed().as_millis()
            );
            Ok(backend)
        }
        Err(err) => {
            log::warn!("{} 后端不可用（{err}），回退 GDI", config.capture.backend.label());
            desktop_capture().map_err(|e| fill!(tr!("捕获后端不可用：{}"), e))
        }
    }
}

fn print_monitors() -> Result<(), String> {
    let backend = desktop_capture().map_err(|e| fill!(tr!("捕获后端不可用：{}"), e))?;
    println!("捕获后端：{}", backend.backend_name());
    let monitors = backend.monitors().map_err(|e| fill!(tr!("枚举显示器失败：{}"), e))?;
    for monitor in &monitors {
        println!(
            "  {} 位置=({},{}) 物理={}×{} 逻辑={:.0}×{:.0} 缩放={:.0}%{}",
            monitor.name,
            monitor.bounds.x,
            monitor.bounds.y,
            monitor.bounds.w,
            monitor.bounds.h,
            monitor.logical_size().0,
            monitor.logical_size().1,
            monitor.scale_factor * 100.0,
            if monitor.is_primary { "  [主屏]" } else { "" }
        );
    }
    Ok(())
}

/// 无界面捕获。
fn run_headless(cli: &Cli) -> Result<(), String> {
    if cli.list_monitors {
        return print_monitors();
    }

    let backend = headless_backend()?;

    let frame = if let Some(region) = &cli.region {
        let region = parse_region(region)?;
        backend
            .capture_region(region)
            .map_err(|e| fill!(tr!("捕获区域失败：{}"), e))?
    } else if cli.monitor {
        let shot = backend
            .capture_desktop()
            .map_err(|e| fill!(tr!("捕获屏幕失败：{}"), e))?;
        let cursor = shot.cursor.unwrap_or(shot.bounds.origin());
        let monitor = shot
            .monitor_at(cursor)
            .ok_or_else(|| "找不到鼠标所在的显示器".to_owned())?
            .clone();
        shot.crop_global(monitor.bounds)
    } else {
        backend
            .capture_desktop()
            .map_err(|e| fill!(tr!("捕获屏幕失败：{}"), e))?
            .frame
    };

    println!("捕获成功：{} × {} 像素", frame.size().w, frame.size().h);

    if cli.copy {
        desktop::copy_image(&frame)?;
        println!("已复制到剪贴板");
    }

    let config = Config::load();
    let path = match &cli.out {
        Some(path) => std::path::PathBuf::from(path),
        None => {
            let dir = config.save.resolved_dir();
            std::fs::create_dir_all(&dir).map_err(|e| fill!(tr!("创建目录失败：{}"), e))?;
            // 无界面路径没有"前台程序"这个概念，用最小上下文（`{app}` 会是空串）。
            let stem = cuter_core::config::render_filename(
                &config.save.filename_pattern,
                &cuter_core::config::NameContext::minimal(
                    cuter_core::config::LocalTime::now(),
                    frame.size(),
                ),
            );
            dir.join(format!("{stem}.{}", config.save.image_format().extension()))
        }
    };

    // 与界面里的保存共用同一条规则：**扩展名优先，质量来自配置**。
    // 直接用 `Frame::save` 会从扩展名反推一个内置质量，把 `save.quality` 吃掉。
    let format = cuter_core::frame::ImageFormat::from_path(&path)
        .unwrap_or_else(|| config.save.image_format())
        .with_quality(config.save.quality);
    frame
        .save_as(&path, format)
        .map_err(|e| fill!(tr!("保存失败：{}"), e))?;
    println!("已保存：{}", path.display());
    Ok(())
}

fn run_gui(cli: Cli) -> ExitCode {
    let config = Config::load();

    // GUI 路径下 panic 除了写日志，还要给用户一个能看懂的提示框
    // （托盘程序 panic 之后是"闪一下消失"，用户完全不知道发生了什么）。
    logging::enable_panic_dialog();

    // 单实例守卫必须在**创建窗口之前**拿到：全局热键是进程独占的，
    // 第二个实例注册不上它，用户看到的是「按了热键没反应」。
    // 无界面子命令不走这里（它们本来就允许并行），所以守卫只覆盖 GUI 路径。
    let instance = match Guard::acquire() {
        Acquire::First(guard) => Some(guard),
        Acquire::AlreadyRunning => {
            // 把「用户又启动了一次」这件事转交给已经在跑的实例，然后自己退出。
            let request = if cli.capture {
                Activation::Capture
            } else if cli.pin {
                Activation::Pin
            } else if cli.history {
                Activation::History
            } else if cli.tray_menu {
                Activation::TrayMenu
            } else if cli.toggle_pins {
                Activation::TogglePins
            } else if cli.next_pin_group {
                Activation::NextPinGroup
            } else if cli.clear_pin_click_through {
                Activation::ClearPinClickThrough
            } else {
                Activation::Show
            };
            if Guard::forward(request) {
                println!("Cuter 已经在运行（托盘里），已把这次启动转交给它。");
                return ExitCode::SUCCESS;
            }
            // 转交失败只有一种常见原因：那个实例正在退出。如实报告，不假装成功。
            eprintln!("Cuter 已经在运行，但没能把这次启动转交给它，请稍后再试。");
            return ExitCode::FAILURE;
        }
        Acquire::Unavailable(err) => {
            // 宁可多开一个实例，也不能因为创建不了内核对象就不让用户用
            log::warn!("单实例守卫不可用（{err}），本次启动没有重复实例保护");
            None
        }
    };

    // `--clear-pin-click-through` 的"本进程就是活着的那个实例"分支：
    // 没有活着的贴图要救，但配置里那一格必须清掉——否则下次贴图一打开又是
    // "收不到任何鼠标消息"的状态（那条出口就失去意义了）。
    if instance.is_some() && cli.clear_pin_click_through {
        let mut cfg = Config::load();
        if cfg.pin.click_through {
            cfg.pin.click_through = false;
            if let Err(err) = cfg.save() {
                log::warn!("取消贴图鼠标穿透时保存配置失败：{err}");
            }
        }
    }

    log::info!(
        "启动时间线[单实例守卫就绪]：+{} ms",
        desktop::since_process_start_ms()
    );

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Cuter")
            .with_app_id("cuter")
            // 自绘标题栏替掉系统装饰：系统那条白条和深色界面完全是两套东西（用户截图指出）。
            // 代价是窗口自己也失去了系统缩放边框，所以在 `app.rs` 里用
            // `ViewportCommand::BeginResize` 接管了八向缩放。
            .with_decorations(false)
            // 高度多给 36px，客户区高度与加标题栏之前保持一致
            .with_inner_size([780.0, 596.0])
            .with_min_inner_size([560.0, 416.0])
            .with_icon(icon::window_icon())
            .with_visible(
                !config.general.start_in_tray
                    && !cli.capture
                    && !cli.tray
                    && !cli.history
                    && !cli.tray_menu
                    && !cli.about,
            ),
        renderer: eframe::Renderer::Glow,
        // 试过 `vsync: false`（想压"按下热键到看见遮罩"的延迟），但 eframe 0.36 的
        // `NativeOptions` 已经没有这个字段了；实测 eframe→第一帧 稳定在 ~140 ms，
        // 那一段是 winit + GL 上下文创建，不是交换缓冲的等待（见 HANDOFF 第 34 节）。
        ..Default::default()
    };

    // 守卫随 `LaunchOptions` 进入 `CuterApp`，因此**活得和窗口一样久**；
    // 进程退出时随 app 一起析构，互斥体自动释放。
    let launch = LaunchOptions {
        start_request: cli.capture.then_some(CaptureRequest::Region),
        start_pin: cli.pin,
        start_history: cli.history,
        start_tray_menu: cli.tray_menu,
        start_about: cli.about,
        // 托盘驻留模式、`--capture` 直接进框选、`--tray`、`--history`、`--tray-menu`：
        // 主窗口都不该出现在桌面上
        start_hidden: config.general.start_in_tray
            || cli.capture
            || cli.tray
            || cli.history
            || cli.tray_menu
            || cli.about,
        instance,
    };

    log::info!(
        "启动时间线[交给 eframe（含窗口与 GL 上下文）]: +{} ms",
        desktop::since_process_start_ms()
    );
    let result = eframe::run_native(
        "Cuter",
        options,
        Box::new(move |cc| Ok(Box::new(CuterApp::new_with(cc, launch)?))),
    );

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("界面启动失败：{err}");
            ExitCode::FAILURE
        }
    }
}

/// 无界面地打开/关闭开机自启（安装器与卸载器调用）。
///
/// 顺序很重要：**先写配置、再写注册表**。如果反过来，中途失败会留下
/// "注册表开着、配置关着"的状态，而下一次启动的对账会把它改回配置说的那个值——
/// 用户会觉得"装了但自启没生效"。
///
/// 两处状态都要**核对一遍**再返回成功：安装器拿到的退出码就是安装成功与否的依据，
/// 悄悄只改一半是最糟的结果。
fn apply_autostart(enabled: bool) -> Result<(), String> {
    let mut config = Config::load();
    config.general.autostart = enabled;
    config.sanitize();
    config.save().map_err(|e| fill!(tr!("保存配置失败：{}"), e))?;

    desktop::reconcile_autostart(enabled);

    match (enabled, desktop::autostart_entry()) {
        (true, Some(command)) => {
            println!("开机自启已打开：{command}");
            Ok(())
        }
        (true, None) => Err("配置已写入，但注册表里没有出现自启项".to_owned()),
        (false, None) => {
            println!("开机自启已关闭");
            Ok(())
        }
        (false, Some(command)) => Err(fill!(tr!("配置已写入，但注册表里的自启项还在：{}"), command)),
    }
}

fn main() -> ExitCode {
    // 第一件事：把双击启动时 Windows 附赠的那个控制台窗口丢掉。
    // 必须排在 `setup_console_utf8` 之前——控制台一旦被释放，改代码页就没有意义了。
    detach_orphan_console();
    setup_console_utf8();
    init_logging();

    let cli = match parse_args(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("参数错误：{err}");
            return ExitCode::from(2);
        }
    };

    if cli.help {
        print_help();
        return ExitCode::SUCCESS;
    }
    if cli.version {
        println!("cuter {}", cuter_core::VERSION);
        return ExitCode::SUCCESS;
    }

    // 必须在创建任何窗口之前设置 DPI 感知，否则 125%/150% 缩放下截出来的图会被拉伸
    setup_dpi_awareness();

    // 自检：验证"panic → 写进日志"这条链路。没有它，一个常驻托盘的程序崩溃之后
    // 我们只能说"应该有日志吧"，而没法证明。
    if cli.self_test_panic {
        log::info!("自检：接下来会故意 panic，用来确认崩溃现场确实写进了日志文件");
        panic!("Cuter 自检：这是一次故意的 panic（用 --self-test-panic 触发）");
    }

    // 安装器/卸载器用的无界面开关：改配置 + 注册表，然后退出（不开窗口、不进 GUI 路径）。
    if let Some(enabled) = cli.set_autostart {
        return match apply_autostart(enabled) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("错误：{err}");
                ExitCode::FAILURE
            }
        };
    }

    if cli.headless() {
        return match run_headless(&cli) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("错误：{err}");
                ExitCode::FAILURE
            }
        };
    }

    run_gui(cli)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Cli, String> {
        parse_args(list.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn parses_flags() {
        let cli = args(&["--full", "--copy", "--out", "a.png"]).unwrap();
        assert!(cli.full && cli.copy);
        assert_eq!(cli.out.as_deref(), Some("a.png"));
        assert!(cli.headless());
    }

    #[test]
    fn parses_equals_form() {
        let cli = args(&["--out=b.png", "--region=1,2,3,4"]).unwrap();
        assert_eq!(cli.out.as_deref(), Some("b.png"));
        assert_eq!(cli.region.as_deref(), Some("1,2,3,4"));
    }

    #[test]
    fn rejects_conflicting_modes() {
        assert!(args(&["--full", "--monitor"]).is_err());
        assert!(args(&["--capture", "--region", "0,0,1,1"]).is_err());
    }

    #[test]
    fn rejects_missing_option_value() {
        assert!(args(&["--out"]).is_err());
        assert!(args(&["--region"]).is_err());
    }

    #[test]
    fn rejects_unknown_flag() {
        assert!(args(&["--nope"]).is_err());
    }

    #[test]
    fn gui_mode_is_not_headless() {
        let cli = args(&["--capture"]).unwrap();
        assert!(!cli.headless(), "--capture 应当走 GUI 路径");
    }

    #[test]
    fn pin_goes_through_gui_path() {
        let cli = args(&["--pin"]).unwrap();
        assert!(cli.pin);
        assert!(!cli.headless(), "--pin 需要 GUI（要创建贴图窗口）");
    }

    #[test]
    fn clear_pin_click_through_is_a_gui_activation() {
        let cli = args(&["--clear-pin-click-through"]).unwrap();
        assert!(cli.clear_pin_click_through);
        // 和 `--toggle-pins` 同一条路：要转交给活着的实例，所以走 GUI 路径而不是无界面子命令
        assert!(!cli.headless(), "--clear-pin-click-through 要把请求转交给运行中的实例");
    }

    #[test]
    fn parse_region_accepts_comma_and_x_separators() {
        assert_eq!(parse_region("10,20,30,40").unwrap(), Rect::new(10, 20, 30, 40));
        assert_eq!(parse_region("10,20,30x40").unwrap(), Rect::new(10, 20, 30, 40));
        assert_eq!(parse_region(" -5 , -5 , 100 , 100 ").unwrap(), Rect::new(-5, -5, 100, 100));
    }

    #[test]
    fn parse_region_rejects_malformed_input() {
        assert!(parse_region("1,2,3").is_err());
        assert!(parse_region("a,b,c,d").is_err());
        assert!(parse_region("1,2,0,10").is_err(), "宽为 0 应当报错");
        assert!(parse_region("1,2,10,-1").is_err(), "高为负应当报错");
    }
}
