//! 端到端验证 Windows OCR 引擎。
//!
//! 思路是闭环：**用我们自己的 CPU 栅格化器（`cuter_ui::flatten`）把已知文字画进一张图，
//! 再用 OCR 把它读回来**。这样一次测试同时覆盖了两条链路：
//!
//! * 栅格化器确实把字形画对了（否则 OCR 读不出正确内容）；
//! * OCR 引擎的「Frame → SoftwareBitmap → 识别 → TextBlock」整条路是通的，
//!   而且坐标落在正确的像素位置上。
//!
//! 依赖系统安装的 OCR 语言组件，因此**组件缺失时安静跳过**（返回而不是失败）——
//! 这跟仓库里其它依赖环境的测试（字体探测、光标位图）保持同一约定。

use cuter_core::frame::Frame;
use cuter_core::geom::{Point, Size};
use cuter_core::ocr::{OcrEngine, WindowsOcr};
use cuter_ui::annotate::Annotation;
use cuter_ui::{flatten, fonts};
use egui::{Color32, Pos2};

/// 画一张白底黑字的图，返回图片和原始文字。
fn render(text: &str, font_size: f32) -> Option<(Frame, cuter_ui::fonts::LoadedFont)> {
    let font = fonts::load_system_font(None)?;

    // 画布按文字长度粗略定宽，保证整句都在图内
    let width = (text.chars().count() as f32 * font_size * 0.75 + 80.0).max(240.0) as u32;
    let height = (font_size * 2.5).max(120.0) as u32;
    let base = Frame::filled(Size::new(width, height), [255, 255, 255, 255]);

    let annotation = Annotation::Text {
        pos: Pos2::new(30.0, font_size * 0.6),
        text: text.to_owned(),
        color: Color32::BLACK,
        size: font_size,
    };

    let baked = flatten::flatten(&base, &[annotation], Some(font.font_ref()));
    Some((baked, font))
}

#[test]
fn ocr_engine_reports_itself_ready() {
    let Ok(engine) = WindowsOcr::new(None) else {
        eprintln!("跳过：本机没有可用的 OCR 语言组件");
        return;
    };
    assert!(engine.is_ready());
    assert!(!engine.language().is_empty(), "引擎应当报告实际语言");
    assert!(engine.max_image_dimension() > 0);
    eprintln!(
        "OCR 引擎：{}，语言 {}，最大边长 {}",
        engine.name(),
        engine.language(),
        engine.max_image_dimension()
    );
}

#[test]
fn empty_image_is_rejected_with_a_clear_error() {
    let Ok(engine) = WindowsOcr::new(None) else {
        eprintln!("跳过：本机没有可用的 OCR 语言组件");
        return;
    };
    let err = engine
        .recognize(&Frame::new(Size::ZERO))
        .expect_err("空图必须被拒绝");
    let message = err.to_string();
    assert!(
        message.contains("空"),
        "错误信息应当说明图片为空，实际是：{message}"
    );
}

#[test]
fn oversize_image_is_rejected_instead_of_silently_truncated() {
    let Ok(engine) = WindowsOcr::new(None) else {
        eprintln!("跳过：本机没有可用的 OCR 语言组件");
        return;
    };
    let limit = engine.max_image_dimension();
    // 只构造尺寸元数据，不真的分配 (limit+1)^2 的内存
    let huge = Frame::new(Size::new(limit + 1, 8));
    let err = engine.recognize(&huge).expect_err("超限图片必须被拒绝");
    let message = err.to_string();
    assert!(
        message.contains("超过引擎上限"),
        "错误信息应当说明超限，实际是：{message}"
    );
}

/// **核心测试**：自己的栅格化器写进去，OCR 读出来。
#[test]
fn rendered_latin_text_survives_a_roundtrip() {
    let Some((image, _font)) = render("Hello Cuter 12345", 48.0) else {
        eprintln!("跳过：找不到系统字体");
        return;
    };
    let Ok(engine) = WindowsOcr::new(None) else {
        eprintln!("跳过：本机没有可用的 OCR 语言组件");
        return;
    };

    let result = engine.recognize(&image).expect("识别调用本身不应失败");
    let text = result.plain_text(false);
    eprintln!("原图 {}x{}，识别到 {} 个文本块：{text}", image.size().w, image.size().h, result.blocks.len());

    let lowered = text.to_lowercase();
    assert!(
        lowered.contains("hello"),
        "应当识别出 Hello，实际是：{text:?}"
    );
    assert!(
        text.contains("12345"),
        "应当识别出 12345，实际是：{text:?}"
    );

    // 文本块的位置必须落在图片内，否则说明「Frame → SoftwareBitmap」这一步尺寸/通道错了
    for block in &result.blocks {
        assert!(
            block.bounds.intersect(cuter_core::geom::Rect::new(
                0,
                0,
                image.size().w,
                image.size().h
            )) == block.bounds,
            "文本块 {:?} 超出了图片范围——多半是位图尺寸或通道顺序错了",
            block.bounds
        );
        assert!(block.bounds.w > 0 && block.bounds.h > 0);
    }
}

#[test]
fn rendered_chinese_text_survives_a_roundtrip() {
    // 只有装了中文 OCR 组件时才有意义
    let Ok(engine) = WindowsOcr::new(Some("zh-Hans-CN")) else {
        eprintln!("跳过：本机没有中文 OCR 组件");
        return;
    };
    if !engine.language().to_lowercase().contains("zh") {
        eprintln!("跳过：回退到了 {}，不是中文引擎", engine.language());
        return;
    }

    let Some((image, _font)) = render("截图工具测试", 48.0) else {
        eprintln!("跳过：找不到系统字体");
        return;
    };

    let result = engine.recognize(&image).expect("识别调用本身不应失败");
    let text: String = result.plain_text(false).chars().filter(|c| !c.is_whitespace()).collect();
    eprintln!("中文识别结果：{text:?}（去掉空格后比较）");

    // Windows OCR 对中文常在字之间插空格，所以比较前先去空白
    assert!(
        text.contains("截图") || text.contains("工具") || text.contains("测试"),
        "至少要认出「截图工具测试」中的一部分，实际是：{text:?}"
    );

    // 位置校验：识别到的方块应当在图片左半边（我们把它画在了左边）
    let center = image.size().w / 2;
    assert!(
        result.blocks.iter().any(|b| b.bounds.x < center as i32),
        "文字画在左侧，识别出的方块也应当在左侧"
    );
}

#[test]
fn blank_image_yields_no_text_blocks() {
    let Ok(engine) = WindowsOcr::new(None) else {
        eprintln!("跳过：本机没有可用的 OCR 语言组件");
        return;
    };
    let blank = Frame::filled(Size::new(400, 200), [255, 255, 255, 255]);
    let result = engine.recognize(&blank).expect("空白图识别不应失败");
    assert!(
        result.plain_text(true).trim().is_empty(),
        "空白图不该识别出文字，实际是：{:?}",
        result.plain_text(true)
    );
    // 顺带确认坐标系原点没被平移过：空白图不该有越界的方块
    for block in &result.blocks {
        assert!(block.bounds.x >= 0 && block.bounds.y >= 0);
        let _ = Point::ZERO;
    }
}
