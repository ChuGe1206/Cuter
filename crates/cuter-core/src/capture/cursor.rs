//! 鼠标光标位图提取与合成。
//!
//! 屏幕 `BitBlt` **不会**包含鼠标指针：光标由桌面窗口管理器在窗口合成之后直接叠上去，
//! 它既不属于任何窗口的绘制结果，也不在屏幕 DC 里。因此截图设置里的「包含鼠标指针」
//! 开关必须在截图之后单独取一次光标位图再合成回来，本模块就是干这件事的：
//!
//! * [`current_cursor`]：`GetCursorInfo` + `GetIconInfo` 拿到光标的颜色位图、尺寸与
//!   热点，还原出**带真实 alpha** 的 RGBA [`Frame`]，并裁剪掉四周全透明的空白；
//! * [`composite_cursor`]：把该位图按「全局坐标 → 帧内局部坐标」的平移用
//!   [`Frame::blend`] 做一次直通 alpha 的 source-over 合成。
//!
//! 设计原则：**光标相关的任何失败都只是「这次没画上指针」**。取不到时返回 `Ok(false)`
//! 并记一条 debug 日志，绝不让整张截图失败（`composite_cursor` 因此也不会 panic）。
//!
//! 已知限制：
//!
//! * 老式光标（`.cur` 里的 1bpp AND/XOR 两位掩码）没有 alpha 通道，这里用 AND 掩码
//!   近似出 alpha：掩码位 0 视为不透明、位 1 视为透明。被 XOR 掩码取反的像素本应是
//!   白色，按这个近似会变成透明；
//! * 光标位图由系统按当前 DPI 生成，缩放变化时拿到的是不同尺寸的位图（32/48/64…），
//!   这里不做重采样，直接按原尺寸合成，因此截图与屏幕像素仍然一一对应。
//!
//! 所有 Win32 句柄都由 RAII guard 持有（内存 DC、`SelectObject` 的还原、`GetIconInfo`
//! 新建的两个位图），任何提前 `return` 或 `?` 都不会泄漏资源。

use crate::error::Result;
use crate::frame::Frame;
use crate::geom::{Point, Rect};

#[cfg(any(windows, test))]
use crate::error::Error;
#[cfg(any(windows, test))]
use crate::geom::Size;

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::mem::size_of;

#[cfg(windows)]
use windows::Win32::Foundation::GetLastError;
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CURSOR_SHOWING, CURSORINFO, GetCursorInfo, GetIconInfo, HCURSOR, HICON, ICONINFO,
};

/// 当前鼠标光标：位图 + 热点 + 屏幕位置。
#[derive(Clone, Debug)]
pub struct CursorImage {
    pub image: Frame,
    /// 热点在 `image` 内的相对坐标（光标"尖"的位置）。
    pub hotspot: Point,
    /// 取到光标那一刻系统报告的光标位置（全局物理坐标）。
    ///
    /// 调用方没提供位置时用它作为回退（见 [`composite_cursor`]）；裁剪位图不会改变
    /// 这个值，它始终指向同一个屏幕点。
    pub screen_pos: Point,
}

// ---------------------------------------------------------------------------
// 合成
// ---------------------------------------------------------------------------

/// 把光标按 alpha 合成到冻结帧上。
///
/// `desktop_bounds` 是该帧对应的全局矩形，`cursor_pos` 是光标的全局坐标；
/// 返回是否真的画了东西。
///
/// `cursor_pos` 为 `None` 时退回 [`CursorImage::screen_pos`]（即捕获光标位图那一瞬间
/// 的系统光标位置）。本函数永不 panic：光标取不到、位置拿不到、完全落在帧外，都只是
/// 返回 `Ok(false)` 并记一条 debug 日志。
pub fn composite_cursor(
    frame: &mut Frame,
    desktop_bounds: Rect,
    cursor_pos: Option<Point>,
) -> Result<bool> {
    if frame.is_empty() {
        return Ok(false);
    }

    // 取不到光标位图不值得让整张截图失败，任何错误都退化成「没画指针」。
    let cursor = match current_cursor() {
        Ok(Some(cursor)) => cursor,
        Ok(None) => {
            log::debug!("系统当前没有可见光标，本次截图不含鼠标指针");
            return Ok(false);
        }
        Err(err) => {
            log::debug!("取光标位图失败，本次截图不含鼠标指针：{err}");
            return Ok(false);
        }
    };

    Ok(composite_with(frame, &cursor, desktop_bounds, cursor_pos))
}

/// 用**给定的**光标位图做合成（注入点：单元测试借此验证几何，无需真实光标）。
///
/// 返回是否真的覆盖到了帧的像素：光标整个落在帧外时返回 `false`，
/// 部分越界由 [`Frame::blend`] 内部裁剪。
fn composite_with(
    frame: &mut Frame,
    cursor: &CursorImage,
    desktop_bounds: Rect,
    cursor_pos: Option<Point>,
) -> bool {
    if frame.is_empty() || cursor.image.is_empty() {
        return false;
    }
    let cursor_pos = cursor_pos.unwrap_or(cursor.screen_pos);
    let at = cursor_origin(cursor_pos, cursor.hotspot, desktop_bounds);
    let target = Rect::new(at.x, at.y, cursor.image.width(), cursor.image.height());
    if !frame.bounds().intersects(target) {
        log::debug!(
            "光标 {cursor_pos:?}（热点 {:?}）落在大屏范围 {desktop_bounds:?} 之外，跳过合成",
            cursor.hotspot
        );
        return false;
    }
    frame.blend(&cursor.image, at);
    true
}

/// 光标位图左上角在帧内的局部坐标。
///
/// 全局坐标 → 帧内局部坐标的平移：先减去热点（热点是光标位图内那个"尖"的位置），
/// 再减去帧原点（多显示器布局下 `desktop_bounds` 的 origin 可能是负数）。
///
/// 结果允许越界——越界裁剪是 [`Frame::blend`] 的职责，这里只做纯算术，
/// 便于单独测试。用 `i32` 运算，负数坐标系下不会出现无符号回绕。
fn cursor_origin(cursor_pos: Point, hotspot: Point, desktop_bounds: Rect) -> Point {
    Point::new(
        cursor_pos.x - hotspot.x - desktop_bounds.x,
        cursor_pos.y - hotspot.y - desktop_bounds.y,
    )
}

// ---------------------------------------------------------------------------
// 光标位图提取
// ---------------------------------------------------------------------------

/// 取当前光标位图；光标不可获取时返回 `Ok(None)`。
///
/// `Ok(None)` 覆盖这些**正常**情况：没有输入桌面（服务会话）、光标被隐藏
/// （`CURSOR_SHOWING` 未置位，例如全屏视频或游戏里）、系统给的是「不可见的空光标」
/// 或纯单色光标（`hbmColor` 为空）、位图尺寸为 0。真正的 Win32 调用失败
/// （`GetCursorInfo` / `GetIconInfo` / `GetObjectW` / `GetDIBits`）才返回 `Err`。
#[cfg(windows)]
pub fn current_cursor() -> Result<Option<CursorImage>> {
    let Some((cursor, screen_pos)) = query_cursor_info()? else {
        return Ok(None);
    };
    // SAFETY: cursor 来自 GetCursorInfo 的成功返回，是系统持有的光标句柄，
    // 调用方不负责释放，这里只借用它查询图标信息。
    let icon = IconInfo::acquire(cursor)?;
    build_cursor_image(&icon, screen_pos)
}

/// 非 Windows 平台：光标合成尚未实现，调用方会退化成「不含鼠标指针」。
#[cfg(not(windows))]
pub fn current_cursor() -> Result<Option<CursorImage>> {
    Ok(None)
}

/// 只取当前鼠标的屏幕位置（全局物理坐标），不需要光标位图。
///
/// 「把剪贴板图片贴到鼠标位置」这类功能只需要位置，为此抓一整张屏太浪费，
/// 所以单独暴露一个廉价的查询。系统调用失败时返回 `None`。
#[cfg(windows)]
#[must_use]
pub fn cursor_position() -> Option<Point> {
    let (_, position) = query_cursor_info().ok().flatten()?;
    Some(position)
}

/// 非 Windows 平台：拿不到光标位置。
#[cfg(not(windows))]
#[must_use]
pub fn cursor_position() -> Option<Point> {
    None
}

/// 询问系统当前光标句柄与位置；光标不可用时返回 `Ok(None)`。
#[cfg(windows)]
fn query_cursor_info() -> Result<Option<(HCURSOR, Point)>> {
    let mut info = CURSORINFO {
        // cbSize 是 Win32 的版本协商字段，必须精确等于结构体大小。
        cbSize: size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: info 已填好 cbSize，且是有效的可写 CURSORINFO。
    unsafe { GetCursorInfo(&raw mut info) }.map_err(|err| win32_error("GetCursorInfo", &err))?;

    if (info.flags.0 & CURSOR_SHOWING.0) == 0 {
        log::debug!(
            "光标当前不可见（flags = {:#x}），本次不合成光标",
            info.flags.0
        );
        return Ok(None);
    }
    if info.hCursor.is_invalid() {
        log::debug!("CURSORINFO 未返回有效光标句柄，本次不合成光标");
        return Ok(None);
    }

    Ok(Some((
        info.hCursor,
        Point::new(info.ptScreenPos.x, info.ptScreenPos.y),
    )))
}

/// 从 `ICONINFO` 的位图读出像素，构造带 alpha 的光标图。
#[cfg(windows)]
fn build_cursor_image(icon: &IconInfo, screen_pos: Point) -> Result<Option<CursorImage>> {
    let color = icon.color();
    if color.is_invalid() {
        // 系统在这些情况下会返回 hbmColor 为空的 ICONINFO：纯单色光标（只有 AND/XOR
        // 掩码），以及「不可见的空光标」——后者在隐藏了鼠标的会话里很常见
        // （实测：CURSOR_SHOWING 已置位、hCursor 有效，但 hbmColor = NULL、
        // hbmMask 有效、热点非 0）。两种情况都按「不合成」处理。
        log::debug!("光标没有颜色位图（单色光标或不可见的空光标），本次不合成光标");
        return Ok(None);
    }

    let Some(size) = bitmap_size(color)? else {
        log::debug!("光标位图尺寸为 0，本次不合成光标");
        return Ok(None);
    };

    let bgra = read_bitmap_bgra(color, size)?;
    // from_bgrx 负责 BGRA→RGBA 的通道交换，但它会把第 4 通道强制成 255
    // （屏幕捕获没有有效 alpha，而光标位图有），所以先克隆一份原始字节，
    // 之后再把真正的 alpha 写回去。光标位图很小（几十 KB），这次拷贝不心疼。
    let mut frame = Frame::from_bgrx(size, bgra.clone())?;
    restore_alpha(&mut frame, &bgra)?;

    if !has_opaque_pixel(&frame) {
        log::debug!("光标位图 alpha 全为 0（老式光标只有 1 位掩码），改用 AND 掩码生成 alpha");
        apply_mask_alpha(&mut frame, icon.mask());
    }

    // 兜底：alpha 全 0 的光标合成上去什么都看不见，还不如按全不透明处理。
    if !has_opaque_pixel(&frame) {
        log::debug!("光标位图没有任何不透明像素，按全不透明处理");
        for px in frame.data_mut().as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
    }

    let (image, hotspot) = crop_to_opaque(frame, icon.hotspot());
    Ok(Some(CursorImage {
        image,
        hotspot,
        screen_pos,
    }))
}

/// `GetIconInfo` 借出的两个位图；`Drop` 时归还（`DeleteObject`）。
///
/// `GetIconInfo` 会**新建** `hbmColor` / `hbmMask` 两张位图并把所有权交给调用方，
/// 不删除就是每次截图泄漏两个 GDI 对象，最终耗尽进程的 GDI 句柄配额。
#[cfg(windows)]
struct IconInfo(ICONINFO);

#[cfg(windows)]
impl IconInfo {
    /// 查询光标图标信息。
    fn acquire(cursor: HCURSOR) -> Result<Self> {
        let mut info = ICONINFO::default();
        // SAFETY: info 是有效的可写 ICONINFO；HCURSOR 与 HICON 在 Win32 里是同一种
        // 句柄（都是 HICON），windows-rs 把它们分成了两个 newtype，因此这里按位转换。
        unsafe { GetIconInfo(HICON(cursor.0), &raw mut info) }
            .map_err(|err| win32_error("GetIconInfo", &err))?;
        Ok(Self(info))
    }

    /// 颜色位图（32bpp 带 alpha；老式光标里它是 XOR 掩码）。
    fn color(&self) -> HBITMAP {
        self.0.hbmColor
    }

    /// 单色 AND 掩码。
    fn mask(&self) -> HBITMAP {
        self.0.hbmMask
    }

    /// 热点：光标位图内那个"尖"的像素坐标。
    fn hotspot(&self) -> Point {
        // Win32 给的是 u32，但语义上不会超过位图尺寸（远小于 i32::MAX）。
        Point::new(self.0.xHotspot as i32, self.0.yHotspot as i32)
    }
}

#[cfg(windows)]
impl Drop for IconInfo {
    fn drop(&mut self) {
        for bitmap in [self.0.hbmColor, self.0.hbmMask] {
            // 单色光标可能只有其中一张，空句柄跳过（DeleteObject(NULL) 会失败）。
            if bitmap.is_invalid() {
                continue;
            }
            // SAFETY: 位图由 GetIconInfo 新建，本结构体是它的唯一所有者，
            // 因此只会删除一次；调用方已在删除前读完像素。
            let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
        }
    }
}

/// 读取位图的宽高；`GetObjectW` 失败返回 `Err`，宽高为 0 返回 `Ok(None)`。
#[cfg(windows)]
fn bitmap_size(bitmap: HBITMAP) -> Result<Option<Size>> {
    let mut bmp = BITMAP::default();
    // SAFETY: bmp 是有效的可写 BITMAP，第二个参数与它的字节大小一致；
    // bitmap 由 IconInfo 持有，在本函数返回前一直有效。
    let written = unsafe {
        GetObjectW(
            HGDIOBJ(bitmap.0),
            size_of::<BITMAP>() as i32,
            Some((&raw mut bmp).cast::<c_void>()),
        )
    };
    if written == 0 {
        return Err(Error::Win32 {
            api: "GetObjectW",
            code: last_error(),
        });
    }
    if bmp.bmWidth <= 0 || bmp.bmHeight <= 0 {
        return Ok(None);
    }
    Ok(Some(Size::new(bmp.bmWidth as u32, bmp.bmHeight as u32)))
}

/// 把位图读成 32bpp **顶向下** 的 BGRA 缓冲区（长度恰为 `size.rgba_len()`）。
///
/// 先按「位图选入内存 DC」的常见写法试一次，失败再按文档要求（`GetDIBits` 时位图
/// 不得被选入 DC）不选入重试——两种写法在实测里的差别见 `get_dibits_selected_vs_unselected`
/// 这个忽略测试的打印输出。
#[cfg(windows)]
fn read_bitmap_bgra(bitmap: HBITMAP, size: Size) -> Result<Vec<u8>> {
    let mut bmi = target_bmi(size);
    let mut buffer = vec![0u8; size.rgba_len()];
    let dc = MemoryDc::create()?;

    let mut lines = get_dibits(&dc, bitmap, size, &mut bmi, &mut buffer, true);
    if lines == 0 {
        log::debug!("位图选入 DC 后 GetDIBits 失败（文档不允许该用法），改为不选入重试");
        lines = get_dibits(&dc, bitmap, size, &mut bmi, &mut buffer, false);
    }
    if lines == 0 {
        return Err(Error::Win32 {
            api: "GetDIBits",
            code: last_error(),
        });
    }
    if lines != size.h as i32 {
        // 极少见：GDI 只换了部分扫描行，剩下的仍是 0（透明），不会画错位置。
        log::debug!("GetDIBits 只读回 {lines} / {} 行扫描线", size.h);
    }
    Ok(buffer)
}

/// 目标 DIB 格式：32bpp、负高度（顶向下）、`BI_RGB`，行序与 [`Frame`] 完全一致。
#[cfg(windows)]
fn target_bmi(size: Size) -> BITMAPINFO {
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = size.w as i32;
    // 负高度 = 顶向下：第 0 行就是位图最上面那一行，免去上下翻转。
    bmi.bmiHeader.biHeight = -(size.h as i32);
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    // windows-rs 0.62 的不一致：字段是裸 u32，而 BI_RGB 是 newtype，要取 .0。
    bmi.bmiHeader.biCompression = BI_RGB.0;
    bmi
}

/// 调用一次 `GetDIBits` 并返回它报告的扫描行数（0 表示失败）。
#[cfg(windows)]
fn get_dibits(
    dc: &MemoryDc,
    bitmap: HBITMAP,
    size: Size,
    bmi: &mut BITMAPINFO,
    buffer: &mut [u8],
    selected: bool,
) -> i32 {
    // guard 必须绑到具名变量上才能活到函数末尾（`let _ =` 会立刻释放）。
    let _selection = if selected {
        SelectedObject::select(dc.0, bitmap)
    } else {
        None
    };
    // SAFETY: buffer 长度恰为 w*h*4（32bpp 无行填充）；bmi 已描述好目标格式；
    // bitmap 由 IconInfo 持有，在本函数返回前一直有效。
    unsafe {
        GetDIBits(
            dc.0,
            bitmap,
            0,
            size.h,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            bmi,
            DIB_RGB_COLORS,
        )
    }
}

/// `CreateCompatibleDC` 创建的内存 DC，离开作用域时删除。
#[cfg(windows)]
struct MemoryDc(HDC);

#[cfg(windows)]
impl MemoryDc {
    /// 创建与当前屏幕兼容的内存 DC（`GetDIBits` 需要一个 DC 来解释颜色表）。
    fn create() -> Result<Self> {
        // SAFETY: 传 None 表示与屏幕兼容的 DC；返回空句柄表示失败。
        let hdc = unsafe { CreateCompatibleDC(None) };
        if hdc.is_invalid() {
            return Err(Error::Win32 {
                api: "CreateCompatibleDC",
                code: last_error(),
            });
        }
        Ok(Self(hdc))
    }
}

#[cfg(windows)]
impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: 句柄由 CreateCompatibleDC 创建，本结构体独占所有权、只会删除一次。
        let _ = unsafe { DeleteDC(self.0) };
    }
}

/// 记录 `SelectObject` 换进 DC 之前的旧对象，离开作用域时换回去。
///
/// 顺序很重要：必须在 `DeleteObject(bitmap)` **之前**还原，否则 DC 会继续引用一张
/// 已被删除的位图。
#[cfg(windows)]
struct SelectedObject {
    dc: HDC,
    previous: HGDIOBJ,
}

#[cfg(windows)]
impl SelectedObject {
    /// 换入位图；失败时返回 `None`（调用方退化为「不选入」）。
    fn select(dc: HDC, bitmap: HBITMAP) -> Option<Self> {
        // SAFETY: dc 有效；bitmap 由 IconInfo 持有，比本结构体活得更久。
        let previous = unsafe { SelectObject(dc, HGDIOBJ(bitmap.0)) };
        if previous.is_invalid() {
            log::debug!(
                "SelectObject 失败（错误码 {}），本次不把位图选入 DC",
                last_error()
            );
            return None;
        }
        Some(Self { dc, previous })
    }
}

#[cfg(windows)]
impl Drop for SelectedObject {
    fn drop(&mut self) {
        // SAFETY: dc 在本结构体存活期间保持有效；previous 正是它原本持有的 GDI
        // 对象（CreateCompatibleDC 出来的 DC 初始持有 1x1 单色位图，不会是空句柄）。
        let _ = unsafe { SelectObject(self.dc, self.previous) };
    }
}

// ---------------------------------------------------------------------------
// alpha 处理（本模块最容易出错的部分）
// ---------------------------------------------------------------------------

/// 把原始 BGRA 缓冲区的第 4 字节（真正的 alpha）写回 [`Frame`] 的 alpha 通道。
///
/// [`Frame::from_bgrx`] 会把 alpha 一律置为 255，而光标位图的 alpha 正是它「边缘
/// 平滑」与「形状镂空」的全部信息来源：丢掉它，光标会变成一块实心方块。
///
/// 两个缓冲区都是行主序紧密排列且长度相同，因此按 `chunks_exact(4)` 顺序对齐即可。
/// 长度不匹配返回 [`Error::BadBufferSize`]，调用方据此退化为「不改 alpha」。
#[cfg(any(windows, test))]
fn restore_alpha(frame: &mut Frame, bgra: &[u8]) -> Result<()> {
    let size = frame.size();
    let expected = size.rgba_len();
    if bgra.len() != expected {
        return Err(Error::BadBufferSize {
            w: size.w,
            h: size.h,
            expected,
            actual: bgra.len(),
        });
    }
    for (px, src) in frame
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(bgra.as_chunks::<4>().0)
    {
        px[3] = src[3];
    }
    Ok(())
}

/// 是否至少有一个不透明像素。
#[cfg(any(windows, test))]
fn has_opaque_pixel(frame: &Frame) -> bool {
    frame.data().as_chunks::<4>().0.iter().any(|px| px[3] != 0)
}

/// 32bpp 掩码缓冲 → 每像素一字节的掩码位（1 = 白 = AND 掩码置位）。
///
/// 掩码本身是 1bpp，交给 `GetDIBits` 转换成 32bpp 后位 0 → 黑（`0x00000000`）、
/// 位 1 → 白（`0x00FFFFFF`），因此「三个通道里有任意一个非 0」就是掩码置位。
#[cfg(windows)]
fn mask_bits(mask_bgra: &[u8]) -> Vec<u8> {
    mask_bgra
        .as_chunks::<4>()
        .0
        .iter()
        .map(|px| u8::from(px[0] != 0 || px[1] != 0 || px[2] != 0))
        .collect()
}

/// AND 掩码位 → alpha 通道：位 0（不透明）→ 255，位 1（透明）→ 0。
///
/// `bits` 与 `alpha` 都是每像素一字节，长度必须一致；不一致时返回
/// [`Error::BadBufferSize`] 而不是写半截数据。
#[cfg(any(windows, test))]
fn mask_bits_to_alpha(bits: &[u8], alpha: &mut [u8]) -> Result<()> {
    if bits.len() != alpha.len() {
        return Err(Error::BadBufferSize {
            // 这里描述的是「每像素一字节」的掩码缓冲，h 固定为 1：w 即像素总数。
            w: bits.len() as u32,
            h: 1,
            expected: bits.len(),
            actual: alpha.len(),
        });
    }
    for (out, bit) in alpha.iter_mut().zip(bits) {
        *out = if *bit == 0 { 255 } else { 0 };
    }
    Ok(())
}

/// 老式光标（1bpp AND/XOR 掩码，没有 alpha 通道）的退化路径。
///
/// 读取掩码的任何失败都只记一条 debug 日志并按「全不透明」处理，绝不返回错误：
/// 光标画不出来不值得让整张截图失败。
#[cfg(windows)]
fn apply_mask_alpha(frame: &mut Frame, mask: HBITMAP) {
    if mask.is_invalid() {
        log::debug!("光标没有 AND 掩码，按全不透明处理");
        return;
    }
    let size = frame.size();
    let mask_bgra = match read_bitmap_bgra(mask, size) {
        Ok(bgra) => bgra,
        Err(err) => {
            log::debug!("读取光标 AND 掩码失败，按全不透明处理：{err}");
            return;
        }
    };
    let bits = mask_bits(&mask_bgra);
    let mut alpha = vec![0u8; bits.len()];
    if let Err(err) = mask_bits_to_alpha(&bits, &mut alpha) {
        log::debug!("AND 掩码转换失败，按全不透明处理：{err}");
        return;
    }
    for (px, a) in frame.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(alpha) {
        px[3] = a;
    }
}

// ---------------------------------------------------------------------------
// 裁剪与热点
// ---------------------------------------------------------------------------

/// 非透明像素的外接矩形；整张全透明时返回 `None`。
#[cfg(any(windows, test))]
fn opaque_bounds(frame: &Frame) -> Option<Rect> {
    let w = frame.width() as i32;
    let mut min_x = w;
    let mut min_y = frame.height() as i32;
    let mut max_x = -1i32;
    let mut max_y = -1i32;
    for (index, px) in frame.data().as_chunks::<4>().0.iter().enumerate() {
        if px[3] == 0 {
            continue;
        }
        let index = index as i32;
        let (x, y) = (index % w, index / w);
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    if max_x < min_x || max_y < min_y {
        return None;
    }
    // 右下角是开区间，所以要 +1。
    Some(Rect::from_ltrb(min_x, min_y, max_x + 1, max_y + 1))
}

/// 把热点夹进图像范围内（保证「热点一定落在位图里」这个不变量）。
#[cfg(any(windows, test))]
fn clamp_hotspot(hotspot: Point, size: Size) -> Point {
    if size.is_empty() {
        return Point::ZERO;
    }
    Point::new(
        hotspot.x.clamp(0, size.w as i32 - 1),
        hotspot.y.clamp(0, size.h as i32 - 1),
    )
}

/// 裁到非透明边界，并把热点同步平移。
///
/// 系统给的光标位图常常是 32x32 而实际图形只有 12x19，四周全是透明像素：裁掉它们
/// 既省内存，也让调试时看到的位图就是光标本身。
#[cfg(any(windows, test))]
fn crop_to_opaque(frame: Frame, hotspot: Point) -> (Frame, Point) {
    // 理论上到不了 None（调用方已保证存在不透明像素），真到了就退化为不裁。
    let crop = opaque_bounds(&frame).unwrap_or_else(|| frame.bounds());
    let image = if crop == frame.bounds() {
        frame
    } else {
        frame.crop(crop)
    };
    let hotspot = clamp_hotspot(
        Point::new(hotspot.x - crop.x, hotspot.y - crop.y),
        image.size(),
    );
    (image, hotspot)
}

// ---------------------------------------------------------------------------
// windows-rs 错误码转换
// ---------------------------------------------------------------------------

/// 最近一次 Win32 错误码；用于不返回 `Result` 的老式 API（`GetDIBits`、`GetObjectW` …）。
#[cfg(windows)]
fn last_error() -> u32 {
    // SAFETY: GetLastError 无参数、无副作用，只读当前线程的错误码。
    unsafe { GetLastError() }.0
}

/// 把 windows-rs 的错误转成本 crate 的 [`Error::Win32`]（错误码已还原为原始 Win32 码）。
#[cfg(windows)]
fn win32_error(api: &'static str, err: &windows::core::Error) -> Error {
    Error::Win32 {
        api,
        code: win32_code(err),
    }
}

/// 从 [`windows::core::Error`] 还原原始 Win32 错误码。
///
/// 生成的绑定会把失败的 `BOOL` 变成 `Error::from_thread()`，其中 `GetLastError()` 的
/// 值被 `HRESULT::from_win32` 塞进了 FACILITY_WIN32（`0x8007_xxxx`）。直接打印
/// HRESULT 与 MSDN 上的错误码对不上，所以这里把高位剥掉。
#[cfg(windows)]
fn win32_code(err: &windows::core::Error) -> u32 {
    let hr = err.code().0 as u32;
    if hr & 0xFFFF_0000 == 0x8007_0000 {
        hr & 0xFFFF
    } else {
        hr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一块 3x3 的纯色假光标，热点在 (1, 1)（正中间）。
    fn fake_cursor(screen_pos: Point) -> CursorImage {
        CursorImage {
            image: Frame::filled(Size::new(3, 3), [255, 0, 0, 255]),
            hotspot: Point::new(1, 1),
            screen_pos,
        }
    }

    // ------------------------------------------------------------------
    // 几何
    // ------------------------------------------------------------------

    #[test]
    fn cursor_origin_maps_global_to_frame_local() {
        let hotspot = Point::new(3, 4);
        // 帧原点为 0：全局坐标减去热点即可
        assert_eq!(
            cursor_origin(Point::new(100, 200), hotspot, Rect::new(0, 0, 1920, 1080)),
            Point::new(97, 196)
        );
        // 帧原点为负（副屏在主屏左侧）：还要再加上帧原点的绝对值
        assert_eq!(
            cursor_origin(
                Point::new(-1800, 200),
                hotspot,
                Rect::new(-1920, 0, 4480, 1440)
            ),
            Point::new(117, 196)
        );
        // 光标在帧外：这里只做纯算术，越界裁剪交给 Frame::blend
        assert_eq!(
            cursor_origin(Point::new(5000, 5000), hotspot, Rect::new(0, 0, 1920, 1080)),
            Point::new(4997, 4996)
        );
    }

    #[test]
    fn composite_with_injected_cursor_lands_on_the_hotspot() {
        // 帧原点为 0：光标全局坐标 (5, 6)、热点 (1, 1) → 位图左上角落在 (4, 5)
        let cursor = fake_cursor(Point::new(5, 6));
        let mut frame = Frame::filled(Size::new(20, 20), [0, 0, 0, 255]);
        assert!(composite_with(
            &mut frame,
            &cursor,
            Rect::new(0, 0, 20, 20),
            Some(Point::new(5, 6))
        ));
        assert_eq!(frame.pixel(4, 5), [255, 0, 0, 255], "热点应对准光标尖");
        assert_eq!(frame.pixel(6, 6), [255, 0, 0, 255], "3x3 光标应盖住右下角");
        assert_eq!(
            frame.pixel(3, 4),
            [0, 0, 0, 255],
            "位图之外的像素不能被改动"
        );

        // 帧原点为负（左侧多显示器）：全局 (-1917, 3) → 局部 (2, 2)
        let cursor = fake_cursor(Point::new(-1917, 3));
        let mut frame = Frame::filled(Size::new(20, 20), [0, 0, 0, 255]);
        assert!(composite_with(
            &mut frame,
            &cursor,
            Rect::new(-1920, 0, 4480, 1440),
            Some(Point::new(-1917, 3))
        ));
        assert_eq!(frame.pixel(2, 2), [255, 0, 0, 255]);

        // 没有显式位置时退回 CursorImage::screen_pos
        let cursor = fake_cursor(Point::new(10, 10));
        let mut frame = Frame::filled(Size::new(20, 20), [0, 0, 0, 255]);
        assert!(composite_with(
            &mut frame,
            &cursor,
            Rect::new(0, 0, 20, 20),
            None
        ));
        assert_eq!(frame.pixel(9, 9), [255, 0, 0, 255]);

        // 光标完全在帧外：不报告已绘制，且帧保持原样
        let cursor = fake_cursor(Point::new(1000, 1000));
        let mut frame = Frame::filled(Size::new(20, 20), [7, 7, 7, 255]);
        let before = frame.clone();
        assert!(!composite_with(
            &mut frame,
            &cursor,
            Rect::new(0, 0, 20, 20),
            Some(Point::new(1000, 1000))
        ));
        assert_eq!(frame, before);

        // 部分越界：只画相交的部分，不能 panic
        let cursor = fake_cursor(Point::new(0, 0));
        let mut frame = Frame::filled(Size::new(20, 20), [7, 7, 7, 255]);
        assert!(composite_with(
            &mut frame,
            &cursor,
            Rect::new(0, 0, 20, 20),
            Some(Point::new(0, 0))
        ));
        // 位图左上角在 (-1, -1)，因此只有 (0, 0)…(1, 1) 这四个像素被覆盖
        assert_eq!(frame.pixel(0, 0), [255, 0, 0, 255]);
        assert_eq!(frame.pixel(1, 1), [255, 0, 0, 255]);
        assert_eq!(
            frame.pixel(2, 2),
            [7, 7, 7, 255],
            "越界部分必须被裁掉，不能画到帧外"
        );
    }

    #[test]
    fn composite_cursor_without_cursor_image_never_panics() {
        let mut frame = Frame::filled(Size::new(8, 8), [0, 0, 0, 255]);
        let before = frame.clone();
        let drawn = composite_cursor(&mut frame, Rect::new(0, 0, 8, 8), None)
            .expect("光标取不到时必须退化成 Ok(false)，不能返回 Err");

        // 无桌面会话 / 光标不可见（CI、服务会话）时必须是 false 且帧不变。
        if !matches!(current_cursor(), Ok(Some(_))) {
            assert!(!drawn, "没有光标位图时不应报告已绘制");
            assert_eq!(frame, before, "没有光标时帧必须保持原样");
        } else {
            println!("当前环境有可见光标，composite_cursor 返回 {drawn}");
        }

        // 空帧同样必须安全返回
        let mut empty = Frame::new(Size::ZERO);
        assert!(
            !composite_cursor(&mut empty, Rect::ZERO, None).unwrap(),
            "空帧上没有可画的像素"
        );
    }

    // ------------------------------------------------------------------
    // alpha
    // ------------------------------------------------------------------

    #[test]
    fn alpha_channel_is_restored_from_bgra() {
        // 三个像素：alpha 分别取 0 / 128 / 255 三种情况
        let bgra: Vec<u8> = vec![
            10, 20, 30, 0, // B=10 G=20 R=30，全透明
            40, 50, 60, 128, // 半透明
            70, 80, 90, 255, // 不透明
        ];
        let size = Size::new(3, 1);
        let mut frame = Frame::from_bgrx(size, bgra.clone()).unwrap();
        // from_bgrx 会把 alpha 一律置为 255（屏幕捕获没有有效 alpha）
        assert!(
            frame.data().as_chunks::<4>().0.iter().all(|px| px[3] == 255),
            "from_bgrx 应先把 alpha 强制成不透明"
        );

        restore_alpha(&mut frame, &bgra).unwrap();

        assert_eq!(frame.pixel(0, 0), [30, 20, 10, 0], "BGRA → RGBA 通道应交换");
        assert_eq!(
            frame.pixel(1, 0),
            [60, 50, 40, 128],
            "半透明 alpha 必须保留"
        );
        assert_eq!(frame.pixel(2, 0), [90, 80, 70, 255]);
        assert!(has_opaque_pixel(&frame), "有 128 / 255 的像素就不是全透明");

        // 长度不匹配必须报错，而不是写半截数据
        assert!(restore_alpha(&mut frame, &bgra[..8]).is_err());
        assert!(restore_alpha(&mut frame, &[]).is_err());

        // 全 0 的 alpha 会被判定为「没有不透明像素」，触发掩码退化路径
        let mut frame = Frame::from_bgrx(size, vec![0; 12]).unwrap();
        restore_alpha(&mut frame, &[0; 12]).unwrap();
        assert!(!has_opaque_pixel(&frame));
    }

    #[test]
    fn mask_bits_map_zero_to_opaque_and_one_to_transparent() {
        let bits = [0u8, 1, 0, 1, 1, 0];
        let mut alpha = [7u8; 6];
        mask_bits_to_alpha(&bits, &mut alpha).unwrap();
        assert_eq!(alpha, [255, 0, 255, 0, 0, 255], "位 0 不透明、位 1 透明");

        // 任何非 0 的掩码字节都按「置位」处理（GetDIBits 转换后位 1 是 0xFF）
        let mut any_nonzero = [7u8; 2];
        mask_bits_to_alpha(&[0, 200], &mut any_nonzero).unwrap();
        assert_eq!(any_nonzero, [255, 0]);

        // 长度不匹配必须返回 Err（安全退化，不写半截）
        let mut too_short = [7u8; 3];
        assert!(mask_bits_to_alpha(&bits, &mut too_short).is_err());
        assert_eq!(too_short, [7, 7, 7], "出错时不能改动输出缓冲");

        let mut too_long = [7u8; 8];
        assert!(mask_bits_to_alpha(&bits, &mut too_long).is_err());

        // 32bpp 掩码 → 掩码位：黑（0x00000000）= 0，白（0x00FFFFFF）= 1
        let bgra = [0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 1, 255];
        assert_eq!(mask_bits(&bgra), vec![0, 1, 1], "任意通道非 0 即为掩码置位");
    }

    #[test]
    fn crop_to_opaque_trims_transparent_border_and_moves_hotspot() {
        let mut frame = Frame::new(Size::new(8, 8));
        frame.fill_rect(Rect::new(3, 2, 2, 3), [255, 255, 255, 255]);
        // 热点原本在 (4, 3)，裁掉左侧 3 列、上方 2 行后应变成 (1, 1)
        let (image, hotspot) = crop_to_opaque(frame, Point::new(4, 3));
        assert_eq!(image.size(), Size::new(2, 3));
        assert_eq!(hotspot, Point::new(1, 1));
        assert!(image.pixel(0, 0)[3] > 0, "裁剪后左上角就是第一个不透明像素");

        // 整张全透明：不裁（退化），热点被夹回图内
        let (image, hotspot) = crop_to_opaque(Frame::new(Size::new(4, 4)), Point::new(9, 9));
        assert_eq!(image.size(), Size::new(4, 4));
        assert_eq!(hotspot, Point::new(3, 3));
        assert_eq!(opaque_bounds(&image), None);

        // 热点必须始终落在图像范围内
        let (image, hotspot) = crop_to_opaque(Frame::new(Size::ZERO), Point::new(-5, -5));
        assert!(image.is_empty());
        assert_eq!(hotspot, Point::ZERO);
    }

    // ------------------------------------------------------------------
    // 真实光标（需要交互式桌面）
    // ------------------------------------------------------------------

    #[test]
    #[ignore = "需要交互式桌面（真实调用 GetCursorInfo / GetIconInfo）"]
    fn cursor_bitmap_is_plausible() {
        let cursor = match current_cursor() {
            Ok(Some(cursor)) => cursor,
            // 无桌面会话、光标被隐藏、或系统只给了单色掩码时都不算失败
            Ok(None) => {
                let _ = print_cursor_diagnostics();
                println!("当前环境没有可用光标位图（诊断见上），测试直接通过");
                return;
            }
            Err(err) => panic!("取光标位图失败：{err}"),
        };

        let (w, h) = (cursor.image.width(), cursor.image.height());
        assert!(w > 0 && h > 0, "光标位图尺寸非法：{w}x{h}");
        assert!(
            cursor.hotspot.x >= 0 && cursor.hotspot.x < w as i32,
            "热点 x={} 越出位图宽度 {w}",
            cursor.hotspot.x
        );
        assert!(
            cursor.hotspot.y >= 0 && cursor.hotspot.y < h as i32,
            "热点 y={} 越出位图高度 {h}",
            cursor.hotspot.y
        );

        let opaque = cursor
            .image
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[3] > 0)
            .count();
        assert!(opaque > 0, "光标位图没有任何不透明像素，合成时什么都看不见");
        assert_eq!(
            cursor.image.data().len(),
            w as usize * h as usize * 4,
            "位图必须是紧密排列的 RGBA8"
        );

        // 证据：尺寸、热点、非透明像素数、alpha 分布、半透明像素数（抗锯齿边缘）
        let partial = cursor
            .image
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[3] > 0 && px[3] < 255)
            .count();
        let mut colors = std::collections::HashSet::new();
        for px in cursor.image.data().as_chunks::<4>().0 {
            if px[3] > 0 {
                colors.insert([px[0], px[1], px[2]]);
            }
        }
        println!(
            "光标证据：位图 {w}x{h}，热点 ({}, {})，非透明像素 {opaque}/{}，其中半透明 {partial}，\
             不透明区域含 {} 种颜色，screen_pos {:?}",
            cursor.hotspot.x,
            cursor.hotspot.y,
            w as usize * h as usize,
            colors.len(),
            cursor.screen_pos
        );
    }

    /// 打印「为什么没有光标位图」：`CURSORINFO` 的 flags / 句柄 / 位置，以及 `ICONINFO`
    /// 的两张位图句柄与热点。
    ///
    /// 无桌面会话（`GetCursorInfo` 失败）、光标被隐藏（`CURSOR_SHOWING` 未置位、
    /// 句柄为空）、纯单色光标（`hbmColor` 为空）这三种情况返回值都是 `Ok(None)`，
    /// 只有把原始结构打出来才能区分。
    #[cfg(windows)]
    fn print_cursor_diagnostics() -> Result<()> {
        let mut info = CURSORINFO {
            cbSize: size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: info 已填好 cbSize，且是有效的可写 CURSORINFO。
        let query = unsafe { GetCursorInfo(&raw mut info) };
        println!(
            "诊断：GetCursorInfo = {query:?}，flags = {:#x}（CURSOR_SHOWING = {:#x}），\
             hCursor = {:p}（空句柄：{}），ptScreenPos = ({}, {})",
            info.flags.0,
            CURSOR_SHOWING.0,
            info.hCursor.0,
            info.hCursor.is_invalid(),
            info.ptScreenPos.x,
            info.ptScreenPos.y
        );
        if query.is_err() || info.hCursor.is_invalid() {
            return Ok(());
        }
        match IconInfo::acquire(info.hCursor) {
            Ok(icon) => println!(
                "诊断：GetIconInfo 成功 → hbmColor = {:p}（空句柄：{}），\
                 hbmMask = {:p}（空句柄：{}），热点 = {:?}，颜色位图尺寸 = {:?}",
                icon.color().0,
                icon.color().is_invalid(),
                icon.mask().0,
                icon.mask().is_invalid(),
                icon.hotspot(),
                bitmap_size(icon.color())
            ),
            Err(err) => println!("诊断：GetIconInfo 失败：{err}"),
        }
        // SAFETY: GetDpiForSystem 无参数、无副作用，只返回当前系统的 DPI。
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() };
        println!(
            "诊断：系统 DPI = {dpi}（{}%）；Windows 按该 DPI 生成光标位图，100% 下系统光标\
             通常为 32x32，150% / 200% 下会是 48x48 / 64x64（本模块不做重采样，直接按原尺寸合成）",
            dpi * 100 / 96
        );
        Ok(())
    }

    /// 用 `CreateIconIndirect` 造出的真实 GDI 光标（连同两张源位图），析构时全部释放。
    ///
    /// 有了它，`GetIconInfo` → `GetObjectW` → `GetDIBits` → alpha 还原 → 裁剪这条链路
    /// 就能在**任何**会话里被真实调用验证，不需要系统光标可见。
    #[cfg(windows)]
    struct TestCursor {
        cursor: HICON,
        color: HBITMAP,
        mask: HBITMAP,
    }

    #[cfg(windows)]
    impl TestCursor {
        /// 造一个 4x4 的 32bpp 光标：
        ///
        /// * 最上面一行、最左边一列全透明（alpha = 0）；
        /// * 其余 3x3 是 alpha 各不相同的图案，(1, 1) 的 alpha 是 128；
        /// * AND 掩码全 0（不透明），形状完全由 alpha 决定；
        /// * 热点声明为 (1, 1)。
        ///
        /// 因此「裁剪到非透明边界」应当正好裁掉首行与首列，(1, 1) 变成 (0, 0)。
        /// 环境不支持创建光标（极少数受限会话）时返回 `None`，调用方跳过测试。
        fn create() -> Option<Self> {
            use windows::Win32::Foundation::FALSE;
            use windows::Win32::Graphics::Gdi::{CreateBitmap, CreateDIBSection};
            use windows::Win32::UI::WindowsAndMessaging::CreateIconIndirect;

            let size = Size::new(4, 4);
            // 行主序写下每个像素的 BGRA——DIB 里的通道顺序就是 B、G、R、A
            #[rustfmt::skip]
            let pattern: [[u8; 4]; 16] = [
                // y = 0：整行全透明
                [255, 0, 255, 0], [255, 0, 255, 0], [255, 0, 255, 0], [255, 0, 255, 0],
                // y = 1：x = 0 透明，(1, 1) 半透明红、(2, 1) 不透明绿、(3, 1) alpha 200
                [255, 0, 255, 0], [30, 20, 10, 128], [60, 50, 40, 255], [90, 80, 70, 200],
                [255, 0, 255, 0], [120, 110, 100, 255], [150, 140, 130, 255], [180, 170, 160, 255],
                [255, 0, 255, 0], [210, 200, 190, 255], [240, 230, 220, 255], [250, 250, 250, 255],
            ];

            let dc = MemoryDc::create().ok()?;
            let bmi = target_bmi(size);
            let mut bits: *mut c_void = std::ptr::null_mut();
            // SAFETY: bmi 描述 32bpp 顶向下 DIB；bits 是有效的可写局部变量；dc 有效。
            let color =
                unsafe { CreateDIBSection(Some(dc.0), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) }
                    .ok()?;
            // SAFETY: bits 指向 w*h*4 字节的可写 DIB 像素，长度与 pattern 一一对应。
            unsafe {
                let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), size.rgba_len());
                for (dst, src) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(pattern) {
                    dst.copy_from_slice(&src);
                }
            }

            // AND 掩码：1bpp、全 0（= 全不透明），每行按 WORD 对齐
            let mask_row_bytes = (size.w as usize).div_ceil(16) * 2;
            let mask_data = vec![0u8; mask_row_bytes * size.h as usize];
            // SAFETY: CreateBitmap 只读取 mask_data（长度已按 1bpp 行对齐算好）。
            let mask = unsafe {
                CreateBitmap(
                    size.w as i32,
                    size.h as i32,
                    1,
                    1,
                    Some(mask_data.as_ptr().cast::<c_void>()),
                )
            };
            if mask.is_invalid() {
                // SAFETY: color 由 CreateDIBSection 创建，本函数是它的唯一所有者。
                let _ = unsafe { DeleteObject(HGDIOBJ(color.0)) };
                return None;
            }

            let info = ICONINFO {
                // FALSE = 光标（而不是图标）
                fIcon: FALSE,
                xHotspot: 1,
                yHotspot: 1,
                hbmMask: mask,
                hbmColor: color,
            };
            // SAFETY: 两张位图都有效、尺寸一致且在本函数返回前一直存活；
            // CreateIconIndirect 只读取它们，创建出的光标由 Drop 释放。
            match unsafe { CreateIconIndirect(&raw const info) } {
                Ok(cursor) => Some(Self {
                    cursor,
                    color,
                    mask,
                }),
                Err(err) => {
                    println!("诊断：CreateIconIndirect 失败（{err}），本环境无法造测试光标");
                    // SAFETY: 两张源位图由本函数创建，尚未交给任何人。
                    unsafe {
                        let _ = DeleteObject(HGDIOBJ(color.0));
                        let _ = DeleteObject(HGDIOBJ(mask.0));
                    }
                    None
                }
            }
        }

        /// 供 `GetIconInfo` 查询的光标句柄。
        fn handle(&self) -> HCURSOR {
            HCURSOR(self.cursor.0)
        }
    }

    #[cfg(windows)]
    impl Drop for TestCursor {
        fn drop(&mut self) {
            use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

            // SAFETY: 三个句柄都由 TestCursor::create 创建，本结构体独占所有权，
            // 因此每个都只会被释放一次（顺序无关，但先销毁光标再删源位图更稳妥）。
            unsafe {
                let _ = DestroyIcon(self.cursor);
                let _ = DeleteObject(HGDIOBJ(self.color.0));
                let _ = DeleteObject(HGDIOBJ(self.mask.0));
            }
        }
    }

    /// 端到端验证真实 32bpp 光标：`GetIconInfo` → `GetObjectW` → `GetDIBits` →
    /// alpha 还原 → 裁剪全部走真实 GDI 调用，最后再合成一次。
    ///
    /// 不依赖交互式桌面，因此在光标被隐藏的无头会话里也能跑，正好补上
    /// [`cursor_bitmap_is_plausible`] 拿不到系统光标时的空白。
    #[test]
    #[cfg(windows)]
    fn synthetic_color_cursor_roundtrips_through_gdi() {
        let Some(test_cursor) = TestCursor::create() else {
            return;
        };
        let icon =
            IconInfo::acquire(test_cursor.handle()).expect("刚创建的光标应能查询到 ICONINFO");
        let cursor = build_cursor_image(&icon, Point::new(-7, -9))
            .expect("构建光标位图不应失败")
            .expect("32bpp 颜色光标必然有颜色位图");

        assert_eq!(
            cursor.image.size(),
            Size::new(3, 3),
            "全透明的首行与首列应被裁掉"
        );
        assert_eq!(cursor.hotspot, Point::new(0, 0), "热点应随裁剪同步平移");
        assert_eq!(cursor.screen_pos, Point::new(-7, -9), "屏幕位置应原样带出");
        assert_eq!(
            cursor.image.pixel(0, 0),
            [10, 20, 30, 128],
            "半透明像素的 alpha 必须原样保留（from_bgrx 会把它冲成 255）"
        );
        assert_eq!(cursor.image.pixel(1, 0), [40, 50, 60, 255]);
        assert_eq!(
            cursor.image.pixel(2, 0),
            [70, 80, 90, 200],
            "alpha=200 的像素"
        );
        assert_eq!(cursor.image.pixel(0, 1), [100, 110, 120, 255]);
        assert_eq!(
            cursor.image.pixel(2, 2),
            [250, 250, 250, 255],
            "BGRA → RGBA 的通道顺序不能反"
        );

        // 合成一次：半透明的热点像素应被真正混合，而不是当作不透明色盖上去
        let mut frame = Frame::filled(Size::new(10, 10), [0, 0, 0, 255]);
        assert!(composite_with(
            &mut frame,
            &cursor,
            Rect::new(0, 0, 10, 10),
            Some(Point::new(5, 6))
        ));
        assert_eq!(
            frame.pixel(5, 6),
            [5, 10, 15, 255],
            "alpha=128 的红叠在黑底上应约等于 RGB 减半"
        );
        assert_eq!(frame.pixel(9, 9), [0, 0, 0, 255], "光标之外的像素不能变");
    }

    /// 证据性测试：`GetDIBits` 在「位图已选入 DC」与「未选入」两种写法下的真实结果。
    ///
    /// MSDN 要求调用 `GetDIBits` 时位图不得被选入 DC，而 [`read_bitmap_bgra`] 先按常见
    /// 示例代码的写法（选入）试了一次；这个测试确认两种写法在真实系统上的行为与像素
    /// 是否一致，也是「先选入 + 失败再重试」这个策略的依据。
    #[test]
    #[cfg(windows)]
    fn get_dibits_selected_vs_unselected() {
        let Some(test_cursor) = TestCursor::create() else {
            return;
        };
        let icon =
            IconInfo::acquire(test_cursor.handle()).expect("刚创建的光标应能查询到 ICONINFO");
        let size = bitmap_size(icon.color())
            .expect("GetObjectW 不应失败")
            .expect("颜色位图必然有尺寸");
        let dc = MemoryDc::create().expect("CreateCompatibleDC 不应失败");
        let read = |selected: bool| {
            let mut bmi = target_bmi(size);
            let mut buffer = vec![0u8; size.rgba_len()];
            let lines = get_dibits(&dc, icon.color(), size, &mut bmi, &mut buffer, selected);
            (lines, buffer)
        };
        let (selected_lines, selected_bytes) = read(true);
        let (plain_lines, plain_bytes) = read(false);

        println!(
            "GetDIBits 证据（{}x{}）：选入 DC 返回 {selected_lines} 行，不选入返回 {plain_lines} 行，\
             两者像素{}",
            size.w,
            size.h,
            if selected_bytes == plain_bytes {
                "完全一致"
            } else {
                "不一致"
            }
        );
        assert!(
            selected_lines == size.h as i32 || plain_lines == size.h as i32,
            "两种写法都没能读出完整的位图：{selected_lines} / {plain_lines}（期望 {}）",
            size.h
        );
        if selected_lines == size.h as i32 && plain_lines == size.h as i32 {
            assert_eq!(
                selected_bytes, plain_bytes,
                "两种写法的读回结果必须一致，否则 read_bitmap_bgra 的优先级顺序会改变结果"
            );
        }
    }
}
