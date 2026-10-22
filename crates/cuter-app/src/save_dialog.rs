//! 原生「另存为」文件对话框。
//!
//! 工具条上的「另存为…」以前只是把图片存进配置目录、再打开资源管理器，按钮文案和
//! 实际行为对不上。本模块用 `IFileSaveDialog` 提供**真正的**系统保存对话框：用户选
//! 目录、填文件名、挑格式，点「保存」之后我们把最终路径交回调用方。
//!
//! # 会阻塞调用线程（预期行为）
//!
//! [`ask_save_path`] 内部会调用 `IFileSaveDialog::Show`，这是一个**模态**调用：它会
//! 一直阻塞**当前线程**，直到用户点了「保存」或「取消」为止。这是预期行为——只有
//! 用户在界面上主动点「另存为…」才会走到这里，而且必须由 UI 线程同步调用（对话框
//! 要靠该线程的消息循环派发自己的消息）。不要在工作线程里调用它。
//!
//! # 永不 panic
//!
//! 本模块所有函数在失败时都只 `log::warn!`/`log::debug!` 然后返回 `None`。用户取消
//! 属于正常操作，只打 `debug` 日志，不打 `warn`。
//!
//! # 过滤器下拉框
//!
//! 文件类型过滤器用的是类型化 API：`IFileDialog::SetFileTypes` 配
//! `Win32::UI::Shell::Common::COMDLG_FILTERSPEC`。两者都被 `windows` crate 关在
//! **`Win32_UI_Shell_Common` feature** 后面（`Win32_UI_Shell` 不会自动带上它），
//! 所以根 `Cargo.toml` 里必须保留这个 feature，删掉就会编译不过。
//!
//! 另外注意 Common Item Dialog **只认第一次** `SetFileTypes`：同一根接口再调一次
//! （哪怕参数一模一样、哪怕传空数组）都会返回 `E_UNEXPECTED`（`0x8000FFFF`）。
//! 所以 `configure_filter` 对每个对话框只调一次。

use std::path::{Path, PathBuf};

/// 弹出系统「另存为」对话框，返回用户选择的路径。
///
/// 用户取消、或对话框无法创建时返回 `None`（调用方应回退到默认目录）。
///
/// # 会阻塞调用线程
///
/// 这是**模态**调用：它会一直阻塞**当前线程**，直到用户在对话框里点「保存」或
/// 「取消」。这是预期行为——只有用户在界面上主动点「另存为…」才会走到这里，并且
/// 必须由 UI 线程同步调用（对话框要靠该线程的消息循环派发消息）。不要放到工作线程。
///
/// `default_name` 里的非法文件名字符不需要在这里清理，调用方已经用
/// `render_filename` 处理过了。
#[cfg(windows)]
pub fn ask_save_path(
    default_dir: &Path,
    default_name: &str,
    filter_label: &str,
    extensions: &[&str],
) -> Option<PathBuf> {
    win_dialog::ask_save_path(default_dir, default_name, filter_label, extensions)
}

/// 非 Windows 平台没有 `IFileSaveDialog`，恒返回 `None`。
///
/// 保留这个桩是为了让交叉编译 / `cargo check --target` 之类的检查也能过，调用方
/// 不必自己写 `cfg`。
#[cfg(not(windows))]
pub fn ask_save_path(
    default_dir: &Path,
    default_name: &str,
    filter_label: &str,
    extensions: &[&str],
) -> Option<PathBuf> {
    let _ = (default_dir, default_name, filter_label, extensions);
    log::debug!("另存为对话框只在 Windows 上可用，调用方应回退到默认目录");
    None
}

/// Windows 实现。
#[cfg(windows)]
mod win_dialog {
    use core::ffi::c_void;
    use std::path::{Path, PathBuf};

    use windows::Win32::Foundation::{
        E_ABORT, ERROR_CANCELLED, RPC_E_CHANGED_MODE, S_FALSE, S_OK,
    };
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
        CoUninitialize, IBindCtx,
    };
    use windows::Win32::UI::Shell::{
        Common, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST,
        FileSaveDialog, IFileSaveDialog, IShellItem, SHCreateItemFromParsingName, SIGDN,
        SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH,
    };
    use windows::core::{HRESULT, PCWSTR, PWSTR};

    /// 没有给出（或给不出）任何扩展名时的兜底过滤器显示名。
    const FALLBACK_FILTER_LABEL: &str = "所有文件";

    /// 没有给出（或给不出）任何扩展名时的兜底通配符。
    const FALLBACK_FILTER_PATTERN: &str = "*.*";

    /// 对话框标题。
    const DIALOG_TITLE: &str = "保存截图";

    /// 单个扩展名的最大长度，用来挡掉明显异常的输入。
    const MAX_EXTENSION_LEN: usize = 32;

    /// 可以交给 `SetFileTypes` 的过滤器数组，连同它背后活着的宽字符串。
    ///
    /// `COMDLG_FILTERSPEC` 里只存指针（`pszName`/`pszSpec` 是 `PCWSTR`，系统只读指针、
    /// 不拷内容），所以宽字符串必须活到调用结束。把两者放进同一个结构体，就不会写出
    /// 「临时 `Vec` 提前析构、指针悬垂」这种代码。
    struct RawFilterSpecs {
        /// 宽字符串本体；`specs` 里的指针都指向这里。
        ///
        /// 这里只负责持有（`Vec<u16>` 的堆缓冲区不会因为外层 `Vec` 扩容或结构体搬家
        /// 而移动），所以不需要被读取，用下划线开头。
        _wide: Vec<Vec<u16>>,
        /// 可以直接传给 `SetFileTypes` 的数组。
        specs: Vec<Common::COMDLG_FILTERSPEC>,
    }

    impl RawFilterSpecs {
        /// 由 `build_filter_specs` 的输出构造。
        fn new(items: &[(String, String)]) -> Self {
            // 预先留够容量，避免外层 `Vec` 反复扩容（虽然扩容也不会让内层缓冲区搬家，
            // 但一次分配更省事）。
            let mut wide: Vec<Vec<u16>> = Vec::with_capacity(items.len() * 2);
            let mut specs: Vec<Common::COMDLG_FILTERSPEC> = Vec::with_capacity(items.len());

            for (name, pattern) in items {
                let name = to_wide(name);
                let pattern = to_wide(pattern);
                // 先取指针再入栈：`Vec<u16>` 搬家时它的堆缓冲区并不会动。
                specs.push(Common::COMDLG_FILTERSPEC {
                    pszName: PCWSTR::from_raw(name.as_ptr()),
                    pszSpec: PCWSTR::from_raw(pattern.as_ptr()),
                });
                wide.push(name);
                wide.push(pattern);
            }

            Self { _wide: wide, specs }
        }
    }

    /// COM 初始化状态的 RAII 守卫。
    ///
    /// 只有**我们自己**成功初始化了套间，`drop` 时才调用 `CoUninitialize`。
    struct ComGuard {
        /// 是否需要在析构时反初始化。
        uninitialize: bool,
    }

    impl ComGuard {
        /// 把当前线程初始化成单线程套间（STA）。
        ///
        /// * `S_OK` / `S_FALSE` —— 正常（`S_FALSE` 表示之前已经初始化过，引用计数 +1），
        ///   结束时必须配对调用 `CoUninitialize`。
        /// * `RPC_E_CHANGED_MODE` —— 当前线程**已经**是别的套间模型（多半是宿主程序
        ///   自己建的 MTA）。这不是失败，文件对话框在这种线程上照样能弹出来；但此时
        ///   我们**绝不能**调用 `CoUninitialize`，否则会把宿主已经建好的 COM 拆掉，
        ///   之后宿主所有的 COM 调用都会失败。所以照常返回 `Some`，只把
        ///   `uninitialize` 标成 `false`。
        /// * 其它错误 —— 记录 `warn` 并返回 `None`。
        fn init() -> Option<Self> {
            // SAFETY: 第一个参数是保留参数，约定传 null；第二个参数是合法的套间枚举
            // 取值。`CoInitializeEx` 只改变当前线程的 COM 状态，不涉及内存所有权。
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            if hr == S_OK || hr == S_FALSE {
                return Some(Self { uninitialize: true });
            }
            if hr == RPC_E_CHANGED_MODE {
                log::debug!(
                    "当前线程已进入其它 COM 套间（RPC_E_CHANGED_MODE），沿用现成套间弹对话框"
                );
                return Some(Self { uninitialize: false });
            }
            log::warn!("COM 初始化失败（0x{:08X}），无法弹出另存为对话框", hr.0);
            None
        }
    }

    impl Drop for ComGuard {
        fn drop(&mut self) {
            if !self.uninitialize {
                return;
            }
            // SAFETY: 与上面返回 S_OK/S_FALSE 的那次 `CoInitializeEx` 一一配对，
            // 且每个守卫只析构一次，不会多调。
            unsafe { CoUninitialize() };
        }
    }

    /// `CoTaskMemFree` 的 RAII 守卫。
    ///
    /// `IShellItem::GetDisplayName` 返回的缓冲区由 COM 的任务分配器分配，只能用
    /// `CoTaskMemFree` 释放（不是 `free`，也不是 `drop`）。用守卫包起来，是为了保证
    /// 任何提前 `return` 的路径也不会漏掉释放。
    struct CoTaskMemGuard(PWSTR);

    impl Drop for CoTaskMemGuard {
        fn drop(&mut self) {
            if self.0.is_null() {
                return;
            }
            // SAFETY: 指针来自 `GetDisplayName`（CoTaskMemAlloc 系列），且只在析构时
            // 释放一次，不会重复释放。
            unsafe { CoTaskMemFree(Some(self.0.0.cast::<c_void>().cast_const())) };
        }
    }

    /// `Show` 之后的结局。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ShowOutcome {
        /// 用户点了「保存」，可以取结果了。
        Confirmed,
        /// 用户取消，属于正常操作。
        Cancelled,
        /// 其它失败，已经打过 `warn` 日志。
        Failed,
    }

    /// `ask_save_path` 的 Windows 实现（公开文档见父模块中的同名函数）。
    ///
    /// **注意：`Show` 是模态阻塞调用**，会卡住当前线程直到用户操作完成。
    pub(super) fn ask_save_path(
        default_dir: &Path,
        default_name: &str,
        filter_label: &str,
        extensions: &[&str],
    ) -> Option<PathBuf> {
        // COM 必须先初始化才能用任何 COM 接口；守卫在函数返回时按需反初始化。
        let Some(_com) = ComGuard::init() else {
            // `ComGuard::init` 内部已经打过 warn 了。
            return None;
        };

        // 创建对话框；创建失败的原因已经在 `create_dialog` 里记过日志。
        match create_dialog(default_dir, default_name, filter_label, extensions) {
            Some(dialog) => match show_dialog(&dialog) {
                ShowOutcome::Confirmed => result_path(&dialog),
                // 用户取消不是错误：静默返回 None，让调用方回退到默认目录。
                ShowOutcome::Cancelled | ShowOutcome::Failed => None,
            },
            None => None,
        }
    }

    /// 创建并配置保存对话框。
    ///
    /// 单独抽出来是为了让 `#[ignore]` 的交互式测试只验证到「创建 + 配置」为止，
    /// 不去碰会阻塞线程的 `Show`。
    fn create_dialog(
        default_dir: &Path,
        default_name: &str,
        filter_label: &str,
        extensions: &[&str],
    ) -> Option<IFileSaveDialog> {
        // SAFETY: `FileSaveDialog` 是系统注册的 CLSID；`punkouter` 传 null 表示不聚合
        // 其它对象；`CLSCTX_ALL` 允许进程内 / 本地服务器。返回的接口指针由 windows
        // crate 管理引用计数，用完自动 Release。
        let dialog: IFileSaveDialog =
            match unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_ALL) } {
                Ok(dialog) => dialog,
                Err(err) => {
                    log::warn!("创建 IFileSaveDialog 失败：{err}");
                    return None;
                }
            };

        configure_filter(&dialog, filter_label, extensions);
        configure_default_extension(&dialog, extensions);
        configure_file_name(&dialog, default_name);
        configure_folder(&dialog, default_dir);
        configure_options(&dialog);
        configure_title(&dialog);

        Some(dialog)
    }

    /// 设置「文件类型」下拉框。
    ///
    /// `SetFileTypes` 只能调**一次**：Common Item Dialog 上第二次调用（哪怕参数一样、
    /// 哪怕传空数组）都会返回 `E_UNEXPECTED`（`0x8000FFFF`），所以这里就一次，失败也只
    /// 记日志——过滤器设不上不影响保存，用户照样能手输文件名。
    fn configure_filter(dialog: &IFileSaveDialog, filter_label: &str, extensions: &[&str]) {
        // `COMDLG_FILTERSPEC` 里存的是指针而不是字符串内容，所以 `raw` 必须活到
        // `SetFileTypes` 调用结束（`RawFilterSpecs` 把两者绑在一起）。
        let raw = RawFilterSpecs::new(&build_filter_specs(extensions, filter_label));

        // SAFETY: `raw` 在本次调用期间一直存活，`specs` 里的两个 `PCWSTR` 都指向
        // `raw._wide` 中的缓冲区；数组非空（`build_filter_specs` 保证至少一项）。
        if let Err(err) = unsafe { dialog.SetFileTypes(&raw.specs) } {
            log::warn!("设置保存对话框过滤器失败：{err}");
        }

        // 过滤器索引从 1 开始（0 是「自定义」的哨兵值）。
        // SAFETY: 参数是普通整数，接口指针由调用方保证有效。
        if let Err(err) = unsafe { dialog.SetFileTypeIndex(1) } {
            log::warn!("设置保存对话框默认过滤器失败：{err}");
        }
    }

    /// 设置默认扩展名（只取第一个有效扩展名，没有就跳过）。
    fn configure_default_extension(dialog: &IFileSaveDialog, extensions: &[&str]) {
        let Some(extension) = first_valid_extension(extensions) else {
            // 传空字符串会被系统拒绝，干脆不设：对话框会用用户输入的后缀。
            log::debug!("没有可用的扩展名，跳过 SetDefaultExtension");
            return;
        };

        let wide = to_wide(&extension);
        // SAFETY: `wide` 以 0 结尾，并且在本次调用期间一直存活。
        if let Err(err) = unsafe { dialog.SetDefaultExtension(PCWSTR::from_raw(wide.as_ptr())) } {
            log::warn!("设置保存对话框默认扩展名失败：{err}");
        }
    }

    /// 设置默认文件名。
    fn configure_file_name(dialog: &IFileSaveDialog, default_name: &str) {
        let wide = to_wide(default_name);
        // SAFETY: `wide` 以 0 结尾，并且在本次调用期间一直存活。
        if let Err(err) = unsafe { dialog.SetFileName(PCWSTR::from_raw(wide.as_ptr())) } {
            log::warn!("设置保存对话框默认文件名失败：{err}");
        }
    }

    /// 设置对话框打开时的目录。
    ///
    /// 这一步失败**不算致命**：对话框会落在系统默认位置，用户照样能保存。
    fn configure_folder(dialog: &IFileSaveDialog, default_dir: &Path) {
        let Some(folder) = parse_folder(default_dir) else {
            return;
        };
        // SAFETY: `folder` 是有效的 IShellItem，`SetFolder` 只读取它，不接管所有权。
        if let Err(err) = unsafe { dialog.SetFolder(&folder) } {
            log::debug!("设置保存对话框初始目录失败（{err}），对话框会落在系统默认目录");
        }
    }

    /// 把默认目录解析成 `IShellItem`；解析不了就返回 `None`（不致命，只记 debug）。
    fn parse_folder(dir: &Path) -> Option<IShellItem> {
        let Some(text) = dir.to_str() else {
            log::debug!(
                "默认目录不是合法的 UTF-8 路径，跳过初始目录设置：{}",
                dir.display()
            );
            return None;
        };

        let wide = to_wide(text);
        // SAFETY: `wide` 以 0 结尾且在本次调用期间存活；`pbc` 传 null 表示不做额外
        // 绑定；第三个泛型参数决定要请求的接口 IID（这里是 IShellItem）。
        let item = unsafe {
            SHCreateItemFromParsingName::<PCWSTR, Option<&IBindCtx>, IShellItem>(
                PCWSTR::from_raw(wide.as_ptr()),
                None,
            )
        };

        match item {
            Ok(item) => Some(item),
            Err(err) => {
                log::debug!("默认目录「{text}」无法解析成 IShellItem（{err}），跳过初始目录设置");
                None
            }
        }
    }

    /// 设置对话框选项。
    fn configure_options(dialog: &IFileSaveDialog) {
        let options = FOS_FORCEFILESYSTEM
            | FOS_OVERWRITEPROMPT
            | FOS_PATHMUSTEXIST
            | FOS_NOCHANGEDIR;
        // SAFETY: 参数是四个标志位的按位或，`FILEOPENDIALOGOPTIONS` 只是 u32 的
        // 透明包装，没有指针或所有权。
        if let Err(err) = unsafe { dialog.SetOptions(options) } {
            log::warn!("设置保存对话框选项失败：{err}");
        }
    }

    /// 设置对话框标题。
    fn configure_title(dialog: &IFileSaveDialog) {
        let wide = to_wide(DIALOG_TITLE);
        // SAFETY: `wide` 以 0 结尾，并且在本次调用期间一直存活。
        if let Err(err) = unsafe { dialog.SetTitle(PCWSTR::from_raw(wide.as_ptr())) } {
            log::warn!("设置保存对话框标题失败：{err}");
        }
    }

    /// 弹出模态对话框并等待用户操作。
    ///
    /// 这个调用会**阻塞当前线程**，直到用户点「保存」或「取消」。
    fn show_dialog(dialog: &IFileSaveDialog) -> ShowOutcome {
        // SAFETY: 父窗口传 None（Cuter 主窗口不是 Win32 窗口，传它没有意义）；`Show`
        // 期间 `dialog` 一直被借用，不会提前释放。
        match unsafe { dialog.Show(None) } {
            Ok(()) => ShowOutcome::Confirmed,
            Err(err) if is_cancelled(err.code()) => {
                // 用户取消是正常操作，不打 warn。
                log::debug!("用户在另存为对话框中取消");
                ShowOutcome::Cancelled
            }
            Err(err) => {
                log::warn!("另存为对话框返回失败：{err}");
                ShowOutcome::Failed
            }
        }
    }

    /// 取回用户最终选定的路径。
    fn result_path(dialog: &IFileSaveDialog) -> Option<PathBuf> {
        // SAFETY: 只有用户点了「保存」才会走到这里，此时 GetResult 一定可用。
        let item = match unsafe { dialog.GetResult() } {
            Ok(item) => item,
            Err(err) => {
                log::warn!("取回另存为结果失败：{err}");
                return None;
            }
        };

        // 首选文件系统路径；`FOS_FORCEFILESYSTEM` 已经让这一路径覆盖绝大多数情况。
        if let Some(path) = item_path(&item, SIGDN_FILESYSPATH) {
            return Some(path);
        }

        // 极少数情况下拿不到文件系统路径（例如用户把目标放进了「此电脑」这类虚拟
        // 位置），退化成解析名：它至少还是 `::{GUID}\...` 这种可读字符串，交给调用方
        // 去决定能不能用。再失败就返回 None，让调用方回退到默认目录。
        log::debug!("拿不到文件系统路径，退化成 SIGDN_DESKTOPABSOLUTEPARSING");
        item_path(&item, SIGDN_DESKTOPABSOLUTEPARSING)
    }

    /// 用指定的方式取 `IShellItem` 的显示名并转成路径。
    fn item_path(item: &IShellItem, sigdn: SIGDN) -> Option<PathBuf> {
        // SAFETY: `item` 有效；成功时返回的是 COM 分配的 `PWSTR`，下面用守卫释放。
        let raw = match unsafe { item.GetDisplayName(sigdn) } {
            Ok(raw) => raw,
            Err(err) => {
                log::debug!("GetDisplayName 失败：{err}");
                return None;
            }
        };

        // 守卫先建好，后面无论怎么提前 return 都不会漏释放。
        let _guard = CoTaskMemGuard(raw);
        if raw.is_null() {
            log::warn!("GetDisplayName 返回了空指针");
            return None;
        }

        // SAFETY: 指针非空、来自 `GetDisplayName`，指向一段以 0 结尾的宽字符串。
        let units = unsafe { raw.as_wide() };
        let name = decode_wide(units);
        let path = path_from_display_name(&name);
        if path.is_none() {
            log::debug!("对话框返回了空路径");
        }
        path
    }

    /// `HRESULT_FROM_WIN32(code)` 的等价实现。
    ///
    /// `windows` crate 0.62 没有导出这个宏（`Win32::Foundation` 里找不到），但转换
    /// 规则是固定的：把 Win32 错误码塞进 `HRESULT` 的 `FACILITY_WIN32` 段。
    const fn hresult_from_win32(code: u32) -> HRESULT {
        HRESULT((0x8007_0000u32 | (code & 0xFFFF)) as i32)
    }

    /// 判断 `Show` 返回的错误码是不是「用户取消」。
    ///
    /// 不同 Windows 版本给的码不一样：老的给 `E_ABORT`，新的给
    /// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`。两种都必须当成取消处理，而且不能打
    /// `warn`——用户取消是正常操作。
    fn is_cancelled(code: HRESULT) -> bool {
        code == E_ABORT || code == hresult_from_win32(ERROR_CANCELLED.0)
    }

    /// 生成「文件类型」下拉框的内容，返回 `(显示名, "*.png;*.jpg" 形式的通配符)`。
    ///
    /// 目前恒为一项，返回 `Vec` 只是为了能直接喂给 `SetFileTypes` 需要的数组参数。
    ///
    /// 扩展名的写法很宽容：`png`、`.png`、`*.png`、`  PNG  ` 都会归一成 `*.png`；
    /// 全是空串、通配符残渣之类的垃圾输入会退化成「所有文件 / `*.*`」——
    /// **永远不要构造空数组交给 `SetFileTypes`**：文档说它会返回 `E_INVALIDARG`
    /// （本机实测倒是接受了空数组，但空过滤器在下拉框里毫无意义，而且别的 Windows
    /// 版本未必这么宽容）。
    fn build_filter_specs(extensions: &[&str], filter_label: &str) -> Vec<(String, String)> {
        let mut patterns: Vec<String> = Vec::new();
        for extension in extensions {
            let normalized = normalize_extension(extension);
            if normalized.is_empty() {
                continue;
            }
            let pattern = format!("*.{normalized}");
            // 去掉重复项，免得多扩展名时出现 `*.png;*.png`。
            if !patterns.contains(&pattern) {
                patterns.push(pattern);
            }
        }

        if patterns.is_empty() {
            return vec![(
                FALLBACK_FILTER_LABEL.to_owned(),
                FALLBACK_FILTER_PATTERN.to_owned(),
            )];
        }

        // 空标签会在下拉框里显示成一片空白，所以也退化成兜底文案。
        let label = filter_label.trim();
        let label = if label.is_empty() {
            FALLBACK_FILTER_LABEL
        } else {
            label
        };
        vec![(label.to_owned(), patterns.join(";"))]
    }

    /// 取第一个能用的扩展名（已归一化、小写、不带点号和通配符）。
    fn first_valid_extension(extensions: &[&str]) -> Option<String> {
        extensions
            .iter()
            .map(|extension| normalize_extension(extension))
            .find(|normalized| !normalized.is_empty())
    }

    /// 把各种写法的扩展名归一成「不带 `*`、不带点号、全小写」的形式。
    ///
    /// 非法输入（空串、含通配符或路径分隔符、超长、非 ASCII）统一归一成空串，
    /// 由调用方当作「这一项不可用」处理。
    fn normalize_extension(extension: &str) -> String {
        let trimmed = extension
            .trim()
            .trim_start_matches('*')
            .trim_start_matches('.');
        if is_valid_extension(trimmed) {
            trimmed.to_ascii_lowercase()
        } else {
            String::new()
        }
    }

    /// 扩展名是否合法。
    ///
    /// 只允许 ASCII 字母数字与 `.`、`-`、`_`：扩展名里出现 `*`、`;`、路径分隔符会
    /// 破坏过滤器的语法，出现非 ASCII 则多半是调用方把别的东西当扩展名传进来了。
    fn is_valid_extension(extension: &str) -> bool {
        !extension.is_empty()
            && extension.len() <= MAX_EXTENSION_LEN
            && !extension.starts_with('.')
            && !extension.ends_with('.')
            && extension
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_')
    }

    /// 把 `str` 转成以 0 结尾的 UTF-16 序列。
    ///
    /// Win32 只认这种「宽字符串」，而且所有 `PCWSTR` 参数都要求内存在**调用期间**
    /// 保持存活。所以调用方必须先把返回值存进变量（例如 `Vec<Vec<u16>>`）再取指针，
    /// 不能写成 `PCWSTR::from_raw(to_wide(s).as_ptr())` 这种临时值写法——那个 `Vec`
    /// 会在语句结束时析构，指针立刻悬垂。
    fn to_wide(text: &str) -> Vec<u16> {
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        wide.push(0);
        wide
    }

    /// 把一段 UTF-16 码元解成 `String`。
    ///
    /// 比直接用 `String::from_utf16_lossy` 多做两件事，所以单独抽出来测：
    /// 1. 遇到 0 就截断（`GetDisplayName` 说好不带结尾 NUL，但不能假设）；
    /// 2. 有损解码，遇到落单的代理项既不返回 `Err` 也不 panic。
    fn decode_wide(units: &[u16]) -> String {
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        let units = units.get(..end).unwrap_or(units);
        String::from_utf16_lossy(units)
    }

    /// 把显示名转成路径；空白字符串视为失败（返回 `None`）。
    ///
    /// 只在「整串都是空白」时才认为无效，不会去 `trim` 真实路径——Windows 路径本身
    /// 不会以空格结尾，但没必要替系统做这个决定。
    fn path_from_display_name(name: &str) -> Option<PathBuf> {
        if name.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(name))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// 断言某个扩展名列表归一化后的通配符。
        fn spec_of(extensions: &[&str]) -> (String, String) {
            let specs = build_filter_specs(extensions, "图片");
            match specs.first() {
                Some(spec) => spec.clone(),
                None => panic!("build_filter_specs 至少应当产出一项"),
            }
        }

        #[test]
        fn build_filter_specs_falls_back_when_empty() {
            assert_eq!(
                build_filter_specs(&[], "图片"),
                vec![(FALLBACK_FILTER_LABEL.to_owned(), "*.*".to_owned())]
            );
        }

        #[test]
        fn build_filter_specs_single_extension() {
            assert_eq!(spec_of(&["png"]), ("图片".to_owned(), "*.png".to_owned()));
        }

        #[test]
        fn build_filter_specs_joins_multiple_extensions() {
            assert_eq!(
                spec_of(&["jpg", "jpeg"]),
                ("图片".to_owned(), "*.jpg;*.jpeg".to_owned())
            );
        }

        #[test]
        fn build_filter_specs_normalizes_prefixes_and_case() {
            // 带点、带通配符、带空白、大小写不同，结果都应当一样。
            assert_eq!(spec_of(&[".png"]), spec_of(&["png"]));
            assert_eq!(spec_of(&["*.png"]), spec_of(&["png"]));
            assert_eq!(spec_of(&["  PNG  "]), spec_of(&["png"]));
            assert_eq!(spec_of(&["*.PNG"]), spec_of(&["png"]));
            assert_eq!(spec_of(&["png"]).1, "*.png");
        }

        #[test]
        fn build_filter_specs_keeps_inner_dots_and_dedupes() {
            assert_eq!(spec_of(&["tar.gz"]).1, "*.tar.gz");
            assert_eq!(spec_of(&["png", ".PNG", "*.png"]).1, "*.png");
        }

        #[test]
        fn build_filter_specs_drops_garbage_and_falls_back() {
            // 全是被拒绝的输入 → 兜底过滤器，而不是空数组。
            for garbage in [
                vec![""],
                vec!["   "],
                vec!["*"],
                vec!["."],
                vec!["*."],
                vec!["p*g"],
                vec!["a/b"],
                vec!["a\\b"],
                vec!["图"],
                vec!["*.!", "  "],
            ] {
                assert_eq!(
                    build_filter_specs(&garbage, "图片"),
                    vec![(FALLBACK_FILTER_LABEL.to_owned(), "*.*".to_owned())],
                    "输入 {garbage:?} 应当退化成兜底过滤器"
                );
            }

            // 混进垃圾项时只丢掉垃圾，有效项照常保留。
            assert_eq!(
                build_filter_specs(&["png", "*.!", "  "], "图片"),
                vec![("图片".to_owned(), "*.png".to_owned())]
            );
        }

        #[test]
        fn build_filter_specs_falls_back_when_label_blank() {
            assert_eq!(spec_of(&[]).0, FALLBACK_FILTER_LABEL);
            let specs = build_filter_specs(&["png"], "   ");
            assert_eq!(
                specs,
                vec![(FALLBACK_FILTER_LABEL.to_owned(), "*.png".to_owned())]
            );
        }

        #[test]
        fn first_valid_extension_skips_garbage() {
            assert_eq!(first_valid_extension(&[]), None);
            assert_eq!(first_valid_extension(&["*", "", "  "]), None);
            assert_eq!(
                first_valid_extension(&["*", ".PNG", "jpg"]),
                Some("png".to_owned())
            );
        }

        #[test]
        fn to_wide_is_nul_terminated() {
            assert_eq!(to_wide("ab"), vec![0x0061, 0x0062, 0x0000]);
            assert_eq!(to_wide(""), vec![0x0000]);
        }

        #[test]
        fn to_wide_encodes_chinese() {
            // 截 = U+622A，图 = U+56FE
            assert_eq!(to_wide("截图"), vec![0x622A, 0x56FE, 0x0000]);
        }

        #[test]
        fn decode_wide_round_trips_to_wide() {
            assert_eq!(decode_wide(&to_wide("C:\\Users\\截图.png")), "C:\\Users\\截图.png");
            assert_eq!(decode_wide(&[]), "");
        }

        #[test]
        fn decode_wide_truncates_at_nul() {
            assert_eq!(decode_wide(&[0x0061, 0x0000, 0x0062]), "a");
            assert_eq!(decode_wide(&[0x0000, 0x0061]), "");
        }

        #[test]
        fn decode_wide_replaces_lone_surrogate() {
            // 落单的高代理项要被替换成 U+FFFD，而不是 panic 或返回 Err。
            assert_eq!(decode_wide(&[0xD800]), "\u{FFFD}");
        }

        #[test]
        fn path_from_display_name_rejects_blank() {
            assert_eq!(path_from_display_name(""), None);
            assert_eq!(path_from_display_name("   \t"), None);
            assert_eq!(
                path_from_display_name("C:\\Users\\a.png"),
                Some(PathBuf::from("C:\\Users\\a.png"))
            );
        }

        #[test]
        fn hresult_from_win32_matches_known_value() {
            // HRESULT_FROM_WIN32(ERROR_CANCELLED) == 0x800704C7
            assert_eq!(
                hresult_from_win32(ERROR_CANCELLED.0),
                HRESULT(0x8007_04C7u32 as i32)
            );
        }

        #[test]
        fn is_cancelled_accepts_both_known_codes() {
            assert!(is_cancelled(E_ABORT));
            assert!(is_cancelled(hresult_from_win32(ERROR_CANCELLED.0)));
            assert!(!is_cancelled(HRESULT(0)));
            // 其它失败码（E_FAIL / E_UNEXPECTED / E_INVALIDARG 之类）不能当取消。
            assert!(!is_cancelled(HRESULT(0x8000_4005u32 as i32)));
            assert!(!is_cancelled(HRESULT(0x8000_FFFFu32 as i32)));
            assert!(!is_cancelled(HRESULT(0x8007_0057u32 as i32)));
        }

        /// 交互式冒烟测试：只验证「创建 + 配置」能走通，**不调用 `Show`**。
        ///
        /// `Show` 会阻塞线程等用户操作，在自动化里跑必然卡死，所以这里只检查
        /// `CoCreateInstance` 能拿到 `IFileSaveDialog`、配置步骤能被系统接受，
        /// 顺便用一根全新的接口验证类型化 `SetFileTypes` 这条路真的可用。
        #[test]
        #[ignore = "会弹出模态对话框，需要交互式桌面"]
        fn dialog_opens() {
            let Some(_com) = ComGuard::init() else {
                panic!("COM 初始化失败，无法验证另存为对话框");
            };

            let dialog = create_dialog(
                Path::new("C:\\Windows"),
                "cuter-test.png",
                "PNG 图片",
                &["png", ".jpg", "*.jpeg"],
            );
            let Some(dialog) = dialog else {
                panic!("create_dialog 返回 None，IFileSaveDialog 不可用");
            };

            // SAFETY: `dialog` 是刚刚创建出来的有效接口指针，GetFileTypeIndex 只读状态。
            let index = unsafe { dialog.GetFileTypeIndex() };
            assert_eq!(index.ok(), Some(1), "过滤器索引应当是我们设置的 1");

            // 必须用一根**全新**的接口：`SetFileTypes` 只允许调一次，第二次开始一律返回
            // `E_UNEXPECTED`（实测行为，见 `configure_filter` 的文档），而
            // `create_dialog` 里已经用掉那一次了。
            let fresh: IFileSaveDialog =
                match unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_ALL) } {
                    Ok(fresh) => fresh,
                    Err(err) => panic!("二次创建 IFileSaveDialog 失败：{err}"),
                };
            let raw = RawFilterSpecs::new(&build_filter_specs(&["png", "jpg"], "PNG 图片"));
            // SAFETY: `raw` 在本次调用期间存活，`specs` 里的 PCWSTR 都指向 `raw._wide`。
            let applied = unsafe { fresh.SetFileTypes(&raw.specs) };
            assert!(applied.is_ok(), "类型化 SetFileTypes 应当成功：{applied:?}");
        }
    }
}

/// 非 Windows 平台的定向测试。
#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn ask_save_path_returns_none_on_non_windows() {
        let path = ask_save_path(Path::new("."), "a.png", "PNG 图片", &["png"]);
        assert_eq!(path, None, "非 Windows 平台没有 IFileSaveDialog");
    }
}
