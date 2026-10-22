//! 把图标与版本信息编进 exe 的**文件资源**。
//!
//! ## 为什么需要
//!
//! 没有资源段的话：资源管理器里看不到版本号、属性页里产品名是空的、
//! 文件只有默认图标——一个"不像正经软件"的 exe。安装包（Inno Setup / MSI）也要求
//! exe 自己带版本信息与图标。
//!
//! ## 图标为什么是一份二进制文件
//!
//! 图标本身是**运行时用代码画的**（`src/icon.rs`），仓库里不躺图片。但文件资源必须在
//! **编译期**就存在，所以这里读的是 `assets/cuter.ico`——它由同一个 `icon.rs` 生成：
//!
//! ```shell
//! cargo run -p cuter-app --example dump_icon -- crates/cuter-app/assets/cuter.ico
//! ```
//!
//! **改了 `icon.rs` 的图形记得重跑这条命令并提交新的 `.ico`**，否则资源管理器里的图标
//! 会和托盘/标题栏里的不一样。
//!
//! ## 版本号从哪来
//!
//! `Cargo.toml` 的 `version`（`CARGO_PKG_VERSION`），打包成 Windows 要求的
//! 四段式 `major.minor.patch.build`（多出来的段忽略、缺的补 0）。

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/cuter.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");

    #[cfg(windows)]
    {
        let version = packed_version(std::env::var("CARGO_PKG_VERSION").as_deref().unwrap_or("0.0.0"));

        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/cuter.ico");
        res.set("ProductName", "Cuter");
        res.set("FileDescription", "Cuter —— 截图 / 标注 / OCR / 贴图");
        res.set("CompanyName", "Cuter contributors");
        res.set("LegalCopyright", "GPL-3.0-or-later");
        res.set("OriginalFilename", "cuter.exe");
        res.set("InternalName", "cuter");
        res.set_version_info(winresource::VersionInfo::FILEVERSION, version);
        res.set_version_info(winresource::VersionInfo::PRODUCTVERSION, version);

        if let Err(err) = res.compile() {
            // 资源编译失败不该让整个构建挂掉（比如换了台没装 Windows SDK 的机器）：
            // 大不了这个 exe 没有版本信息，功能一点不受影响。
            println!("cargo:warning=编译 Windows 资源失败（exe 将没有版本号与图标）：{err}");
        }
    }
}

/// `major.minor.patch.build` → 四段各 16 位拼成的 u64（Windows 的约定）。
#[cfg_attr(not(windows), allow(dead_code))]
fn packed_version(version: &str) -> u64 {
    let mut parts = [0u16; 4];
    for (index, part) in version.split(['.', '-', '+']).take(4).enumerate() {
        // 预发布版本号里的非数字段（如 `1.0.0-beta.1` 的 `beta`）直接当 0。
        parts[index] = part.parse().unwrap_or(0);
    }
    (u64::from(parts[0]) << 48)
        | (u64::from(parts[1]) << 32)
        | (u64::from(parts[2]) << 16)
        | u64::from(parts[3])
}

#[cfg(test)]
mod tests {
    use super::packed_version;

    #[test]
    fn version_is_packed_into_four_words() {
        assert_eq!(packed_version("1.2.3.4"), (1 << 48) | (2 << 32) | (3 << 16) | 4);
        assert_eq!(packed_version("0.1.0"), 1 << 32 | 0 << 16);
        assert_eq!(packed_version("2.0.0-beta.1"), 2 << 48, "非数字段按 0 处理");
        assert_eq!(packed_version(""), 0);
    }
}
