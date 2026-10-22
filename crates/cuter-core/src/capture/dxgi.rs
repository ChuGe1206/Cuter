//! DXGI 桌面复制（Desktop Duplication）捕获后端。
//!
//! ## 为什么要它
//!
//! 实测（HANDOFF 第 19 节，1920×1080）：整屏 `BitBlt` 是当前捕获的**全部**成本
//! ——33.3 ms，而 Rust 侧的拷贝与通道交换在合计里被完全掩盖。所以想再快只能换 API：
//! DXGI 拿的是 DWM 已经合成好的表面，预期落到 10~15 ms。
//!
//! ## 设计取舍
//!
//! * **每个输出一条 duplication，长期持有**。桌面静止时 `AcquireNextFrame` 会超时
//!   （`DXGI_ERROR_WAIT_TIMEOUT`），这时只能复用**上一次**那一帧，而那一帧只在
//!   duplication 存活期间有效；每次捕获都重建 device + duplication 要几十毫秒，
//!   等于把省下来的又花掉。代价写进文档：本程序运行期间占着这些输出的 duplication，
//!   别的录屏程序可能拿不到（它们得自己回退）。
//! * 拿不到 duplication（RDP 会话、被别的程序独占、老驱动）→ 返回错误，
//!   由上层**回退 GDI**，绝不假装成功。
//! * `DXGI_ERROR_ACCESS_LOST`（改分辨率、进全屏独占、切换用户）→ 标记这条输出需要重建，
//!   本次返回错误，下一次调用时重建。
//! * 像素布局：DXGI 给的是 `B8G8R8A8_UNORM`，与 GDI 的 BGRX 一致，都走
//!   [`Frame::from_bgrx`]（顺带把 alpha 拉成 255——屏幕像素没有有效 alpha）。
//! * **光标不在桌面复制里**（它在 `DXGI_OUTDUPL_FRAME_INFO.PointerPosition` 里单独报告），
//!   所以「包含鼠标指针」仍然走原有的系统光标位图合成路径，两边不冲突。

use std::sync::Mutex;

use windows::Win32::Foundation::{HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
};
use windows::core::Interface;

use crate::error::{Error, Result};
use crate::frame::Frame;
use crate::geom::{Rect, Size};

/// 一次 `AcquireNextFrame` 最多等多久。桌面静止时它会一直超时，所以不能等太久：
/// 我们本来就打算在超时时复用上一帧。
const ACQUIRE_TIMEOUT_MS: u32 = 8;

/// 首次抓取时"还没有任何一帧"的兜底等待：桌面完全静止时可能连续超时，
/// 这时最多重试这么多次（每次 `ACQUIRE_TIMEOUT_MS`），仍然没有就报错让上层回退 GDI。
const COLD_START_TRIES: u32 = 12;

/// 一个显示器的复制通道。
///
/// **device / context 是每条输出各带一份的**：多显卡机器上不同显示器可能挂在不同的
/// 适配器上，而 `DuplicateOutput` 要求设备与输出同属一个适配器；`CopyResource` / `Map`
/// 也必须在创建纹理的那个设备上调用。共用一份看起来能省内存，实际会在双显卡机器上失败。
struct Output {
    /// 与 [`crate::capture::MonitorInfo::name`] 相同的设备名（如 `\\.\DISPLAY1`）。
    device_name: String,
    /// 该显示器在虚拟桌面里的矩形（物理像素，可能是负坐标）。
    bounds: Rect,
    duplication: IDXGIOutputDuplication,
    /// 回读用的暂存纹理。`CopyResource` 到它，再 `Map` 读出来。
    staging: ID3D11Texture2D,
    context: ID3D11DeviceContext,
    /// 暂存纹理里是否已经有内容（首次抓取时可能一直是空的）。
    has_content: bool,
    /// 上一帧是不是"桌面没变、复用了旧内容"。
    stale: bool,
    /// 有没有丢掉"建立复制通道后的第一帧"。
    ///
    /// **必须丢**：`DuplicateOutput` 之后的第一次 `AcquireNextFrame` 给出的是**空帧**
    /// （整屏纯黑），真正的内容在下一帧。实测就是这样——探针 `dxgi_backend_e2e.ps1`
    /// 抓到 `mean 0.0 / distinct colours 1`（整张纯黑），而 GDI 同期是 `mean 251`。
    /// 只比对"第二次之后"的抓取是发现不了的，所以那条端到端探针很值。
    primed: bool,
}

/// DXGI 桌面复制后端。
///
/// **可以跨线程调用**（`ScreenCapture` 要求 `Send + Sync`：滚动截屏在自己的采集线程上
/// 调 `capture_region`）。D3D11 的 immediate context 不是线程安全的，所以所有设备调用
/// 都串行化在这把 `Mutex` 后面；我们**没有**开 `D3D11_CREATE_DEVICE_SINGLETHREADED`，
/// 因此"任意线程调用、但同一时刻只有一个"正是官方支持的用法。
pub struct DxgiCapture {
    outputs: Outputs,
}

/// COM 接口在 windows-rs 里默认是 `!Send + !Sync`（COM 有套间规则）。
///
/// 这里由 `Mutex` 保证同一时刻只有一个线程在碰这些设备对象，符合 D3D11 的线程模型，
/// 所以显式声明可以跨线程传递。**改动这里之前先想清楚并发**。
struct Outputs(Mutex<Vec<Output>>);

// SAFETY: 见上：所有对 `Output` 内部 COM 对象的访问都经过 `Outputs::0` 这把互斥锁。
unsafe impl Send for Outputs {}
unsafe impl Sync for Outputs {}

impl DxgiCapture {
    /// 建立每个显示器一条复制通道。**任何一步失败都直接返回错误**，由上层回退 GDI。
    pub fn new() -> Result<Self> {
        // SAFETY: 下面全部是标准的 DXGI / D3D11 初始化序列，句柄由 windows crate 的
        // COM 包装管理；没有裸指针越界或生命周期问题。
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
            let mut outputs = Vec::new();
            // 每个没能建起来的输出都记一条原因：`DuplicateOutput` 失败的原因有好几种
            // （RDP 会话不支持 / 已被别的程序独占 / 权限不足），报错时必须能区分，
            // 否则只能看到一句"没有显示器能建立桌面复制"。
            let mut reasons: Vec<String> = Vec::new();

            let mut adapter_index = 0u32;
            while let Ok(adapter) = factory.EnumAdapters1(adapter_index) {
                adapter_index += 1;
                // **先看这个适配器上有没有输出，再建设备**：建 D3D11 设备在这台机器上
                // 实测要几百毫秒（合计 1.76 s），给每个空适配器都建一遍纯属浪费
                // （本机 3 个适配器里只有 1 个接了显示器）。
                if !adapter_has_output(&adapter) {
                    continue;
                }
                // 设备必须建在**与输出同一个适配器**上，否则 `DuplicateOutput` 会失败。
                let (device, context) = match create_device(&adapter) {
                    Ok(pair) => pair,
                    Err(err) => {
                        reasons.push(format!("适配器 {adapter_index} 建 D3D11 设备失败：{err}"));
                        continue;
                    }
                };

                let mut output_index = 0u32;
                while let Ok(output) = adapter.EnumOutputs(output_index) {
                    output_index += 1;
                    let desc = match output.GetDesc() {
                        Ok(desc) => desc,
                        Err(err) => {
                            reasons.push(format!("取输出描述失败：{err}"));
                            continue;
                        }
                    };
                    if !desc.AttachedToDesktop.as_bool() {
                        continue;
                    }
                    let device_name = utf16_to_string(&desc.DeviceName);
                    let r: RECT = desc.DesktopCoordinates;
                    let bounds = Rect::new(
                        r.left,
                        r.top,
                        (r.right - r.left) as u32,
                        (r.bottom - r.top) as u32,
                    );
                    if bounds.is_empty() {
                        continue;
                    }

                    let output1: IDXGIOutput1 = match output.cast() {
                        Ok(output1) => output1,
                        Err(err) => {
                            reasons.push(format!("{device_name}: IDXGIOutput1 转换失败（{err}）"));
                            continue;
                        }
                    };
                    let duplication = match output1.DuplicateOutput(&device) {
                        Ok(duplication) => duplication,
                        Err(err) => {
                            reasons.push(format!(
                                "{device_name}: DuplicateOutput 失败（{} / {:#x}）",
                                err.message(),
                                err.code().0
                            ));
                            continue;
                        }
                    };
                    let staging = create_staging_texture(&device, bounds.size())?;
                    log::info!(
                        "DXGI 桌面复制就绪：{device_name} {}×{} @({},{})",
                        bounds.w,
                        bounds.h,
                        bounds.x,
                        bounds.y
                    );
                    outputs.push(Output {
                        device_name,
                        bounds,
                        duplication,
                        staging,
                        // 同一适配器上的多个输出共用这个 context（COM 指针 clone 只是加引用计数）
                        context: context.clone(),
                        has_content: false,
                        stale: false,
                        primed: false,
                    });
                }
            }

            if outputs.is_empty() {
                return Err(Error::Unavailable(format!(
                    "没有任何显示器能建立 DXGI 桌面复制（{}）",
                    if reasons.is_empty() {
                        "没有找到已连接到桌面的输出".to_owned()
                    } else {
                        reasons.join("；")
                    }
                )));
            }
            Ok(Self {
                outputs: Outputs(Mutex::new(outputs)),
            })
        }
    }

    /// 该后端覆盖的显示器矩形（并集）。
    #[must_use]
    pub fn virtual_bounds(&self) -> Rect {
        self.lock()
            .iter()
            .fold(None, |acc: Option<Rect>, output| {
                Some(match acc {
                    None => output.bounds,
                    Some(acc) => acc.union(output.bounds),
                })
            })
            .unwrap_or(Rect::ZERO)
    }

    /// 抓一块区域。跨显示器时逐条输出抓取再拼接。
    pub fn grab(&self, region: Rect) -> Result<Frame> {
        if region.is_empty() {
            return Err(Error::EmptyRegion(region));
        }
        // 目标帧先按 BGRA 布局铺好，最后统一做一次通道交换（见 `Frame::from_bgrx`）
        let mut target = Frame::new(region.size());
        let mut touched = 0usize;

        let mut outputs = self.lock();
        for output in outputs.iter_mut() {
            let Some(inter) = intersect(output.bounds, region) else {
                continue;
            };
            touched += 1;
            // 先把矩形拷出来：闭包里要用它，而 `with_surface` 需要 `&mut output`
            let src_bounds = output.bounds;
            output.with_surface(COLD_START_TRIES, |pixels, pitch| {
                copy_subrect(pixels, pitch, src_bounds, &mut target, region, inter);
            })?;
        }
        drop(outputs);

        if touched == 0 {
            return Err(Error::EmptyRegion(region));
        }
        Frame::from_bgrx(region.size(), target.into_data())
    }

    /// 当前可用的输出数量（诊断用）。
    #[must_use]
    pub fn output_count(&self) -> usize {
        self.lock().len()
    }

    /// 每个输出的设备名 + 矩形（诊断用）。
    #[must_use]
    pub fn output_list(&self) -> Vec<(String, Rect)> {
        self.lock()
            .iter()
            .map(|output| (output.device_name.clone(), output.bounds))
            .collect()
    }

    /// 拿锁时容忍中毒：这里保护的只是设备对象，中毒说明上次调用 panic 了，
    /// 但继续用剩下的对象比让整个程序挂掉更合理。
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Output>> {
        self.outputs
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Output {
    /// 确保暂存纹理里有一帧，然后把映射出来的像素交给 `f`（BGRA，行距 `pitch`）。
    fn with_surface<R>(&mut self, cold_tries: u32, f: impl FnOnce(&[u8], u32) -> R) -> Result<R> {
        // SAFETY: 所有调用都在同一个线程上顺序执行；`Map`/`Unmap` 严格配对；
        // `mapped.pData` 在 `Unmap` 之前一直有效，闭包不会把它带出去。
        unsafe {
            let mut tries = 0u32;
            loop {
                tries += 1;
                let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
                let mut resource: Option<IDXGIResource> = None;
                match self
                    .duplication
                    .AcquireNextFrame(ACQUIRE_TIMEOUT_MS, &mut info, &mut resource)
                {
                    Ok(()) => {
                        let texture: ID3D11Texture2D = match resource
                            .ok_or_else(|| Error::Unavailable("桌面复制没有返回资源".to_owned()))?
                            .cast()
                        {
                            Ok(texture) => texture,
                            Err(err) => {
                                let _ = self.duplication.ReleaseFrame();
                                return Err(Error::WinRt(err));
                            }
                        };
                        // 第一帧是空的，丢掉它再要一帧（见 `primed` 的说明）
                        if !self.primed {
                            self.primed = true;
                            let _ = self.duplication.ReleaseFrame();
                            continue;
                        }
                        self.context.CopyResource(&self.staging, &texture);
                        let _ = self.duplication.ReleaseFrame();
                        self.has_content = true;
                        self.stale = false;
                        break;
                    }
                    Err(err) if err.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                        // 桌面没变：暂存纹理里还是上一次的内容，直接复用
                        if self.has_content {
                            self.stale = true;
                            break;
                        }
                        if tries >= cold_tries {
                            return Err(Error::Unsupported(
                                "桌面复制一直超时（拿不到第一帧），这次改用 GDI",
                            ));
                        }
                    }
                    Err(err) if err.code() == DXGI_ERROR_ACCESS_LOST => {
                        // 分辨率变了 / 进了全屏独占 / 切了用户：这条通道作废
                        self.has_content = false;
                        return Err(Error::Unsupported(
                            "桌面复制失效（ACCESS_LOST），下次会重建",
                        ));
                    }
                    Err(err) => {
                        return Err(Error::WinRt(err));
                    }
                }
            }

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(Error::WinRt)?;
            let len = (mapped.RowPitch as usize) * self.bounds.h as usize;
            // SAFETY: `Map` 成功时 `pData` 指向至少 `RowPitch * Height` 字节的可读内存
            let pixels = std::slice::from_raw_parts(mapped.pData.cast::<u8>(), len);
            let result = f(pixels, mapped.RowPitch);
            self.context.Unmap(&self.staging, 0);
            Ok(result)
        }
    }
}

/// 把 `src`（整块显示器，行距 `pitch`）里 `inter` 那块拷进 `target`（以 `target_origin` 为原点的 BGRA 缓冲）。
fn copy_subrect(
    src: &[u8],
    pitch: u32,
    src_bounds: Rect,
    target: &mut Frame,
    target_origin: Rect,
    inter: Rect,
) {
    let row_bytes = (inter.w as usize) * 4;
    let src_x = (inter.x - src_bounds.x).max(0) as usize;
    let src_y = (inter.y - src_bounds.y).max(0) as usize;
    let dst_x = (inter.x - target_origin.x).max(0) as usize;
    let dst_y = (inter.y - target_origin.y).max(0) as usize;
    let target_w = target.width() as usize;

    for row in 0..inter.h as usize {
        let src_offset = (src_y + row) * pitch as usize + src_x * 4;
        let dst_offset = ((dst_y + row) * target_w + dst_x) * 4;
        let src_end = src_offset + row_bytes;
        let dst_end = dst_offset + row_bytes;
        if src_end > src.len() || dst_end > target.data().len() {
            log::warn!("桌面复制的行越界，跳过这一行（源 {src_offset}..{src_end}）");
            continue;
        }
        target.data_mut()[dst_offset..dst_end].copy_from_slice(&src[src_offset..src_end]);
    }
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x + a.w as i32).min(b.x + b.w as i32);
    let bottom = (a.y + a.h as i32).min(b.y + b.h as i32);
    if right <= left || bottom <= top {
        return None;
    }
    Some(Rect::new(
        left,
        top,
        (right - left) as u32,
        (bottom - top) as u32,
    ))
}

/// 这个适配器上是否挂着"已连接到桌面"的输出。
///
/// 用来避免给没有显示器的适配器白建 D3D11 设备（实测建一个要几百毫秒）。
///
/// # Safety
/// 调用方保证 `adapter` 有效。
unsafe fn adapter_has_output(adapter: &IDXGIAdapter1) -> bool {
    let mut index = 0u32;
    // SAFETY: 只枚举输出；失败就当没有。
    while let Ok(output) = unsafe { adapter.EnumOutputs(index) } {
        index += 1;
        if let Ok(desc) = unsafe { output.GetDesc() }
            && desc.AttachedToDesktop.as_bool()
        {
            return true;
        }
    }
    false
}

/// 建 D3D11 设备（BGRA 支持是 `DuplicateOutput` 的硬要求）。
///
/// # Safety
/// 调用方保证 `adapter` 是有效的 DXGI 适配器。
unsafe fn create_device(
    adapter: &IDXGIAdapter1,
) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let levels = [D3D_FEATURE_LEVEL(0xb000), D3D_FEATURE_LEVEL(0xa000)];
    unsafe {
        D3D11CreateDevice(
            adapter,
            // ⚠️ 传了具体适配器时**必须是 UNKNOWN**：`HARDWARE` 只能与"适配器为 NULL"搭配，
            // 否则 `D3D11CreateDevice` 直接返回 `E_INVALIDARG (0x80070057)`（实测踩过，
            // 而且那句"参数错误"完全看不出跟适配器有关）。
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    match (device, context) {
        (Some(device), Some(context)) => Ok((device, context)),
        _ => Err(Error::Unsupported("D3D11 设备创建不完整")),
    }
}

/// 建一张 CPU 可读的暂存纹理（回读用）。
///
/// # Safety
/// `device` 必须有效。
unsafe fn create_staging_texture(device: &ID3D11Device, size: Size) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size.w,
        Height: size.h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut texture: Option<ID3D11Texture2D> = None;
    unsafe {
        device.CreateTexture2D(&desc, None, Some(&mut texture))?;
    }
    texture.ok_or(Error::Unsupported("暂存纹理创建失败"))
}

fn utf16_to_string(raw: &[u16]) -> String {
    let end = raw.iter().position(|c| *c == 0).unwrap_or(raw.len());
    String::from_utf16_lossy(&raw[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersect_matches_expected_regions() {
        let a = Rect::new(0, 0, 100, 100);
        let b = Rect::new(50, 50, 100, 100);
        assert_eq!(intersect(a, b), Some(Rect::new(50, 50, 50, 50)));
        assert_eq!(intersect(a, Rect::new(200, 200, 10, 10)), None);
        // 负坐标（左边那台显示器）也要对
        let left = Rect::new(-1920, 0, 1920, 1080);
        assert_eq!(
            intersect(left, Rect::new(-2000, 100, 200, 200)),
            Some(Rect::new(-1920, 100, 120, 200))
        );
    }

    /// 真机验证：DXGI 与 GDI 抓同一块区域，逐像素比对，并把两者的耗时打出来。
    ///
    /// 只打印不硬断言"完全一致"：两次抓取之间隔着几十毫秒，时钟秒数、动画、鼠标都可能变。
    /// 但**系统性错误**（格式/通道错序、坐标偏移、拿到的是黑帧或旧帧）会让偏差远超阈值，
    /// 所以这里卡两个宽松但有意义的门槛：平均偏差 < 6、偏差 ≤ 8 的像素占比 ≥ 90%。
    ///
    /// 跑法：`cargo test -p cuter-core dxgi_vs_gdi --release -- --ignored --nocapture`
    #[test]
    #[ignore = "需要交互式桌面，且要独占屏幕；只在手动验证后端时跑"]
    fn dxgi_vs_gdi_matches_and_times() {
        use crate::capture::{desktop_capture, desktop_capture_with, CaptureBackend};
        use std::time::Instant;

        let gdi = desktop_capture().expect("GDI 后端");
        let start = Instant::now();
        let dxgi = desktop_capture_with(CaptureBackend::Dxgi).expect("DXGI 后端");
        let dxgi_setup_ms = start.elapsed().as_secs_f64() * 1000.0;
        println!("GDI  后端：{}", gdi.backend_name());
        println!("DXGI 后端：{}", dxgi.backend_name());
        println!("DXGI 建立（设备 + 桌面复制）：{dxgi_setup_ms:.1} ms");

        // 整屏：这是热键延迟里真正花掉的那一段，与 HANDOFF 第 19 节的 33.3 ms 对比。
        // **第一轮也要逐像素比对**：桌面复制"建立后的第一帧是空帧"这个坑，
        // 只比对后面的抓取是发现不了的（当时就漏过一次，靠端到端探针才抓到）。
        for round in 0..3 {
            let start = Instant::now();
            let gdi_shot = gdi.capture_desktop().expect("GDI 整屏");
            let gdi_full = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            let dxgi_shot = dxgi.capture_desktop().expect("DXGI 整屏");
            let dxgi_full = start.elapsed().as_secs_f64() * 1000.0;
            println!(
                "整屏第 {} 轮：GDI {:.1} ms / DXGI {:.1} ms（{}×{}）",
                round + 1,
                gdi_full,
                dxgi_full,
                dxgi_shot.frame.width(),
                dxgi_shot.frame.height()
            );
            assert_eq!(gdi_shot.bounds, dxgi_shot.bounds, "两种后端的虚拟桌面范围必须一致");
            let (mean, close_ratio, worst) = compare(&gdi_shot.frame, &dxgi_shot.frame);
            println!(
                "          平均偏差 {mean:.2} · ≤8 的占比 {:.1}% · 最大 {worst}",
                close_ratio * 100.0
            );
            assert!(
                mean < 6.0 && close_ratio > 0.9,
                "整屏第 {} 轮的像素偏差过大（平均 {mean:.2}，≤8 占比 {:.1}%，最大 {worst}）\
                 ——如果差得离谱，多半是拿到了桌面复制的\"第一帧空帧\"",
                round + 1,
                close_ratio * 100.0
            );
        }

        let monitors = gdi.monitors().expect("显示器列表");
        let bounds = crate::capture::virtual_bounds(&monitors);
        // 取虚拟桌面中间一块，避开任务栏与屏幕边缘
        let region = Rect::new(
            bounds.x + (bounds.w as i32 / 4),
            bounds.y + (bounds.h as i32 / 4),
            (bounds.w / 2).max(64),
            (bounds.h / 2).max(64),
        );
        println!("比对区域：{region:?}");

        let rounds = 5;
        let mut gdi_ms = 0.0f64;
        let mut dxgi_ms = 0.0f64;
        for round in 0..rounds {
            let start = Instant::now();
            let gdi_frame = gdi.capture_region(region).expect("GDI 抓取");
            let gdi_elapsed = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            let dxgi_frame = dxgi.capture_region(region).expect("DXGI 抓取");
            let dxgi_elapsed = start.elapsed().as_secs_f64() * 1000.0;
            gdi_ms += gdi_elapsed;
            dxgi_ms += dxgi_elapsed;

            assert_eq!(gdi_frame.size(), dxgi_frame.size(), "两种后端给的尺寸必须一致");
            assert!(
                dxgi_frame.data().chunks(4).all(|px| px[3] == 255),
                "DXGI 帧的 alpha 必须是不透明的"
            );

            let (mean, close_ratio, worst) = compare(&gdi_frame, &dxgi_frame);
            println!(
                "第 {} 轮：GDI {:.1} ms / DXGI {:.1} ms · 平均偏差 {:.2} · ≤8 的占比 {:.1}% · 最大 {}",
                round + 1,
                gdi_elapsed,
                dxgi_elapsed,
                mean,
                close_ratio * 100.0,
                worst
            );
            assert!(
                mean < 6.0 && close_ratio > 0.9,
                "DXGI 与 GDI 的像素偏差过大（平均 {mean:.2}，≤8 占比 {:.1}%）——通道顺序或坐标映射可能有问题",
                close_ratio * 100.0
            );
        }
        println!(
            "平均：GDI {:.1} ms / DXGI {:.1} ms（{} 轮）",
            gdi_ms / f64::from(rounds),
            dxgi_ms / f64::from(rounds),
            rounds
        );
    }

    /// 逐像素比对两帧，返回（平均偏差、偏差 ≤8 的占比、最大偏差）。
    ///
    /// 只看 RGB：屏幕像素的 alpha 没有意义（DXGI 给 0xFF，GDI 给 0x00）。
    fn compare(a: &Frame, b: &Frame) -> (f64, f64, u8) {
        assert_eq!(a.size(), b.size(), "比对的两帧尺寸必须一致");
        let mut sum = 0u64;
        let mut close = 0u64;
        let mut total = 0u64;
        let mut worst = 0u8;
        for (pa, pb) in a.data().chunks(4).zip(b.data().chunks(4)) {
            for channel in 0..3 {
                let diff = pa[channel].abs_diff(pb[channel]);
                sum += u64::from(diff);
                if diff <= 8 {
                    close += 1;
                }
                worst = worst.max(diff);
                total += 1;
            }
        }
        if total == 0 {
            return (0.0, 1.0, 0);
        }
        (sum as f64 / total as f64, close as f64 / total as f64, worst)
    }

    #[test]
    fn copy_subrect_gathers_into_the_right_place() {
        // 源 4x2（BGRA），行距故意比行宽大，模拟 D3D 的 RowPitch 对齐
        let pitch = 4 * 8; // 8 像素/行
        let mut src = vec![0u8; pitch * 2];
        for y in 0..2usize {
            for x in 0..4usize {
                let offset = y * pitch + x * 4;
                src[offset] = (x + y * 10) as u8;
            }
        }
        let mut target = Frame::new(Size::new(4, 1));
        // 目标区域是「y=1 那一行」，源里取 (1,1) 起的 3 个像素：
        // 于是 dst_y 必须是 0（相对目标），src_y 是 1 —— 正是容易写错的地方
        copy_subrect(
            &src,
            pitch as u32,
            Rect::new(0, 0, 4, 2),
            &mut target,
            Rect::new(0, 1, 4, 1),
            Rect::new(1, 1, 3, 1),
        );
        // 目标第 0 列留空（源从 x=1 开始），源 (1,1)/(2,1)/(3,1) 落在目标第 1/2/3 列。
        // 源里的值是 `x + y*10`，所以这一行读到 11 / 12 / 13。
        let pixel = |index: usize| target.data()[index * 4..index * 4 + 4].to_vec();
        assert_eq!(pixel(0), vec![0, 0, 0, 0], "目标第 0 列不该有数据");
        assert_eq!(pixel(1), vec![11, 0, 0, 0], "第 1 列应是源的 (1,1)");
        assert_eq!(pixel(2), vec![12, 0, 0, 0], "第 2 列应是源的 (2,1)");
        assert_eq!(pixel(3), vec![13, 0, 0, 0], "第 3 列应是源的 (3,1)");
    }
}
