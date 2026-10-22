//! 把 [`icon::render_for`] 画出来的**曜夜紫**图标写成 `.ico`，供 `build.rs` 打进 exe 的资源段。
//!
//! exe 资源只能有一张图，所以固定用深色主题那套皮肤；浅色主题的薄荷奶白版由运行时
//! `icon::window_icon_for(IconSkin::Mint)` 现画（见 `docs/HANDOFF.md` 第 44 节）。
//!
//! ## 为什么需要它
//!
//! 图标是**运行时用代码画的**（`src/icon.rs`）：托盘要 RGBA、窗口要 `IconData`，
//! 而且这样仓库里不用躺一个二进制图片。但 exe 的**文件资源**必须在**编译期**就有，
//! 两者必须是同一份图形——否则资源管理器里的图标和程序自己画的图标会长得不一样。
//!
//! 所以：需要时跑一次这个 example 生成 `.ico`（产物入库），`build.rs` 再把它编进资源段。
//!
//! ```shell
//! cargo run -p cuter-app --example dump_icon -- crates/cuter-app/assets/cuter.ico
//! ```
//!
//! ## ICO 格式（手写，不引依赖）
//!
//! `ICONDIR` + N × `ICONDIRENTRY` + 各尺寸的图像数据。小尺寸用经典 32 位 DIB
//! （BITMAPINFOHEADER + 自下而上的 BGRA + 全零 AND 掩码），256 用 PNG 压缩
//! （Vista 起支持，也是各家的通行做法，不然光这一张就 256 KB）。

#[path = "../src/icon.rs"]
#[allow(dead_code)] // 这个 example 只用到 `render`，其余是给窗口/托盘用的
mod icon;

/// 需要打进 `.ico` 的尺寸。资源管理器/任务栏/Alt-Tab 各取所需，缺哪个就在哪糊。
const SIZES: [u32; 6] = [16, 24, 32, 48, 64, 128];
/// 256 单独用 PNG 存（DIB 太占体积），0 表示 256（ICONDIRENTRY 里宽度只有 1 字节）。
const LARGE_PNG: u32 = 256;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "crates/cuter-app/assets/cuter.ico".to_owned());
    let path = std::path::PathBuf::from(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("创建 assets 目录");
    }

    let mut images: Vec<(u32, Vec<u8>)> = SIZES
        .iter()
        .map(|size| (*size, dib_entry(*size)))
        .collect();
    images.push((LARGE_PNG, png_entry(LARGE_PNG)));

    let bytes = build_ico(&images);
    std::fs::write(&path, &bytes).expect("写 .ico");
    println!(
        "已写出 {}（{} 字节，{} 个尺寸：{}）",
        path.display(),
        bytes.len(),
        images.len(),
        images
            .iter()
            .map(|(size, _)| size.to_string())
            .collect::<Vec<_>>()
            .join("/")
    );
}

/// `ICONDIR` + 目录项 + 图像数据。
fn build_ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let count = u16::try_from(images.len()).expect("尺寸数量");
    let mut out = Vec::new();
    // ICONDIR: reserved=0, type=1(icon), count
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());

    let mut offset = 6 + 16 * usize::from(count);
    for (size, data) in images {
        let dim = u8::try_from(*size).unwrap_or(0); // 256 → 0
        out.push(dim); // width
        out.push(dim); // height
        out.push(0); // 调色板颜色数（真彩色为 0）
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(
            &u32::try_from(data.len())
                .expect("图像长度")
                .to_le_bytes(),
        );
        out.extend_from_slice(&u32::try_from(offset).expect("偏移").to_le_bytes());
        offset += data.len();
    }
    for (_, data) in images {
        out.extend_from_slice(data);
    }
    out
}

/// 经典 32 位 DIB 条目：`BITMAPINFOHEADER` + 自下而上的 BGRA + 全零 AND 掩码。
///
/// 32 位图标实际靠 alpha 通道做透明，但 AND 掩码字段在格式里**必须存在**，
/// 所以老老实实写一块全零的（1bpp，每行按 4 字节对齐）。
fn dib_entry(size: u32) -> Vec<u8> {
    let rgba = icon::render_for(icon::IconSkin::Purple, size);
    let mut out = Vec::with_capacity((size * size * 4) as usize + 128);

    // BITMAPINFOHEADER：高度要写两倍（XOR 图 + AND 掩码）
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&size.to_le_bytes()); // biWidth
    out.extend_from_slice(&(size * 2).to_le_bytes()); // biHeight
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    out.extend_from_slice(&(size * size * 4).to_le_bytes()); // biSizeImage
    out.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    out.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

    // 像素：DIB 是自下而上、BGRA
    for y in (0..size).rev() {
        for x in 0..size {
            let o = ((y * size + x) * 4) as usize;
            let alpha = rgba[o + 3];
            // 32 位图标的约定：alpha=0 时 RGB 也应当是 0（我们直接透传即可，
            // 因为 `render` 只在 alpha>0 的地方写颜色）。
            out.push(rgba[o + 2]); // B
            out.push(rgba[o + 1]); // G
            out.push(rgba[o]); // R
            out.push(alpha); // A
        }
    }

    // AND 掩码：1bpp，行宽按 4 字节对齐，全零
    let mask_row = (size.div_ceil(32)) * 4;
    out.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize));
    out
}

/// PNG 条目：Vista 起的 `.ico` 允许直接嵌 PNG。
fn png_entry(size: u32) -> Vec<u8> {
    let rgba = icon::render_for(icon::IconSkin::Purple, size);
    let mut bytes = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    image::ImageEncoder::write_image(
        encoder,
        &rgba,
        size,
        size,
        image::ExtendedColorType::Rgba8,
    )
    .expect("PNG 编码");
    bytes
}
