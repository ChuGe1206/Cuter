"""把 `format!(tr!("…"))` 改成能编译的形态。

`format!` 要求**字面量**格式串，`tr!(...)` 是函数调用，所以 `format!(tr!("…"))` 编译不过。
三种形态分别处理：

* 没有参数：`format!(tr!("共 {}"))`             → `tr!("共 {}")`（直接去掉 format!）
* 位置参数：`format!(tr!("共 {}"), n)`           → `i18n::fill(tr!("共 {}"), &[n])`
* 具名捕获：留给人工（只有 4 处，模板里 `{name}` 要改成位置 `{}` 并补参数）

用法：python tools/i18n/fix_format_tr.py [--write]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGETS = [ROOT / "crates" / "cuter-ui" / "src", ROOT / "crates" / "cuter-app" / "src"]

NO_ARGS = re.compile(r'format!\(tr!\("((?:[^"\\]|\\.)*)"\)\)')
POSITIONAL = re.compile(r'format!\(tr!\("((?:[^"\\]|\\.)*)"\),\s*(.+)\)\s*$')


def split_args(text: str) -> list[str]:
    """按顶层逗号切分参数（跳过括号/引号里的逗号）。"""
    parts, depth, current, in_string = [], 0, [], False
    i = 0
    while i < len(text):
        ch = text[i]
        if in_string:
            current.append(ch)
            if ch == "\\":
                current.append(text[i + 1])
                i += 2
                continue
            if ch == '"':
                in_string = False
        elif ch == '"':
            in_string = True
            current.append(ch)
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


def main() -> int:
    write = "--write" in sys.argv
    changed_files = 0
    no_args_count = 0
    positional_count = 0
    for directory in TARGETS:
        for path in sorted(directory.glob("*.rs")):
            source = path.read_text(encoding="utf-8")
            lines = source.splitlines(keepends=True)
            out, touched = [], False
            for line in lines:
                new = line
                if "format!(tr!(" in line and "{" not in line.split("tr!(")[1].split('")')[0]:
                    replaced = NO_ARGS.sub(lambda m: f'tr!("{m.group(1)}")', new)
                    if replaced != new:
                        no_args_count += 1
                        new = replaced
                elif "format!(tr!(" in line:
                    match = POSITIONAL.search(new)
                    if match and not re.search(r"\{[a-zA-Z_][a-zA-Z0-9_]*\}", match.group(1)):
                        template = match.group(1)
                        args = split_args(match.group(2).rstrip("\n").rstrip())
                        rendered = ", ".join(f"&{a}" for a in args)
                        prefix = new[: match.start()]
                        suffix = new[match.end() :]
                        new = (
                            f'{prefix}i18n::fill(tr!("{template}"), &[{rendered}]){suffix}'
                        )
                        positional_count += 1
                if new != line:
                    touched = True
                out.append(new)
            if touched:
                changed_files += 1
                print(f"--- {path.relative_to(ROOT)}")
                if write:
                    path.write_text("".join(out), encoding="utf-8")
    print(f"files={changed_files} no-args={no_args_count} positional={positional_count}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
