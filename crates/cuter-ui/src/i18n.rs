//! 界面多语言（zh-CN / en-US）。
//!
//! ## 为什么用"中文原文当键"
//!
//! 界面文案分散在两个 crate、十几个文件里。如果给每条文案起一个符号名
//! （`SETTINGS_TITLE_CAPTURE` 之类），改文案时要同时改几百个符号，而且漏改的地方
//! 编译不会报错（符号还在，只是内容旧了）。
//!
//! 这里反过来：**中文原文本身就是键**。`tr!("框选截图")`：
//!
//! * 语言是中文，或者表里查不到 → **原样返回中文**，界面永远不会变成空白或 `???`；
//! * 语言是英文且表里有 → 返回英文。
//!
//! 好处是改动可以**逐块进行**，中间状态始终是可用的界面；代价是英文表要用测试兜住
//! （`every_tr_key_has_a_translation`：它扫源码里所有 `tr!("…")`，要求表里都有条目，
//! 反过来也查"表里有、代码里没人用"的死条目）。
//!
//! ## 日志不翻译
//!
//! 本项目的注释、日志、文档一律中文（见 `docs/HANDOFF.md` 开头的约定）。多语言只覆盖
//! **用户看得见的界面**：按钮、标签、提示、托盘菜单、窗口标题。`log::*` 里的字符串
//! 一个都不该出现在 `tr!` 里，测试也顺手查这一点。

use std::sync::atomic::{AtomicU8, Ordering};

/// 界面语言。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    /// 简体中文（默认）。
    #[default]
    ZhCn,
    /// 英文。
    EnUs,
}

impl Language {
    /// 配置文件里的写法。
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::EnUs => "en-US",
        }
    }

    /// 从配置值解析；`auto`（默认）与认不出来的值都回落到 `fallback`。
    ///
    /// **不报错**：用户手改配置文件写错一个词，不该让界面变成未知状态。
    /// "跟随系统"这件事由宿主做（`cuter-ui` 的硬约定是不碰任何平台 API），
    /// 宿主把系统语言探测好之后传给这里当 `fallback`。
    #[must_use]
    pub fn parse(value: &str, fallback: Self) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "en" | "en-us" | "en_us" | "english" => Self::EnUs,
            "zh" | "zh-cn" | "zh_cn" | "chinese" => Self::ZhCn,
            _ => fallback,
        }
    }

    /// 界面里显示的语言名（用它自己的语言写，这是通行做法）。
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ZhCn => "简体中文",
            Self::EnUs => "English",
        }
    }
}

/// 当前语言。界面是单线程的，用个原子量足够了。
static ACTIVE: AtomicU8 = AtomicU8::new(0);

/// 设置当前语言。
pub fn set_language(language: Language) {
    ACTIVE.store(
        match language {
            Language::ZhCn => 0,
            Language::EnUs => 1,
        },
        Ordering::Relaxed,
    );
}

/// 当前语言。
#[must_use]
pub fn language() -> Language {
    match ACTIVE.load(Ordering::Relaxed) {
        1 => Language::EnUs,
        _ => Language::ZhCn,
    }
}

/// 查一条文案。找不到就返回中文原文（永远不返回空串）。
#[must_use]
pub fn tr(key: &'static str) -> &'static str {
    if language() == Language::ZhCn {
        return key;
    }
    match lookup(key) {
        Some(text) => text,
        None => {
            // 只在真正缺翻译时提醒一次一条，方便用探针查完整性
            // 带一个 ASCII 标记：探针要能在纯 ASCII 的 .ps1 里匹配它（PowerShell 5.1 读
            // 没有 BOM 的脚本会按 GBK 解析，脚本里写中文会直接把语法搞坏）。
            log::debug!("界面文案缺英文翻译（i18n-missing）：{key}");
            key
        }
    }
}

fn lookup(key: &str) -> Option<&'static str> {
    TABLE
        .binary_search_by(|(k, _)| (*k).cmp(key))
        .ok()
        .map(|index| TABLE[index].1)
}

/// 把 `{}` 依次换成参数。
///
/// `format!` 只吃**字面量**格式串，而翻译表里的模板是运行时的，所以需要这么一层。
/// 参数已经转成 `String`，避免 `&[&dyn Display]` 那种"数组第一个元素把类型钉死"的坑
/// （混着 `&String` 和 `&str` 时会编译不过）；调用一律走 [`fill!`] 宏，它会替我们
/// 逐个 `to_string()`。
#[must_use]
pub fn join(template: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(template.len() + args.len() * 8);
    let mut rest = template;
    for arg in args {
        match rest.find("{}") {
            Some(index) => {
                out.push_str(&rest[..index]);
                out.push_str(&arg.to_string());
                rest = &rest[index + 2..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// 拼一条带占位符的界面文案：`fill!(tr!("已保存到 {}"), path.display())`。
#[macro_export]
macro_rules! fill {
    ($template:expr $(, $arg:expr)*) => {
        $crate::i18n::join($template, &[$(&$arg as &dyn ::std::fmt::Display),*])
    };
}

/// 取一条文案（宏形态，用字面量当键）。
///
/// `#[macro_export]` 之后它落在 crate 根：同 crate 里 `use crate::tr;`，
/// 下游 crate 里 `use cuter_ui::tr;`。
#[macro_export]
macro_rules! tr {
    ($key:literal) => {
        $crate::i18n::tr($key)
    };
}

/// 中文 → 英文对照表。
///
/// **必须按中文原文排序**（查询走二分）。测试会检查这一点，也会检查"每个 `tr!(...)`
/// 都有条目"以及"没有死条目"。
static TABLE: &[(&str, &str)] = include!("i18n.table.rs");

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// 扫源码里所有 `tr!("…")` 的键。
    ///
    /// 为什么要扫源码而不是运行时收集：翻译表是**运行时**查的，缺一条只会静默显示中文，
    /// 编译不会报错。用测试把"界面里用了但表里没有"和"表里有但没人用"两头都钉住，
    /// 才能保证英文界面不出现半中半英。
    fn used_keys() -> Vec<(String, String)> {
        let mut found = Vec::new();
        let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        // 枚举的 `label()` 住在 cuter-core（那个 crate 不做 i18n），宿主侧用
        // `tr(kind.label())` 翻，所以它的匹配臂字面量也要算"用到的键"。
        let core = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("cuter-core")
            .join("src")
            .join("config.rs");
        let app = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("cuter-app")
            .join("src");
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        if core.exists() {
            files.push(core);
        }
        for directory in [ui, app] {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|ext| ext != "rs") {
                    continue;
                }
                if path.file_name().is_some_and(|name| name == "i18n.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let mut rest = text.as_str();
                while let Some(index) = rest.find("tr!(\"") {
                    // +5：跳过 `tr!(` 和那个开引号（少跳一个会把 key 读成空串）
                    let after = &rest[index + 5..];
                    let Some(end) = after.find('"') else { break };
                    let key = &after[..end];
                    // 源码里写的是转义后的文本（`\n`、`\"`），运行时是真换行/真引号：
                    // 这里必须还原，否则带转义的键会被误判成"还没翻译"。
                    let backslash = char::from(92);
                    let key = key
                        .replace(&format!("{backslash}n"), "\n")
                        .replace(&format!("{backslash}\""), "\"")
                        .replace(&format!("{backslash}{backslash}"), &backslash.to_string());
                    found.push((
                        key,
                        path.file_name().unwrap_or_default().to_string_lossy().to_string(),
                    ));
                    rest = &after[end..];
                }
            }
        }
        found
    }

    #[test]
    fn every_tr_key_has_a_translation() {
        let mut missing = Vec::new();
        for (key, file) in used_keys() {
            if lookup(&key).is_none() {
                missing.push(format!("{key}（{file}）"));
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "这些界面文案还没有英文翻译（{} 条）：\n{}",
            missing.len(),
            missing.join("\n")
        );
    }

    #[test]
    fn no_dead_translations() {
        // 来自 `cuter-core` 枚举 `label()` 的键：那些枚举不做 i18n（`cuter-core` 不依赖
        // 界面层），宿主侧用 `cuter_ui::i18n::tr(kind.label())` 翻——那种调用点没有
        // `tr!("…")` 字面量，扫源码扫不到，所以在这里显式列出来，并照样要求有译文。
        const CORE_ENUM_LABELS: &[&str] = &[
            "跟随系统",
            "深色",
            "浅色",
            "选择",
            "矩形",
            "椭圆",
            "箭头",
            "画笔",
            "荧光笔",
            "马赛克",
            "模糊",
            "文字",
            "序号",
            "激光笔",
            "关闭",
            "内置离线",
            "在线接口",
            "Windows DXGI（桌面复制）",
        ];
        for key in CORE_ENUM_LABELS {
            assert!(
                lookup(key).is_some(),
                "来自 cuter-core 枚举的文案「{key}」没有英文翻译"
            );
        }
        let used: std::collections::HashSet<String> =
            used_keys().into_iter().map(|(key, _)| key).collect();
        let dead: Vec<&str> = TABLE
            .iter()
            .map(|(key, _)| *key)
            .filter(|key| !used.contains(*key) && !CORE_ENUM_LABELS.contains(key))
            .collect();
        assert!(
            dead.is_empty(),
            "翻译表里这些条目已经没有代码在用了（{} 条）：\n{}",
            dead.len(),
            dead.join("\n")
        );
    }

    #[test]
    fn table_is_sorted_for_binary_search() {
        for pair in TABLE.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "翻译表必须按中文原文升序（二分查找的前提）：{:?} 在 {:?} 之后",
                pair[1].0,
                pair[0].0
            );
        }
    }

    #[test]
    fn placeholders_survive_translation() {
        for (key, value) in TABLE {
            assert_eq!(
                key.matches("{}").count(),
                value.matches("{}").count(),
                "「{key}」的 {{}} 占位符数量与译文不一致（译文：{value}）"
            );
        }
    }

    #[test]
    fn chinese_language_returns_the_key_itself() {
        set_language(Language::ZhCn);
        assert_eq!(tr("关闭"), "关闭");
        set_language(Language::EnUs);
        assert_eq!(tr("关闭"), "Close");
        // 表里没有的键也必须原样返回，绝不返回空串
        assert_eq!(tr("这条文案不存在"), "这条文案不存在");
        set_language(Language::ZhCn);
    }

    #[test]
    fn parse_accepts_the_documented_values() {
        assert_eq!(Language::parse("en-US", Language::ZhCn), Language::EnUs);
        assert_eq!(Language::parse("EN", Language::ZhCn), Language::EnUs);
        assert_eq!(Language::parse("zh-CN", Language::EnUs), Language::ZhCn);
        // auto 与认不出来的值都回落到调用方给的系统语言
        assert_eq!(Language::parse("auto", Language::EnUs), Language::EnUs);
        assert_eq!(Language::parse("klingon", Language::ZhCn), Language::ZhCn);
    }

    #[test]
    fn fill_replaces_placeholders_in_order() {
        assert_eq!(fill!("已保存到 {}", "C:\\a.png"), "已保存到 C:\\a.png");
        assert_eq!(fill!("{} × {} 像素", 800, 600), "800 × 600 像素");
        // 参数多了/少了都不该 panic
        assert_eq!(fill!("没有占位符", "x"), "没有占位符");
        assert_eq!(fill!("两个 {} {}", "a"), "两个 a {}");
    }
}
