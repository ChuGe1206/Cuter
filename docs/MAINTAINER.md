# 维护者手册

给接手这个代码库的人看的，包括三个月后的我自己。`docs/HANDOFF.md` 是按轮次的流水账，这里按"遇到问题怎么查、当初为什么这么定"来写。

想知道某个功能怎么实现的，看 HANDOFF 和代码注释。想知道哪些规矩不能破、踩过哪些坑、出问题先看哪，就是本文。

## 先跑起来

```
cargo build --release                        # 产物 target/release/cuter.exe
cargo test --workspace                       # 348 个单测，全部无头
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\ci\check.ps1 -WithProbes
```

最后一条是提交门禁：clippy（`-D warnings`）、单测、i18n 表完整性、release 构建，再加一遍桌面探针，任何一步失败都非零退出。文档里的每个数字都是这样跑出来的。

### 分支

| 分支 | 用途 |
|---|---|
| `develop` | 日常开发，所有改动先落这里 |
| `main` | 发版，只接受从 `develop` 合并过来的提交，每次打 `vX.Y.Z` tag |
| `master`（只在本地） | 2026 年 10 月之前的开发历史，不推送 |

发版流程：在 `develop` 上写完版本号、CHANGELOG、RELEASES，跑门禁和打包，然后 `git checkout main && git merge --ff-only develop`，打 tag，推 `main` 和 tag，回 `develop` 继续。

`master` 和 `develop` 没有共同祖先，不要 merge 进来。它的内容全都包含在开源后的第一条提交里，留着只是让旧提交还能用 `git show` 查到。

### 三层 workspace

| crate | 职责 | 不许做 |
|---|---|---|
| `cuter-core` | 位图、几何、捕获后端、滚动拼接、OCR 接口、配置 | 不许 `use` winit/egui，否则没法无头单测 |
| `cuter-ui` | 遮罩、工具条、标注、贴图、面板、i18n | 不许直接碰 Win32 |
| `cuter-app` | 窗口和视口、托盘、热键、单实例、桌面集成 | 无 |

用户数据的位置（便携模式下全部跟着 exe 走，见 `Config::is_portable`）：

```
%APPDATA%\Cuter\config.toml              配置
%APPDATA%\Cuter\logs\cuter.log           日志（2 MB 滚动，留三份）
%LOCALAPPDATA%\Cuter\history\            截图历史（原图、缩略图、index.toml）
```

## 规矩

每条都附上当初怎么发现的。没有代价的规矩守不住。

### 设置里的每个开关都要真的有效果

`docs/config-audit.md` 是这张契约表：每个字段一行，写清它在代码哪里生效、哪个探针证明它不是装饰。历史上有一批"设置里有、代码里没有"的假开关，那是最伤信任的一类问题，用户改完没反应，之后就不信任何开关了。

新加字段要同时给出能看出差别的验证：单测、探针的反向用例，或者实测的开关两组数字。做不到就别加这个开关。

### 数字要量出来

文档、提交信息、CHANGELOG 里出现的每个数字，都要能在 HANDOFF 里找到怎么量的、量了几次。不要把任务管理器上瞟一眼的数字写进文档，不要把一次跑得漂亮的数字当成稳定值，要写中位数和范围。

### 注释、文档、日志一律简体中文

包括日志文案、状态栏、提交信息。界面文案是 i18n 的键。

### `tools/probe/*.ps1` 必须是纯 ASCII

PowerShell 5.1 会把无 BOM 的脚本按 GBK 读，中文注释会让解析失败，或者吞掉换行把下一行并进注释里。所以：

- 探针里的中文用码点拼：`[string][char]0x622A + [char]0x56FE`
- 需要在日志里匹配中文时，给那条日志加英文标记（比如自动滚动的停机日志带 `autoscroll-stop stalled=.. steps=.. height=..`），探针只匹配英文标记
- 写完顺手 `Select-String '[^\x00-\x7F]'` 自查一遍

一次性的临时测量脚本可以存成 `utf-8-sig`，但别放进 `tools/probe/`。

### 探针退出码：0 通过，2 跳过，其它算失败

跳过不是通过，但也不算失败，它表示这个环境没法诚实地跑这条检查，比如别的程序占着键盘、合成输入被吞。`run_all.ps1` 会把跳过明写出来，不允许它伪装成通过。写新探针时环境不满足就 `exit 2`。

### 探针要自己收拾干净

动用户配置的探针都要先备份 `%APPDATA%\Cuter\config.toml`，`trap` 里恢复，结束时杀掉自己起的进程。用户下次开程序看到的应该是他自己的设置。

另外两条是两次假回归换来的（见 HANDOFF 第 69 节）：

- 合成输入的时序：`CuterInputProbe.Drag()` 在 `MoveTo` 之后等 150 ms 再按下。不等的话遮罩会拿上一条消息里的旧坐标当拖拽起点，700×320 的选区变成 584×267
- 先抢焦点再操作：对遮罩或浮动窗口发按键、拖拽之前先 `Activate(那个窗口)`，否则输入落到下面的记事本里，现象是应用什么都没做

### 不要跑 `cargo fmt --all`

这个代码库有大量刻意的手工排版（对齐的注释块、表格化的常量），rustfmt 会重排它们，产生一个和本次改动无关的巨大 diff。clippy 是门禁，rustfmt 不是。

### 改完要复核

见下面 3.1。PowerShell 的字符串替换在这个仓库上会静默失配。

## 踩过的坑

这一节是本文最值钱的部分。每条都真实发生过，而且第一次的判断往往是错的。

### 明明改了却没生效

症状：用 PowerShell 的 `-replace` 或 `Get-Content -Raw` 改 `.rs`，命令没报错，但行为没变。

原因：这些文件是 CRLF，而模式串写的是 LF。替换计数为 0 也不报错。

判据：改完立刻 `Select-String` 搜刚写的那个字符串，搜不到就是没改上。

因为这条，我曾经基于"日志里热键命令数为 0"得出过一个错误结论。

### 探针失败了是不是改坏了

按顺序做，别跳：

1. 找一个已知能过的产物：解开上一版的 `dist\*-portable-x64.zip`，跑同一个探针。旧产物也失败就是环境问题，不是回归
2. 同一时刻跑一个用别的热键的探针（`latency_probe` 用 `Alt+C`）。它能过说明热键通道没问题，是这次合成的输入没送到
3. 看日志里的实测耗时：`截图链路[遮罩首帧可见]：+xxx ms`。平时 143~332 ms，变成 2.4 s 就是机器被压满了
4. 同一个探针单独复跑。单独过、一起不过就是级联干扰

三条证据都指向环境就不要改代码。"看起来像随机抖动"的错，往往是配置漏了一项。`scroll_e2e` 曾报 `width mismatch 756 vs 840`，真因是它的配置漏了 `snap_controls = false`，控件级吸附会改拖框边缘。

### 关窗不要依赖焦点

症状：验证"没窗口要画时字体被卸载"，脚本用 `FocusDesktop()` 加 `Esc` 关设置窗口，日志里 `control_visible` 一直是 true，于是得出结论"字体没卸载"。这个结论是假的。

原因：`Esc` 发给有焦点的窗口，脚本先把焦点还给桌面，键就没送到。

结论：用 `PostMessage(hwnd, WM_CLOSE)`。凡是让某个窗口消失的自动化，都别依赖焦点。

### eframe / egui 的几条硬事实

- 隐藏窗口可能整帧不出。eframe 对不可见窗口有约 100 ms 的重绘节流，如果一个待处理的重绘请求都没有，热键那次的唤醒都不一定换来一次 pass。所以主循环里必须始终排一个下一帧。曾经为了省 GPU 改成"只在有活干时请求"，结果贴图窗口一个都不出现
- 不可见的 deferred viewport 不跑自己的 pass，也就不会请求下一帧。隐藏的贴图组永远没法靠自己重新出现，可见性只能由根视口每帧重新声明
- `ViewportCommand::Visible(false)` 对每帧重新声明的视口不生效，要隐藏就得这一帧不声明它
- 贴图、面板这类 deferred viewport 要好几帧才落地，所以处理完托盘或热键命令之后要留一小段顶帧时间（`keep_awake_until`）
- `NativeOptions`（0.36）没有 `vsync` 字段，`depth_buffer`、`stencil_buffer`、`multisampling` 默认已经是 0

### 内存要看两个数

任务管理器的"内存"是工作集。释放出来的堆页在 Windows 上保持提交，私有内存不降，只是不再常驻。所以报"省了多少"时给工作集，同时也写私有内存的变化；反过来，看到私有内存不降不代表没释放（字体那次工作集 82.1 降到 55.8 MB，私有 53.2 升到 56.7 MB）。

### 死锁与套间

- `match lock(&x).foo() { ... }` 会让 `MutexGuard` 活到 `match` 结束，里面再 `lock(&x)` 就是自己等自己。贴图"一按动作就自锁死"就是这么来的。要 `let value = lock(&x).foo();` 再 `match value`
- WinRT 的 OCR 必须在 MTA 线程上阻塞等待。winit 的事件循环线程可能是 STA，在上面等异步结果会和套间消息泵互相等待而死锁。所以 OCR 跑在自建的后台线程上

### 零散但会咬人的

- UI 自动化的 `ElementFromPoint` 返回的是我们自己的遮罩，因为它是最上层全屏窗口。要吸到底下的控件，必须自己按 z 序找底下那个窗口，再逐层下潜 UIA 树
- `MOUSEEVENTF_HWHEEL` 的符号和 `MOUSEEVENTF_WHEEL` 相反，前者正数向左滚。自动滚动两个方向都用负增量不是笔误，clippy 会误报"两个分支一模一样"
- i18n 用中文原文当键，查不到回落中文，界面永远不会变空白。表必须按运行时字符串排序（源码里的 `\n` 是两个字符），模板要用 `fill!` 而不是 `format!`，`tr` 同时是函数名，运行时字符串要用 `cuter_ui::i18n::tr`
- 热键能被别的程序抢走。这台机器上 `Ctrl+F3` 和 `Shift+F3` 被常驻的截图类程序占着，所以贴图热键默认是 `Alt+V`
- Defender 会把未签名、写自启项、常驻热键的程序判成 `Behavior:Win32/Persistence.A!ml` 并拦截执行。这是代码签名那条路线图项的真实原因

## 出问题先看哪里

| 症状 | 先看 |
|---|---|
| 启动慢、双击没反应 | 日志里的 `启动时间线[...]` 七个阶段；HANDOFF 第 34 节（同机 `cmd /c exit` 都要 1.17 s，瓶颈常在杀软） |
| 按热键没反应 | 日志里的 `已注册热键 N 个`、`截图链路[唤醒到处理]`、`[遮罩首帧可见]` |
| 截图是黑的或内容不对 | `capture.backend`，DXGI 拿不到 duplication 会回退 GDI 并说明原因 |
| 贴图没了或不显示 | `pin_should_show` 是唯一的可见性判据；日志里的 `release-hidden-pin-textures`；HANDOFF 第 22、41 节 |
| 内存不降 | 先分工作集和私有（见上文）；`idle_gpu_probe.ps1` 与 `perf_probe.ps1` 给数 |
| 内存一直涨 | `leak_probe.ps1`，120 次真实截图，量 GDI 和 USER 对象、句柄、私有内存 |
| 滚动截屏拼错 | 日志里的方向切换和 `滚动截屏完成：N 帧`；`scroll_e2e.ps1`、`scroll_horizontal_e2e.ps1`、`scroll_manual_e2e.ps1` |
| 界面出现方框 | 字体没找到 CJK（日志 `界面字体已按需装载`、`has_cjk`），设置里的字体页可以手动指定 |
| 探针大面积失败 | 先按上文三条证据排除环境；`run_all.ps1` 会在探针之间清场 |

## 发布一个版本

1. 改 `Cargo.toml` 的版本号，每轮加一
2. `CHANGELOG.md` 加一节，只写用户能感知的变化和实测数字
3. `docs/roadmap.md` 把做完的项打勾，做不了的写清卡在哪
4. `docs/HANDOFF.md` 加一节，写怎么发现的、怎么验证的、哪些数字是量出来的
5. 跑 `tools/ci/check.ps1 -WithProbes`
6. 打包并自检：

```
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\package\portable_zip.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\package\build_installer.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\installer_e2e.ps1
```

便携包脚本会自己解压验证"日志落在 exe 旁边、`%APPDATA%` 那份没被动"。`installer_e2e.ps1` 会真的静默安装、检查快捷方式和自启、启动、再卸载并确认配置保留。

7. 提交，顺手删掉 `dist/` 里上一版的产物和 `dist/verify-portable`

用户是拿 `dist\*-portable-x64.zip` 解压后驱动真程序来验收的，所以每轮有意义的改动都要给出新包。
