"""把**跨行**的 `format!(tr!("…"), 参数…)` 改写成 `fill!(tr!("…"), 参数…)`。

单行的那种由 `i18n_convert.process_line` 处理；跨行写法（`format!(` 换行之后才是
`tr!("…")`）在逐行扫描里看不出来——第一版漏了它们，于是 `format!` 收到了一个函数调用
当格式串，编译直接失败 15 处。

用法：python tools/i18n/fix_multiline_format.py [--write]
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import i18n_convert as c  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
TARGETS = [ROOT / "crates" / "cuter-ui" / "src", ROOT / "crates" / "cuter-app" / "src"]


def main() -> int:
    write = "--write" in sys.argv
    total = 0
    for directory in TARGETS:
        for path in sorted(directory.glob("*.rs")):
            if path.name == "i18n.rs":
                continue
            text = path.read_text(encoding="utf-8")
            if "format!(" not in text:
                continue
            out = ""
            index = 0
            replaced = 0
            while True:
                start = text.find("format!(", index)
                if start < 0:
                    out += text[index:]
                    break
                open_paren = text.index("(", start)
                close = c.find_balanced(text, open_paren)
                if close < 0:
                    out += text[index:]
                    break
                inner = text[open_paren + 1 : close]
                args = c.split_top_level(inner)
                if not args or not args[0].startswith('tr!("'):
                    out += text[index : close + 1]
                    index = close + 1
                    continue
                template_literal = args[0]
                rest = args[1:]
                template_text = template_literal[len('tr!("') : -2]
                template_text, extra = c.convert_template(template_text, rest)
                rest = rest + extra
                if rest:
                    rendered = ", ".join(rest)
                    replacement = f'fill!(tr!("{template_text}"), {rendered})'
                else:
                    replacement = f'tr!("{template_text}")'
                out += text[index:start] + replacement
                index = close + 1
                replaced += 1
            if replaced:
                total += replaced
                print(f"{path.name}: {replaced}")
                if write:
                    path.write_text(out, encoding="utf-8")
    print(f"{'converted' if write else 'would convert'}: {total}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
