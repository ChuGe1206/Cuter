import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import i18n_convert as c

ROOT = Path(__file__).resolve().parents[2]
total = 0
for directory in [ROOT / "crates" / "cuter-ui" / "src", ROOT / "crates" / "cuter-app" / "src"]:
    for path in sorted(directory.glob("*.rs")):
        source = path.read_text(encoding="utf-8")
        if "fill!(tr!(" not in source:
            continue
        out, changed = [], 0
        for line in source.splitlines(keepends=True):
            pos = 0
            while True:
                start = line.find("fill!(tr!(", pos)
                if start < 0:
                    break
                open_paren = line.index("(", start)
                close = c.find_balanced(line, open_paren)
                if close < 0:
                    break
                inner = line[open_paren + 1 : close]
                parts = c.split_top_level(inner)
                template = parts[0]
                rest = parts[1:]
                if len(rest) == 1 and rest[0].startswith("[") and rest[0].endswith("]"):
                    rest = c.split_top_level(rest[0][1:-1])
                rest = [a.lstrip("&").strip() for a in rest if a.strip()]
                replacement = "fill!(" + ", ".join([template] + rest) + ")"
                line = line[:start] + replacement + line[close + 1 :]
                pos = start + len(replacement)
                changed += 1
            if changed:
                total += changed
            out.append(line)
        if changed:
            print(f"{path.name}: {changed}")
            path.write_text("".join(out), encoding="utf-8")
print("sites:", total)
