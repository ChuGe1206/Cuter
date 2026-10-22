# 脚本化 UI 测试探针

这些 C# 片段配合 `Add-Type -Path` 使用，用来**在没有人类操作的情况下**测一个 GUI 截图工具。
项目里关于窗口几何、遮罩渲染、贴图内容的结论都来自这些探针。

```powershell
Add-Type -Path "tools\probe\WinDpi.cs"
Add-Type -Path "tools\probe\CuterInputProbe.cs"
```

## 退出码约定

| 码 | 含义 |
|---|---|
| 0 | 通过 |
| 1 | **失败**：测出来的行为不对（这才是"应用有 bug"） |
| 2 | **没跑成（SKIPPED）**：环境不允许，不能据此判断应用好坏 |

现在只有一种 SKIPPED：**别人把键盘抢走了**。运行中的虚拟机窗口（VirtualBox / VMware）
或远程桌面客户端**在它自己处于前台时**会独占键盘，此时我们合成的 `Alt+C` / `Enter`
根本送不到应用——现象就是"遮罩没出现"，看起来和应用的 bug 一模一样。
探针在重试仍失败后会自动查前台窗口属于谁，如果不是我们自己，就打印

```
foreground app is 'VirtualBoxVM' - [win10 (备份 2) [正在运行] - Oracle VirtualBox]
  (that is not us: a VM / remote-desktop window swallows synthesized keys)
RESULT: SKIPPED - another program owns the keyboard, so synthesized keys never arrive
```

并以 2 退出（`history_e2e.ps1` / `leak_probe.ps1` / `hide_own_windows_e2e.ps1`）。

**这条规则本身就是一次教训**：曾经把它当成应用的 bug 追了很久（`Alt+C` 第一次好、
第二次就不响应），最后发现是桌面上跑着一个虚拟机窗口抢走了键盘；同一个环境下
`cuter --capture`（单实例转交，完全不碰键盘）**照样正常**，这才把方向掰回来。
探针会先尝试把桌面切到前台（`FocusDesktop`）释放这个独占，但用户正在虚拟机里操作时
它会立刻再抢回去——所以 SKIPPED 是正常结局，不是探针坏了。要跑这些探针时，
把虚拟机窗口最小化或让它失焦即可。

## 为什么需要它们

GUI 工具的很多问题是"看起来不对"，但**肉眼判断不可靠、模型也读不了截图**。
这里的做法是：把界面状态用 Win32 查询出来、把画面用像素统计出来，
于是"对不对"变成一个可以打印的数字。

## 三个必须记住的陷阱

1. **`GetWindowRect` 返回的是「调用进程 DPI 上下文」里的坐标。**
   PowerShell 是 DPI 不感知的，于是 125% 缩放下 1920x1080 的窗口会被报告成 1536x864。
   直接用这个数会得出"窗口尺寸错了"的错误结论（我为此白改过一轮代码）。
   → 用 `WinDpi.cs`，它用 `GetDpiForWindow` 把缩放比乘回去；
   或者调 `[CuterInputProbe]::SetPhysicalDpiAwareness()` 把线程切成 Per-Monitor-V2，
   之后所有坐标都是真物理像素。**用 `[CuterInputProbe]::Awareness()` 实测**（0=不感知、
   2=每屏感知），不要假设切换一定成功。
2. **`SetCursorPos` 的坐标同样是虚拟化的**：传 (300,250) 实际把光标放在物理 (375,312)。
   切到 Per-Monitor-V2 之后就没有这回事了，传什么就是什么。
3. **截图里出现深色矩形，先想想是不是自己的控制台窗口。**
   排查踩过的坑：一块 1192x592 的"神秘黑块"其实是运行脚本的 PowerShell 控制台窗口
   被冻进了截图里，完全正常。

## 另外五个坑（滚动截屏这一轮踩到的）

4. **残留实例会抢走全局热键。** 上一轮失败没清场，`cuter.exe` 还在跑；新实例注册不上
   `Alt+S`，热键进了旧实例，于是"新实例的遮罩永远不出现"。症状和热键注册完全无关，
   极难联想。→ 每轮开始先 `Get-Process cuter | Stop-Process -Force`，并用 `trap` 保证收尾。
5. **注入的滚轮事件只送到前台窗口。** Win10 的「悬停滚动非活动窗口」对 `mouse_event`
   注入的输入不生效。而每启动一次 `cuter.exe` CLI 都会把控制台拉到前台，
   所以**每次发滚轮前都要重新 `Activate` 目标窗口**，否则测出来的位移恒为 0。
6. **周期性内容不能用互相关量位移。** 记事本每行文字都一样高，位移取行高整数倍时
   逐行互相关同样会给出接近 0 的偏差，"没滚动"和"滚了 3 行"分不出来。
   量的对象必须有唯一内容（或者干脆别量，改用确定性的 Rust 测试）。
7. **`GetWindowRect` 与客户区不一定重合。** 遮罩窗口（无边框、`with_decorations(false)`）
   仍有 1 像素的非客户区上边：窗口矩形是 `(0,0,1920,1080)`，客户区原点却是 `(0,1)`。
   egui 全程用**客户区**坐标，所以"光标停在屏幕 y=170"实际选中的是桌面帧第 169 行。
   这个内缩量用 `ClientInfo()` 实测，别假设是 0。
8. **写 TOML 的 Windows 路径要用单引号字面量字符串。** `dir = "E:\x"` 里的 `\x` 不是合法转义，
   整份配置会被解析器拒绝；app 会把坏文件改名成 `config.toml.bak` 并**回退默认值**，
   于是测试看起来"跑通了"，其实 `auto_save`、保存目录全都没生效。
   → 用 `dir = 'E:\x'`，并在 app 启动后断言 `config.toml.bak` 没有出现。

## 各文件

### `WinList.cs`
按 z 序列出当前可见的顶层窗口（含 `[前台]` 标记、PID、类名、矩形、转义后的标题）。
用来回答"现在屏幕上到底是什么"。

```powershell
[WinList]::Visible() | Select-Object -First 12
```

### `WinDpi.cs`
按 PID 枚举某进程的顶层窗口，并且**用 `GetDpiForWindow` 还原真实物理尺寸与位置**。
验证"遮罩窗口是否精确覆盖了虚拟桌面"就靠它。

```powershell
Add-Type -Path "tools\probe\WinDpi.cs"
[WinDpi]::Dump($pid, '\u622A\u56FE')   # 第二个参数是标题里必须包含的转义片段，$null 表示全部
```

### `WinEnum.cs`
同上的按 PID 枚举，但把标题输出成 `\uXXXX` 转义序列，**彻底绕开控制台代码页问题**
（中文标题在 GBK 控制台下会被写成乱码而无法比对）。

### `python.ps1`
给需要 Python 的探针（假服务端、OCR 辅助脚本）找解释器：`Require-Python` 依次试
`-Preferred` → `$env:CUTER_PYTHON` → PATH 上的 `python` → `py` → 随 agent 运行时自带的那个 →
用户目录里的安装，**每个候选都真的跑一次 `--version`** 才认。

为什么不能只看 `Test-Path`：Windows 上 PATH 里的 `python` 常常是应用商店的别名存根，
文件存在、执行时什么都不打印、退出码 **9009**。第一版只判断存在性，门禁因此把 i18n 那一步
跑成了"退出码 9009"，而报错信息看起来像脚本坏了。

找不到解释器时探针报 **SKIPPED（退出码 2）**，不伪装成 PASS，也不伪装成产品坏了。

### `CuterInputProbe.cs`
模拟真实输入与窗口查询：

| 方法 | 用途 |
| --- | --- |
| `SetPhysicalDpiAwareness()` / `Awareness()` / `DpiOf(hwnd)` / `ScreenSize()` | 坐标系自检 |
| `MoveTo` / `Press` / `Release` / `Drag` / `Key(vk)` / `CtrlKey` / `AltKey` | 键鼠输入 |
| `Wheel(clicks)` | 滚轮（正数向上）。**只对前台窗口生效** |
| `FindWindow(pid, 标题子串)` / `Describe(hwnd)` | 找窗口、拿 `hwnd\|x\|y\|w\|h\|visible` |
| `ClientInfo(hwnd)` / `MoveWindow` / `Activate` | 客户区原点与尺寸、摆放、抢前台 |

```powershell
# 先确认遮罩窗口已经出现，再发输入——否则点击会落到用户的桌面上
$p = Start-Process .\target\release\cuter.exe -ArgumentList "--capture" -PassThru
foreach($i in 1..30){ Start-Sleep -Milliseconds 300; if([WinDpi]::Dump([uint32]$p.Id,'\u622A\u56FE').Count -gt 0){break} }
[CuterInputProbe]::Drag(300,250,900,650)
[CuterInputProbe]::CtrlKey(0x53)           # Ctrl+S
```

⚠️ **它会真的操作鼠标。务必先确认遮罩已经出现再发输入**，否则点击会落到用户当前的桌面窗口上。

⚠️ `Drag(x0,y0,x1,y1)` 内部是 `MoveTo → Sleep(250) → Press → Sleep(200) → 再 MoveTo(x0,y0)
（零距离，重新锚定）→ 12 步插值 → Release`。少掉这一步"重新锚定"，遮罩会拿**上一条输入消息**
的指针位置当起点：700×320 的选区实测变成 642×294（HANDOFF 第 71 节）。

### `scroll_e2e.ps1`
滚动截屏的端到端测试：记事本 300 行 → `Alt+S` → 框选 → 回车开始采集 → 真实滚轮 →
回车完成 → 断言落盘的长图。断言不止"变高了"：

- 长图的前 N 行必须与 `--region` 普通截图**逐像素一致**（实测平均偏差 0.000）；
- 末尾那一段不能与首屏相同（否则说明"拼接"只是重复抄了一遍）；
- 控制窗不得与采集区域相交（否则它会被逐帧截进长图）。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\scroll_e2e.ps1
```

它会在 `target\e2e\scroll\` 留下 `reference.png`、长图 PNG 和 `cuter-stderr.log`
（app 的 stderr 里有"滚动截屏开始：区域 Rect { .. }"这样的硬证据）。

### `overlay_client_probe.ps1`
只查一件事：遮罩窗口的窗口矩形与客户区原点是否重合。用来定位"选区与像素差 1 行"。

### `tray_close_e2e.ps1`
`general.close_to_tray` 的端到端测试：真窗口 + 真 `WM_CLOSE`，两条分支都要断言。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\tray_close_e2e.ps1
```

- `close_to_tray = true` → 窗口消失，**进程仍在**（托盘模式）
- `close_to_tray = false` → 进程退出

它会临时改写 `%APPDATA%\Cuter\config.toml`（先备份，`finally` 里还原），
写的是 `start_in_tray = false`，这样主窗口才会直接可见。脚本是**纯 ASCII**的，
所以不需要 UTF-8 BOM。

⚠️ **它踩过的坑值得单独记住**（详见 `docs/HANDOFF.md` 12.6）：
`cuter.exe` 是控制台子系统程序，`Start-Process` 会给它一个**可见的、有标题的**
`ConsoleWindowClass` 窗口。按"可见 + 有标题"枚举窗口时它会和主窗口一起被选中，
`WM_CLOSE` 发到它身上只是关掉了控制台 —— 而进程会在几秒宽限期后被系统一起结束，
**现象和"程序没有退到托盘"一模一样**。所以 `TitledWindows()` 里必须排除该类名。

（13 轮之后这个问题从源头上消失了：程序启动时会主动 `FreeConsole` 掉"只属于自己"的
控制台窗口。但枚举窗口时排除 `ConsoleWindowClass` 仍然是个好习惯，见 HANDOFF 13.1。）

### `hide_own_windows_e2e.ps1`
`capture.hide_own_windows` 的端到端测试：**截图里绝不能出现我们自己的窗口**。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\hide_own_windows_e2e.ps1
```

流程：记事本摆在固定位置（它是"可预测的静态背景"）→ 抓一张基准图 →
让运行中的实例把设置窗口**铺满**记事本的客户区 → 再抓一张（这一张必须与基准图
**差很多**，否则说明这块区域里其实没有我们的窗口，测试会假通过）→ `Alt+C` 框选同一块 →
回车自动保存 → **存下来的 PNG 与基准图的平均偏差必须 ≈ 0**（实测 0.000）。
它同时断言 app 日志里出现过"请求隐藏自身窗口"这条路，所以"为什么通过"也是有据可查的。

四个必须保留的写法：

1. **主题写成 `dark`**：深色设置面板与白色记事本页面差 190 上下，
   判定"漏没漏"一目了然；浅色面板只有 25，和噪声分不开。
2. **先等遮罩几何稳定再拖拽**：遮罩刚创建时物理尺寸可能是 `2400x1350`
   （= 屏幕 × 系统缩放），这时拖拽会按错误的缩放换算，存出 `735x524` 而不是 `882x628`。
3. **额外抓一张 `after.png`**（流程结束、窗口已隐藏时）：`dev(after, reference)` 是
   **背景自身的漂移**（实机 0.000）。区分"漏了窗口"与"背景变了"全靠它。
4. **看残差的形状**：窗口是**整块均匀**覆盖的，而 DWM 重新露出被遮挡窗口时的
   文字重绘残差是**文字形状**的（区域右半边空白处恰好为 0）。
   这一点决定了阈值该取多少，别被一个 10 上下的数字绕进去。

### 尺子（不是门槛）：`bench_capture_breakdown`

调优前先量，否则容易优化错东西。它是 `cuter-core` 里一个 `#[ignore]` 的测试：

```powershell
cargo test -p cuter-core bench_capture_breakdown --release -- --ignored --nocapture
```

它把捕获链路**拆开逐项计时**（`BitBlt` 带/不带 `CAPTUREBLT`、`GdiFlush`、
8 MB 的 `to_vec`、BGRA→RGBA 交换、两条完整的捕获路径、窗口探测、`monitors()`）。
实测结论（1920×1080）：`BitBlt` 33.3 ms 就是全部，软件搬运那 ~8 ms 在合计里被完全掩盖，
`CAPTUREBLT` 不花钱，窗口探测只要 0.2 ms。细节见
[HANDOFF 第 19 节](../../docs/HANDOFF.md#19-捕获耗时拆解下一轮-dxgi-的依据)。

### `overlay_select_e2e.ps1`
遮罩选择行为的端到端测试：**用应用真正保存下来的 PNG 尺寸当判据**（不做 OCR、
不做像素启发式）。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\overlay_select_e2e.ps1
```

它自己摆一个**位置已知的记事本窗口**当标尺，然后断言三件事：

1. **单击**高亮的窗口 → 选区就是那个窗口（实测 784×492，窗口 800×500，
   差值是不可见边框）；
2. 普通拖动 → 选区**正好**是拖出来的矩形（240×160）；
3. 在已有选区内再拖 → **重新框选**（380×230），而不是把旧选区拖走
   （那样会还是 240×160）。

⚠️ `Enter` = 确认 = 保存，**确认会关掉遮罩**，所以每个场景都要重新 `Alt+C`。

### `pin_groups_e2e.ps1`
贴图分组的端到端测试，判据是**可见的 `Cuter 贴图` 窗口数量**：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\pin_groups_e2e.ps1
```

六步：贴一张（1）→ 切组（0，上一组被隐藏但**没销毁**）→ 再贴一张（1）→ 切回来（1，东西还在）
→ 隐藏本组（0）→ 再显示（1）。

**这个探针刻意不用键盘**：动作走 `cuter --pin` / `--next-pin-group` / `--toggle-pins`，
由单实例通道转交给正在跑的实例。第一版用的是 Alt+V / Alt+G / Alt+H，结果一遇到
"VM 抢键盘"或"Windows 搜索浮出抢焦点"就四步全红，看起来像四个功能 bug——
而真相是按键根本没送到（见 HANDOFF 第 22 节）。

### 提交前跑这个：`tools/ci/check.ps1`

一条命令跑 clippy / 单测 / i18n 表 / release 构建，任何一步失败即非零退出；
加 `-WithProbes` 会再调用下面的 `run_all.ps1 -Quick`。
CI（`.github/workflows/ci.yml`）只跑无头的那几步——探针需要交互式桌面。

### 先跑这个：`run_all.ps1`

一条命令跑完所有探针并汇总（`-Only <子串>` 过滤、`-Quick` 跳过慢的）。
退出码 0=PASS / 2=SKIPPED / 其它=FAIL，有 FAIL 就非零退出——**每次有意义的改动后跑它**。

### `pin_rounded_probe.ps1`
贴图圆角的**像素级**验证：把纯红图贴上去，截贴图窗口，比四角与中心的颜色。
`rounded_corners = true` → 四角透出桌面（与中心不同色）；`false` → 四角与中心同色。
反向那一半才是"开关不是装饰"的判据。

### `scroll_auto_e2e.ps1`
自动滚动：打开 300 行长文档 → `Alt+S` → 回车 → `A`，然后读应用日志里的
`autoscroll-stop stalled=.. steps=.. height=..`。`steps > 3` 证明合成滚轮真的滚动了目标，
`height` 证明拼接长上去了，`stalled` 证明它自己停了。输入被吞 → exit 2 (SKIPPED)。

### 全量扫描的假阳性

`run_all.ps1` 会在探针之间清掉残留 `cuter` 进程并停 1.5 秒，但**机器被压满时仍会出假阳性**：
探针要连续多次合成输入，而遮罩首帧会从平时的 143~332 ms 掉到 2.4 秒，于是"遮罩还没出现"。

判据（按顺序看两个证据，都指向环境就别改代码）：
1. 应用日志里的实测耗时（`%APPDATA%\Cuter\logs\cuter.log` 搜 `遮罩首帧可见`）；
2. 同一个探针单独复跑是否通过。

### 关于"输入注入被吞"

合成按键/滚轮在这台机器上**偶尔（有时连续）被吞掉**：症状是探针里"遮罩一直不出现"或
"目标窗口纹丝不动"，而应用日志里热键注册是成功的、也没有任何请求进来。

判定方法：**同一时刻跑一个用别的热键的探针**（例如 `latency_probe.ps1` 用 `Alt+C`）。
如果那个能过，就说明热键通道没问题，是这一次注入没到，不要急着改代码。

顺带一条教训：`scroll_e2e.ps1` 曾经报 `width mismatch 756 vs 840` 这种"数字对不上"的错，
看着像随机抖动，实际是**配置漏了 `snap_controls = false`**（控件级吸附会改拖框的边缘）。
凡是拿"精确选区"当判据的探针，两种吸附都必须显式关掉。

### `paddle_ocr_e2e.ps1`（真引擎，可不装）

把上面"真服务端"那条路线包成一个探针：起 `paddle_ocr_server.py` → 等 `model ready` →
驱动 `ocr_http_e2e.ps1 -Endpoint … -Hotkey ocr` → 断言服务端日志里至少 3 个块 → 关掉服务端。

**没装 venv 或模型拉不下来就 SKIP（退出码 2），不算失败**——这样刚 clone 下来的机器
`run_all.ps1 -Quick` 依然是绿的，而装了 PaddleOCR 的机器上这条会真的跑一遍。

    powershell.exe -File tools\probe\paddle_ocr_e2e.ps1
    ... -Python C:\path\to\venv\Scripts\python.exe -Port 8917 -TimeoutSec 180

端口默认 8917（避开手工调试常用的 8900），日志和工作目录落在
`%TEMP%\cuter-paddle-<时间戳>\`。

### `config_recovery_e2e.ps1`

配置读坏时的端到端行为（0.1.59 的能力）：写一份坏配置 → 起应用 → 断言

1. 应用**照样起得来**（回退默认值，默认配置是"启动即进托盘"，所以再请求一次设置窗口，
   既证明它还活着，也证明它还响应）；
2. 配置目录里出现**带时间戳**的 `config.toml.broken-*`，内容就是那份坏文件；
3. 坏文件不再留在原地当活配置；
4. 日志里记下了"已备份"。

单元测试管加载器，这条探针管接线（`load()` → `App::new` → 日志）。收尾会把
这次派生的 `.broken-*` 删掉并还原用户配置——探针故意弄坏东西，就必须自己收拾干净。

### `ocr_http_e2e.ps1` 与 `paddle_ocr_server.py`

在线 OCR（`[ocr] engine = "http"`）的端到端测试。两种跑法：

    # 假服务端（默认）：固定回三个块，只验"请求发出去了、面板开了"
    powershell.exe -File tools\probe\ocr_http_e2e.ps1

    # 真服务端：本机 PaddleOCR，验"真认出了字"
    %USERPROFILE%\paddle-ocr-venv\Scripts\python.exe tools\probe\paddle_ocr_server.py ^
        --port 8900 --log target\tmp\paddle-requests.log
    powershell.exe -File tools\probe\ocr_http_e2e.ps1 ^
        -Endpoint http://127.0.0.1:8900/ocr -ServerLog target\tmp\paddle-requests.log

`-Endpoint` 给了就不起假服务端；`-ServerLog` 指向谁，就断言谁的日志。
`-Hotkey ocr`（Alt+Shift+R，只识别）能避开"贴图 + 识别"那条热键开的置顶贴图窗口——
取证截图时那个窗口会盖住结果面板右下角。

三条踩过的坑：

1. **`results` 是嵌套的**：hubserving 回 `{"status":"000","results":[[[文字, 置信度, 四点], …]]}`，
   注意 `results[0]` 才是这一页的块列表。摊平一层，客户端会报"没有 results[0] 数组"。
2. **剪贴板里必须是有字的图**：假服务端不在乎图片内容，真 PaddleOCR 对风景照回 0 块。
   现在先开 `OcrTarget.cs` 截一张三行大字再放进剪贴板。
3. **paddleocr 要用 2.x**：3.x 在 Windows CPU 上撞 oneDNN + PIR
   （`ConvertPirAttribute2RuntimeAttribute not support ...`），`FLAGS_enable_pir_api=0` 也救不回来。
   2.9.1 一次就通。venv 放仓库外（`cargo clean` 不会删它）。

### `ocr_linkage_e2e.ps1`
OCR 面板 + 文本↔位置联动：靶子 `OcrTarget.cs` 三行大字 → `Alt+C` → 框选 → `Ctrl+O` → 断言面板出现 →
按 `L` 切逐行对照 → 只截面板左侧的图片区一张 → 沿行列表往下走，直到某一行悬停真的把图片区重绘 →
断言那两张图不同。命中时另外存一张整块面板（`panel-hover-full.png`），供归档证据图用。

判据刻意是"**图片区变了**"而不是"整块面板变了"，更不是"没报错"：只断言面板出现的话，
识别把两行并成一段、或者高亮指错块都发现不了；而比整块面板会被列表行自己的悬停底色骗过去——
0.1.49 修的那个 bug（行悬停不点亮图上的框）就是收紧到图片区之后才暴露的。

四条"别再退回去"的写法（`docs/HANDOFF.md` 69.6 与 70.6 就是它们的由来）：

1. **靶子必须是大字号纯 ASCII**（`OcrTarget.cs`：70px Consolas，三行固定矩形）。
   原来用记事本写三行中英混排小字，系统引擎有时只认出两行、有时整段认成乱码，
   于是"该悬停哪一行"变成猜谜——同一构建今天过两次、明天红一次。
2. **行矩形由靶子自己报**（`[OcrTarget]::LineRect(i)`），探针不按窗口偏移量加减。
3. **只截面板、并且往下扫**：整屏截图会让桌面上的时钟每分钟制造一次"两张图不同"的假通过；
   行高随主题字体变，所以按 16px 步进扫过行列表，取第一处真的重绘。
4. **框选边距给足（现在 40/30 px）**：行矩形是 STATIC 控件的矩形，字形自带左边距，
   12 px 的余量会切进第一个字母——OCR 读出来就少一个字（`ALPHA` → `LPHA`），看着像面板
   裁了图。`docs/HANDOFF.md` 70.6 记着这次误判。

（OCR 质量本身取决于系统引擎，这是引擎的事；探针只保证"有干净的三行可悬停"。）

### `scroll_horizontal_e2e.ps1`
横向滚动截屏的端到端测试：靶子是 `ScrollTarget.cs`（父窗口 + 一个 40000×60 的子 STATIC 装整行
伪随机字母，"滚动" = `SetWindowPos` 把那条长行往左挪）。流程与 `scroll_e2e.ps1` 一样
（Alt+S → 拖选区 → Enter → 滚 → 激活控制窗 → Enter 完成），断言"长图**变宽**、高度等于选区高"。

* 横向滚轮必须用 **`SendInput`**（`CuterInputProbe::HWheel`）：`mouse_event` 会忽略
  `MOUSEEVENTF_HWHEEL`，症状是目标窗口纹丝不动。
* 靶子**不要自己画像素**：自绘窗口 + PowerShell 的 C# 编译器很容易把进程搞崩
  （`PAINTSTRUCT` 里的 `ByValArray` 未初始化 → 0xC0000417），交给系统控件最稳。
* 靶子**不能用多行 EDIT**：即使给了 `ES_AUTOHSCROLL`，多行 EDIT 也按控件宽度**折行**、
  没有横向范围，`EM_LINESCROLL` 和横向滚轮都不可能让它动。曾经因此让 `ScrollX()` 长期为 0，
  把「横向一行都拼不出来」这个真 bug 藏了好几轮（HANDOFF 第 71 节）。
* 子 STATIC 要用 **`SS_LEFTNOWORDWRAP`**：`SS_LEFT` 会在控件宽度处折行，横向滚动就没意义了。
* 建窗线程必须 `SetThreadDpiAwarenessContext(-4)`：否则 125% 缩放下
  `(320,260)+900×420` 会变成 `(400,325)+1125×525`。
* 探针会把选区尺寸从应用日志里**回读**（`滚动截屏开始：区域 Rect { … }`），不一致当场抛错——
  "产品拼错了"与"探针拖歪了"必须能一眼分开。

### `scroll_manual_e2e.ps1`
手动滚动截屏（人滚、程序只负责拼接）的端到端测试。`scroll_auto_e2e.ps1` 测的是"程序自己
发滚轮"，而用户真正在用的是这一条：滚轮事件来自人，程序只观察画面有没有动。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\scroll_manual_e2e.ps1
```

* 流程：把配置写成 `[scroll] auto_scroll = false` → Alt+S → 拖选区 → Enter 开始 → **探针自己**
  `Wheel(-3)` 十四轮、每轮间隔 450 ms（模拟"边看边滚"）→ 激活控制窗 → Enter 完成 → 读日志。
* 断言只认应用自己打的 ASCII 标记 `scroll-done frames=23 w=750 h=3493`：帧数 > 1 证明首帧之后
  还在收帧，高度 > 选区高度证明帧被拼上了而不是被丢掉。
* 收尾前必须 `Activate` 控制窗：滚动的人滚轮时焦点在目标窗口上，直接敲 Enter 会打空，
  现场看着就像"拼接坏了"。
* 滚轮没送到目标（帧数 ≤ 1）报 SKIPPED 而不是 FAIL——那是环境把合成输入吞了，没测到东西。

### `pin_drag_out_e2e.ps1`
贴图拖出的端到端测试（键盘相关，被抢键盘时如实 SKIPPED）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\pin_drag_out_e2e.ps1
```

热键贴一张 240×160 的图，再用**另一个进程**里的真 `IDropTarget`
（`tools/probe/DropTarget.cs`）接收：`Ctrl+左键拖动` 与 `中键拖动` 各拖一次。断言
"目标每个手势都收到了拖放" + "数据对象带着 CF_HDROP"（`QueryGetData` S_OK）+
"每次写出 240×160 的 PNG"，并回归"对贴图按 Ctrl+C 不会卡死"。

**没有断言**"把文件读回来"：那个 C# 接收端在 PowerShell 5.1 的旧编译器 + COM 编组下
`GetData` 返回 `DV_E_FORMATETC`，这半步只能手动拖进资源管理器/微信确认（见 HANDOFF 第 26 节）。

### `latency_probe.ps1`
量"按下热键 → 屏幕上真的出现遮罩"这条链路（键盘相关，被抢键盘时如实 SKIPPED）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\latency_probe.ps1 -Cycles 12 -Backend dxgi
```

按 Alt+C → 等遮罩窗口 → Esc，重复 N 次；每一次都从**应用自己的阶段日志**里取分段耗时：

```
截图链路[唤醒到处理] → [进入捕获] → [捕获完成] → [遮罩首帧可见]
```

最后一段（`遮罩首帧可见`）是用户在意的那个数。2026-10 实测：中位 **GDI 156 ms / DXGI 116 ms**
（同一台机器上，第 24 节还是 251 / 235 ms；那一轮之后加了"等首帧期间催帧"，
见 HANDOFF 第 25 节），其中捕获 39 / 12 ms，剩下 ~100 ms 是建遮罩视口窗口 + 首帧。

**它会整份重写 `%APPDATA%\Cuter\config.toml`**（先备份、结束时还原）：
手改 TOML 曾经写出过重复的 `[capture]` 表，导致应用静默回退默认值、
"dxgi 那一次"其实在测 GDI。现在探针一旦在日志里看到 `TOML parse error` 就直接失败。

### `dxgi_backend_e2e.ps1`
DXGI 桌面复制后端的端到端测试（键盘无关）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\dxgi_backend_e2e.ps1
```

把 `capture.backend` 依次改成 `dxgi` / `gdi` / 一个乱写的值，各跑一次
`cuter --full -o <png>`，然后：

* 日志里必须真的出现 `Windows DXGI`（配置被采纳），拿不到复制通道时**如实报 SKIPPED（退出码 2）**
  而不是 FAIL——RDP 会话里本来就没有桌面复制；
* 两个后端输出的 PNG 尺寸一致、平均亮度差 < 12（抽样测量）、不是纯黑/单色；
* 乱写的值必须回退到能用的捕获（手改配置的防御性测试）；
* 无论成败都还原用户配置。

逐像素的严格比对在 Rust 那边（需要同时持有两个后端）：
`cargo test -p cuter-core dxgi_vs_gdi --release -- --ignored --nocapture`
——它同时给出整屏/区域的耗时对照与平均像素偏差。
**"建立后第一帧是空帧"这个 bug 就是这条端到端探针抓到的**（单测当时只比对了第二帧之后）。

### `leak_probe.ps1`
长驻程序的死法回归：**连做 N 次真实截图，看资源计数涨不涨**。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\leak_probe.ps1 -Cycles 120
```

每次循环都是真的一次「Alt+C → Esc」：创建遮罩、整屏捕获、销毁遮罩，不写任何文件。
预热 5 次后取基线，再量：

| 指标 | 为什么量它 |
|---|---|
| `GetGuiResources(GR_GDIOBJECTS)` | 漏一个 DC / 位图立刻在这里现形（截图的经典漏法） |
| `GetGuiResources(GR_USEROBJECTS)` | 漏窗口/菜单类对象 |
| `Process.HandleCount` | 内核句柄；撞上 10 000 上限时进程直接崩 |
| `PrivateMemorySize64` | 内存（涨得最慢，所以只看它会漏掉真问题） |

容差刻意远小于"每次一个"（GDI/USER 各 +10、句柄 +32、私有内存 +24 MB）。
实测 120 次循环（每次约 1.4 s，含探针自己的等待）：**GDI 29→29、USER 26→26、
句柄 315→315、私有内存不增反降**——`crates/cuter-core/src/capture/windows.rs` 里那套
`ScreenDc` / `MemoryDc` / `DibSection` / `SelectedObject` RAII 守卫确实一个都没漏。

### `history_e2e.ps1`
截图历史的端到端测试：**真的截、真的落盘、真的淘汰**。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\history_e2e.ps1
```

它断言四件事（每一条都对应一个"看起来做了、其实没做"的坑）：

1. `limit = 2` 时连截 3 张 → 清单里正好 2 条、`images/` 与 `thumbs/` 各 2 个文件，
   而且**被淘汰那条的文件真的没了**（否则缓存会一直涨）；
2. 清单里记的 `bytes` 与实际文件大小一致（占用数字不能是拍脑袋来的）；
3. **历史里那张"原图"与同一次截图的自动另存文件逐字节一致**——
   这是"历史"与"缩略图画廊"的分界线：只存缩略图的话"再复制一次"会悄悄降质；
4. `limit = 0`（关闭）时**一个文件都不写**；
5. `cuter --history` 转交给正在跑的实例后，**历史面板窗口真的出现**（按标题找）。

⚠️ 它按 `%LOCALAPPDATA%\Cuter\history` 找文件。本项目一开始把 `dirs::data_dir()`
当成了 Local，其实那是 **Roaming**——历史最早写进了 `%APPDATA%`，探针一条都找不到。
现在 `Config::data_dir()` 用 `data_local_dir()`（缓存不该跟着漫游配置文件跑），
并有单测守住这一点。

### `installer_e2e.ps1`
安装包的端到端测试：**真的装、真的卸**（装在 `%TEMP%` 的临时目录里，
全程备份/还原用户配置与自启项）。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\installer_e2e.ps1
# 也可以指定产物：
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\installer_e2e.ps1 -Setup .\dist\cuter-0.1.0-x64-setup.exe
```

它断言的是"装完到底成不成"，而不是"setup 文件存在"：

1. 静默安装（`/VERYSILENT /TASKS=desktopicon,autostart`）退出码 0，
   `cuter.exe` / `README.txt` / `unins000.exe` 都在；
2. **安装目录里不许有 `cuter.portable`**——有的话安装版会把配置写进安装目录；
3. 开始菜单与桌面快捷方式都建出来了；
4. 自启任务**两侧都要到位**：`HKCU\...\Run\Cuter` 指向装好的 exe 且带 `--tray`，
   **并且** `config.toml` 里 `autostart = true`。
   天真的安装器只写注册表，于是第一次启动就被程序自己的对账改回去——这里就是防它的；
5. 装好的程序能跑（`--version`）；
6. 卸载后程序文件没了、**自己那条自启项也没了**，而**用户配置还在**
   （安装包在确认页承诺过"设置、日志与截图会保留"）。

### `autostart_e2e.ps1`
开机自启的四种情形（对真实注册表，含自动备份/还原）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\autostart_e2e.ps1
```

A. 配置关 + 别人的自启项 → **不许动**（便携版不能把已安装版本的那条顺手删掉）；
B. 配置开 → 写入并指向本 exe、带 `--tray`；
C. 配置开 + 旧路径（程序被搬走了）→ 启动时自愈；
D. 配置关 + 自己那条 → 删掉。

### `perf_probe.ps1`
内存与延迟的实测脚本。**发版前跑一次，把表贴进提交信息或 README。**

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\perf_probe.ps1
# 也可以指定产物：-Exe .\target\release\cuter.exe -SettleSeconds 6
```

它量：**托盘模式下真注入 `Alt+C` 到遮罩可见的延迟**（5 次，报 min/median/max）、
托盘常驻内存、设置窗口内存、冻结遮罩峰值内存、空闲 6 秒的增长、
`--full` 端到端耗时（跑三次取 min/median/max），以及冷启动到遮罩可见。
最后还会把 app stderr 里"截图链路"的阶段耗时打出来——**延迟高的时候先看这一段，
它直接告诉你时间花在唤醒、捕获还是建窗上**。

四个已经踩过的坑（脚本里已处理，改脚本时别退回去）：

1. **托盘模式下窗口是隐藏的，`ui()` 不跑，18.8 MB 的中文字体不会被解析**——
   第一次采样可能只有 48 MB，几秒后变成 82 MB，看起来像"空闲时涨了 33 MB"。
   脚本会先让实例打开一次设置窗口，逼出这份一次性开销再采样。
2. **第一次 `--full` 可能是 6.5 秒**（Defender 扫描 + 冷缓存），
   所以必须跑多次报 min/median/max，只报一个数会被噪声带偏。
3. **不要用 `CuterInputProbe::AltKey` 量延迟**：它内部有 50+50+30+400 ms 的 `Sleep`
   （是给人眼观察留的），套在它外面的秒表量到的是脚本自己的睡眠，恒 ≥530 ms。
   现在用 `SendInput` 一次性投递 4 个键盘事件、零睡眠。
4. **`INPUT` 结构体在 x64 上必须正好 40 字节**（联合体里要有 `MOUSEINPUT`）。
   只放 `KEYBDINPUT` 时 `Marshal.SizeOf` 是 32，`SendInput` 会以
   `ERROR_INVALID_PARAMETER` **静默失败**——现象是每次测量都 "FAIL (no overlay)"，
   看起来像 app 的热键坏了。脚本启动时会自检这个尺寸，不对就直接抛异常。

### `hide_own_windows_e2e.ps1`
`capture.hide_own_windows` 的端到端测试：**截图里绝不能出现我们自己的窗口**。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\hide_own_windows_e2e.ps1
```

流程：记事本摆在固定位置（它是"可预测的静态背景"）→ 抓一张基准图 →
让运行中的实例把设置窗口**铺满**记事本的客户区 → 再抓一张（这一张必须与基准图
**差很多**，否则说明这块区域里其实没有我们的窗口，测试会假通过）→ `Alt+C` 框选同一块 →
回车自动保存 → **存下来的 PNG 与基准图的平均偏差必须 ≈ 0**（实测 0.000）。

它同时断言 app 日志里出现过"请求隐藏自身窗口"这条路，所以"为什么通过"也是有据可查的。
四个必须保留的写法：

1. **主题写成 `dark`**：深色设置面板与白色记事本页面差 190 上下，
   判定"漏没漏"一目了然；浅色面板只有 25，和噪声分不开。
2. **先等遮罩几何稳定再拖拽**：遮罩刚创建时物理尺寸可能是 `2400x1350`
   （= 屏幕 × 系统缩放），这时拖拽会按错误的缩放换算，存出 `735x524` 而不是 `882x628`。
3. **额外抓一张 `after.png`**（流程结束、窗口已隐藏时）：`dev(after, reference)` 是
   **背景自身的漂移**（实机 0.000）。区分"漏了窗口"与"背景变了"全靠它。
4. **看残差的形状**：窗口是**整块均匀**覆盖的，而 DWM 重新露出被遮挡窗口时的
   文字重绘残差是**文字形状**的（区域右半边空白处恰好为 0）。
   这一点决定了阈值该取多少，别再被一个 10 上下的数字绕进去。

### `perf_probe.ps1` 之外的性能事实

`notepad.exe` 在同一台机器上是 21.5 MB 工作集 / 5.1 MB 私有，
可以用来校准"82 MB 到底算不算大"。

### `single_instance_e2e.ps1`
单实例守卫的端到端测试：真进程、真转交。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\single_instance_e2e.ps1
```

1. 没有实例在跑时启动 `cuter.exe` → 必须**活着**（守卫不能挡住合法启动）；
2. 再跑 `cuter.exe --capture` → 必须**退出**、退出码 0；
3. 第一个实例必须仍然活着，并且**真的弹出了遮罩**（按标题 `Cuter 截图` 找窗口）。
   第 3 条才是关键：只断言"第二个进程退出了"的话，
   一个"发现已有实例就直接退出"的实现也能通过，而那正是我们要避免的坏体验。

标题里的中文用 `[char]0x622A + [char]0x56FE` 在运行时拼出来，脚本本身保持纯 ASCII。
同样会临时改配置并在 `finally` 里还原。

它默认测 `target\debug\cuter.exe`；**发布前请对真正要交付的产物再跑一遍**：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\single_instance_e2e.ps1 -Exe .\target\release\cuter.exe
```

## 像素统计的做法

模型读不了截图，所以用亮度图代替"看"：

```powershell
$ramp=' .:-=+*#@'
for($r=0;$r -lt 14;$r++){ $line=""
  for($c=0;$c -lt 40;$c++){
    # 对每个格子取平均亮度，映射到 $ramp 的一个字符
  }
  Write-Host "  |$line|" }
```

再加上"接近全黑的像素占比""区域平均亮度""两个区域的平均差"这类标量，
就能把"看起来不对"变成可比较的数字。本次会话中：

- 判定遮罩是否覆盖整屏 → 比较遮罩前后的区域平均亮度比（0.488 即为正常压暗）
- 判定贴图是否正确显示 → 贴图区域 vs 被截取源区域的**平均差 5.34**，vs 无关区域 66.62
- 判定悬停高亮是否变成实心黑 → "接近全黑(<30)的像素占比"（修复后 1.01%）
- 判定长图是否拼对 → 与普通截图的**平均绝对偏差 0.000**（错 1 行就会变成 7.1）

比"平均差"更好用的是**在候选偏移里找最小值**：如果某个偏移处的偏差是 0，就同时证明了
"内容一致"和"偏移量是多少"。本轮就是靠它在 7 个候选里一眼看出"整体差了 1 行"。

## PowerShell 的四个坑

- **变量名大小写不敏感**：`$A` 和 `$a` 是同一个变量。我因此把 `[Bitmap]` 覆盖成了 `[double]`，
  报错信息是"`System.Double` 不含 `GetPixel` 方法"。同样的事还发生在参数上：
  `ocr_http_e2e.ps1` 声明了 `param([string]$Endpoint = '')`，脚本里又写了
  `$endpoint = if ($Endpoint) { ... }`——这行会把 `$Endpoint` **一起改掉**，于是后面
  `$useFake = -not $Endpoint` 永远是 `$false`，假服务端根本不启动，探针却继续往下跑并
  报"面板没出现 / 服务端没收到请求"。**参数的"是否给过"要在覆盖之前判定。**
- **绝不要用 `Set-Content` 改写 UTF-8 源码**：会把中文注释写成 GBK 乱码。
  改代码一律用编辑工具。
- **无 BOM 的 UTF-8 `.ps1` 会被 Windows PowerShell 5.1 当成 ANSI(GBK) 解析。**
  中文注释的字节会被吃进多字节序列，**连换行符一起吞掉**，下一行代码就并进注释里了。
  症状是"某个变量莫名其妙是 `$null`"（本轮 `$selLeft` 就是这么消失的），完全不报错。
  → `.ps1` 一律存成带 UTF-8 BOM；或者干脆只用 ASCII 注释。
  （PowerShell 7 默认按 UTF-8 读，所以同一个文件在 `pwsh` 下正常、在 `powershell` 下出错。）
- **`trap` 里要自己拼路径**：脚本中途失败时，前面定义的变量可能还没赋值，
  trap 里引用它们会再抛一次异常，把真正的错误盖掉。

