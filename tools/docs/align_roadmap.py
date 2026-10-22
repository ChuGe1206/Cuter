"""把 roadmap 里「已经做完但还挂着 ☐」的条目对齐。

每一处替换都必须命中（脚本会报 MISS），避免又出现「看着改了其实没改」。
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGET = ROOT / "docs" / "roadmap.md"

EDITS: list[tuple[str, str]] = [
    # M2：控件级吸附已完成（商用化第十五轮）
    (
        "- [ ] 控件级吸附：需 UIAutomation `IUIAutomation::ElementFromPoint`（或退化为\n"
        "      `WindowFromPoint` + 子窗口枚举），命中优先级为 控件 > 窗口边缘 > 屏幕边缘",
        "- [x] **控件级吸附**（商用化第十五轮，见 HANDOFF 第 28 节）：悬停/点击优先吸到光标下\n"
        "      最深的控件，命中优先级 控件 > 窗口边缘 > 屏幕边缘。⚠️ `ElementFromPoint` 只会返回\n"
        "      遮罩自己（它是最上层的全屏窗口），所以走的是「自己按 z 序找底下那个窗口 +\n"
        "      逐层下潜 UIA 树」。探针 `control_snap_e2e.ps1` 三个场景全绿",
    ),
    # M2：拖拽落盘已完成
    (
        "- [ ] 拖拽落盘（把图片拖到别的程序）\n\n### 原生「另存为」对话框",
        "- [x] **拖拽落盘**（商用化第十三轮，见 HANDOFF 第 26 节）：OLE 拖放 + shell 数据对象，\n"
        "      Ctrl+左键 / 中键拖动即可拖进资源管理器、聊天窗口等\n\n### 原生「另存为」对话框",
    ),
    # M2：删掉重复的那一条
    (
        "      取消/失败时回退到配置目录并如实提示\n- [ ] 拖拽落盘（把图片拖到别的程序）\n",
        "      取消/失败时回退到配置目录并如实提示\n",
    ),
    # M3：OCR 双向联动已完成
    (
        "- [ ] 文本光标 ↔ 图上位置的双向联动（目前只有图 → 文本方向）",
        "- [x] **文本光标 ↔ 图上位置的双向联动**（商用化第十七轮，见 HANDOFF 第 30 节）：\n"
        "      面板分「编辑全文 / 逐行对照 (L)」两态，悬停行→图上高亮块、点框→选中并滚动到该行",
    ),
    # M3：滚动截屏那条尾巴过时了，并补上反向滚动
    (
        "      + `scroll_e2e.ps1` 回归（840×978 / 840×1005，宽度始终等于选区）（当前只支持向下）",
        "      + `scroll_e2e.ps1` 回归（840×978 / 840×1005，宽度始终等于选区）\n"
        "- [x] **反向（往上）滚动截屏**（商用化第十九轮，见 HANDOFF 第 33 节）：往回滚同样拼接；\n"
        "      回到已拍过的区间时靠画布两侧的 `head_room` / `tail_room` 余量避免重复。\n"
        "      顺带修掉「切到横向之后每一帧都被丢掉」（帧没转置）。四个单测逐像素比对真值",
    ),
    # M5-A：代码签名附上实测证据
    (
        "- [ ] 代码签名（取决于证书；没有证书就在文档里写明 SmartScreen 提示）",
        "- [ ] **代码签名（阻塞项，取决于证书）**：实测本程序被 Defender 判成\n"
        "      `Behavior:Win32/Persistence.A!ml` 并**拦截执行**（我们确实会写自启项 + 常驻热键，\n"
        "      无签名时这类行为没有任何信任来源）。见 HANDOFF 第 32 节；证书只能由人申请，\n"
        "      另外应做：向 Microsoft 提交误报、README 写明手动放行步骤",
    ),
    # M5-C：多显示器那条注明缺硬件
    (
        "- [ ] 多显示器混合缩放：每屏一个遮罩窗口",
        "- [ ] 多显示器混合缩放：每屏一个遮罩窗口（**本机只有一块屏，做了也无法验证**，\n"
        "      所以一直没动；`copy_subrect` 的偏移逻辑已有单测）",
    ),
    # M5-D：冷启动已经量过并有结论
    (
        "- [ ] 冷启动到遮罩可见再压：目前 ~0.5 s，其中含加载 8.1 MB 的 exe、解析 18.8 MB 字体",
        "- [x] **冷启动：已测量并摊开**（商用化第二十轮，见 HANDOFF 第 34 节）：进程创建 →\n"
        "      第一帧中位 **364 ms**，我们的代码只占 85（装载+CRT）+ 1（托盘+7 热键）+ 16\n"
        "      （第一帧）ms，大头是 eframe/GL 的 126 ms；同机对照 `cmd /c exit` 都要 1172 ms。\n"
        "      结论：不值得再压，真凶是杀软扫描（压它要靠代码签名）。诊断页已显示实测启动耗时",
    ),
    # M5-D：内存那条拆开（字体那半已做）
    (
        "- [ ] 内存继续压：字体在托盘模式下**按需**解析（不显示窗口就不解析）、\n"
        "      遮罩关闭后主动释放纹理与 `Frame`",
        "- [x] **界面字体按需装载**（商用化第十四轮，见 HANDOFF 第 27 节）：托盘常驻\n"
        "      **86.0 → 57.2 MB 工作集**（私有 48.9 → 28.6 MB），代价是首次画界面多 22 ms\n"
        "- [ ] 内存继续压：遮罩关闭后主动释放纹理与 `Frame`；贴图多开时的纹理总量上限",
    ),
    # M5-E：配置审计表已经在跑
    (
        "- [ ] 每个设置项在文档里标注对应的代码位置（`config-audit.md` 已经是这张表）",
        "- [x] 每个设置项在文档里标注对应的代码位置：`docs/config-audit.md` 就是这张表，\n"
        "      每加一个开关都会补一行（并有探针/单测证明它不是假开关）",
    ),
    # M5-E：CHANGELOG 本轮补上
    (
        "- [ ] `CHANGELOG.md`",
        "- [x] `CHANGELOG.md`（商用化第二十二轮）：按版本倒序记「用户能感知的变化 + 实测数字」，\n"
        "      数据来自 `git log` 与 `docs/HANDOFF.md`，不凭记忆写",
    ),
]


def main() -> int:
    write = "--write" in sys.argv
    text = TARGET.read_text(encoding="utf-8")
    misses = []
    for old, new in EDITS:
        if old not in text:
            misses.append(old.splitlines()[0][:60])
            continue
        text = text.replace(old, new, 1)
    print(f"applied {len(EDITS) - len(misses)}/{len(EDITS)}")
    for miss in misses:
        print(f"  MISS: {miss}")
    if write and not misses:
        TARGET.write_text(text, encoding="utf-8")
        print("written")
    return 1 if misses else 0


if __name__ == "__main__":
    raise SystemExit(main())
