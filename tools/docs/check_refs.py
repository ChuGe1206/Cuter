"""检查文档里引用的仓库路径与 HANDOFF 小节号是否真的存在。

文档最容易悄悄腐烂：改了文件名、拆了小节，引用还挂在那里，读者点进去是空的。
这个脚本把"引用"当成断言来跑——**每次提交门禁都会跑它**。

检查两类东西：
  1. 反引号里的、看起来像本仓库路径的字符串（`docs/xxx.md` / `tools/...` / `crates/...`）；
  2. "HANDOFF 第 N 节" 这样的小节引用（以及 "第 N.M 节"）。

刻意保守：只对"明显是路径"的（含 `/` 且以已知顶层目录开头，或者以 .md/.rs/.ps1/.toml/.yml 结尾）
做检查，免得把配置项、字段名误判成路径。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DOCS = [
    ROOT / "README.md",
    ROOT / "CHANGELOG.md",
    *sorted((ROOT / "docs").glob("*.md")),
    *sorted((ROOT / "tools").rglob("*.md")),
    *sorted((ROOT / ".github").rglob("*.md")),
]

TOP_LEVEL = ("crates/", "docs/", "tools/", "assets/", "dist/", ".github/")
SUFFIXES = (".md", ".rs", ".ps1", ".toml", ".yml", ".yaml", ".cs", ".txt", ".iss")
# 只对这些顶层名字开头的引用做存在性检查；其它（含外部依赖源码路径）跳过
KNOWN_ROOTS = (
    "crates",
    "cuter-core",
    "cuter-ui",
    "cuter-app",
    "src",
    "docs",
    "tools",
    "assets",
    ".github",
)

BACKTICK = re.compile(r"`([^`\n]+)`")
HANDOFF_SECTION = re.compile(r"HANDOFF\s*第\s*(\d+)(?:\.(\d+))?\s*节")


def looks_like_path(text: str) -> bool:
    if " " in text or text.startswith("http"):
        return False
    if text.startswith("-") or text.endswith("/"):
        # 命令行参数与目录名（目录单独判断）
        return False
    if any(text.startswith(prefix) for prefix in TOP_LEVEL):
        return True
    return text.endswith(SUFFIXES) and "/" in text


def repo_paths() -> list[str]:
    """仓库里所有文件的相对路径（跳过 target/.git 与构建产物）。"""
    skip = {"target", ".git", "dist"}
    out = []
    for path in ROOT.rglob("*"):
        if not path.is_file():
            continue
        rel = path.relative_to(ROOT)
        if rel.parts and rel.parts[0] in skip:
            continue
        out.append(rel.as_posix())
    return out


def resolves(part: str, index: list[str]) -> bool:
    """这个引用能不能在仓库里找到。

    文档里常按 crate 内部路径简写（`cuter-core/src/scroll.rs` 而不是
    `crates/cuter-core/src/scroll.rs`），所以除了精确匹配，再按后缀匹配一次。
    """
    if (ROOT / part).exists():
        return True
    suffix = "/" + part
    return any(path.endswith(suffix) for path in index)


def main() -> int:
    problems: list[str] = []
    index = repo_paths()

    handoff = (ROOT / "docs" / "HANDOFF.md").read_text(encoding="utf-8")
    sections = {m.group(1) for m in re.finditer(r"^##\s+(\d+)\.", handoff, re.M)}
    if not sections:
        problems.append("docs/HANDOFF.md 里一个小节都没找到（格式变了？）")

    for doc in DOCS:
        if not doc.exists():
            continue
        rel_doc = doc.relative_to(ROOT).as_posix()
        text = doc.read_text(encoding="utf-8")
        for raw in BACKTICK.findall(text):
            candidate = raw.strip().strip("'\"")
            # `tools/...` 里常带通配符与省略号，跳过
            if "*" in candidate or "…" in candidate or candidate.endswith("..."):
                continue
            for part in re.split(r"[、,，]|\s+", candidate):
                part = part.strip().strip("()（）")
                if not part or not looks_like_path(part):
                    continue
                # 只检查仓库里真实存在的相对路径；带前缀的（如 %APPDATA%）不算
                if part.startswith(("C:", "~", "%", ".")):
                    continue
                first = part.split("/")[0]
                if first not in KNOWN_ROOTS:
                    # 外部路径（例如 cargo registry 里的 egui 源码）不检查
                    continue
                if not resolves(part, index):
                    problems.append(f"{rel_doc}: 引用了不存在的路径 `{part}`")
        for match in HANDOFF_SECTION.finditer(text):
            number = match.group(1)
            if number not in sections:
                problems.append(
                    f"{rel_doc}: 引用了 HANDOFF 第 {number} 节，但该小节不存在"
                )

    # 版本清单必须对齐：CHANGELOG 里有的版本，面向用户的发布说明里也要有（或明确归档）。
    changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    releases_path = ROOT / "docs" / "RELEASES.md"
    if releases_path.exists():
        releases = releases_path.read_text(encoding="utf-8")
        changelog_versions = re.findall(r"^##\s+(\d+\.\d+\.\d+)", changelog, re.M)
        release_versions = set(re.findall(r"^###\s+(\d+\.\d+\.\d+)", releases, re.M))
        # 发布说明只写最近若干个版本，更早的用一句话归档；所以这里检查的是
        # "发布说明里提到的版本必须在 CHANGELOG 里存在"（反向由归档那句话负责）。
        for version in sorted(release_versions - set(changelog_versions)):
            problems.append(
                f"docs/RELEASES.md: 写了版本 {version}，但 CHANGELOG.md 里没有"
            )
        if not release_versions:
            problems.append("docs/RELEASES.md: 一个版本小节都没有（格式变了？）")
        if not changelog_versions:
            problems.append("CHANGELOG.md: 一个版本小节都没有（格式变了？）")

    if problems:
        print(f"发现问题 {len(problems)} 处：")
        for problem in problems[:40]:
            print(f"  - {problem}")
        if len(problems) > 40:
            print(f"  ...（还有 {len(problems) - 40} 处）")
        return 1
    print(f"文档引用检查通过（{len(DOCS)} 个文件，HANDOFF 共 {len(sections)} 个小节）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
