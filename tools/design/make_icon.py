"""Cuter 应用图标：概念稿生成器。

    python tools/design/make_icon.py                 # 出对照表 + 每个概念的 SVG
    python tools/design/make_icon.py --ico region    # 把选定概念导出多尺寸 .ico

为什么要连 16px 一起画：图标好不好看**只能在 16px 下判断**——托盘区就那么大，
大图好看、缩到 16 变糊的方案在这个项目里没有意义。

三个概念：
  region  框选     —— 圆角徽章 + 白色选框 + 四角控制点（截图工具最直接的语言）
  corner  C 裁切角 —— 两个对角括号，中间的缺口读成字母 C（和产品名呼应，且单色下也立得住）
  shutter 快门     —— 圆环 + 中心光点（相机语言，最保守也最"工具"）

每个概念两版：
  badge  彩色徽章版：accent 渐变圆角方 + 白色标记（exe / 安装包 / 开始菜单）
  mono   单色标记版：只有标记，颜色跟随（托盘图标、深浅任务栏都能用）
"""

from __future__ import annotations

import argparse
import math
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "docs" / "design" / "icons"

ACCENT_TOP = "#5C9BFF"
ACCENT_BOTTOM = "#2F63D8"
ACCENT = "#4C8DFF"
WHITE = "#FFFFFF"
INK = "#17181C"

FONTS = [r"C:\Windows\Fonts\msyh.ttc", r"C:\Windows\Fonts\simhei.ttf"]


# ---------------------------------------------------------------- 图元
@dataclass
class RRect:
    x: float
    y: float
    w: float
    h: float
    r: float
    fill: str | None = None
    grad: tuple[str, str] | None = None
    stroke: str | None = None
    sw: float = 0


@dataclass
class Ring:
    cx: float
    cy: float
    r: float
    sw: float
    stroke: str


@dataclass
class Disc:
    cx: float
    cy: float
    r: float
    fill: str


@dataclass
class Bar:
    """圆头粗线（括号、框线都用它，保证 16px 下不断线）。"""

    x1: float
    y1: float
    x2: float
    y2: float
    sw: float
    stroke: str


@dataclass
class Shape:
    items: list = field(default_factory=list)
    # 徽章底色渐变：默认沿用蓝色 accent；新概念（甘字取景）可以覆盖成自己的配色
    badge_grad: tuple[str, str] | None = None

    def rrect(self, *a, **kw):
        self.items.append(RRect(*a, **kw))

    def ring(self, *a, **kw):
        self.items.append(Ring(*a, **kw))

    def disc(self, *a, **kw):
        self.items.append(Disc(*a, **kw))

    def bar(self, *a, **kw):
        self.items.append(Bar(*a, **kw))

    def rot_bar(self, cx, cy, length, angle_deg, sw, stroke):
        a = math.radians(angle_deg)
        dx, dy = math.cos(a) * length / 2, math.sin(a) * length / 2
        self.items.append(Bar(cx - dx, cy - dy, cx + dx, cy + dy, sw, stroke))


# ---------------------------------------------------------------- 概念
def concept_region() -> Shape:
    """框选：白色选框 + 四角控制点。"""
    s = Shape()
    inset, radius, stroke = 60.0, 34.0, 13.0
    s.rrect(inset, inset, 256 - inset * 2, 256 - inset * 2, radius, stroke=WHITE, sw=stroke)
    # 四角控制点：白色小方块稍微压在框线上（16px 下靠它认出"可拖拽"）
    for cx in (inset, 256 - inset):
        for cy in (inset, 256 - inset):
            s.rrect(cx - 12.5, cy - 12.5, 25, 25, 8, fill=WHITE)
    return s


def concept_corner() -> Shape:
    """C 裁切角：左上 + 右下两个括号，缺口读成 C。"""
    s = Shape()
    arm, sw, pad = 76.0, 26.0, 54.0
    s.bar(pad, pad + arm, pad, pad, sw, WHITE)
    s.bar(pad, pad, pad + arm, pad, sw, WHITE)
    e = 256 - pad
    s.bar(e, e - arm, e, e, sw, WHITE)
    s.bar(e, e, e - arm, e, sw, WHITE)
    # 中心一个 accent 小点：暗示"焦点"，也让徽章中央不至于空
    s.disc(128, 128, 17, WHITE)
    return s


def concept_shutter() -> Shape:
    """快门：圆环 + 中心光点。"""
    s = Shape()
    s.ring(128, 128, 66, 18, WHITE)
    s.disc(128, 128, 26, WHITE)
    # 环外四个方向的短刻度，避免 16px 下只剩一个甜甜圈
    for angle in (45, 135, 225, 315):
        s.rot_bar(128, 128, 0, angle, 0, WHITE)  # 占位，下面手动画
        a = math.radians(angle)
        x1, y1 = 128 + math.cos(a) * 76, 128 + math.sin(a) * 76
        x2, y2 = 128 + math.cos(a) * 86, 128 + math.sin(a) * 86
        s.bar(x1, y1, x2, y2, 11, WHITE)
    return s


def concept_viewfinder() -> Shape:
    """甘字取景框（用户选定方向）：圆角方框 + 四角刻度 + 中心点。

    配色换成"紫曜 + 霓虹青"：底色深紫渐变、框线霓虹青、辉光用同色系半透明描边叠两圈
    （不用滤镜，导 SVG / ICO 都不丢效果）。
    """
    s = Shape()
    s.badge_grad = ("#2A1650", "#0B0618")
    line = "#8CF8FF"
    # 辉光：由外到内两圈半透明描边（8 位 hex 的 alpha，PIL 与 SVG 都认）
    s.rrect(58, 58, 140, 140, 30, stroke="#7DF9FF1A", sw=30)
    s.rrect(66, 66, 124, 124, 24, stroke="#7DF9FF2E", sw=20)
    # 方框 + 四角刻度
    s.rrect(76, 76, 104, 104, 18, stroke=line, sw=15)
    s.bar(76, 44, 76, 64, 13, line)
    s.bar(44, 76, 64, 76, 13, line)
    s.bar(180, 44, 180, 64, 13, line)
    s.bar(212, 76, 192, 76, 13, line)
    s.disc(128, 128, 14, WHITE)
    return s


CONCEPTS = {
    "region": ("框选", concept_region),
    "viewfinder": ("甘字取景", concept_viewfinder),
    "corner": ("C 裁切角", concept_corner),
    "shutter": ("快门", concept_shutter),
}


# ---------------------------------------------------------------- SVG
def shape_to_svg(shape: Shape, badge: bool, size: int) -> str:
    svg_top, svg_bot = shape.badge_grad or (ACCENT_TOP, ACCENT_BOTTOM)
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 256 256">',
        "<defs>",
        f'<linearGradient id="badge" x1="0" y1="0" x2="0" y2="1">'
        f'<stop offset="0" stop-color="{svg_top}"/><stop offset="1" stop-color="{svg_bot}"/>'
        "</linearGradient>",
        "</defs>",
    ]
    if badge:
        parts.append('<rect x="0" y="0" width="256" height="256" rx="56" fill="url(#badge)"/>')
    for it in shape.items:
        if isinstance(it, RRect):
            attrs = [
                f'x="{it.x:g}"',
                f'y="{it.y:g}"',
                f'width="{it.w:g}"',
                f'height="{it.h:g}"',
                f'rx="{it.r:g}"',
            ]
            attrs.append(f'fill="{it.fill}"' if it.fill else 'fill="none"')
            if it.grad:
                attrs[-1] = 'fill="url(#badge)"'
            if it.stroke:
                attrs += [f'stroke="{it.stroke}"', f'stroke-width="{it.sw:g}"']
            parts.append(f'<rect {" ".join(attrs)}/>')
        elif isinstance(it, Ring):
            parts.append(
                f'<circle cx="{it.cx:g}" cy="{it.cy:g}" r="{it.r:g}" fill="none" '
                f'stroke="{it.stroke}" stroke-width="{it.sw:g}"/>'
            )
        elif isinstance(it, Disc):
            parts.append(f'<circle cx="{it.cx:g}" cy="{it.cy:g}" r="{it.r:g}" fill="{it.fill}"/>')
        elif isinstance(it, Bar):
            parts.append(
                f'<line x1="{it.x1:g}" y1="{it.y1:g}" x2="{it.x2:g}" y2="{it.y2:g}" '
                f'stroke="{it.stroke}" stroke-width="{it.sw:g}" stroke-linecap="round"/>'
            )
    parts.append("</svg>")
    return "\n".join(parts)


# ---------------------------------------------------------------- PNG
def render(shape: Shape, badge: bool, size: int, bg: str | None = None):
    """把概念渲染成 size×size 的 PIL 图像（4x 超采样）。"""
    from PIL import Image, ImageDraw

    ss = 4
    n = size * ss
    k = n / 256.0
    img = Image.new("RGBA", (n, n), (0, 0, 0, 0))

    if badge:
        # 渐变底：先画竖向渐变，再用圆角矩形当遮罩
        grad = Image.new("RGBA", (n, n))
        gd = ImageDraw.Draw(grad)
        gtop, gbot = shape.badge_grad or (ACCENT_TOP, ACCENT_BOTTOM)
        top = tuple(int(gtop[i : i + 2], 16) for i in (1, 3, 5))
        bot = tuple(int(gbot[i : i + 2], 16) for i in (1, 3, 5))
        for y in range(n):
            t = y / max(1, n - 1)
            gd.line(
                [(0, y), (n, y)],
                fill=(
                    int(top[0] + (bot[0] - top[0]) * t),
                    int(top[1] + (bot[1] - top[1]) * t),
                    int(top[2] + (bot[2] - top[2]) * t),
                    255,
                ),
            )
        mask = Image.new("L", (n, n), 0)
        ImageDraw.Draw(mask).rounded_rectangle([0, 0, n - 1, n - 1], radius=56 * k, fill=255)
        img.paste(grad, (0, 0), mask)
        # 顶部一道极淡的内高光：让扁平渐变有一点体积感（不做出 2010 年的斜面）
        gloss = Image.new("RGBA", (n, n), (0, 0, 0, 0))
        ImageDraw.Draw(gloss).rounded_rectangle(
            [0, 0, n - 1, n - 1], radius=56 * k, fill=(255, 255, 255, 26)
        )
        fade = Image.new("L", (n, n), 0)
        ImageDraw.Draw(fade).rectangle([0, 0, n, int(n * 0.42)], fill=255)
        img.paste(gloss, (0, 0), Image.composite(gloss.split()[3], Image.new("L", (n, n), 0), fade))

    draw = ImageDraw.Draw(img)

    def px(v):
        return v * k

    for it in shape.items:
        if isinstance(it, RRect):
            box = [px(it.x), px(it.y), px(it.x + it.w), px(it.y + it.h)]
            draw.rounded_rectangle(
                box, radius=px(it.r), fill=it.fill, outline=it.stroke, width=max(1, round(px(it.sw)))
            )
        elif isinstance(it, Ring):
            box = [px(it.cx - it.r), px(it.cy - it.r), px(it.cx + it.r), px(it.cy + it.r)]
            draw.ellipse(box, outline=it.stroke, width=max(1, round(px(it.sw))))
        elif isinstance(it, Disc):
            box = [px(it.cx - it.r), px(it.cy - it.r), px(it.cx + it.r), px(it.cy + it.r)]
            draw.ellipse(box, fill=it.fill)
        elif isinstance(it, Bar):
            w = max(1, round(px(it.sw)))
            draw.line([px(it.x1), px(it.y1), px(it.x2), px(it.y2)], fill=it.stroke, width=w)
            for (x, y) in ((it.x1, it.y1), (it.x2, it.y2)):
                r = w / 2
                draw.ellipse([px(x) - r, px(y) - r, px(x) + r, px(y) + r], fill=it.stroke)

    img = img.resize((size, size), Image.LANCZOS)
    if bg:
        base = Image.new("RGBA", (size, size), bg)
        base.alpha_composite(img)
        return base
    return img


# ---------------------------------------------------------------- 对照表
def sheet() -> None:
    from PIL import Image, ImageDraw, ImageFont

    font_path = next((p for p in FONTS if Path(p).exists()), None)
    f_small = ImageFont.truetype(font_path, 13) if font_path else None
    f_tiny = ImageFont.truetype(font_path, 11) if font_path else None

    sizes = [128, 48, 32, 16]
    pad, row_h = 32, 200
    width = 1080
    height = pad * 2 + row_h * len(CONCEPTS) + 150
    img = Image.new("RGB", (width, height), "#141518")
    draw = ImageDraw.Draw(img)

    draw.text((pad, pad - 6), "Cuter 图标 · 三个概念 · 真实尺寸对照（128 / 48 / 32 / 16）", fill="#E8EAF0", font=f_small)

    y = pad + 34
    for name, (label, make) in CONCEPTS.items():
        shape = make()
        draw.text((pad, y + 8), f"{label}（{name}）", fill="#A8AEBD", font=f_small)
        x = pad + 150
        for size in sizes:
            icon = render(shape, True, size)
            img.paste(icon, (x, y + 60 - size // 2), icon)
            draw.text((x + size // 2 - len(str(size)) * 4, y + 140), f"{size}", fill="#6F7686", font=f_tiny)
            x += size + 40
        # 单色标记版（托盘用）
        mono = render(shape, False, 48)
        tinted = Image.new("RGBA", (48, 48), (0, 0, 0, 0))
        tinted.paste((76, 141, 255, 255), (0, 0, 48, 48), mono.split()[3])
        y_center = y + 60 - 24
        # 深底上
        draw.rounded_rectangle([x + 2, y_center - 6, x + 2 + 60, y_center + 54], radius=8, fill="#1B1D22")
        img.paste(tinted, (x + 8, y_center), tinted)
        # 浅底上
        draw.rounded_rectangle([x + 70, y_center - 6, x + 70 + 60, y_center + 54], radius=8, fill="#F2F3F5")
        img.paste(tinted, (x + 76, y_center), tinted)
        draw.text((x + 8, y + 140), "单色（深/浅底）", fill="#6F7686", font=f_tiny)
        y += row_h

    # 任务栏条：托盘区真实观感（深色任务栏 + 16/24px 图标）
    strip_y = y + 10
    draw.text((pad, strip_y - 22), "托盘区观感（深色任务栏，图标 16 / 24 px）", fill="#A8AEBD", font=f_small)
    draw.rectangle([pad, strip_y, width - pad, strip_y + 48], fill="#1F2024")
    x = pad + 16
    for name, (_, make) in CONCEPTS.items():
        shape = make()
        for size in (16, 24):
            icon = render(shape, True, size)
            img.paste(icon, (x, strip_y + 24 - size // 2), icon)
            x += size + 10
        x += 16
    # 右侧放系统图标当参照（画几个方块示意）
    for i in range(3):
        draw.rounded_rectangle(
            [width - pad - 90 + i * 28, strip_y + 16, width - pad - 74 + i * 28, strip_y + 32],
            radius=4,
            fill="#9AA0AC",
        )

    OUT.mkdir(parents=True, exist_ok=True)
    img.save(OUT / "icon-sheet.png")
    print(f"  icon-sheet.png  ({width}x{height})")


def write_svgs() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, (_, make) in CONCEPTS.items():
        shape = make()
        (OUT / f"{name}-badge.svg").write_text(shape_to_svg(shape, True, 256), encoding="utf-8")
        (OUT / f"{name}-mono.svg").write_text(shape_to_svg(shape, False, 256), encoding="utf-8")
        print(f"  {name}-badge.svg / {name}-mono.svg")


def export_ico(name: str) -> None:
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    shape = CONCEPTS[name][1]()
    icons = [render(shape, True, s) for s in sizes]
    OUT.mkdir(parents=True, exist_ok=True)
    path = OUT / f"cuter-{name}.ico"
    icons[-1].save(path, format="ICO", sizes=[(s, s) for s in sizes])
    print(f"  {path.name}（{len(sizes)} 个尺寸：{'/'.join(str(s) for s in sizes)}）")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ico", choices=sorted(CONCEPTS), help="把选定概念导出成多尺寸 .ico")
    args = parser.parse_args()
    if args.ico:
        export_ico(args.ico)
        return
    print("生成图标概念稿：")
    sheet()
    write_svgs()


if __name__ == "__main__":
    main()
