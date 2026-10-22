"""生成 Cuter 的设计稿：一份几何，两个渲染器。

    python tools/design/make_mockups.py

产物（docs/design/mockups/）：
  tokens.svg / .png          设计令牌板：色板、字阶、间距、圆角、控件状态
  settings.svg / .png        设置窗口（980x660，1:1 替换件）
  ocr-panel.svg / .png       OCR 识别结果面板（980x620，1:1）
  overlay-toolbar.svg / .png 遮罩浮动工具条 + 工具选项弹窗（1:1）

为什么要"一份几何两个渲染器"：SVG 是给 Figma 导入用的（Figma 原生支持 SVG），
但 SVG 在这台机器上没法可靠地渲染成预览；PNG 用 Pillow 画，保证**我看到的和你看到的是同一张图**。
两边共用同一份图元列表，就不会出现"稿子和预览不一致"。

字体：PNG 用系统自带 msyh.ttc（只有一个字重），所以**层次靠字号与颜色**表达——
这和 egui 里的实际情况一致（我们就加载一份字体），不是偷懒。
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

# ---------------------------------------------------------------- 设计令牌
BG_BASE = "#17181C"       # 窗口底
BG_SUNKEN = "#131418"     # 输入框 / 侧栏
BG_RAISED = "#1F2127"     # 卡片
BG_OVERLAY = "#262931"    # 弹层 / 工具条
BORDER_SUBTLE = "#2C2F38"
BORDER_STRONG = "#3A3F4B"
TEXT_PRIMARY = "#E8EAF0"
TEXT_SECONDARY = "#A8AEBD"
TEXT_TERTIARY = "#6F7686"
ACCENT = "#4C8DFF"
ACCENT_HOVER = "#6BA1FF"
ACCENT_SUBTLE = "#22304B"   # accent 14% 叠在 bg/sunken 上的等效实色
SUCCESS = "#3DD68C"
WARNING = "#F5A524"
DANGER = "#F2555A"
WHITE = "#FFFFFF"

FONT_CANDIDATES = [
    r"C:\Windows\Fonts\msyh.ttc",
    r"C:\Windows\Fonts\msyh.ttf",
    r"C:\Windows\Fonts\simhei.ttf",
]

OUT = Path(__file__).resolve().parents[2] / "docs" / "design" / "mockups"


# ---------------------------------------------------------------- 图元
@dataclass
class Rect:
    x: float
    y: float
    w: float
    h: float
    r: float = 0
    fill: str | None = None
    stroke: str | None = None
    sw: float = 1.0
    opacity: float = 1.0


@dataclass
class Text:
    x: float
    y: float
    s: str
    size: float
    fill: str
    anchor: str = "start"   # start | middle | end
    opacity: float = 1.0


@dataclass
class Circle:
    cx: float
    cy: float
    r: float
    fill: str | None = None
    stroke: str | None = None
    sw: float = 1.0


@dataclass
class Line:
    x1: float
    y1: float
    x2: float
    y2: float
    stroke: str
    sw: float = 1.0
    opacity: float = 1.0


@dataclass
class Board:
    name: str
    w: float
    h: float
    items: list = field(default_factory=list)
    bg: str = BG_BASE

    def rect(self, *a, **kw):
        self.items.append(Rect(*a, **kw))
        return self

    def text(self, *a, **kw):
        self.items.append(Text(*a, **kw))
        return self

    def circle(self, *a, **kw):
        self.items.append(Circle(*a, **kw))
        return self

    def line(self, *a, **kw):
        self.items.append(Line(*a, **kw))
        return self


# ---------------------------------------------------------------- SVG 渲染
def to_svg(board: Board) -> str:
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{board.w:g}" height="{board.h:g}" '
        f'viewBox="0 0 {board.w:g} {board.h:g}">',
        f'<rect width="{board.w:g}" height="{board.h:g}" fill="{board.bg}"/>',
    ]
    for it in board.items:
        if isinstance(it, Rect):
            attrs = [f'x="{it.x:g}"', f'y="{it.y:g}"', f'width="{it.w:g}"', f'height="{it.h:g}"']
            if it.r:
                attrs.append(f'rx="{it.r:g}"')
            attrs.append(f'fill="{it.fill}"' if it.fill else 'fill="none"')
            if it.stroke:
                attrs += [f'stroke="{it.stroke}"', f'stroke-width="{it.sw:g}"']
            if it.opacity != 1.0:
                attrs.append(f'opacity="{it.opacity:g}"')
            parts.append(f'<rect {" ".join(attrs)}/>')
        elif isinstance(it, Text):
            anchor = {"start": "start", "middle": "middle", "end": "end"}[it.anchor]
            esc = (
                it.s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
            )
            opacity = f' opacity="{it.opacity:g}"' if it.opacity != 1.0 else ""
            parts.append(
                f'<text x="{it.x:g}" y="{it.y:g}" font-family="Microsoft YaHei, Segoe UI, sans-serif" '
                f'font-size="{it.size:g}" fill="{it.fill}" text-anchor="{anchor}"{opacity}>{esc}</text>'
            )
        elif isinstance(it, Circle):
            attrs = [f'cx="{it.cx:g}"', f'cy="{it.cy:g}"', f'r="{it.r:g}"']
            attrs.append(f'fill="{it.fill}"' if it.fill else 'fill="none"')
            if it.stroke:
                attrs += [f'stroke="{it.stroke}"', f'stroke-width="{it.sw:g}"']
            parts.append(f'<circle {" ".join(attrs)}/>')
        elif isinstance(it, Line):
            opacity = f' opacity="{it.opacity:g}"' if it.opacity != 1.0 else ""
            parts.append(
                f'<line x1="{it.x1:g}" y1="{it.y1:g}" x2="{it.x2:g}" y2="{it.y2:g}" '
                f'stroke="{it.stroke}" stroke-width="{it.sw:g}"{opacity}/>'
            )
    parts.append("</svg>")
    return "\n".join(parts)


# ---------------------------------------------------------------- PNG 渲染
def to_png(board: Board, path: Path) -> None:
    from PIL import Image, ImageDraw, ImageFont

    font_path = next((p for p in FONT_CANDIDATES if Path(p).exists()), None)
    if font_path is None:
        raise SystemExit("找不到中文字体，PNG 预览会变成方框")

    scale = 2  # 2x 超采样，边缘更干净
    img = Image.new("RGB", (int(board.w * scale), int(board.h * scale)), board.bg)
    draw = ImageDraw.Draw(img, "RGBA")
    cache: dict[int, "ImageFont.FreeTypeFont"] = {}

    def font(size: float):
        key = int(size * scale)
        if key not in cache:
            cache[key] = ImageFont.truetype(font_path, key)
        return cache[key]

    def px(v: float) -> float:
        return v * scale

    for it in board.items:
        if isinstance(it, Rect):
            box = [px(it.x), px(it.y), px(it.x + it.w) - 1, px(it.y + it.h) - 1]
            fill = it.fill
            if fill and it.opacity != 1.0:
                fill = fill + format(int(it.opacity * 255), "02X")
            draw.rounded_rectangle(
                box,
                radius=px(it.r),
                fill=fill,
                outline=it.stroke,
                width=max(1, int(px(it.sw))),
            )
        elif isinstance(it, Text):
            anchor = {"start": "l", "middle": "m", "end": "r"}[it.anchor]
            draw.text(
                (px(it.x), px(it.y)),
                it.s,
                font=font(it.size),
                fill=it.fill + format(int(it.opacity * 255), "02X") if it.opacity != 1.0 else it.fill,
                anchor=anchor + "s",   # 基线对齐，和 SVG 的 y 语义一致
            )
        elif isinstance(it, Circle):
            box = [
                px(it.cx - it.r),
                px(it.cy - it.r),
                px(it.cx + it.r),
                px(it.cy + it.r),
            ]
            draw.ellipse(box, fill=it.fill, outline=it.stroke, width=max(1, int(px(it.sw))))
        elif isinstance(it, Line):
            draw.line(
                [px(it.x1), px(it.y1), px(it.x2), px(it.y2)],
                fill=it.stroke + format(int(it.opacity * 255), "02X") if it.opacity != 1.0 else it.stroke,
                width=max(1, int(px(it.sw))),
            )

    img = img.resize((int(board.w), int(board.h)), Image.LANCZOS)
    img.save(path)


def write(board: Board) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / f"{board.name}.svg").write_text(to_svg(board), encoding="utf-8")
    to_png(board, OUT / f"{board.name}.png")
    print(f"  {board.name}.svg / .png  ({board.w:g}x{board.h:g})")


# ---------------------------------------------------------------- 常用零件
def card(b: Board, x, y, w, h, r=10, fill=BG_RAISED, stroke=BORDER_SUBTLE):
    b.rect(x, y, w, h, r, fill, stroke, 1)


def icon_rect(b: Board, cx, cy, color, size=9):
    b.rect(cx - size / 2, cy - size / 2, size, size * 0.78, 2, None, color, 1.6)


def icon_ellipse(b: Board, cx, cy, color, size=9):
    b.circle(cx, cy, size / 2, None, color, 1.6)


def icon_arrow(b: Board, cx, cy, color):
    b.line(cx - 5, cy + 5, cx + 5, cy - 5, color, 1.6)
    b.line(cx + 5, cy - 5, cx + 5, cy - 1, color, 1.6)
    b.line(cx + 5, cy - 5, cx + 1, cy - 5, color, 1.6)


def icon_pen(b: Board, cx, cy, color):
    b.line(cx - 5, cy + 5, cx + 3, cy - 3, color, 1.8)
    b.line(cx + 3, cy - 3, cx + 5, cy - 5, color, 1.8)


def icon_text(b: Board, cx, cy, color):
    b.line(cx - 5, cy - 4, cx + 5, cy - 4, color, 1.8)
    b.line(cx, cy - 4, cx, cy + 5, color, 1.8)


def icon_dot(b: Board, cx, cy, color):
    b.circle(cx, cy, 2.2, color, None)


def icon_blur(b: Board, cx, cy, color):
    for dx, dy in ((-3.5, -3.5), (0, 0), (3.5, 3.5), (-3.5, 3.5), (3.5, -3.5)):
        b.circle(cx + dx, cy + dy, 1.8, color, None)


def icon_check(b: Board, cx, cy, color, size=5):
    b.line(cx - size, cy, cx - size * 0.2, cy + size * 0.75, color, 2.2)
    b.line(cx - size * 0.2, cy + size * 0.75, cx + size, cy - size * 0.7, color, 2.2)


def checkbox(b: Board, x, y, checked: bool, size=18):
    if checked:
        b.rect(x, y, size, size, 6, ACCENT, None)
        icon_check(b, x + size / 2, y + size / 2, WHITE)
    else:
        b.rect(x, y, size, size, 6, BG_SUNKEN, BORDER_STRONG, 1.2)


def chip(b: Board, x, y, label, size=11, pad=8, fill=BG_RAISED, fg=TEXT_SECONDARY):
    w = len(label) * size * 0.62 + pad * 2
    b.rect(x, y, w, size + 8, 6, fill)
    b.text(x + pad, y + size + 1, label, size, fg)
    return w


# ---------------------------------------------------------------- 1. 令牌板
def tokens_board() -> Board:
    b = Board("tokens", 980, 620)
    b.text(32, 46, "Cuter 设计令牌", 24, TEXT_PRIMARY)
    b.text(32, 70, "现代深色工具风 · 单一强调色 · 4px 网格 · 层次靠字号与颜色（egui 只有一份字体）", 12, TEXT_SECONDARY)

    def swatch(x, y, color, name, hexv):
        b.rect(x, y, 72, 40, 8, color, BORDER_SUBTLE, 1)
        b.text(x, y + 58, name, 11, TEXT_PRIMARY)
        b.text(x, y + 73, hexv, 10, TEXT_TERTIARY)

    b.text(32, 116, "色板", 15, TEXT_PRIMARY)
    pal = [
        (BG_BASE, "bg/base", BG_BASE), (BG_SUNKEN, "bg/sunken", BG_SUNKEN),
        (BG_RAISED, "bg/raised", BG_RAISED), (BG_OVERLAY, "bg/overlay", BG_OVERLAY),
        (BORDER_SUBTLE, "border/subtle", BORDER_SUBTLE), (BORDER_STRONG, "border/strong", BORDER_STRONG),
        (TEXT_PRIMARY, "text/primary", TEXT_PRIMARY), (TEXT_SECONDARY, "text/secondary", TEXT_SECONDARY),
        (TEXT_TERTIARY, "text/tertiary", TEXT_TERTIARY),
    ]
    for i, (color, name, hexv) in enumerate(pal):
        swatch(32 + (i % 5) * 88, 132 + (i // 5) * 92, color, name, hexv)
    acc = [
        (ACCENT, "accent", ACCENT), (ACCENT_HOVER, "accent/hover", ACCENT_HOVER),
        (ACCENT_SUBTLE, "accent/subtle", ACCENT_SUBTLE),
        (SUCCESS, "success", SUCCESS), (WARNING, "warning", WARNING), (DANGER, "danger", DANGER),
    ]
    for i, (color, name, hexv) in enumerate(acc):
        swatch(32 + i * 88, 316, color, name, hexv)

    b.text(32, 448, "字阶", 15, TEXT_PRIMARY)
    for i, (size, label, color) in enumerate([
        (24, "title-1 / 24", TEXT_PRIMARY),
        (20, "title-2 / 20", TEXT_PRIMARY),
        (15, "title-3 / 15", TEXT_PRIMARY),
        (13, "body / 13", TEXT_PRIMARY),
        (12, "label / 12", TEXT_SECONDARY),
        (11, "caption / 11", TEXT_TERTIARY),
    ]):
        y = 480 + i * 22
        b.text(32, y, "截图 Cuter", size, color)
        b.text(178, y, label, 11, TEXT_TERTIARY)

    b.text(420, 448, "间距 / 圆角 / 状态", 15, TEXT_PRIMARY)
    for i, (w, label) in enumerate([(4, "space-1 = 4"), (8, "space-2 = 8"), (12, "space-3 = 12"),
                                    (16, "space-4 = 16"), (24, "space-5 = 24"), (32, "space-6 = 32")]):
        y = 480 + i * 22
        b.rect(420, y - 8, w * 2, 10, 2, ACCENT, None)
        b.text(420 + w * 2 + 10, y, label, 11, TEXT_TERTIARY)

    b.text(660, 448, "控件状态（按钮）", 15, TEXT_PRIMARY)
    states = [("normal", BG_RAISED, BORDER_SUBTLE, TEXT_PRIMARY),
              ("hover", "#2A2D36", BORDER_STRONG, TEXT_PRIMARY),
              ("pressed", "#333743", BORDER_STRONG, TEXT_PRIMARY),
              ("focus", BG_RAISED, ACCENT, TEXT_PRIMARY),
              ("disabled", BG_RAISED, BORDER_SUBTLE, TEXT_TERTIARY)]
    for i, (label, fill, stroke, fg) in enumerate(states):
        y = 470 + i * 28
        b.rect(660, y, 100, 24, 8, fill, stroke, 1.4 if label == "focus" else 1)
        b.text(710, y + 16, "复制", 12, fg, "middle")
        b.text(770, y + 17, label, 11, TEXT_TERTIARY)

    b.text(32, 614, "圆角：输入框 6 · 按钮 8 · 卡片 10 · 窗口与弹层 14　　阴影：弹层 0 8 24 rgba(0,0,0,.45)",
           11, TEXT_TERTIARY)
    return b


# ---------------------------------------------------------------- 2. 设置窗口
def nav_icon(b: Board, kind: str, x: float, y: float, color: str) -> None:
    """侧栏图标：16x16 网格、1.6px 描边、圆头。（真实实现里这些是矢量绘制的。）"""
    cx, cy = x + 8, y + 8
    if kind == "gear":
        b.circle(cx, cy, 3.2, None, color, 1.6)
        for dx, dy in ((0, -6), (0, 6), (-6, 0), (6, 0)):
            b.line(cx + dx * 0.7, cy + dy * 0.7, cx + dx, cy + dy, color, 1.6)
    elif kind == "keys":
        b.rect(cx - 7, cy - 5, 14, 10, 3, None, color, 1.6)
        b.line(cx - 4, cy - 1, cx + 4, cy - 1, color, 1.4)
    elif kind == "crop":
        b.line(cx - 5, cy - 7, cx - 5, cy + 3, color, 1.6)
        b.line(cx - 7, cy - 5, cx + 3, cy - 5, color, 1.6)
        b.line(cx + 3, cy - 5, cx + 3, cy + 7, color, 1.6)
        b.line(cx - 5, cy + 3, cx + 7, cy + 3, color, 1.6)
    elif kind == "save":
        b.rect(cx - 6, cy - 6, 12, 12, 2, None, color, 1.6)
        b.rect(cx - 2.5, cy - 6, 5, 4, 1, color)
    elif kind == "pen":
        icon_pen(b, cx, cy, color)
    elif kind == "scroll":
        b.rect(cx - 5, cy - 7, 10, 14, 3, None, color, 1.6)
        b.line(cx, cy - 3, cx, cy + 3, color, 1.6)
        b.line(cx, cy + 1, cx - 2, cy - 1, color, 1.4)
        b.line(cx, cy + 1, cx + 2, cy - 1, color, 1.4)
    elif kind == "ocr":
        icon_text(b, cx, cy, color)
    elif kind == "pin":
        b.circle(cx - 1, cy - 1, 4, None, color, 1.6)
        b.line(cx + 2, cy + 2, cx + 6, cy + 6, color, 1.6)
    elif kind == "history":
        b.circle(cx, cy, 6, None, color, 1.6)
        b.line(cx, cy, cx, cy - 3, color, 1.5)
        b.line(cx, cy, cx + 3, cy + 1, color, 1.5)
    elif kind == "font":
        b.line(cx - 5, cy - 5, cx + 5, cy - 5, color, 1.6)
        b.line(cx, cy - 5, cx, cy + 5, color, 1.6)
    elif kind == "diag":
        b.line(cx - 6, cy, cx - 2, cy, color, 1.6)
        b.line(cx - 2, cy, cx, cy - 5, color, 1.6)
        b.line(cx, cy - 5, cx + 2, cy + 5, color, 1.6)
        b.line(cx + 2, cy + 5, cx + 6, cy, color, 1.6)
    else:  # about
        b.circle(cx, cy, 6, None, color, 1.6)
        b.line(cx, cy - 1, cx, cy + 4, color, 1.6)
        b.circle(cx, cy - 4, 1.0, color)


def settings_board() -> Board:
    b = Board("settings", 980, 660)

    # 侧栏
    b.rect(0, 0, 220, 660, 0, BG_SUNKEN)
    b.line(220, 0, 220, 660, BORDER_SUBTLE)
    b.rect(24, 22, 22, 22, 6, ACCENT)
    b.circle(35, 33, 5.5, None, WHITE, 1.6)
    b.line(35, 24.5, 35, 28, WHITE, 1.6)
    b.line(35, 38, 35, 41.5, WHITE, 1.6)
    b.text(56, 38, "Cuter", 15, TEXT_PRIMARY)

    groups = [
        ("常规", [("gear", "通用", True), ("keys", "快捷键", False)]),
        ("截图", [("crop", "截图", False), ("save", "保存", False),
                  ("pen", "标注", False), ("scroll", "滚动截屏", False)]),
        ("智能", [("ocr", "识别文字", False), ("pin", "贴图", False)]),
        ("系统", [("history", "截图历史", False), ("font", "字体", False),
                  ("diag", "诊断", False), ("about", "关于", False)]),
    ]
    y = 74
    for title, items in groups:
        b.text(28, y + 10, title, 11, TEXT_TERTIARY)
        y += 22
        for kind, label, active in items:
            if active:
                b.rect(16, y, 188, 32, 8, ACCENT_SUBTLE)
                b.rect(16, y + 8, 3, 16, 2, ACCENT)
            nav_icon(b, kind, 30, y + 8, ACCENT if active else TEXT_SECONDARY)
            b.text(56, y + 21, label, 13, ACCENT if active else TEXT_SECONDARY)
            y += 36
        y += 8

    b.text(24, 636, "v0.1.23 · 纯 Rust / egui", 10, TEXT_TERTIARY)

    # 内容区
    cx = 220
    b.text(cx + 32, 52, "通用", 20, TEXT_PRIMARY)
    b.text(cx + 32, 74, "启动方式、外观与语言；每一项都会立刻生效。", 12, TEXT_SECONDARY)

    # 主操作：一个主按钮 + 溢出菜单（不再横铺七个）
    b.rect(cx + 560, 30, 120, 32, 8, ACCENT)
    b.text(cx + 620, 51, "开始截图", 12, WHITE, "middle")
    b.rect(cx + 688, 30, 32, 32, 8, BG_RAISED, BORDER_SUBTLE, 1)
    for i in range(3):
        b.circle(cx + 704, 38 + i * 4, 1.6, TEXT_SECONDARY)

    # 卡片：外观与启动（行距 52，五行必须都装得下）
    card(b, cx + 24, 100, 712, 308)
    b.text(cx + 44, 130, "外观与启动", 15, TEXT_PRIMARY)
    rows = [
        ("语言", "界面语言；「跟随系统」按 Windows 显示语言决定", None, "select", "跟随系统"),
        ("主题", "深色 / 浅色 / 跟随系统", None, "select", "深色"),
        ("启动后只驻留托盘", "不自动显示这个窗口；热键依然生效", True, "check", ""),
        ("关闭窗口时退到托盘", "关掉窗口不退出程序，托盘图标仍然在", True, "check", ""),
        ("开机自动启动", "只写一条指向本程序的 HKCU 注册表项", False, "check", ""),
    ]
    ry = 158
    for i, (label, hint, checked, kind, value) in enumerate(rows):
        if i:
            b.line(cx + 44, ry - 14, cx + 716, ry - 14, BORDER_SUBTLE)
        b.text(cx + 44, ry + 12, label, 13, TEXT_PRIMARY)
        b.text(cx + 44, ry + 30, hint, 11, TEXT_TERTIARY)
        if kind == "check":
            checkbox(b, cx + 690, ry + 6, bool(checked))
        else:
            b.rect(cx + 560, ry + 2, 156, 30, 6, BG_SUNKEN, BORDER_STRONG, 1)
            b.text(cx + 574, ry + 22, value, 12, TEXT_PRIMARY)
            b.line(cx + 690, ry + 15, cx + 696, ry + 15, TEXT_SECONDARY, 1.6)
            b.line(cx + 696, ry + 15, cx + 693, ry + 12, TEXT_SECONDARY, 1.6)
            b.line(cx + 696, ry + 15, cx + 693, ry + 18, TEXT_SECONDARY, 1.6)
        ry += 52

    # 卡片：贴图
    card(b, cx + 24, 424, 712, 152)
    b.text(cx + 44, 454, "贴图", 15, TEXT_PRIMARY)
    pin_rows = [
        ("贴图图片裁成圆角", "半径随图片大小自适应（短边 6%，2~24 px）", True),
        ("截图保存后弹托盘通知", "启动时也会提示一次热键", True),
    ]
    ry = 482
    for i, (label, hint, checked) in enumerate(pin_rows):
        if i:
            b.line(cx + 44, ry - 14, cx + 716, ry - 14, BORDER_SUBTLE)
        b.text(cx + 44, ry + 12, label, 13, TEXT_PRIMARY)
        b.text(cx + 44, ry + 30, hint, 11, TEXT_TERTIARY)
        checkbox(b, cx + 690, ry + 6, checked)
        ry += 52

    b.text(cx + 24, 612, "改完立刻生效，配置写在 %APPDATA%\\Cuter\\config.toml（便携版在 exe 旁边）。",
           11, TEXT_TERTIARY)
    b.text(cx + 24, 632, "这是窗口客户区；外面还有系统的标题栏与边框。", 11, TEXT_TERTIARY)
    return b


# ---------------------------------------------------------------- 3. OCR 面板
def ocr_board() -> Board:
    b = Board("ocr-panel", 980, 620)
    b.rect(0, 0, 980, 52, 0, BG_RAISED)
    b.line(0, 52, 980, 52, BORDER_SUBTLE)
    b.text(24, 32, "识别结果", 15, TEXT_PRIMARY)
    x = 108
    for label in ("zh-Hans-CN", "2 行", "55 ms"):
        x += chip(b, x, 18, label) + 6

    # 分段控件：编辑全文 / 逐行对照
    b.rect(700, 12, 256, 30, 8, BG_SUNKEN, BORDER_SUBTLE, 1)
    b.rect(702, 14, 126, 26, 6, ACCENT_SUBTLE)
    b.text(765, 32, "编辑全文", 12, ACCENT, "middle")
    b.text(891, 32, "逐行对照", 12, TEXT_SECONDARY, "middle")

    # 左：图像 + 识别框
    card(b, 24, 76, 560, 452, 10, BG_SUNKEN)
    b.rect(40, 92, 528, 420, 6, "#0F1013")
    for i, (bx, by, bw) in enumerate([(60, 140, 300), (60, 196, 210)]):
        b.rect(bx, by, bw, 34, 4, None, ACCENT, 1.6)
        b.text(bx, by - 8, str(i + 1), 10, ACCENT)
    b.text(60, 140 + 24, "ter OCR 缩 導琛?second line abcdef", 15, "#D8DCE6")
    b.text(40, 500, "这里显示截图本身；蓝框是识别出的文本块，编号与右侧行号一一对应（示意）。", 11, TEXT_TERTIARY)
    b.text(60, 196 + 24, "笙琛?12345", 15, "#D8DCE6")

    # 右：文本列
    b.text(608, 96, "识别文本", 12, TEXT_TERTIARY)
    rows = ["ter OCR 缩 導琛?second line abcdef", "笙琛?12345"]
    ry = 108
    for i, line in enumerate(rows):
        hover = i == 0
        b.rect(604, ry, 352, 34, 8, ACCENT_SUBTLE if hover else BG_RAISED)
        b.text(616, ry + 22, str(i + 1), 11, ACCENT if hover else TEXT_TERTIARY)
        b.text(636, ry + 22, line, 13, TEXT_PRIMARY)
        if hover:
            b.rect(920, ry + 8, 24, 18, 6, BG_OVERLAY, BORDER_SUBTLE, 1)
            b.text(932, ry + 21, "复制", 9, TEXT_SECONDARY, "middle")
        ry += 40

    b.rect(604, 200, 352, 48, 10, BG_RAISED, BORDER_SUBTLE, 1)
    b.text(620, 224, "按 Ctrl+C 复制选中行 · H 悬停高亮对应框", 11, TEXT_TERTIARY)
    b.text(620, 240, "识别结果可编辑校对，改动会写回历史", 11, TEXT_TERTIARY)

    # 底栏
    b.line(0, 556, 980, 556, BORDER_SUBTLE)
    b.rect(0, 556, 980, 64, 0, BG_RAISED)
    b.text(24, 592, "识别 2 行，用时 55 ms", 11, TEXT_TERTIARY)
    b.rect(760, 572, 92, 32, 8, BG_RAISED, BORDER_STRONG, 1)
    b.text(806, 593, "重新识别", 12, TEXT_PRIMARY, "middle")
    b.rect(864, 572, 92, 32, 8, ACCENT)
    b.text(910, 593, "复制全部", 12, WHITE, "middle")
    return b


# ---------------------------------------------------------------- 4. 遮罩工具条
def toolbar_board() -> Board:
    b = Board("overlay-toolbar", 700, 300)

    # 选区信息胶囊
    b.rect(24, 20, 200, 30, 8, BG_OVERLAY, BORDER_SUBTLE, 1)
    b.text(36, 40, "840 × 600", 12, TEXT_PRIMARY)
    b.text(124, 40, "(130, 170)", 11, TEXT_TERTIARY)

    # 先排版、再画底板：底板宽度必须由实际占用算出来，否则按钮会画到外面去。
    def draw_tool(kind, x, y, active):
        if active:
            b.rect(x, y, 34, 34, 8, ACCENT_SUBTLE)
        cx, cy = x + 17, y + 17
        color = ACCENT if active else TEXT_SECONDARY
        if kind == "rect":
            icon_rect(b, cx, cy, color)
        elif kind == "ellipse":
            icon_ellipse(b, cx, cy, color)
        elif kind == "arrow":
            icon_arrow(b, cx, cy, color)
        elif kind == "pen":
            icon_pen(b, cx, cy, color)
        elif kind == "high":
            b.rect(cx - 6, cy - 2, 12, 6, 2, None, color, 1.8)
            b.line(cx - 6, cy + 5, cx + 6, cy + 5, color, 1.8)
        elif kind == "mosaic":
            for dx in (-5, 0):
                for dy in (-5, 0):
                    b.rect(cx + dx, cy + dy, 4, 4, 1, color)
        elif kind == "blur":
            icon_blur(b, cx, cy, color)
        elif kind == "text":
            icon_text(b, cx, cy, color)
        else:
            b.circle(cx, cy, 6, None, color, 1.6)
            b.text(cx, cy + 4, "1", 9, color, "middle")

    def draw_style(x, y, which):
        b.rect(x, y, 34, 34, 8, BG_RAISED)
        if which == 0:      # 线宽
            b.rect(x + 8, y + 20, 18, 4, 2, TEXT_SECONDARY)
            b.rect(x + 8, y + 12, 18, 2, 1, TEXT_TERTIARY)
            b.rect(x + 8, y + 26, 18, 6, 3, TEXT_TERTIARY)
        elif which == 1:    # 颜色
            b.rect(x + 8, y + 8, 18, 18, 4, None, TEXT_SECONDARY, 1.6)
            b.rect(x + 12, y + 12, 10, 10, 2, DANGER)
        else:               # 更多
            for i in range(3):
                b.circle(x + 17, y + 11 + i * 6, 1.6, TEXT_SECONDARY)

    def draw_undo(x, y, undo):
        b.rect(x, y, 34, 34, 8, BG_RAISED)
        color = TEXT_TERTIARY if undo else TEXT_SECONDARY
        b.line(x + 10, y + 19, x + 24, y + 19, color, 1.8)
        tip = x + 10 if undo else x + 24
        sgn = 1 if undo else -1
        b.line(tip, y + 19, tip + 5 * sgn, y + 14, color, 1.8)
        b.line(tip, y + 19, tip + 5 * sgn, y + 24, color, 1.8)

    # 布局：[(kind, 宽度)]，separator 用窄项表示
    layout: list[tuple[str, float]] = []
    tools = ["rect", "ellipse", "arrow", "pen", "high", "mosaic", "blur", "text", "num"]
    layout += [("tool:" + t, 34) for t in tools]
    layout.append(("sep", 13))
    layout += [("style:0", 34), ("style:1", 34), ("style:2", 34)]
    layout.append(("sep", 13))
    layout += [("undo:1", 34), ("undo:0", 34)]
    layout.append(("sep", 13))
    layout.append(("done", 34))

    tx, ty, th = 24, 72, 48
    inner = 12
    bar_w = inner * 2 + sum(w for _, w in layout)
    b.rect(tx, ty, bar_w, th, 12, BG_OVERLAY, BORDER_SUBTLE, 1)

    x = tx + inner
    for kind, w in layout:
        if kind == "sep":
            b.line(x + w / 2, ty + 14, x + w / 2, ty + 34, BORDER_STRONG, 1)
        elif kind.startswith("tool:"):
            draw_tool(kind.split(":", 1)[1], x, ty + 7, kind == "tool:rect")
        elif kind.startswith("style:"):
            draw_style(x, ty + 7, int(kind.split(":")[1]))
        elif kind.startswith("undo:"):
            draw_undo(x, ty + 7, kind == "undo:1")
        else:
            b.rect(x, ty + 7, 34, 34, 8, ACCENT)
            icon_check(b, x + 17, ty + 17, WHITE, 6)
        x += w

    b.text(24, 152, f"工具条 1:1（高 48，按钮 34，圆角 12，实际宽 {bar_w:g}）——选中工具用 accent/subtle 底 + accent 图标。",
           11, TEXT_TERTIARY)
    b.text(24, 170, "分组顺序：9 种标注工具 ｜ 线宽·颜色·更多 ｜ 撤销·重做 ｜ 完成（强调色，唯一的实心按钮）。",
           11, TEXT_TERTIARY)
    b.text(24, 188, "数字键 1-9 切工具、Enter 完成、Esc 取消——快捷键写进 tooltip，不占界面。", 11, TEXT_TERTIARY)

    # 工具选项弹窗（线宽）
    px, py, pw = 24, 212, 340
    b.rect(px, py, pw, 64, 12, BG_OVERLAY, BORDER_SUBTLE, 1)
    b.text(px + 14, py + 24, "线宽", 11, TEXT_TERTIARY)
    b.rect(px + 52, py + 14, 130, 6, 3, BORDER_STRONG)
    b.rect(px + 52, py + 14, 74, 6, 3, ACCENT)
    b.circle(px + 126, py + 17, 7, WHITE)
    b.text(px + 192, py + 24, "3 px", 12, TEXT_PRIMARY)
    b.line(px + 236, py + 12, px + 236, py + 44, BORDER_SUBTLE, 1)
    for i, w in enumerate((2, 5, 9)):
        b.rect(px + 250, py + 14 + i * 11, 24, w, 4, ACCENT if i == 1 else BORDER_STRONG)
    b.text(px + 288, py + 24, "预设", 11, TEXT_TERTIARY)
    return b


# ---------------------------------------------------------------- main
def main() -> None:
    print("生成设计稿：")
    for board in (tokens_board(), settings_board(), ocr_board(), toolbar_board()):
        write(board)


if __name__ == "__main__":
    main()
