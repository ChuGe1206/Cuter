"""把 `en_table.py` 生成的条目**原地替换**进 `crates/cuter-ui/src/i18n.rs` 的 TABLE。

必须幂等：跑两次的结果和跑一次一样（前几次手工拼接就是这么把 `];` 拼重了的）。
做法是定位 `static TABLE: &[(&str, &str)] = &[` 到其后第一个单独的 `];`，整段换掉。

用法：python tools/i18n/splice_table.py [--write]
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

ROOT = Path(__file__).resolve().parents[2]
TARGET = ROOT / "crates" / "cuter-ui" / "src" / "i18n.rs"
HEADER = "static TABLE: &[(&str, &str)] = &["


def main() -> int:
    write = "--write" in sys.argv
    import en_table

    keys_path = ROOT / "target" / "i18n-keys.json"
    if not keys_path.exists():
        print("先跑 extract_keys.py 生成 target/i18n-keys.json")
        return 1
    en_table.main  # noqa: B018 - 只是让静态检查知道我们依赖它
    lines = TARGET.read_text(encoding="utf-8").splitlines()
    try:
        start = next(i for i, line in enumerate(lines) if line.startswith(HEADER))
    except StopIteration:
        print("找不到 TABLE 起点")
        return 1
    end = next(
        (i for i in range(start + 1, len(lines)) if lines[i].strip() == "];"),
        None,
    )
    if end is None:
        print("找不到 TABLE 终点")
        return 1

    body = []
    # 排序要按**运行时**的字符串：源码字面量里的 `\n` 是换行（0x0A），排在 `[`（0x5B）前面，
    # 按字面量（反斜杠 0x5C）排就反了——第一版就是这么让二分查找找不到三条文案的。
    # 用 chr(92) 拼反斜杠，免得被各层引号转义绕进去。
    backslash = chr(92)

    def sort_key(text: str) -> str:
        return text.replace(backslash + "n", "\n").replace(backslash + '"', '"')

    for key in sorted(en_table.EN, key=sort_key):
        value = en_table.EN[key].replace('"', '\\"')
        escaped = key.replace('"', '\\"')
        body.append(f'    ("{escaped}", "{value}"),')
    new_lines = lines[:start] + [HEADER] + body + ["];"] + lines[end + 1 :]
    # 去掉紧跟其后的重复 `];`（历史遗留）
    while new_lines[len(lines[:start]) + len(body) + 2 :][:1] == ["];"]:
        del new_lines[len(lines[:start]) + len(body) + 2]
    print(f"entries: {len(body)}")
    if write:
        TARGET.write_text("\n".join(new_lines) + "\n", encoding="utf-8")
        print(f"wrote {TARGET.relative_to(ROOT)}")
    else:
        print("(dry-run)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
