//! 单实例守卫。
//!
//! ## 为什么必须有
//!
//! 全局热键是**进程独占**的资源。第二个实例注册不上它（`RegisterHotKey` 直接失败），
//! 于是用户看到的现象是「按了 `Alt+S`，遮罩永远不出现」——因为热键进了**旧实例**。
//! `docs/HANDOFF.md` 5.1 把这个坑记在测试注意事项里，但它同样是**用户会遇到的真实问题**：
//! 用户双击两次图标，或从「开始」菜单点第二次，就多了一个什么都做不了的实例。
//!
//! ## 怎么做
//!
//! 1. **命名互斥体**：`Local\cuter-single-<user>`。`CreateMutexW` 返回
//!    `ERROR_ALREADY_EXISTS` 说明已经有实例；
//! 2. **命名事件**：每个请求（显示窗口 / 框选截图 / 贴图）一个自动重置事件
//!    `Local\cuter-req-<请求>-<user>`。第一个实例创建它们，后来的实例
//!    `OpenEventW` + `SetEvent` 把请求递过去，然后自己退出；
//! 3. 第一个实例在 `logic()` 里用超时为 0 的 `WaitForSingleObject` 非阻塞轮询，
//!    所以不需要额外线程，也不需要管道（管道要么多一个线程，要么多几个 feature）。
//!
//! 选**事件**而不是**管道**的理由很实际：管道要 `Win32_System_Pipes`（外加它签名里
//! 引用的 `Win32_System_IO` / `Win32_Storage_FileSystem`）三个 feature、一个后台线程、
//! 以及 `ConnectNamedPipe` 的阻塞语义；而事件只用已经在用的 `Win32_System_Threading`，
//! 请求种类也只有三种，一个事件一个语义反而更清楚。
//!
//! ## 什么情况**不**参与
//!
//! 无界面子命令（`--full` / `--monitor` / `--region` / `--list-monitors`）**不做**单实例
//! 检查：它们本来就是可以并行的一次性任务，挡住它们只会让脚本莫名其妙地失败。
//! 守卫只在 `run_gui` 里生效。
//!
//! ## `Local\` 与用户名的意义
//!
//! `Local\` 让对象只在本登录会话里可见（多用户 / 远程桌面下互不干扰）；
//! 名字里再带上用户名，是为了避免同一台机器上不同用户的实例互相抢请求。
//! 用户名会被过滤成 ASCII 字母数字，避免命名对象里出现奇怪字符。


use std::time::Duration;

/// 后来的实例要转达给第一个实例的请求。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    /// 用户又启动了一次：把设置窗口拿出来。
    Show,
    /// `cuter --capture`：开始框选截图。
    Capture,
    /// `cuter --pin`：把剪贴板图片贴到屏幕上。
    Pin,
    /// `cuter --history`：打开截图历史面板。
    History,
    /// `cuter --tray-menu`：打开自绘的托盘菜单面板。
    TrayMenu,
    /// `cuter --toggle-pins`：隐藏 / 显示当前组的贴图。
    TogglePins,
    /// `cuter --next-pin-group`：切到下一个贴图分组。
    NextPinGroup,
    /// `cuter --clear-pin-click-through`：取消所有贴图的鼠标穿透（和托盘菜单那一项同一个处理函数）。
    ClearPinClickThrough,
}

impl Activation {
    /// 全部请求（顺序即轮询顺序）。
    pub const ALL: [Self; 8] = [
        Self::Show,
        Self::Capture,
        Self::Pin,
        Self::History,
        Self::TrayMenu,
        Self::TogglePins,
        Self::NextPinGroup,
        Self::ClearPinClickThrough,
    ];

    /// 命名事件里用的英文短名。
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Capture => "capture",
            Self::Pin => "pin",
            Self::History => "history",
            Self::TrayMenu => "tray-menu",
            Self::TogglePins => "toggle-pins",
            Self::NextPinGroup => "next-pin-group",
            Self::ClearPinClickThrough => "clear-pin-click-through",
        }
    }
}

/// `acquire` 的结果。
#[derive(Debug)]
pub enum Acquire {
    /// 本进程是唯一实例，守卫必须活到进程结束。
    First(Guard),
    /// 已经有实例在跑：调用方应当转发请求后退出。
    AlreadyRunning,
    /// 内核对象创建失败（罕见）。调用方应当**照常启动**，只是没有单实例保护——
    /// 宁可多开一个实例，也不能因为创建不了互斥体就不让用户用。
    Unavailable(String),
}

/// 把环境里的用户名过滤成可用作内核对象名的 ASCII 串。
fn tag_from(raw: &str) -> String {
    let tag: String = raw.chars().filter(char::is_ascii_alphanumeric).collect();
    if tag.is_empty() {
        "default".to_owned()
    } else {
        tag
    }
}

fn user_tag() -> String {
    tag_from(&std::env::var("USERNAME").unwrap_or_default())
}

fn mutex_name(tag: &str) -> String {
    format!("Local\\cuter-single-{tag}")
}

fn event_name(tag: &str, request: Activation) -> String {
    format!("Local\\cuter-req-{}-{tag}", request.slug())
}

/// 转发请求时等待第一个实例把事件建出来的最长时间。
///
/// 竞态是真实存在的：第二个实例可能在第一个实例刚 `CreateMutexW` 完、还没建事件
/// 的时候启动。2 秒足够覆盖一次正常的冷启动。
#[cfg(windows)]
const FORWARD_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(windows)]
const FORWARD_RETRY: Duration = Duration::from_millis(100);

// ---------------------------------------------------------------------------
// Windows 实现
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use super::{
        Acquire, Activation, FORWARD_RETRY, FORWARD_TIMEOUT, Guard, event_name, mutex_name,
    };
    use std::time::Instant;
    use windows::Win32::Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0,
    };
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, OpenEventW, SetEvent, WaitForMultipleObjects,
        WaitForSingleObject,
    };
    use windows::core::PCWSTR;

    /// 转成以 NUL 结尾的 UTF-16（Win32 的 `LPCWSTR`）。
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    impl Guard {
        /// 用给定的命名后缀尝试拿到守卫。
        ///
        /// `tag` 是一个参数而不是直接读环境变量，是为了让测试能用**独立的命名对象**
        /// 跑——否则测试会和用户桌面上正在运行的 Cuter 抢同一个互斥体。
        pub(super) fn acquire_with(tag: &str) -> Acquire {
            let name = wide(&mutex_name(tag));
            // SAFETY: name 是以 NUL 结尾的 UTF-16；要求初始所有权，失败时返回 Err。
            let mutex = match unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) } {
                Ok(handle) => handle,
                Err(err) => return Acquire::Unavailable(fill!(tr!("创建互斥体失败：{}"), err)),
            };
            // 必须紧接着读：任何其它 Win32 调用都会覆盖 last error
            let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
            if existed {
                // 「已存在」不等于「有人在跑」：上一个实例可能是崩溃退出的，
                // 那时互斥体对象还在、但已经没人持有。官方信号是 WAIT_ABANDONED
                // （持有者没释放就死了），此时我们自己接管。
                let state = unsafe { WaitForSingleObject(mutex, 0) };
                if state != WAIT_OBJECT_0 && state != WAIT_ABANDONED {
                    // SAFETY: mutex 由本函数创建，且我们没能取得所有权，直接关闭。
                    let _ = unsafe { CloseHandle(mutex) };
                    return Acquire::AlreadyRunning;
                }
            }

            let mut events = [HANDLE::default(); Activation::ALL.len()];
            for (index, request) in Activation::ALL.iter().enumerate() {
                let event = wide(&event_name(tag, *request));
                // 自动重置、初始无信号：来一个请求亮一下，等到的人把它按下去
                // SAFETY: 名字是以 NUL 结尾的 UTF-16；无安全属性、非手动重置。
                match unsafe { CreateEventW(None, false, false, PCWSTR(event.as_ptr())) } {
                    Ok(handle) => events[index] = handle,
                    Err(err) => {
                        // SAFETY: 前面创建的句柄与互斥体都由本函数拥有，逐一回收。
                        for handle in events.iter().take(index) {
                            let _ = unsafe { CloseHandle(*handle) };
                        }
                        let _ = unsafe { CloseHandle(mutex) };
                        return Acquire::Unavailable(fill!(tr!("创建事件对象失败：{}"), err));
                    }
                }
            }

            Acquire::First(Guard {
                mutex,
                events,
                pending: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
                watched: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            })
        }

        /// 起一个等待线程：任何一个请求事件亮起来，就把 UI 线程**立刻**叫醒。
        ///
        /// 没有它的话，外部请求（`cuter --pin` 这类走单实例通道的）只能等主循环下一次
        /// 轮询；而待命心跳最长退到 4 s，于是"点一下要等好几秒"（`pin_groups_e2e`
        /// 就是这么暴露出来的）。线程本身只做 `WaitForMultipleObjects` + 一次投递，
        /// 不画界面，也不占 GPU。
        pub(super) fn spawn_waker_impl<F>(&self, post: F)
        where
            F: Fn() + Send + Sync + 'static,
        {
            // `HANDLE` 是 `*mut c_void`，不是 `Send`；线程里要自己重建一遍。
            let raw: Vec<isize> = self.events.iter().map(|h| h.0 as isize).collect();
            let pending = std::sync::Arc::clone(&self.pending);
            let watched = std::sync::Arc::clone(&self.watched);
            let post: std::sync::Arc<dyn Fn() + Send + Sync> = std::sync::Arc::new(post);
            self.watched
                .store(true, std::sync::atomic::Ordering::SeqCst);
            std::thread::spawn(move || {
                let handles: Vec<windows::Win32::Foundation::HANDLE> = raw
                    .iter()
                    .map(|&value| windows::Win32::Foundation::HANDLE(value as *mut core::ffi::c_void))
                    .collect();
                loop {
                    // SAFETY: 句柄数组由 Guard 拥有，Guard 活到进程退出（Drop 会关掉它们）。
                    let state = unsafe { WaitForMultipleObjects(&handles, false, u32::MAX) };
                    let index = state.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
                    if index >= handles.len() {
                        // WAIT_FAILED 之类：退化成慢轮询，别把线程烧死。
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        continue;
                    }
                    pending.fetch_or(1u64 << index, std::sync::atomic::Ordering::SeqCst);
                    if watched.load(std::sync::atomic::Ordering::SeqCst) {
                        post();
                    }
                }
            });
        }

        /// 非阻塞地取出一个待处理请求。
        pub(super) fn take_request_with(&self) -> Option<Activation> {
            // 起了等待线程之后，事件由它消费，这里改读它记下的位图。
            if self.watched.load(std::sync::atomic::Ordering::SeqCst) {
                let bits = self.pending.load(std::sync::atomic::Ordering::SeqCst);
                for (index, request) in Activation::ALL.iter().enumerate() {
                    let bit = 1u64 << index;
                    if bits & bit != 0 {
                        self.pending
                            .fetch_and(!bit, std::sync::atomic::Ordering::SeqCst);
                        return Some(*request);
                    }
                }
                return None;
            }
            for (index, request) in Activation::ALL.iter().enumerate() {
                let handle = self.events[index];
                // SAFETY: 句柄由 CreateEventW 创建并活到 Guard 析构；
                // 超时 0 表示只查询、不阻塞。
                if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
                    return Some(*request);
                }
            }
            None
        }

        pub(super) fn spawn_waker_with<F>(&self, post: F)
        where
            F: Fn() + Send + Sync + 'static,
        {
            self.spawn_waker_impl(post);
        }

        pub(super) fn forward_with(tag: &str, request: Activation) -> bool {
            let name = wide(&event_name(tag, request));
            let deadline = Instant::now() + FORWARD_TIMEOUT;            loop {
                // SAFETY: name 是以 NUL 结尾的 UTF-16；只需要置位权限。
                match unsafe { OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(name.as_ptr())) } {
                    Ok(event) => {
                        // SAFETY: 句柄来自 OpenEventW，SetEvent 只是把它置为有信号。
                        let signaled = unsafe { SetEvent(event) }.is_ok();
                        // SAFETY: 我们自己打开的句柄，自己关闭。
                        let _ = unsafe { CloseHandle(event) };
                        return signaled;
                    }
                    Err(_) if Instant::now() < deadline => {
                        std::thread::sleep(FORWARD_RETRY);
                    }
                    Err(_) => return false,
                }
            }
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: 全部句柄都由本结构拥有，且只会在这里被关闭一次。
            unsafe {
                for handle in self.events {
                    let _ = CloseHandle(handle);
                }
                let _ = CloseHandle(self.mutex);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 非 Windows 占位
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
mod imp {
    use super::{Acquire, Activation, Guard};

    impl Guard {
        pub(super) fn acquire_with(_tag: &str) -> Acquire {
            Acquire::First(Guard)
        }

        pub(super) fn take_request_with(&self) -> Option<Activation> {
            None
        }

        /// 非 Windows 没有跨进程事件，等待线程也就无从谈起。
        pub(super) fn spawn_waker_impl<F>(&self, _post: F)
        where
            F: Fn() + Send + Sync + 'static,
        {
        }

        pub(super) fn forward_with(_tag: &str, _request: Activation) -> bool {
            false
        }
    }
}

/// 单实例守卫。持有它 = 本进程是唯一实例。
#[cfg(windows)]
#[derive(Debug)]
pub struct Guard {
    mutex: windows::Win32::Foundation::HANDLE,
    /// 与 [`Activation::ALL`] 一一对应（数量由它决定，避免两处各写一个数字）。
    events: [windows::Win32::Foundation::HANDLE; Activation::ALL.len()],
    /// 等待线程记下的待处理请求位（`Activation::ALL` 的下标）；没起线程时恒为 0。
    pending: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// 是否已经起了等待线程。起了之后事件由它消费，`take_request` 改读 `pending`。
    watched: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// 单实例守卫（非 Windows 平台的占位：不支持跨进程转发）。
#[cfg(not(windows))]
#[derive(Debug)]
pub struct Guard;

impl Guard {
    /// 尝试成为唯一实例。
    #[must_use]
    pub fn acquire() -> Acquire {
        Self::acquire_with(&user_tag())
    }

    /// 是否有别的实例转达过请求（非阻塞）。
    #[must_use]
    pub fn take_request(&self) -> Option<Activation> {
        self.take_request_with()
    }

    /// 起一个等待线程：有人转发请求时，先把它记下，再调用 `post` 把 UI 捅醒。
    ///
    /// 只该调一次；重复调用会多起一条线程（第二条第第一次请求之后就白等了）。
    pub fn spawn_waker<F>(&self, post: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.spawn_waker_with(post);
    }

    /// 把请求发给已经在跑的实例。返回是否送达。
    #[must_use]
    pub fn forward(request: Activation) -> bool {
        Self::forward_with(&user_tag(), request)
    }
}

/// 等待一个请求到达的便利函数（只给测试用：生产代码在 `logic()` 里轮询）。
#[cfg(test)]
fn wait_for_request(guard: &Guard, timeout: Duration) -> Option<Activation> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(request) = guard.take_request() {
            return Some(request);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// 每个测试用自己的命名对象，避免互相抢，也避免和桌面上真正在跑的 Cuter 抢。
    fn unique_tag() -> String {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        format!(
            "test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    #[test]
    fn tag_is_sanitized_for_kernel_object_names() {
        assert_eq!(tag_from("ChuGe"), "ChuGe");
        assert_eq!(tag_from(r"DOMAIN\chu ge"), "DOMAINchuge");
        assert_eq!(tag_from("中文用户"), "default", "全是非 ASCII 时必须有兜底");
        assert_eq!(tag_from(""), "default");
    }

    #[test]
    fn kernel_object_names_are_distinct_and_scoped() {
        let tag = "alice";
        assert_eq!(mutex_name(tag), r"Local\cuter-single-alice");
        for request in Activation::ALL {
            let name = event_name(tag, request);
            assert!(name.starts_with(r"Local\cuter-req-"), "{name}");
            assert!(name.ends_with("-alice"), "{name}");
        }
        // 三种请求的名字必须互不相同，否则会串味
        let a = event_name(tag, Activation::Show);
        let b = event_name(tag, Activation::Capture);
        let c = event_name(tag, Activation::Pin);
        assert!(a != b && b != c && a != c);
    }

    /// 真实内核对象上的闭环：拿到守卫 → 转发 → 读回。
    ///
    /// 这条测试只有 Windows 上有意义（其它平台是占位实现），但它是整套机制里
    /// 唯一能证明「转发真的能送达」的东西——`forward` 走的是独立的 `OpenEventW`，
    /// 和守卫里的句柄不是同一个。
    #[test]
    #[cfg(windows)]
    fn a_forwarded_request_reaches_the_first_instance_exactly_once() {
        let tag = unique_tag();
        let Acquire::First(guard) = Guard::acquire_with(&tag) else {
            panic!("测试用的独立命名对象不应该被别人占用");
        };
        assert_eq!(guard.take_request(), None, "刚创建时不该有请求");

        assert!(Guard::forward_with(&tag, Activation::Capture), "转发应当送达");
        assert_eq!(
            wait_for_request(&guard, Duration::from_secs(2)),
            Some(Activation::Capture)
        );
        // 自动重置事件：读过一次就该回到无信号
        assert_eq!(guard.take_request(), None, "自动重置事件不应该重复触发");

        assert!(Guard::forward_with(&tag, Activation::Pin));
        assert_eq!(
            wait_for_request(&guard, Duration::from_secs(2)),
            Some(Activation::Pin)
        );
    }

    /// 没人监听时转发必须**如实失败**，而不是假装成功。
    #[test]
    #[cfg(windows)]
    fn forwarding_without_a_listener_reports_failure() {
        let tag = unique_tag();
        assert!(!Guard::forward_with(&tag, Activation::Show));
    }
}
