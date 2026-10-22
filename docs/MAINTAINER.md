# 维护者手册

这份文档面向**接手这个代码库的人**（包括三个月后的我自己）。它不重复 `docs/HANDOFF.md`
的逐轮流水，而是按"遇到问题怎么查、为什么当初这么定"来组织。

- 想知道某个功能怎么实现的 → 看 `docs/HANDOFF.md`（按轮次）与代码注释
- 想知道**这个项目有哪些不能破的规矩**、**踩过哪些坑**、**出问题先看哪里** → 就是本文
- 想知道当前进度与还没做的 → `docs/roadmap.md` 与 `CHANGELOG.md`
- 想用用户能听懂的话介绍这个工具 / 写更新提示 → `docs/RELEASES.md`
  （它的版本清单由门禁脚本与 `CHANGELOG.md` 对齐，改一处忘另一处会被拦下）

---

## 1. 五分钟上手

```
cargo build --release                        # 产物 target/release/cuter.exe（约 8 MB）
cargo test --workspace                       # 348 个单测，全部无头
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\ci\check.ps1 -WithProbes
```

最后那条是**提交门禁**：clippy（`-D warnings`）/ 单测 / i18n 表完整性 / release 构建，
再加一遍桌面探针。任何一步失败就非零退出。`docs/` 里的每个数字都是这样跑出来的。

### 分支怎么用

| 分支 | 用途 | 规矩 |
|---|---|---|
| `develop` | 日常开发 | 所有改动先落这里；门禁绿了再提交 |
| `main` | 发版 | 只接受从 `develop` 来的发布提交，每次发版打 `vX.Y.Z` tag |
| `master`（仅本地） | 2026-10 之前的历史 | 开源前的开发记录，**不推送到远端**，需要考古时用 |

发版流程：`develop` 上把版本号、CHANGELOG、RELEASES 写完 → 门禁 + 打包 →
`git checkout main && git merge --ff-only develop` → 打 tag → 推 `main` 与 tag →
回到 `develop` 继续。

`master` 是独立历史（与 `develop`/`main` 无共同祖先），所以**不要**试图把它 merge 进来；
它的内容已经全部包含在开源后的第一条提交里，留着只是为了让旧提交仍然能被 `git log`/`git show` 查到。


三层 workspace：

| crate | 职责 | 不许做的事 |
|---|---|---|
| `cuter-core` | 位图、几何、捕获后端、滚动拼接、OCR 接口、配置 | 不许 `use` winit/egui（否则没法无头单测） |
| `cuter-ui` | 遮罩、工具条、标注、贴图、各个面板、i18n | 不许直接碰 Win32（除了已经在里面的平台细节） |
| `cuter-app` | 窗口/视口、托盘、热键、单实例、桌面集成 | —— |

用户数据的位置（便携模式下全部跟着 exe 走，见 `Config::is_portable`）：

```
%APPDATA%\Cuter\config.toml              配置
%APPDATA%\Cuter\logs\cuter.log           日志（2 MB 滚动、留 3 份）
%LOCALAPPDATA%\Cuter\history\            截图历史（原图 + 缩略图 + index.toml）
```

---

## 2. 不能破的规矩

每一条都配了"为什么"和"当初怎么发现的"——**没有代价的规矩守不住**。

### 2.1 设置里的每个开关都必须真的有效果

`docs/config-audit.md` 就是这张契约表：每个字段一行，写清它在代码里哪里生效、哪个探针
证明它不是装饰。历史上有一批"设置里有、代码里没有"的假开关（`show_tray_notifications`
曾经就是），那是最伤信任的一类问题——用户改完没有任何反应，然后就不再相信任何开关。

**新加字段时的动作**：加进 `config-audit.md`，并且**必须**同时给出一个能看出差别的验证
（单测、探针的反向用例，或者实测的"开/关"两组数字）。做不到就别加这个开关。

### 2.2 数字必须是量出来的

文档、提交信息、CHANGELOG 里出现的每个数字，都要能在 `docs/HANDOFF.md` 里找到
"怎么量的、量了几次"。不做的事：把 Task Manager 上瞟一眼的数字写进文档；
把"大约"写成"约"就当精确；把一次跑得漂亮的数字当成稳定值（要写中位数/范围）。

### 2.3 注释、文档、日志一律简体中文

包括日志文案、状态栏、提交信息。界面文案本身是 i18n 的键（见 3.7）。

### 2.4 `tools/probe/*.ps1` 必须是纯 ASCII

PowerShell 5.1 会把**无 BOM** 的脚本按 GBK 读，中文注释会让脚本直接解析失败
（`字符串缺少终止符` 之类）。所以：

* 探针里的中文一律用码点拼：`[string][char]0x622A + [char]0x56FE`；
* 需要在日志里匹配中文时，**给那条日志加一个英文标记**（例如自动滚动的停机日志带
  `autoscroll-stop stalled=.. steps=.. height=..`），探针只匹配英文标记；
* 写完顺手 `Select-String '[^\x00-\x7F]'` 自查一遍。

（例外：一次性的临时测量脚本可以存成 `utf-8-sig`，但别放进 `tools/probe/`。）

### 2.5 探针的退出码约定：0 = PASS，2 = SKIPPED，其它 = FAIL

**SKIPPED 不是通过**，但也不算失败：它表示"这个环境没法诚实地跑这条检查"（比如别的程序
占着键盘、合成输入被吞）。`tools/probe/run_all.ps1` 会把 SKIP 明写出来，
绝不允许它伪装成 PASS。写新探针时：环境不满足就 `exit 2`，别 `exit 0`。

### 2.6 探针必须自己收拾干净

动用户配置的探针（几乎所有端到端探针）都要：先备份 `%APPDATA%\Cuter\config.toml`，
`trap` 里恢复，结束时杀掉自己起的进程。**不能留下被改过的用户配置**——用户下次开程序
看到的应该是他自己的设置。

另外两条是 0.1.48 收尾时用两次假回归换来的（详见 `docs/HANDOFF.md` 第 69 节）：

* **合成输入的时序**：`CuterInputProbe.Drag()` 会在 `MoveTo` 之后等 150 ms 再按下——不等的话
  遮罩会拿上一条消息里的旧坐标当拖拽起点（700×320 的选区变成 584×267）；
* **先抢焦点再操作**：对遮罩/浮动窗口发按键或拖拽之前，先 `Activate(那个窗口)`，否则输入
  会落到下面的记事本/浏览器里，现象是"应用什么都没做"。

### 2.7 不要跑 `cargo fmt --all`

这个代码库有大量刻意的手工排版（对齐的注释块、表格化的常量），`rustfmt` 会重排它们，
产生一个巨大的、与本次改动无关的 diff。clippy 是门禁，rustfmt 不是。

### 2.8 改完要用可靠的方式改，并且复核

见 3.1 —— PowerShell 的字符串替换在这个仓库上会**静默失配**。

---

## 3. 踩过的坑（症状 → 判据 → 结论）

这一节是本文最有价值的部分。每条都真实发生过，而且**第一次的判断往往是错的**。

### 3.1 "我明明改了，怎么没生效"

**症状**：用 PowerShell 的 `-replace` / `Get-Content -Raw` 改 `.rs` 文件，
命令没报错，但行为没变、日志没出现。

**原因**：这些文件是 **CRLF**，而 `-replace` 里的模式串是 LF；替换计数为 0 也不报错。

**判据**：改完立刻 `Select-String` 搜你刚写的那个字符串。搜不到就是没改上。

**结论**：用编辑工具改（或 Python 读整个文件再写回），并且**每次改完都复核**。
因为这条，我曾经基于"日志里热键命令数为 0"得出过一个错误结论。

### 3.2 "探针失败了，是不是我改坏了"

**症状**：`pin_groups_e2e` 从 `1,0,1,1` 变成全 `0`；`scroll_e2e` 报尺寸对不上。

**判据**（按顺序做，别跳）：

1. **回退/并存一个已知能过的产物**：把上一版的 `dist\*-portable-x64.zip` 解开，
   跑同一个探针。旧产物也失败 → 环境问题，不是回归。
2. **同一时刻跑一个用别的热键的探针**（`latency_probe` 用 `Alt+C`）。它能过 →
   热键通道没问题，是这次合成的输入没到。
3. **看应用日志里的实测耗时**：`截图链路[遮罩首帧可见]：+xxx ms`。平时 143~332 ms，
   变成 2.4 s 就说明机器被压满了（跑全量探针时很常见）。
4. **同一个探针单独复跑**。单独过、一起不过 → 级联干扰。

**结论**：三条证据都指向环境就不要改代码。**"看起来像随机抖动"的错，往往是配置漏了一项**
（`scroll_e2e` 曾报 `width mismatch 756 vs 840`，真因是它的配置漏了 `snap_controls = false`
——控件级吸附会改拖框边缘）。

### 3.3 关窗不要依赖焦点

**症状**：验证"没窗口要画时字体被卸载"，脚本用 `FocusDesktop()` + `Esc` 关设置窗口，
日志里 `control_visible` 一直是 `true`，于是得出"字体没卸载"——**这个结论是假的**。

**原因**：`Esc` 发给**有焦点**的窗口；脚本先把焦点还给桌面，键就没送到。

**结论**：用 `PostMessage(hwnd, WM_CLOSE)`（已通过的 `tray_close_e2e.ps1` 就是这么做的）。
凡是"让某个窗口消失"的自动化，都别依赖焦点。

### 3.4 eframe / egui 的几条硬事实

这些是从源码和实测里确认的，改 UI 之前建议先读一遍：

* **隐藏窗口可能整帧不出**：eframe 对不可见窗口有约 100 ms 的重绘节流；
  如果**一个待处理的重绘请求都没有**，连热键那次的唤醒都不一定换来一次 pass
  ——所以主循环里必须**始终**排一个下一帧（我们现在是"忙时 120 ms / 待命 1000 ms"）。
  曾经为了省 GPU 把它改成"只在有活干时请求"，结果贴图窗口一个都不出现。
* **不可见的 deferred viewport 不会跑自己的 pass**：它也就不会请求下一帧。
  所以"隐藏的贴图组"永远没法靠自己重新出现——可见性只能由根视口每帧重新声明。
* **`ViewportCommand::Visible(false)` 对每帧重新声明的视口不生效**：要隐藏就得
  "这一帧不声明它"。
* 贴图/面板这类 **deferred viewport 需要好几帧才落地**，所以处理完托盘/热键命令之后
  要有一小段"顶帧"时间（`keep_awake_until`）。
* `NativeOptions`（0.36）**没有 `vsync` 字段**，`depth_buffer`/`stencil_buffer`/`multisampling`
  默认已经是 0。

### 3.5 内存要看两个数

任务管理器的"内存"是**工作集**。释放出来的堆页在 Windows 上**保持提交**（私有内存不降），
只是不再常驻。所以：

* 报"省了多少"时给工作集（用户看得到的那个），同时**也写出私有内存的变化**；
* 反过来，看到私有内存不降**不代表没释放**（字体那次：工作集 82.1 → 55.8 MB，
  私有 53.2 → 56.7 MB）。

### 3.6 死锁与套间：两个非 Windows 直觉的坑

* `match lock(&x).foo() { ... }` 会让 `MutexGuard` 活到 `match` 结束——里面再 `lock(&x)`
  就是**自己等自己**（贴图"一按动作就自锁死"就是这么来的）。要 `let value = lock(&x).foo();`
  再 `match value`。
* **WinRT 的 OCR 必须在 MTA 线程上阻塞等待**；winit 的事件循环线程可能是 STA，
  在它上面等异步结果会与套间消息泵互相等待而死锁。所以 OCR 跑在自建的后台线程上。

### 3.7 其他零散但会咬人的

* **UI 自动化 `ElementFromPoint` 返回的是我们自己的遮罩**（它是最上层全屏窗口）。
  要吸到底下的控件，必须"自己按 z 序找底下那个窗口，再逐层下潜 UIA 树"。
* **`MOUSEEVENTF_HWHEEL` 的符号与 `MOUSEEVENTF_WHEEL` 相反**：前者正数向左滚。
  自动滚动两个方向都用负增量不是笔误（clippy 会误报"两个分支一模一样"）。
* **i18n 用中文原文当键**：查不到回落中文（界面永远不会变空白）。
  表必须按**运行时字符串**排序（源码里的 `\n` 是两个字符）；模板要用 `fill!` 而不是
  `format!`（后者要字面量）；`tr` 同时是函数名，运行时字符串要用 `cuter_ui::i18n::tr`。
* **热键能被别的程序抢走**：这台机器上 `Ctrl+F3`/`Shift+F3` 被常驻的截图类程序占着，
  所以贴图热键默认是 `Alt+V`。
* **Defender 误报**：未签名 + 写自启项 + 常驻热键，被判成 `Behavior:Win32/Persistence.A!ml`
  并**拦截执行**。这是代码签名那条路线图项的真实原因，不是"以防万一"。

---

## 4. 出问题先看哪里

| 症状 | 先看 |
|---|---|
| 程序启动慢 / 双击没反应 | `%APPDATA%\Cuter\logs\cuter.log` 里的 `启动时间线[...]`（七个阶段）；HANDOFF 第 34 节（同机 `cmd /c exit` 都要 1.17 s，瓶颈常在杀软） |
| 按热键没反应 | 日志里的 `已注册热键 N 个`（有没有注册失败）、`截图链路[唤醒到处理]`、`[遮罩首帧可见]` |
| 截图是黑的 / 内容不对 | `capture.backend`（`dxgi` 拿不到 duplication 时会回退 GDI 并说明原因）；日志里 `DXGI 后端已就绪` 或 `建不起来` |
| 贴图没了 / 不显示 | `pin_should_show` 是**唯一**的可见性判据；日志里 `release-hidden-pin-textures`；HANDOFF 第 22 / 41 节 |
| 内存不降 | 先分工作集/私有（第 3.5 节）；`idle_gpu_probe.ps1` 与 `perf_probe.ps1` 给数 |
| 内存一直涨 | `leak_probe.ps1`（120 次真实截图，量 GDI/USER 对象、句柄、私有内存） |
| 滚动截屏拼错 / 不动 | 日志里的方向切换与 `滚动截屏完成：N 帧`；`scroll_e2e.ps1` / `scroll_horizontal_e2e.ps1` / `scroll_auto_e2e.ps1` |
| 界面出现方框 | 字体没找到 CJK（日志 `界面字体已按需装载：...`、`has_cjk`）；设置 → 字体里手动指定 |
| 探针大面积失败 | 先按第 3.2 节的三条证据排除环境；`run_all.ps1` 会在探针之间清场 |

---

## 5. 发布一个版本

1. 改 `Cargo.toml` 的版本号（`0.1.x`，每轮 +1）；
2. `CHANGELOG.md` 加一节：**只写用户能感知的变化 + 实测数字**，每条指向 HANDOFF 的小节；
3. `docs/roadmap.md` 把做完的项打勾，并写清"卡在哪"（如果是外部条件）；
4. `docs/HANDOFF.md` 加一节：怎么发现的、怎么验证的、**哪些数字是量出来的**；
5. 跑 `tools/ci/check.ps1 -WithProbes`；
6. 打包并自检：

   ```
   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\package\portable_zip.ps1
   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\package\build_installer.ps1
   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\installer_e2e.ps1
   ```

   （便携包脚本会自己解压验证"日志落在 exe 旁边、`%APPDATA%` 那份没被动"；
   `installer_e2e.ps1` 真的静默安装、检查快捷方式/自启、启动、再卸载并确认配置保留。）
7. 提交；顺手删掉 `dist/` 里上一版的产物和 `dist/verify-portable`。

**用户是拿 `dist\*-portable-x64.zip` 解压后驱动真程序来验收的**，所以每轮有意义的改动
都要给出新包。
