# Cuter 路线图

按「先让核心链路完全可用，再叠加智能能力」的顺序推进。每一档都以**可以独立交付、可以独立验证**为标准。

---

## M1 · 截图核心链路  ✅ 已完成

目标：从热键到出图，全链路可用，且性能与体积达到设计指标。

- [x] Cargo workspace 三层结构（core / ui / app），release 启用 LTO + strip
- [x] `Frame` 位图容器：裁剪、模糊（滑动窗口盒式）、马赛克、缩放、PNG/JPEG/WebP/BMP 编解码
- [x] 整型像素几何 `Rect` / `Point` / `Size`，虚拟桌面负坐标安全
- [x] Windows GDI 捕获后端：`CreateDIBSection` + `BitBlt(CAPTUREBLT)`，一次抓完整虚拟桌面
- [x] Per-Monitor-V2 DPI 感知，保证物理像素与屏幕一一对应
- [x] 全屏冻结遮罩：框选、八向控制点、方向键微调、双击确认、右键取消
- [x] 放大镜（8×）+ 取色器（HEX / RGB / 全局坐标）
- [x] 标注工具：矩形、椭圆、箭头、画笔、荧光笔、马赛克、模糊、文字、序号
- [x] 矢量标注的 CPU 栅格化（导出与预览一致，不依赖 GPU 回读）
- [x] 撤销 / 重做（矢量走对象栈，像素效果走分区快照）
- [x] 浮动工具条，图标全部矢量绘制
- [x] 复制到剪贴板 / 保存 / 贴图窗口（滚轮缩放、透明度、中键复位）
- [x] 托盘 + 全局热键 + 设置界面 + 诊断面板
- [x] 无界面 CLI（`--full` / `--monitor` / `--region` / `--list-monitors`）
- [x] 无头单元测试 + 真实截屏冒烟测试

---

## M2 · 手感与精度  ✅ 主体已完成

目标：把「能用」打磨成「比系统自带好用」。全部是 M1 暴露出来或刻意推迟的问题。

### 界面外壳 DPI 自适应
工具条目前固定物理像素尺寸，在 200% 缩放的 4K 屏上会偏小。
方案：把 `toolbar.rs` / `theme.rs` 里的 `metrics` 常量改成由 `Metrics::scaled(scale)` 生成的结构体，`scale` 取当前显示器缩放率。**注意**：只缩放界面外壳，标注的线宽/字号仍然以物理像素为准（它们要烘焙进图片）。

### 窗口 / 控件自动吸附
框选时自动贴合窗口边界与控件矩形，这是截图工具「好不好用」的分水岭。

- [x] **窗口边缘/中线吸附**：`EnumWindows` + `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)`，
      过滤不可见/最小化/cloaked/本进程窗口，收敛到虚拟桌面范围
- [x] 吸附的纯几何层（`cuter-ui/src/snap.rs`）：参考线去重、最近邻、三种手势各自的语义、
      负坐标与退化矩形，全部有单测
- [x] 命中时画出贯穿整屏的参考线；鼠标悬停高亮光标所在窗口；**按住 Alt 临时关闭吸附**
- [x] **控件级吸附**（商用化第十五轮，见 HANDOFF 第 28 节）：悬停/点击优先吸到光标下
      最深的控件，命中优先级 控件 > 窗口边缘 > 屏幕边缘。⚠️ `ElementFromPoint` 只会返回
      遮罩自己（它是最上层的全屏窗口），所以走的是「自己按 z 序找底下那个窗口 +
      逐层下潜 UIA 树」。探针 `control_snap_e2e.ps1` 三个场景全绿

### 区域记忆
- [x] `Ctrl+R` 复原上次框选区域；选区会与当前桌面求交，越界时明确提示而不是给出错误结果
- [x] 每次成功框选后写回 `config.capture.last_region`（可关）

### 贴图增强
- [x] 鼠标穿透：`ViewportBuilder::with_mouse_passthrough`（egui 原生支持，不用自己碰 `WS_EX_TRANSPARENT`）
- [x] 不透明度：贴图视口 `with_transparent(true)` + 用 `Color32::from_white_alpha` 给图片上色调，
      让桌面合成器按帧缓冲的 alpha 混合
- [x] 滚轮缩放可以关（`pin.wheel_zoom`）、窗口不缩放只放大内容（`pin.zoom_within_window`）、
      外部投影（`pin.shadow`）：窗口比内容大一圈留白，用几层渐隐圆角矩形画阴影
- [x] 右键菜单里的「始终置顶」真的改窗口层级，改过的不透明度 / 穿透 / 置顶写回配置
- [x] **穿透逃生门 + 菜单整行命中 + Ctrl+滚轮不透明度**（商用化第四十三轮，见 HANDOFF 第 57 节）：
      穿透是单向陷阱（`WS_EX_TRANSPARENT` 之后收不到任何鼠标消息、连菜单里的开关自己也点不到）→
      托盘菜单加「取消所有贴图的鼠标穿透」（同时清 `pin.click_through`）；`menu_toggle` 的命中判定改成
      `switch.clicked() || row_response.clicked()`；`Ctrl+滚轮` 调不透明度（步长 5%）；提示改成
      1.8 s 自动消失的 toast 且菜单展开时不画
- [x] **圆角**（商用化第二十三轮，见 HANDOFF 第 37 节）：`pin.rounded_corners`（默认开），
      `Frame::rounded_corners` 只改 alpha、带 1 px 抗锯齿，半径按短边 6% 自适应。
      探针 `pin_rounded_probe.ps1` 用像素证明：开=四角裁掉、关=直角（反向那一半才算数）
- [x] **拖拽落盘**（商用化第十三轮，见 HANDOFF 第 26 节）：OLE 拖放 + shell 数据对象，
      Ctrl+左键 / 中键拖动即可拖进资源管理器、聊天窗口等

### 原生「另存为」对话框
- [x] 直接用 `windows` crate 调 `IFileSaveDialog`（不引入 `rfd`，避免多一份 COM 依赖），
      取消/失败时回退到配置目录并如实提示

### 把「假开关」全部接上
M1 遗留了一批"设置里有、代码里没用"的开关，这是最伤信任的一类问题。已全部接上：
- [x] 包含鼠标指针：真的把系统光标位图按 alpha 合成进冻结帧
- [x] 延迟捕获 `delay_ms`
- [x] 窗口吸附开关 `auto_detect_windows`
- [x] 区域记忆开关 `remember_last_region`
- [x] 自动保存 `auto_save`（回车/双击的默认动作改为保存）
- [x] 文件名模板占位符（商用化第二十三轮）：`{date}` `{time}` `{datetime}` `{unix}`
      `{seq}` `{w}` `{h}` `{monitor}` `{app}`；渲染器改用 `NameContext`，有单测
- [x] 保存时弹「另存为」对话框 `ask_path_on_save`
- [x] 提示音 `play_sound`（`MessageBeep`）
- [x] 撤回栈上限 `undo_limit`

### 多显示器混合缩放
两条路，按实测决定：
1. **每显示器一个遮罩窗口** + 跨窗口选区同步（架构更正确，工作量也更大）
2. 保持单窗口，但在检测到混合缩放时主动降级为「当前显示器」并提示（现在已有提示，这里是把它变成默认行为）

---

## M3 · 智能能力  🚧 主体已完成（OCR + 滚动截屏）

### OCR
- [x] 接口与结果模型（`OcrEngine` trait、`TextBlock` 四点多边形、段落合并）
- [x] **内置离线引擎**：Windows 上用系统自带的 `Windows.Media.Ocr`
      （`cuter-core/src/ocr/windows_ocr.rs`）——零下载、零打包、完全离线，
      中文只要系统装了语言包即可用。选它而不是打包 PP-OCR 模型，是为了守住
      「小体积」这条硬约束（PP-OCR 模型 + 运行时是几十 MB 级别）
- [x] 识别跑在独立后台线程上：WinRT 需要 MTA，若在 winit 事件循环线程（可能是 STA）
      上阻塞等待异步结果，会与套间消息泵互相等待而死锁
- [x] 结果面板（`cuter-ui/src/ocr_panel.rs`）：左图右文、文本块悬停高亮、
      单击复制该行、右侧文本可编辑校对
- [x] 遮罩内 `Ctrl+O` 触发；`ocr.auto_after_capture` 让回车直接识别
- [x] 闭包测试：用自己的栅格化器画字 → OCR 读回，逐字一致
- [x] 端到端实测：脚本化框选 → Ctrl+O → 面板出现 + 剪贴板拿到 348 个字符
- [x] 在线引擎：把图片 POST 到 `config.ocr.http_endpoint`，兼容自建 PaddleOCR 服务（0.1.44：自己写 http + base64 + JSON，零新依赖；见 HANDOFF 第 63 节）
- [x] **文本光标 ↔ 图上位置的双向联动**（商用化第十七轮，见 HANDOFF 第 30 节）：
      面板分「编辑全文 / 逐行对照 (L)」两态，悬停行→图上高亮块、点框→选中并滚动到该行
- [x] `TextBlock.quad` 竖排/旋转文本（0.1.46，见 HANDOFF 第 65 节）：高亮框与命中测试都改成认真正的四边形（轴对齐时仍画圆角矩形），在线引擎回的斜框不再套包围盒；竖排文本的阅读顺序已补（0.1.48，见 HANDOFF 第 67 节：整页竖排时按列从右到左重排，横竖混排仍按引擎原序）

### 滚动截屏
- [x] 逐行亮度签名（全分辨率，下标 = 行号）在候选位移上做平均绝对误差匹配，平局取较小位移
- [x] 「粘性页头」：检测固定不动的顶部行，拼接时只保留一次
- [x] `ScrollStitcher` 只追加新露出的行，且**匹配始终以上一张被接受的帧为基准** ——
      抓帧漏帧不会缺行也不会重复（有逐像素单测）
- [x] 交互模型：`Alt+S` → 框选 → 回车开始，**用户自己滚**，控制窗显示实时缩略长图与
      完成/暂停/取消（回车/空格/Esc）
- [x] 控制窗自动避开采集区域（否则会被逐帧截进长图），放不下时明确告知用户
- [x] 端到端实测：840x978 的长图，首屏与普通截图**逐像素一致（平均偏差 0.000）**
- [x] **横向滚动截屏**（商用化第十六轮，见 HANDOFF 第 29 节）：方向**自动判定**，
      用户滚哪边就拼哪边。实现是"每帧转置 → 复用纵向那套匹配/接缝/上限"，
      出入口各转置一次（`DirectedStitcher`）。方向误判的判据最终收敛为四条：
      拼过之后不换方向、重叠区必须有内容、位移要过 `min_offset`、
      **连续两次且步长一致**才翻（周期性内容凑出来的匹配步长是随机的）。
      探针：`scroll_horizontal_e2e.ps1`（760×320 vs 选区 700×320，连跑两轮）
      + `scroll_e2e.ps1` 回归（840×978 / 840×1005，宽度始终等于选区）
- [x] **横向拼接的三个坑**（0.1.50，见 HANDOFF 第 71 节）：① 一片底色能凑出任意位移，
      "追加的全是空白行"也被当成滚动（长图凭空长高一行字，方向还被锁死）；
      ② 抓帧比滚轮快时，两次投票之间夹着静止帧，`Unchanged` 把票清光，方向永远翻不过去；
      ③ 第二票看到的是两次滚轮的位移（120 → 240），"步长一致"要允许整数倍。
      三条都是探针 `scroll_horizontal_e2e.ps1` 抓出来的（它原来一半概率通过，现在连跑四次全过）
- [x] **反向（往上）滚动截屏**（商用化第十九轮，见 HANDOFF 第 33 节）：往回滚同样拼接；
      回到已拍过的区间时靠画布两侧的 `head_room` / `tail_room` 余量避免重复。
      顺带修掉「切到横向之后每一帧都被丢掉」（帧没转置）。四个单测逐像素比对真值
- [x] **自动滚动模式**（商用化第二十六轮，见 HANDOFF 第 40 节）：程序自己用 `SendInput` 合成滚轮
      （先把光标移到选区中央再放回），`AutoScrollTracker` 判定「连续 3 帧没变化 / 到长度上限 /
      转发满 400 次」即停；五个单测 + 探针 `scroll_auto_e2e.ps1`（实测转发 35 次滚轮、
      长图 4141 像素后自己停下）。默认关：它会移动光标，且部分程序忽略合成滚轮

### 屏幕翻译
在贴图窗口基础上叠加：OCR → 翻译 → 用识别框的底色覆盖原文 → 绘制译文。矩形填充与文字栅格化在 M1 已经具备。

### 以图搜图 / 多模态
把选区图片交给可配置的 HTTP 接口，或用 `default_browser` 打开搜索引擎。

---

## M4 · 跨平台

`ScreenCapture` trait 已经把平台差异隔离在 `cuter-core/src/capture/` 内。

- **macOS**：`CGDisplayCreateImage` + `CGWindowListCreateImage`；热键换成 `CGEventTap`；托盘已有 `tray-icon` 支持
- **Linux/X11**：`XShmGetImage`；热键用 `XGrabKey`
- **Linux/Wayland**：只能走 `xdg-desktop-portal` 的 `org.freedesktop.portal.Screenshot`，**无法做冻结遮罩**（portal 不给帧缓冲），需要按 compositor 分别适配（KDE 的 `zkde-screencast` 协议等）。同类工具在 Wayland 下也踩过这个坑。

---

## 持续事项

- 每次提交前跑 `cargo clippy --workspace --all-targets -- -D warnings`
- 关注 release 产物体积：`cargo bloat --release --crates` 找出体积大户
- 基准测试：4K 多屏捕获耗时、遮罩首帧延迟、导出耗时（目标分别为 <30 ms / <120 ms / <80 ms）

---

## M5 · 商用化  🚧 进行中

前四档解决的是「功能有没有」，这一档解决的是「能不能交给一个不认识它的用户」。
判据很具体：**双击安装包能装上、开机自动就绪、出错有据可查、不会悄悄吃掉用户的内存或数据。**

### A. 交付与安装
- [x] 发布产物不留控制台黑窗：保留控制台子系统（CLI 的等待 / 重定向 / 退出码都要它），
      启动时用 `GetConsoleProcessList` 判断这个控制台是不是"只属于我们"，
      是就 `FreeConsole()` 丢掉。实测双击场景 200ms 后已无控制台窗口，见 HANDOFF 第 13 节
- [x] exe 资源：版本号（`0.1.0`）、产品名、说明、版权、`OriginalFilename` 与**图标**
      （图标仍由 `src/icon.rs` 在运行时画给托盘/标题栏，`assets/cuter.ico` 由
      `cargo run -p cuter-app --example dump_icon` 从同一份图形生成，`build.rs` 编进资源段）。
      实测：`(Get-Item cuter.exe).VersionInfo` 六项齐全、`ExtractAssociatedIcon` 能取到图标。
      见 HANDOFF 第 17 节
- [x] 免安装 zip（便携版）：`tools/package/portable_zip.ps1` 产出
      `dist\cuter-<版本>-portable-x64.zip`（含 `cuter.exe` + `cuter.portable` 标记 + README），
      并在**打包后自己解压验证**：日志确实落在 exe 旁边、`%APPDATA%` 那份没被动过。
      便携模式下配置 / 日志 / 数据 / 默认保存目录全部跟着 exe 走（`Config::is_portable`）
- [x] 安装包：`tools/package/build_installer.ps1` + `cuter.iss`（Inno Setup 6）
      → `dist\cuter-<版本>-x64-setup.exe`（4.81 MB）。按用户安装
      （默认 `%LOCALAPPDATA%\Programs\Cuter`，不需要管理员）、开始菜单 + 可选桌面图标、
      可选开机自启（**调用程序自己的 `--enable-autostart`**，所以配置与注册表两侧一致）、
      卸载保留用户配置；探针 `installer_e2e.ps1` 真的装、真的卸，全绿。
      Inno Setup 本身是**免安装**解到 `%LOCALAPPDATA%\Cuter\tools\innosetup`
      （`get_innosetup.ps1`，先校验官方签名）。见 HANDOFF 17.5
- [ ] **代码签名（阻塞项，取决于证书；2026-10-22 用户明确：先不考虑，本人自行调研）**：实测本程序被 Defender 判成
      `Behavior:Win32/Persistence.A!ml` 并**拦截执行**（我们确实会写自启项 + 常驻热键，
      无签名时这类行为没有任何信任来源）。见 HANDOFF 第 32 节；证书只能由人申请，
      另外应做：向 Microsoft 提交误报、README 写明手动放行步骤

### B. 稳定性与可维护性
- [x] 单实例守卫：命名互斥体 + 三个命名事件（显示 / 框选 / 贴图），
      第二个实例把请求转交给正在跑的那个再退出。端到端探针见
      `tools/probe/single_instance_e2e.ps1`（HANDOFF 第 13 节）
- [x] 日志落盘：`%APPDATA%\Cuter\logs\cuter.log`，启动时按 2 MB 滚动、保留 3 份；
      控制台与文件用同一套格式；写不进去只降级、绝不影响启动。设置 → 诊断里有
      「打开日志目录」。见 HANDOFF 第 16 节
- [x] panic hook：panic 写成 `ERROR` 日志（带文件:行）+ 日志路径；GUI 路径下再弹一个
      说明框（托盘程序 panic 之后不能"闪一下就没"）。`cuter --self-test-panic` 可以
      一键验证这条链路
- [x] 配置原子写入（同目录临时文件 + `sync_all` + `rename`），断电不会留下半个
      `config.toml`；失败时不留临时文件（有单测）

### C. 用户价值
- [x] **截图历史**：`%LOCALAPPDATA%\Cuter\history`（便携模式跟着 exe）。
      每次**成功交付**的截图都存一份无损原图 + 一张缩略图 + 一份 `index.toml`（原子写入）；
      面板里一行一张（缩略图 / 时间 / 尺寸 / 占用 + 复制 / 贴图 / 另存为 / 删除），
      顶部显示张数与总占用、「清空历史」两次点击确认、「打开目录」一键可达。
      入口：托盘菜单「截图历史…」、`cuter --history`、设置 → 截图历史。
      端到端探针 `history_e2e.ps1`：**历史里的原图与同一次截图的另存文件逐字节一致**、
      `limit` 真的淘汰最旧的、`limit = 0` 一个文件都不写、面板窗口真的出现
- [x] 开机自启开关（`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`）：
      `general.autostart`（默认关）+ 启动时与注册表对账；**只删指向自己的那一条**，
      便携版不会把已安装版本的自启项顺手删掉（探针 `autostart_e2e.ps1` 四种情形全绿）
- [x] **截图遮罩的交互与工具弹窗**（商用化第七轮，用户实测反馈）：单击高亮窗口 =
      选中该窗口（**默认工具从「矩形」改成「选择」**）；选区内拖动 = **重新框选**
      （Ctrl+拖动才是移动选区）；工具选项弹窗按当前工具显示内容（线宽预设 + 实时预览 /
      马赛克效果预览 / 模糊半径 / 字号），并且**写回配置**。
      端到端探针 `tools/probe/overlay_select_e2e.ps1` 用"应用真正保存下来的 PNG 尺寸"
      当判据：单击窗口 → 784×492（窗口 800×500）、拖 240×160 → 240×160、
      在选区内再拖 380×230 → 380×230（是"重新框选"而不是"移动"）
- [x] **界面多语言（zh-CN / en-US）**（商用化第十八轮，见 HANDOFF 第 31 节）：
      282 条界面文案全部可切；`general.language = auto / zh-CN / en-US`，设置里即时生效。
      中文原文当键（查不到回落中文，改造可增量），四个测试兜住完整性
      （每个 `tr!` 有译文、没有死条目、表按运行时字符串排序、占位符个数一致）。
      工具链在 `tools/i18n/`。探针 `i18n_e2e.ps1`：英文进程里 `i18n-missing` 0 条。本轮删掉的 `general.language` 就是为它预留的占位
      （下一轮的头号任务：**设置界面重做成"左侧分类导航 + 右侧卡片"**，顺带把文案集中起来）
- [x] 自动更新检查（0.1.47，见 HANDOFF 第 66 节）：**只查、只提示、只帮打开下载页**——启动时拉一份 JSON 清单对比版本，有新版就在状态栏与「设置 → 关于」里说一声；不自动下载、不静默替换。https 用系统 WinHTTP（零新依赖），地址留空＝不发请求。**仍缺**：真实发布渠道 / 下载与校验 / 换包重启
- [x] **托盘气泡通知**（商用化第二十二轮，见 HANDOFF 第 36 节）：启动驻留托盘时提示一次热键、
      保存成功后提示文件名。`tray-icon` 没有气泡接口且 hwnd/uID 私有，所以自己注册一个
      `NIS_HIDDEN` 的辅助图标当宿主（`desktop::TrayNotifier`）。探针 `tray_notify_e2e.ps1`
      双向验证开关真伪。⚠️ 本开发机整体抑制托盘通知（可见图标对照实验也不显示），
      所以"画面能看到气泡"这一条没有在本机确认过
- [x] **贴图分组**（商用化第九轮）：`pin.groups`（1-9，默认 2）+
      `pin.active_group`（持久化）；**Alt+H** 隐藏/显示当前组、**Alt+G** 切到下一组
      （也可脚本化：`cuter --toggle-pins` / `--next-pin-group`）；托盘菜单与设置页都有入口。
      可见性靠"不声明视口"实现（`Visible(false)` 对每帧重新声明的视口不生效）。
      端到端探针 `pin_groups_e2e.ps1`：1 → 0 → 1 → 1 → 0 → 1，六步全绿
- [x] **贴图拖出到别的程序**（商用化第十三轮，见 HANDOFF 第 26 节）：贴图上
      **Ctrl+左键拖动** 或 **中键拖动** → 拖进资源管理器 / 微信 / 浏览器。
      实现是 OLE 拖放 + shell 数据对象（`SHCreateDataObject` + 文件 PIDL，自带 CF_HDROP），
      贴图先落成 `%TEMP%\Cuter\drag\*.png`（超一天自动清）。
      探针 `pin_drag_out_e2e.ps1` 验证：另一个进程的真 `IDropTarget` 两个手势都收到拖放、
      数据对象确实带 CF_HDROP、每次写出 240×160 的 PNG。
      **未自动验证**：把文件从数据对象读回来（探针那个 C# 接收端的 COM 编组不可靠），
      这半步靠手动拖进资源管理器确认；顺带修掉了"贴图动作处理自锁死"的老 bug
- [x] **控件级吸附（UI Automation）**（商用化第十五轮，见 HANDOFF 第 28 节）：
      悬停/点击优先吸到光标下最深的控件（按钮/输入框），窗口空白处仍退回整窗；
      `capture.snap_controls` 可关。⚠️ `ElementFromPoint` 只会返回遮罩自己（它是最上层全屏窗口），
      所以走的是"自己按 z 序找底下那个窗口 + 逐层下潜 UIA 树"。
      探针 `control_snap_e2e.ps1` 三个场景全绿（点按钮 = 275×80 与实际控件逐像素一致）
- [x] **OCR 文本 ↔ 位置双向联动**（商用化第十七轮，见 HANDOFF 第 30 节）：
      面板分「编辑全文 / 逐行对照 (L)」两态，联动只在只读的逐行列表里做
      （可编辑文本会让行号与文本块失配）。悬停行→图上高亮块、点框→选中并滚动到该行。
      命中测试抽成 `OcrResult::block_at`（重叠取最小、有单测）。
      探针 `ocr_linkage_e2e.ps1`（也是 OCR 链路的第一个端到端测试）PASS
- [x] OCR：在线 HTTP 引擎（0.1.44；`ocr.http_endpoint`，自己写 HTTP 客户端，未引入新依赖）；真实 PaddleOCR 服务上的验证已在 0.1.52 完成（本机 PaddleOCR 2.9.1，见 HANDOFF 第 73 节），0.1.53 起变成探针 `paddle_ocr_e2e.ps1`（没装 venv 就 SKIP，见 HANDOFF 第 74 节）
- [x] **横向滚动截屏**（商用化第十六轮，见 HANDOFF 第 29 节）：方向**自动判定**，
      用户滚哪边就拼哪边。实现是"每帧转置 → 复用纵向那套匹配/接缝/上限"，
      出入口各转置一次（`DirectedStitcher`）。方向误判的判据最终收敛为四条：
      拼过之后不换方向、重叠区必须有内容、位移要过 `min_offset`、
      **连续两次且步长一致**才翻（周期性内容凑出来的匹配步长是随机的）。
      探针：`scroll_horizontal_e2e.ps1`（760×320 vs 选区 700×320，连跑两轮）
      + `scroll_e2e.ps1` 回归（840×978 / 840×1005，宽度始终等于选区）
- [ ] 多显示器混合缩放：每屏一个遮罩窗口（2026-10-22 用户明确：没有多屏条件，先不考虑）（**本机只有一块屏，做了也无法验证**，
      所以一直没动；`copy_subrect` 的偏移逻辑已有单测）

### D. 质量与性能
- [x] clippy 零警告（本轮随 rustc 1.98 的新 lint 重新归零，见提交 `chore: clippy 归零`）
- [x] 配置项 ↔ 代码作用点全量审计（见 [docs/config-audit.md](config-audit.md)）
- [x] **审计表与现实对齐**（0.1.51，见 HANDOFF 第 72 节）：表里删掉重复行与已经落地的"已删除"条目，删掉 `ocr.model_dir` 与 8 个死函数，并把审计方法固化成 `tools/docs/audit_config.py`
- [x] **内存实测**（`tools/probe/perf_probe.ps1`）：常驻托盘 **87~97 MB** 工作集 /
      51~52 MB 私有，遮罩打开峰值 **137~145 MB** / 75 MB，空闲不增长。顺带查出一个真问题：
      18.8 MB 的微软雅黑被**同时存了两份**（`FontData::from_owned` 会再拷一份），
      改成 `Cow::Borrowed` 后直降 **37 MB**。详见 HANDOFF 第 14 节
- [x] **「截图里不得出现我们自己的窗口」有端到端回归测试**
      （`tools/probe/hide_own_windows_e2e.ps1`，实测偏差 0.000）
- [x] `--full` 端到端（进程启动 → PNG 落盘）：中位数 **137~157 ms**；冷启动到遮罩可见 **~0.5 s**
- [x] **把「按热键 → 遮罩可见」量出来并压过一轮**（商用化第三轮，见
      [HANDOFF 第 15 节](HANDOFF.md#15-商用化第三轮热键延迟)）：
      托盘模式下 min **79** / median **113** / max **188 ms**（5 次）。
      做法是（a）热键/托盘线程用 `set_event_handler` + `WM_PAINT` 直接唤醒 UI
      （"被读到"那一段 40~100 ms → **2~13 ms**），（b）托盘模式不再无谓地"等两帧"
      （隐藏窗口一帧就是 100 ms），（c）另一个实例转交的请求在同一帧处理。
      顺带修掉一个真 bug：设置窗口会被截进图里（15.4）
- [x] **捕获耗时拆解**（商用化第六轮，`bench_capture_breakdown`，1920×1080 / 7.9 MB）：
      `BitBlt + CAPTUREBLT` **33.3 ms**、不带 `CAPTUREBLT` **33.2 ms**（**所以它不是负担**，
      之前"去掉它就能省一截"的猜测是错的）、`GdiFlush` ≈0 ms、`to_vec` 8 MB **3.0 ms**、
      `from_bgrx` 通道交换 **5.1 ms**、`capture_rect_impl` 合计 **33.7 ms**、
      `capture_desktop` **33.2 ms**、窗口探测 **0.2 ms**、`monitors()` **0.01 ms**。
      **结论：整屏 `BitBlt` 一项就占了全部（软件搬运那两步在合计里被完全掩盖），
      所以"优化 Rust 侧的拷贝/通道交换"没有意义，唯一的杠杆是换捕获 API。**
      另外它纠正了一条旧结论：窗口探测不是开销（0.2 ms），不必去"优化"它
- [x] **DXGI 桌面复制后端**（商用化第十轮，见 HANDOFF 第 23 节）：`capture.backend = "dxgi"`
      （默认仍是 `gdi`）。实测本机 1920×1080：整屏 **11~15 ms** vs GDI 42~107 ms（约 4 倍）、
      区域 960×540 **4.5 ms** vs 21~26 ms（约 5 倍），**逐像素平均偏差 0.00 / ≤8 占比 100%**。
      建立后端 137 ms（优化前 1761 ms，原因是给每个空适配器都建了设备），
      所以 GUI 在启动时后台预热、无界面路径按配置付这一次成本。
      当初列的难点都处理了：拿不到 duplication 回退 GDI、静态桌面复用上一帧、
      RDP 不支持时报 `Unavailable` 并回退；**多显示器拼接代码写了但本机只有一屏，
      未经真实双屏验证**（`copy_subrect` 的偏移逻辑有单测，端到端只覆盖单屏）。
      三个坑：传适配器必须用 `D3D_DRIVER_TYPE_UNKNOWN`、建立后**第一帧是空帧**、
      建立成本高所以不能随用随建
- [x] **量出"热键 → 遮罩首帧可见"的完整延迟**（商用化第十一轮，见 HANDOFF 第 24 节）：
      新增 `截图链路[遮罩首帧可见]` 阶段日志与探针 `tools/probe/latency_probe.ps1`。
      实测中位 **GDI 251 ms / DXGI 235 ms**，其中捕获只占 45 ms / 11 ms，
      **~200 ms 全花在 eframe 建遮罩视口窗口 + GL 上下文 + 首帧上传**。
      顺带排除"激活窗口"这个猜想（`with_active(false)` 无改善）；DXGI 在总量上只值 35 ms
- [x] **"热键 → 遮罩出现"压到 ~116 ms**（商用化第十二轮，见 HANDOFF 第 25 节；**0.1.54 复测见
      第 75 节**：同一台机器今天量出 GDI 中位 184–196 ms、DXGI 190 ms。**0.1.57 用同机 A/B
      关掉了这个悬案**：0.1.51 与 0.1.56 中位分别是 181 / 180 ms，旧记录的 113/156 ms 在今天
      这台机器上任何构建都复现不出来，属机器状态差异（见第 78 节）。探针输出从 0.1.54 起带
      backend 名、从 0.1.57 起报 9 次采样的 25/75 分位）：
      托盘模式下 eframe 把重绘节流到 ~100 ms，而遮罩要两帧才画出来 → 等首帧期间
      **每帧戳一次 `UiWake::wake`**（上限 40 帧），帧间隔从 ~125 ms 降到 ~17 ms。
      实测中位 **GDI 156 ms / DXGI 116 ms**（第 24 节是 251 / 235 ms）
- [x] ~~遮罩视口窗口常驻复用~~ —— **否决**（第 25 节）：eframe 对不可见的 deferred 视口
      直接 `show_ui = false`（`visible` 状态被缓存，移回屏幕也不再开画，画面永远空白）；
      `ViewportBuilder::active` 一变就重建窗口；`Visible(false)` 又不生效。三条都实测过
- [ ] 再往下压只剩两条"大改"的路（暂不做，收益 ~100 ms；2026-10-22 用户明确：性能暂时够用）：① 热键一到就先声明一个透明
      遮罩窗口，让窗口创建与捕获重叠（少一帧，代价是用户会先看到一瞬间实时桌面）；
      ② 自建 winit 窗口 / 换渲染后端（成本是整个 UI 层）
- [x] **冷启动：已测量并摊开**（商用化第二十轮，见 HANDOFF 第 34 节）：进程创建 →
      第一帧中位 **364 ms**，我们的代码只占 85（装载+CRT）+ 1（托盘+7 热键）+ 16
      （第一帧）ms，大头是 eframe/GL 的 126 ms；同机对照 `cmd /c exit` 都要 1172 ms。
      结论：不值得再压，真凶是杀软扫描（压它要靠代码签名）。诊断页已显示实测启动耗时
- [x] **托盘待命时的 GPU 占用**（商用化第二十一轮，见 HANDOFF 第 35 节）：
      待命时空转重绘 8 fps → 1 fps，GPU 均值 **0.734 % → 0.079 %**（降 9.3 倍）；
      受控 A/B 两轮证明这一档不影响热键延迟（差异在噪声内）。
      新探针 `idle_gpu_probe.ps1` 把这条钉住
- [x] **界面字体按需装载**（商用化第十四轮，见 HANDOFF 第 27 节）：托盘常驻
      **86.0 → 57.2 MB 工作集**（私有 48.9 → 28.6 MB），代价是首次画界面多 22 ms
- [x] **没窗口要画时卸载界面字体**（商用化第二十四轮，见 HANDOFF 第 38 节）：
      开过一次设置窗口之后，字体原本再也不还；现在 `needs_font()` 变假满 2 秒即卸载
      （`ctx.set_fonts(默认)` 才真的丢掉），工作集 **82.1 → 55.8 MB**，重装 29 ms
- [x] **遮罩关闭后的纹理与 `Frame`：已量清，无需再动**（商用化第二十五轮，见 HANDOFF 第 39 节）：
      冻结帧与纹理确实跟着遮罩一起释放（143.3 → 90.3 MB）；剩 10 秒后 +11 MB 工作集
      来自 GL 驱动 / egui 纹理管理器的内部缓存，对象生命周期本来就是对的
- [x] **贴图纹理预算**（商用化第二十七轮，见 HANDOFF 第 41 节）：切走/隐藏的贴图立刻释放纹理
      （判据复用 `pin_should_show`），实测 222.0 → 146.9 MB；`pin.texture_budget_mb`（默认 256）
      超了用托盘通知提醒，**不降质、不拒绝新贴图**（理由见文档）
- [x] 句柄 / GDI 对象增长回归测试（商用化第六轮）：`tools/probe/leak_probe.ps1`
      连做 **120 次**「Alt+C → Esc」真实截图，再量 `GetGuiResources` 的
      GDI/USER 对象数、内核句柄数与私有内存。实测 **GDI 29→29、USER 26→26、
      句柄 315→315、私有内存不增反降**（零增长，常驻 315 个句柄）。
      这条正是长驻托盘程序的死法：每次截图漏一个 DC/位图，几小时后撞上 10 000
      句柄上限崩掉，而内存看起来完全正常——所以它必须**按次数**测，不是看一眼任务管理器
- [x] **端到端回归脚本一条命令跑完，可在 CI / 无头会话里跑**（商用化第二十五轮，见 HANDOFF 第 39 节）：
      `tools/probe/run_all.ps1` 自动发现所有 `*_e2e.ps1` / `*_probe.ps1`，逐个独立进程跑、各自超时，
      汇总成一张表；0=PASS / 2=SKIPPED（不算失败但明写）/ 其它=FAIL，有 FAIL 就非零退出
- [x] **提交门禁 + CI**（商用化第二十八轮，见 HANDOFF 第 42 节）：本地一条命令
      `tools/ci/check.ps1`（clippy / 测试 / i18n 表 / release 构建，任何一步失败即非零退出；
      `-WithProbes` 再跑桌面探针）；`.github/workflows/ci.yml` 在 Windows runner 上跑无头的那几步。
      **探针故意不进 CI**：它们要交互式桌面，静默跳过比没有更糟

### F. 界面重做（商用化第二十九轮起）
- [x] **设计系统 v1 + 4 张高保真稿**（`docs/design/DESIGN-SYSTEM.md` + `mockups/`，SVG 可导入 Figma，
      由 `tools/design/make_mockups.py` 生成；含三个应用图标概念与 9 尺寸 `.ico`）
- [x] **主题令牌落地 + 设置窗口外壳 + 通用页**（HANDOFF 第 43 节）
- [x] **应用图标重做 + 双皮肤（曜夜紫 / 薄荷奶白）**（HANDOFF 第 44 节）：图标由
      `crates/cuter-app/src/icon.rs` 运行时绘制，跟随主题换任务栏/Alt+Tab/托盘三处图标
- [x] **自绘标题栏（替掉系统白条）+ 头部「⋯」对齐**（HANDOFF 第 44 节）
- [x] **浅色主题配色改成"薄荷奶白"**（`crates/cuter-ui/src/theme.rs` 第 103 行，强调色薄荷青）
- [x] 全部设置页按同一套零件推进（通用 / 保存 / 截图 / 快捷键 / 标注 / 滚动截屏 / 识别文字 / 贴图 / 截图历史 / 字体 / 诊断 / 关于，均已改行式并逐页抓图核对）
- [x] 遮罩浮动工具条按设计稿对齐（工具 | 颜色·线宽·更多 | 撤销·重做 | 完成，全条唯一实心按钮是「完成」）+ 选区提示合成一个胶囊「690 × 460　(430, 319)」
- [x] OCR 面板按设计稿对齐（顶栏胶囊 + 编辑全文/逐行对照；右列编号徽标 + 复制 + 提示框；底栏「重新识别 / 复制全部」）
- [x] 贴图右键菜单按设计稿重做（Figma 板「贴图右键菜单 · 设计稿」→ `crates/cuter-ui/src/pin.rs` 的 `menu_row` / `menu_toggle` / `menu_separator`）
- [x] 贴图右键菜单被贴图窗口裁掉 → 菜单打开时先把窗口撑大、关闭再还原（贴图不移位不拉伸）
- [x] 贴图菜单贴屏幕边时看不全：改成「挪窗 + 内容补偿绘制」，装不下时按可用工作区降级（商用化第四十二轮，见 HANDOFF 第 56 节）
- [x] 截图历史面板改成卡片 + 行（顶栏张数/占用胶囊、每条卡片含复制·贴图·另存为·删除、空状态提示）
- [x] 贴图手感按用户实测反馈收口（整行开关 / Ctrl+滚轮不透明度 / 提示 toast 化 / 穿透托盘逃生门），
      真机读数：`topmost True -> False`、`100% -> 70%`（商用化第四十三轮，见 HANDOFF 第 57 节）
- [x] 滚动截屏面板按设计稿改版面（商用化第四十四轮，见 HANDOFF 第 58 节；设计稿：Figma 画板 `40:230`，导出图 `docs/design/mockups/scroll-panel.png`）
- [x] 贴图鼠标穿透的第二个出口：`cuter --clear-pin-click-through`（与托盘菜单同一处理函数），
      真机六步读数：`click_through = true` 的贴图 `exstyle 0xC0138`（含 `WS_EX_TRANSPARENT 0x20`）、
      右键菜单打不开（窗口没变大）→ 跑一次这条命令（转交给活着的实例）→ `0x40118`（0x20 清掉）、
      右键菜单重新打开（864×1137 → 1080×1421）、配置里 `click_through = false`（商用化第四十六轮，见 HANDOFF 第 60 节）
- [x] 每个 deferred 视口都钉死主题（`pin_viewport_theme`：`set_theme` + `set_visuals` 两个都设），
      设计系统文档 §11 里的「浅色主题全套」也做过真机逐屏核对（商用化第四十五轮，见 HANDOFF 第 59 节）
- [x] 托盘右键菜单从系统原生菜单改成本程序自绘面板（`crates/cuter-ui/src/tray_panel.rs`：无边框 268×407 点、顶栏版本号、三组行 + 快捷键右对齐、
      `退出 Cuter` 危险色、底部「左键单击托盘图标 = 框选截图」提示；摘掉 `muda` 菜单后右键事件照常到达），
      真机读数：窗口 335×509 物理 = 268×407 点、点「截图历史…」行 → 日志 `面板返回动作 OpenHistory` →
      `收到动作 OpenHistory` → 历史窗口 918×747（商用化第四十七轮，见 HANDOFF 第 61 节）
- [x] 托盘右键的**系统链路**真机补验（0.1.48 补验，见 HANDOFF 第 68 节）：那一轮是从 `cuter --tray-menu`
      验证的，`系统图标 → tray-icon → TrayCommand::OpenMenu` 这一跳一直没覆盖。`tools/probe/tray_menu_e2e.ps1`
      用 `Shell_NotifyIconGetRect` 拿图标矩形（藏在溢出里时先点任务栏 `TrayNotifyWnd` 里那个 30×38 的 `Button`
      打开溢出面板），在图标上真右键 → 面板出现且 `IsOnDesktop`、`Esc` 关闭；读数 `uid=2`、
      `rect=1518,792 50×50`、面板 `1543,533 335×509`，证据图 `docs/design/shots/0.1.48-tray-menu.png`
- [x] 遮罩放大镜的取色读数区收成一张卡片（`crates/cuter-ui/src/overlay.rs` 的 `fn paint_readout`
      + 纯几何 `fn readout_rect`：与放大镜框等宽、22×22 色块 + `#RRGGBB` 主色 + `rgb(...)` / 全局坐标
      两行暗色、贴屏幕底边自动上翻；`cargo test -p cuter-ui` 107 passed），真机前后对比图
      `docs/design/shots/0.1.43-magnifier-before.png` / `0.1.43-magnifier-after.png`
      （商用化第四十八轮，见 HANDOFF 第 62 节）
- [x] 滚动截屏控制面板：预览框下加一条 4px 进度轨（已拼高度 ÷ 长度上限）（0.1.45）
- [x] 遮罩毛玻璃底板（`crates/cuter-ui/src/frost.rs`：整帧缩到 1/4 + 两遍盒式模糊 + 叠白 10% + 按遮罩比例压暗，垫在工具条下面并外扩 6px；不依赖系统合成器）

### E. 文档与信任
- [x] **面向用户的发布说明**（商用化第三十轮）：`docs/RELEASES.md` —— 这是什么、
      装哪个、热键表、逐版更新（用户语言）、**已知限制**（无签名/SmartScreen、单屏验证、
      系统 OCR 限制、自动滚动的适用范围）。写了「版本清单必须与 CHANGELOG 对齐」的检查，
      并做过反向测试（造一个不存在的版本，确认会被拦下）
- [x] **维护者手册**（商用化第二十九轮）：`docs/MAINTAINER.md` 按「症状 → 判据 → 结论」
      重组织了 28 轮的经验（规矩、坑、诊断索引、发布流程）；配 `tools/docs/check_refs.py`
      在门禁里检查文档引用的路径与 HANDOFF 小节号是否真实存在（防文档腐烂）
- [x] `CHANGELOG.md`（商用化第二十二轮）：按版本倒序记「用户能感知的变化 + 实测数字」，
      数据来自 `git log` 与 `docs/HANDOFF.md`，不凭记忆写
- [x] 每个设置项在文档里标注对应的代码位置：`docs/config-audit.md` 就是这张表，
      每加一个开关都会补一行（并有探针/单测证明它不是假开关）

