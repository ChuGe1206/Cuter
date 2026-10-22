import re, json
from pathlib import Path
ROOT = Path(__file__).resolve().parents[2]
keys = {}
pattern = re.compile(r'tr!\("((?:[^"\\]|\\.)*)"\)')
for directory in [ROOT / "crates" / "cuter-ui" / "src", ROOT / "crates" / "cuter-app" / "src"]:
    for path in sorted(directory.glob("*.rs")):
        if path.name == "i18n.rs":
            continue
        for match in pattern.finditer(path.read_text(encoding="utf-8")):
            keys.setdefault(match.group(1), []).append(path.name)
print("keys:", len(keys))
Path(ROOT / "target" / "i18n-keys.json").write_text(json.dumps(sorted(keys), ensure_ascii=False, indent=1), encoding="utf-8")
for k in sorted(keys)[:8]:
    print(" ", k, "->", keys[k][0])
