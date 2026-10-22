"""配置项 ↔ 代码作用点的静态审计。

用法（在仓库根目录）：

    python tools\\docs\\audit_config.py          # 每个配置字段被引用几次
    python tools\\docs\\audit_config.py --api    # cuter-core 里没有调用者的 pub fn

「引用次数」只统计 config.rs 之外的 .rs 文件，所以次数为 0（或只有 config.rs 里那几处）
就说明这个字段没人读——`docs/config-audit.md` 要求每个字段都有唯一作用点。

这不是万能的：它只能证明"有人读"，不能证明"行为对"（见文档第 6 节）。
真正的背书仍然是真实进程里的探针。
"""

import argparse
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATES = ROOT / "crates"
CONFIG = CRATES / "cuter-core/src/config.rs"


def struct_fields(text: str) -> dict[str, list[str]]:
    structs: dict[str, list[str]] = {}
    for match in re.finditer(r"pub struct (\w+) \{(.*?)\n\}", text, re.S):
        name, body = match.group(1), match.group(2)
        fields = re.findall(r"^\s{4}pub (\w+):", body, re.M)
        if fields:
            structs[name] = fields
    return structs


def rust_sources() -> list[Path]:
    return [path for path in CRATES.rglob("*.rs")]


def audit_config() -> int:
    text = CONFIG.read_text(encoding="utf-8")
    consumers = "\n".join(
        path.read_text(encoding="utf-8", errors="replace")
        for path in rust_sources()
        if path != CONFIG
    )

    dead: list[tuple[str, str]] = []
    print(f"{'字段':44s} 引用次数")
    for struct, fields in struct_fields(text).items():
        for field in fields:
            uses = len(re.findall(rf"\b{re.escape(field)}\b", consumers))
            mark = "  <-- 没人读" if uses == 0 else ""
            print(f"{struct + '.' + field:44s} {uses:3d}{mark}")
            if uses == 0:
                dead.append((struct, field))

    print()
    if dead:
        print(f"没人读的字段 {len(dead)} 个：")
        for struct, field in dead:
            print(f"  {struct}.{field}")
        return 1
    print("所有字段都有读者。")
    return 0


def audit_api() -> int:
    files = {
        path: path.read_text(encoding="utf-8", errors="replace")
        for path in rust_sources()
        if "cuter-core" in str(path)
    }
    core_all = "\n".join(files.values())
    consumers = "\n".join(
        path.read_text(encoding="utf-8", errors="replace")
        for path in rust_sources()
        if "cuter-core" not in str(path)
    )

    names = {
        match.group(1)
        for text in files.values()
        for match in re.finditer(r"pub (?:const )?fn (\w+)", text)
    }

    skipped = {"new", "default", "fmt", "clone", "eq", "cmp", "hash", "drop"}
    dead = []
    for name in sorted(names):
        if name in skipped or name.startswith("from_"):
            continue
        external = len(re.findall(rf"\b{re.escape(name)}\b", consumers))
        if external:
            continue
        internal = len(re.findall(rf"\b{re.escape(name)}\s*\(", core_all)) - len(
            re.findall(rf"pub (?:const )?fn {re.escape(name)}\s*\(", core_all)
        )
        dead.append((name, internal))

    print(f"cuter-core pub fn {len(names)} 个，app/ui 一个都没调用 {len(dead)} 个：")
    for name, internal in dead:
        note = "  <-- core 内部也没人调" if internal == 0 else ""
        print(f"  {name:38s} core 内部 {internal:2d} 次{note}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api", action="store_true", help="改为列出没人调用的 pub fn")
    args = parser.parse_args()
    return audit_api() if args.api else audit_config()


if __name__ == "__main__":
    raise SystemExit(main())
