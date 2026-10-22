"""把界面文案包成 tr!(...) 并正确处理 format!。

为什么要有这个脚本、以及为什么它比"随手替换"复杂：
界面文案里有一大半长在 `format!` 里（"已保存到 {}"、"识别到 {blocks} 行"）。
`format!` 只接受**字面量**格式串，`tr!` 是函数调用，所以 `format!(tr!("…"))` 编译不过。
正确形态是 `i18n::fill(tr!("…"), &[&a, &b])`，而把 `format!` 改写成 `fill` 需要
**按括号配平地切开参数**——用正则做这件事会把 `{:.0}`、`{name}`、嵌套括号全都切坏
（第一版就是这么把 8 个调用点改坏、并把 `RichText::new(...)` 的右括号吃掉的）。

所以：扫描 → 找到 `format!` → 配平取参数 → 判断第一个参数是不是 `tr!("…")` →
按情况改写。具名捕获 `{name}` 会改成位置 `{}` 并把 `name` 追加到参数表末尾（顺序按
模板里出现的先后），带格式说明符的（`{:.0}`、`{:?}`）会变成 `&format!("{:.0}", 变量)`。

只处理"用户看得见"的调用点，跳过日志、注释、测试模块。

用法：
    python tools/i18n/i18n_convert.py            # dry-run
    python tools/i18n/i18n_convert.py --write
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGETS = [ROOT / "crates" / "cuter-ui" / "src", ROOT / "crates" / "cuter-app" / "src"]

UI_HINT = re.compile(
    r"(?:\.button\(|\.label\(|\.checkbox\(|\.radio_value\(|\.selectable_value\(|"
    r"RichText::new\(|hover_text\(|\.heading\(|set_status\(|set_history_status\(|"
    r"notify\(|\.window_title\(|ui\.text\(|parts\.push\(|\.tooltip\(|"
    r"session\.status\s*=|warnings\.push\(|map_err\(|ok_or_else\(|return Err\(|"
    r"Err\(|body\(|format!\()"
)
NEVER = re.compile(r"log::|^\s*//|#\[|panic!|assert")
CJK = re.compile(r"[\u4e00-\u9fff]")
PLACEHOLDER = re.compile(r"\{([^{}]*)\}")


def find_balanced(text: str, start: int) -> int:
    """从 `start`（正指向左括号）开始找配平的右括号位置，跳过字符串。"""
    depth = 0
    i = start
    while i < len(text):
        ch = text[i]
        if ch == '"':
            i += 1
            while i < len(text):
                if text[i] == "\\":
                    i += 2
                    continue
                if text[i] == '"':
                    break
                i += 1
        elif ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return -1


def split_top_level(text: str) -> list[str]:
    """按顶层逗号切分（跳过括号与字符串）。"""
    parts, depth, current, i = [], 0, [], 0
    while i < len(text):
        ch = text[i]
        if ch == '"':
            current.append(ch)
            i += 1
            while i < len(text):
                current.append(text[i])
                if text[i] == "\\":
                    current.append(text[i + 1])
                    i += 2
                    continue
                if text[i] == '"':
                    break
                i += 1
        elif ch in "([{":
            depth += 1
            current.append(ch)
        elif ch in ")]}":
            depth -= 1
            current.append(ch)
        elif ch == "," and depth == 0:
            parts.append("".join(current).strip())
            current = []
        else:
            current.append(ch)
        i += 1
    tail = "".join(current).strip()
    if tail:
        parts.append(tail)
    return parts


def convert_template(template: str, args: list[str]) -> tuple[str, list[str]]:
    """把模板里的占位符排成 fill 能吃的形态，返回 (新模板, 追加的参数)。"""
    extra: list[str] = []
    out = []
    last = 0
    for match in PLACEHOLDER.finditer(template):
        body = match.group(1)
        out.append(template[last : match.start()])
        last = match.end()
        if body == "":
            out.append("{}")
            continue
        name, _, spec = body.partition(":")
        if name == "":
            # 只有格式说明符，例如 {:.0}：拿参数表里对应位置的那个值先格式化
            index = sum(1 for token in out if token == "{}")
            out.append("{}")
            if spec and index < len(args):
                args[index] = f'format!("{{:{spec}}}", {args[index]})'
            continue
        # 具名捕获：模板里改成位置，参数按出现顺序追加
        out.append("{}")
        extra.append(name)
    out.append(template[last:])
    return "".join(out), extra


def process_line(line: str) -> tuple[str, int]:
    """处理一行里所有 `format!(tr!("…")...)`。返回 (新行, 改写个数)。"""
    if NEVER.search(line) or not UI_HINT.search(line):
        return line, 0
    if "format!(tr!(" not in line:
        return line, 0
    count = 0
    while True:
        index = line.find("format!(tr!(")
        if index < 0:
            break
        # 找到 format! 的左括号
        open_paren = line.index("(", index)
        close = find_balanced(line, open_paren)
        if close < 0:
            break
        inner = line[open_paren + 1 : close]
        args = split_top_level(inner)
        if not args:
            break
        first = args[0]
        match = re.fullmatch(r'tr!\("((?:[^"\\]|\\.)*)"\)', first)
        if not match:
            break
        template = match.group(1)
        rest = args[1:]
        template, extra = convert_template(template, rest)
        rest = rest + extra
        if rest:
            rendered = ", ".join(f"&{a}" for a in rest)
            replacement = f'i18n::fill(tr!("{template}"), &[{rendered}])'
        else:
            replacement = f'tr!("{template}")'
        line = line[:index] + replacement + line[close + 1 :]
        count += 1
    return line, count


def strip_format_only(line: str) -> tuple[str, int]:
    """`format!(tr!("…"))`（无参数）直接退化成 `tr!("…")`。"""
    if NEVER.search(line) or "format!(tr!(" not in line:
        return line, 0
    count = 0
    while True:
        index = line.find("format!(tr!(")
        if index < 0:
            break
        open_paren = line.index("(", index)
        close = find_balanced(line, open_paren)
        if close < 0:
            break
        inner = line[open_paren + 1 : close].strip()
        match = re.fullmatch(r'tr!\("((?:[^"\\]|\\.)*)"\)', inner)
        if not match:
            break
        line = line[:index] + f'tr!("{match.group(1)}")' + line[close + 1 :]
        count += 1
    return line, count


def inside_test_module(lines: list[str], index: int) -> bool:
    depth, test_depth = 0, None
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


def main() -> int:
    write = "--write" in sys.argv
    total = 0
    for directory in TARGETS:
        for path in sorted(directory.glob("*.rs")):
            if path.name == "i18n.rs":
                continue
            source = path.read_text(encoding="utf-8")
            lines = source.splitlines(keepends=True)
            # format! 先处理（它们内部一定有 tr!），再处理剩下的裸 tr! 包装
            stage1 = []
            for index, line in enumerate(lines):
                if inside_test_module(lines, index):
                    stage1.append(line)
                    continue
                new, _ = strip_format_only(line)
                new, _ = process_line(new)
                stage1.append(new)
            from wrap_ui_strings import wrap_line  # 复用同一套包装规则

            final, changed_lines = [], []
            stage1_lines = "".join(stage1).splitlines(keepends=True)
            for index, line in enumerate(stage1_lines):
                if inside_test_module(stage1_lines, index):
                    final.append(line)
                    continue
                new, count = wrap_line(line)
                total += count
                final.append(new)
                if new != line:
                    changed_lines.append((line, new))
            new_source = "".join(final)
            if new_source != source:
                print(f"--- {path.relative_to(ROOT)} ({len(changed_lines)} lines)")
                for old, new in changed_lines[:4]:
                    print(f"  - {old.rstrip()}")
                    print(f"  + {new.rstrip()}")
                if write:
                    path.write_text(new_source, encoding="utf-8")
    print(f"{'converted' if write else 'would convert'}: {total}")
    return 0


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).parent))
    raise SystemExit(main())
