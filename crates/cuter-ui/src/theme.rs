//! 视觉主题。
//!
//! 截图工具的所有界面元素都浮在用户的桌面之上，因此配色必须**不抢画面**：
//! 工具条走深色玻璃感，选区描边用高饱和强调色，遮罩用纯黑半透明。

use egui::{Color32, CornerRadius, Stroke, Visuals};

/// 一套界面配色。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    /// 强调色（选区描边、激活的工具按钮）。
    pub accent: Color32,
    /// 工具条背景。
    pub surface: Color32,
    /// 工具条上的次级背景（分组、悬停）。
    pub surface_hover: Color32,
    /// 工具条上的激活按钮背景。
    pub surface_active: Color32,
    /// 工具条前景色（图标）。
    pub on_surface: Color32,
    /// 次级文字。
    pub on_surface_dim: Color32,
    /// 描边（按钮边框、分隔线）。
    pub outline: Color32,
    /// 尺寸标签背景。
    pub label_bg: Color32,
    pub label_fg: Color32,
    /// 十字辅助线。
    pub crosshair: Color32,
    /// 控制点填充 / 描边。
    pub handle_fill: Color32,
    pub handle_stroke: Color32,
    /// 放大镜边框。
    pub magnifier_border: Color32,
    /// 吸附参考线（比强调色更淡，避免抢画面）。
    pub snap_line: Color32,
    /// 悬停窗口的高亮底色。**必须是半透明的**——它盖在用户的桌面上，
    /// 实心色会把整个窗口涂掉。
    pub hover_tint: Color32,
    /// 警告 / 危险（删除、关闭）。
    pub danger: Color32,

    // ---- 设计系统 v1 新增：卡片/侧栏/输入框的层次与语义色 -------------------
    /// 卡片背景（比窗口底亮一档）。
    pub surface_raised: Color32,
    /// 侧栏、输入框、下拉框底（比窗口底暗一档 = "凹"）。
    pub surface_sunken: Color32,
    /// 强调色的低饱和铺底（选中行、选中的工具按钮）。**实色**，不是半透明。
    pub accent_subtle: Color32,
    /// 强调色悬停。
    pub accent_hover: Color32,
    /// 三级文字（说明、页脚、单位）。
    pub on_surface_faint: Color32,
    /// 强边框（输入框、未选中复选框）。
    pub outline_strong: Color32,
    /// 成功。
    pub success: Color32,
    /// 警告。
    pub warning: Color32,
}

impl Theme {
    /// 默认深色主题（绝大多数场景下与桌面内容的对比最自然）。
    ///
    /// **注意 `Color32` 的预乘语义**：想要「某个颜色 + 某个不透明度」时必须用
    /// [`Color32::from_rgba_unmultiplied`]（或 `from_white_alpha` / `from_black_alpha`）。
    /// 用 `from_rgba_premultiplied` 传未预乘的分量会得到完全不同的结果——
    /// 例如 `from_rgba_premultiplied(14,14,16,255)` 是**不透明的近黑色**，
    /// 而不是「14% 灰的半透明遮罩」。这一条曾经真的把悬停窗口涂成了一整块纯黑。
    #[must_use]
    pub fn dark() -> Self {
        Self {
            // 设计系统 v1：现代深色工具风。一个强调色（#4C8DFF），红黄绿只表语义。
            accent: Color32::from_rgb(76, 141, 255),
            surface: Color32::from_rgba_unmultiplied(38, 41, 49, 250),
            surface_hover: Color32::from_rgb(42, 45, 54),
            // 强调按钮的按下态
            surface_active: Color32::from_rgb(58, 122, 232),
            on_surface: Color32::from_rgb(232, 234, 240),
            on_surface_dim: Color32::from_rgb(168, 174, 189),
            outline: Color32::from_rgb(44, 47, 56),
            label_bg: Color32::from_rgba_unmultiplied(38, 41, 49, 235),
            label_fg: Color32::from_rgb(232, 234, 240),
            crosshair: Color32::from_rgba_unmultiplied(255, 255, 255, 200),
            handle_fill: Color32::WHITE,
            handle_stroke: Color32::from_rgb(23, 24, 28),
            magnifier_border: Color32::from_rgb(58, 63, 75),
            snap_line: Color32::from_rgba_unmultiplied(76, 141, 255, 215),
            // 悬停高亮：一层**淡淡的白色纱**（11% 不透明），不能是实心色
            hover_tint: Color32::from_white_alpha(28),
            danger: Color32::from_rgb(242, 85, 90),
            surface_raised: Color32::from_rgb(31, 33, 39),
            surface_sunken: Color32::from_rgb(19, 20, 24),
            accent_subtle: Color32::from_rgb(34, 48, 75),
            accent_hover: Color32::from_rgb(107, 161, 255),
            on_surface_faint: Color32::from_rgb(111, 118, 134),
            outline_strong: Color32::from_rgb(58, 63, 75),
            success: Color32::from_rgb(61, 214, 140),
            warning: Color32::from_rgb(245, 165, 36),
        }
    }

    /// 浅色主题「薄荷奶白」（截图内容偏暗时更清晰）。
    ///
    /// 强调色用薄荷青而不是深色那套蓝：图标的浅色皮肤就是薄荷奶白，界面沿用蓝色会
    /// 出现"图标是薄荷、控件是蓝"的割裂。深色主题保持原来的蓝，两套皮肤因此一眼可辨。
    #[must_use]
    pub fn light() -> Self {
        Self {
            accent: Color32::from_rgb(20, 168, 142),
            surface: Color32::from_rgba_unmultiplied(255, 255, 255, 250),
            surface_hover: Color32::from_rgb(234, 245, 241),
            surface_active: Color32::from_rgb(14, 138, 116),
            on_surface: Color32::from_rgb(27, 29, 34),
            on_surface_dim: Color32::from_rgb(93, 99, 112),
            outline: Color32::from_rgb(228, 234, 231),
            label_bg: Color32::from_rgba_unmultiplied(255, 255, 255, 235),
            label_fg: Color32::from_rgb(27, 29, 34),
            crosshair: Color32::from_rgba_unmultiplied(27, 29, 34, 200),
            handle_fill: Color32::WHITE,
            handle_stroke: Color32::from_rgb(20, 168, 142),
            magnifier_border: Color32::from_rgb(180, 190, 187),
            snap_line: Color32::from_rgba_unmultiplied(20, 168, 142, 215),
            // 浅色主题下用一层淡黑色纱，效果与深色主题的白色纱对称
            hover_tint: Color32::from_black_alpha(28),
            danger: Color32::from_rgb(214, 58, 63),
            surface_raised: Color32::from_rgb(246, 251, 249),
            surface_sunken: Color32::from_rgb(238, 246, 243),
            accent_subtle: Color32::from_rgb(225, 245, 240),
            accent_hover: Color32::from_rgb(47, 195, 168),
            on_surface_faint: Color32::from_rgb(140, 148, 149),
            outline_strong: Color32::from_rgb(203, 213, 209),
            success: Color32::from_rgb(24, 168, 100),
            warning: Color32::from_rgb(200, 126, 12),
        }
    }

    /// 选区描边。
    #[must_use]
    pub fn selection_stroke(&self) -> Stroke {
        Stroke::new(1.0, self.accent)
    }

    /// 工具条的 egui `Visuals`。
    #[must_use]
    pub fn visuals(&self) -> Visuals {
        let mut visuals = if self.on_surface.r() > 128 {
            Visuals::light()
        } else {
            Visuals::dark()
        };
        // 设计系统 v1：圆角刻度（输入 6 / 按钮 8 / 卡片与面板 10 / 浮层 12）、
        // 输入框用"凹"色、浮层给阴影。egui 的默认深色是发黄的灰，必须整组覆盖，
        // 否则会出现"我们的卡片是冷的、原生控件是暖的"这种廉价感。
        visuals.panel_fill = self.surface;
        visuals.window_fill = self.surface;
        visuals.window_stroke = Stroke::new(1.0, self.outline);
        visuals.window_corner_radius = CornerRadius::same(metrics::CORNER_PANEL);
        visuals.window_shadow = egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(115),
        };
        visuals.popup_shadow = visuals.window_shadow;
        // 凹下去的表面：输入框、下拉框、滑轨
        visuals.extreme_bg_color = self.surface_sunken;
        visuals.faint_bg_color = self.surface_raised;
        // 文字层次：正文 / 说明 / 三级
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.on_surface);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.outline);
        visuals.widgets.inactive.bg_fill = self.surface_raised;
        visuals.widgets.inactive.weak_bg_fill = self.surface_raised;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, self.outline_strong);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, self.on_surface);
        visuals.widgets.inactive.corner_radius = CornerRadius::same(metrics::CORNER_BUTTON);
        visuals.widgets.hovered.bg_fill = self.surface_hover;
        visuals.widgets.hovered.weak_bg_fill = self.surface_hover;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, self.outline_strong);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, self.on_surface);
        visuals.widgets.hovered.corner_radius = CornerRadius::same(metrics::CORNER_BUTTON);
        visuals.widgets.active.bg_fill = self.accent_subtle;
        visuals.widgets.active.weak_bg_fill = self.accent_subtle;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, self.accent);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, self.accent);
        visuals.widgets.active.corner_radius = CornerRadius::same(metrics::CORNER_BUTTON);
        visuals.widgets.open.bg_fill = self.surface_hover;
        visuals.widgets.open.weak_bg_fill = self.surface_hover;
        visuals.selection.bg_fill = self.accent_subtle;
        visuals.selection.stroke = Stroke::new(1.0, self.accent);
        visuals.hyperlink_color = self.accent;
        visuals
    }

    /// 判断是否使用浅色前景。
    #[must_use]
    pub fn is_light(&self) -> bool {
        self.on_surface.r() < 128
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

/// 工具条尺寸常量（点 == 物理像素，因为遮罩窗口恒定 1:1）。
pub mod metrics {
    /// 工具条按钮边长。
    pub const BUTTON: f32 = 34.0;
    /// 工具条内边距。
    pub const PADDING: f32 = 6.0;
    /// 工具条与选区之间的间隙。
    pub const GAP: f32 = 8.0;
    /// 工具条圆角（设计系统 v1：浮层 12）。
    pub const CORNER: u8 = 12;
    /// 输入框 / 复选框圆角。
    pub const CORNER_INPUT: u8 = 6;
    /// 按钮圆角。
    pub const CORNER_BUTTON: u8 = 8;
    /// 面板与卡片圆角。
    pub const CORNER_PANEL: u8 = 10;

    // ---- 间距刻度（4px 网格）----------------------------------------------
    /// 4px
    pub const SPACE_1: f32 = 4.0;
    /// 8px
    pub const SPACE_2: f32 = 8.0;
    /// 12px
    pub const SPACE_3: f32 = 12.0;
    /// 16px
    pub const SPACE_4: f32 = 16.0;
    /// 24px
    pub const SPACE_5: f32 = 24.0;
    /// 32px
    pub const SPACE_6: f32 = 32.0;

    // ---- 字阶（见 docs/design/DESIGN-SYSTEM.md）--------------------------
    /// 页面标题
    pub const TEXT_TITLE_1: f32 = 24.0;
    /// 面板标题
    pub const TEXT_TITLE_2: f32 = 20.0;
    /// 卡片标题
    pub const TEXT_TITLE_3: f32 = 15.0;
    /// 正文 / 选项名
    pub const TEXT_BODY: f32 = 13.0;
    /// 按钮文字
    pub const TEXT_LABEL: f32 = 12.0;
    /// 说明、胶囊、页脚
    pub const TEXT_CAPTION: f32 = 11.0;
    /// 控制点半径。
    pub const HANDLE_RADIUS: f32 = 4.0;
    /// 控制点命中半径（比视觉半径大，方便鼠标操作）。
    pub const HANDLE_HIT: f32 = 9.0;
    /// 放大镜外框边长。
    pub const MAGNIFIER: f32 = 140.0;
    /// 放大镜缩放倍率。
    pub const MAGNIFIER_ZOOM: f32 = 8.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_and_dark_are_distinguishable() {
        assert!(!Theme::dark().is_light());
        assert!(Theme::light().is_light());
    }

    #[test]
    fn visuals_panel_fill_matches_surface() {
        let t = Theme::dark();
        assert_eq!(t.visuals().panel_fill, t.surface);
    }

    /// 回归测试：悬停高亮必须是**半透明**的。
    ///
    /// 这里曾经写成 `Color32::from_rgba_premultiplied(14, 14, 16, 255)`。
    /// `from_rgba_premultiplied` 的第 4 个参数就是 alpha，所以那是**不透明的近黑色**；
    /// 于是一层本该"淡淡的纱"变成了一块实心黑矩形，把鼠标下的整个窗口涂黑。
    /// 用户报告为「选中的屏会变黑」，排查了很久才发现是配色常量而不是绘制逻辑。
    #[test]
    fn hover_tint_is_a_translucent_veil() {
        for (name, theme) in [("dark", Theme::dark()), ("light", Theme::light())] {
            let alpha = theme.hover_tint.a();
            assert!(
                alpha > 0,
                "{name} 主题的悬停高亮完全透明，等于没画"
            );
            assert!(
                alpha < 128,
                "{name} 主题的悬停高亮 alpha={alpha}，太实了——它盖在用户桌面上，\
                 必须是一层薄纱；实心色会把整个窗口涂掉"
            );
        }
    }

    /// 细线类元素可以比较实，但不能完全不透明，否则会显得生硬。
    #[test]
    fn guide_lines_are_slightly_transparent() {
        for theme in [Theme::dark(), Theme::light()] {
            assert!(theme.crosshair.a() < 255, "十字辅助线应当略带透明");
            assert!(theme.snap_line.a() < 255, "吸附参考线应当略带透明");
        }
    }
}
