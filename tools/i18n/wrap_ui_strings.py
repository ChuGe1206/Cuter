"""把界面文案包成 tr!(...)：dry-run 优先，改完必须人工看 diff。

只处理"用户看得见"的调用点（按钮/标签/勾选框/提示/状态/托盘菜单/窗口标题），
并且跳过日志、注释、测试模块。日志与注释一律保持中文（项目约定）。

用法：
    python tools/i18n/wrap_ui_strings.py            # dry-run，只打印将要修改的行
    python tools/i18n/wrap_ui_strings.py --write    # 真正写盘
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGETS = [
    ROOT / "crates" / "cuter-ui" / "src",
    ROOT / "crates" / "cuter-app" / "src",
]

# 这些调用点里的字符串是给用户看的
UI_CALLS = re.compile(
    r"(?:\.button\(|\.label\(|\.checkbox\(|\.radio_value\(|\.selectable_value\(|"
    r"RichText::new\(|\.hover_text\(|\.heading\(|\.on_hover_text\(|"
    r"set_status\(|notify\(|\.window_title\(|\.tooltip\(|MenuItem|TrayIcon|"
    r"ui\.text\(|\.text\(|format!\(|settings_card\(|\.subtitle\(|card\()"
)
# 这些行里出现的中文一律不动（开发用）
NEVER = re.compile(r"log::|//|#\[|panic!|assert|expect\(|unwrap_or_else|Error::|bail!")
# 匹配臂：`Self::General => "通用",`（设置页左侧分类名就是这么写的）
MATCH_ARM = re.compile(r'=>\s*"')
CJK = re.compile(r"[\u4e00-\u9fff]")
STRING = re.compile(r'"(?:[^"\\]|\\.)*"')


def inside_test_module(lines: list[str], index: int) -> bool:
    """粗略判断这一行是否落在 `#[cfg(test)] mod tests { ... }` 里（按花括号配平）。"""
    depth = 0
    test_depth = None
    for i, line in enumerate(lines):
        if i > index:
            break
        if "#[cfg(test)]" in line:
            test_depth = depth + line.count("{") - line.count("}")
            continue
        depth += line.count("{") - line.count("}")
        if test_depth is not None and depth < test_depth:
            test_depth = None
    return test_depth is not None


def wrap_line(line: str, pending: bool = False) -> tuple[str, int]:
    """把这一行里"给用户看的中文字面量"包起来。返回 (新行, 包装数量)。

    `pending` 表示**上面几行**有一个 UI 调用还没闭合——`ui.checkbox(` 换行写标签是常见
    写法，第一版按单行匹配，于是设置页里那些勾选项全漏了（英文界面里它们还是中文）。
    匹配臂 `=> "中文"`（设置页左侧的分类名）同样算界面文案。
    """
    if NEVER.search(line):
        return line, 0
    if not pending and not UI_CALLS.search(line) and not MATCH_ARM.search(line):
        return line, 0
    out = []
    last = 0
    wrapped = 0
    for match in STRING.finditer(line):
        text = match.group(0)
        if not CJK.search(text):
            continue
        # 已经包过就跳过
        prefix = line[: match.start()]
        if prefix.rstrip().endswith("tr!("):
            continue
        out.append(line[last : match.start()])
        out.append(f"tr!({text})")
        last = match.end()
        wrapped += 1
    if wrapped == 0:
        return line, 0
    out.append(line[last:])
    return "".join(out), wrapped


def main() -> int:
    write = "--write" in sys.argv
    total = 0
    for directory in TARGETS:
        for path in sorted(directory.glob("*.rs")):
            if path.name == "i18n.rs":
                continue
            source = path.read_text(encoding="utf-8")
            lines = source.splitlines(keepends=True)
            changed = []
            pending = False
            for index, line in enumerate(lines):
                if inside_test_module(lines, index):
                    changed.append(line)
                    continue
                new_line, count = wrap_line(line, pending)
                total += count
                changed.append(new_line)
                # 这一行是一个 UI 调用、而且括号还没闭合 → 后面几行的字面量也算
                opened = line.count("(") - line.count(")")
                if UI_CALLS.search(line) or MATCH_ARM.search(line):
                    pending = opened > 0
                elif opened < 0:
                    pending = False
            new_source = "".join(changed)
            if new_source != source:
                rel = path.relative_to(ROOT)
                print(f"--- {rel}")
                for old, new in zip(lines, changed):
                    if old != new:
                        print(f"  - {old.rstrip()}")
                        print(f"  + {new.rstrip()}")
                if write:
                    path.write_text(new_source, encoding="utf-8")
    print(f"{'wrapped' if write else 'would wrap'}: {total}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
