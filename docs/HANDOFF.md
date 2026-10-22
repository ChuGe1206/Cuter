# 交接文档 · Cuter（Rust 截图工具）

> 这份文档的唯一目的：**让一个全新的会话在不读历史对话的情况下，能够准确接手。**
> 最后更新：商用化第二十九轮（设计系统 v1 落地：主题令牌 + 设置窗口外壳 + 通用页）。
> 三十五处最值得先看的：
> * [第 6 节](#6-bug悬停高亮把窗口涂成纯黑已修复并提交25eccb7) —— 一个已经修掉的 bug，
>   以及**一条必须知道的历史误判更正**（别再去追那个"神秘黑块"）；
> * [第 10 节](#10-ocrm3-已完成) —— OCR 为什么用系统引擎、线程模型为什么必须是 MTA；
> * [第 11 节](#11-滚动截屏m3-已完成) —— 拼接算法、控制窗为什么必须放在选区外、
>   以及那个**1 像素客户区内缩**的坑；
> * [第 12 节](#12-商用化第一轮配置项审计--假开关修复) —— 「假开关」是怎么被系统性地找出来的，
>   以及 [docs/config-audit.md](config-audit.md) 这张必须维护的表；
> * [第 13 节](#13-商用化第二轮交付形态) —— 为什么**没有**改成 GUI 子系统、
>   单实例为什么做成"转交请求"而不是"直接退出"；
> * [第 14 节](#14-性能与内存从形容词变成数字) —— 内存与延迟的**实测数字**、
>   怎么量的、以及那个「18.8 MB 字体存了两份」的 37 MB 发现；
> * [第 15 节](#15-商用化第三轮热键延迟) —— 热键延迟的实测数字、
>   **两个会把测量本身搞错的坑**、以及剩下那 100 ms 到底是谁的锅；
> * [第 16 节](#16-商用化第四轮稳定性日志--panic--原子写入) —— 日志落盘在哪、
>   panic 怎么留现场、配置为什么必须原子写；
> * [第 17 节](#17-商用化第五轮交付形态exe-资源--便携版--自启) —— exe 的版本与图标
>   怎么来的（**改图标要重跑 example**）、便携模式为什么用标记文件、
>   自启"只删自己那条"的理由，以及安装包怎么用**免安装的 Inno Setup** 打出来；
> * [第 18 节](#18-商用化第六轮截图历史) —— 历史为什么必须存原图、
>   面板为什么是"一行一张"、以及 `dirs::data_dir()` 其实是 Roaming 这个坑；
> * [第 19 节](#19-捕获耗时拆解下一轮-dxgi-的依据) —— **捕获那 33 ms 到底花在哪**
>   （两条旧猜测被实测推翻：`CAPTUREBLT` 不背锅、优化 Rust 侧拷贝也没意义），
>   以及下一轮 DXGI 的预期收益与已知坑；
> * [第 20 节](#20-商用化第七轮用户实测反馈六条) —— 用户实测提的六条：
>   **「重新截图变白」的真因**（判断"要不要藏自己"时漏了遮罩）、
>   为什么"单击高亮的窗口"原来没反应（默认工具是矩形），
>   以及"选区内拖动 = 重新框选"这条交互改动；
> * [第 21 节](#21-商用化第八轮设置界面重做win11-风格) —— 设置界面重做时踩的两个
>   egui 视觉坑（透明控件底色导致复选框消失、卡片底色与控件同色），
>   以及**别在本仓库随手跑 `cargo fmt --all`**；
> * [第 22 节](#22-商用化第九轮贴图分组) —— 贴图分组；
>   **可见性靠"不声明视口"而不是 `Visible(false)`**（后者对每帧重新声明的视口不生效），
>   以及三个把"功能没实现"和"环境问题"搞混的坑（键位被别的常驻程序抢走、
>   同一表达式里双重加锁把自己锁死、键盘注入被 VM/搜索浮出吃掉）；
> * [第 23 节](#23-商用化第十轮dxgi-桌面复制捕获后端实测快-35-倍) —— DXGI 后端；
>   **传适配器必须用 `D3D_DRIVER_TYPE_UNKNOWN`**、**建立后第一帧是空帧**、
>   建立要 137 ms 所以只能后台预热（附与 GDI 的逐像素/耗时对照表）；
> * [第 24 节](#24-商用化第十一轮把用户真正感受到的延迟量出来结论瓶颈已经不在捕获) —— 
>   **别再从捕获上找延迟**：捕获只占 4~18%，~200 ms 在建遮罩视口窗口（数据 + 新探针
>   `latency_probe.ps1`），以及两个"探针自己把测量搞错"的坑；
> * [第 25 节](#25-商用化第十二轮把热键--遮罩出现从-250-ms-压到-116-ms) —— 那一轮怎么把延迟
>   压掉一半（**托盘模式要显式催帧**），以及"让遮罩窗口常驻复用"为什么在 eframe 0.36 上
>   走不通（`active` 一变就重建窗口、不可见的 deferred 视口根本不跑 pass）；
> * [第 26 节](#26-商用化第十三轮贴图拖出到别的程序顺带挖出一个点一下就卡死的老-bug) ——
>   贴图拖出（OLE 拖放，用 shell 的数据对象）；以及 `match lock(&x).foo()` 里那只
>   **活到整个 match 结束**的 MutexGuard 怎样让"贴图的每个动作"都自锁死；
> * [第 27 节](#27-商用化第十四轮界面字体按需装载常驻内存-86-mb--57-mb) ——
>   托盘常驻时那 18.8 MB 的字体根本没用上：按需装载之后常驻内存 86 → **57 MB**
>   （代价是第一次画界面多 22 ms，只付一次）；
> * [第 28 节](#28-商用化第十五轮控件级吸附ui-automation) —— 控件级吸附；
>   以及为什么 `ElementFromPoint` 拿不到控件（**它每次都返回遮罩自己**），
>   必须自己走 z 序 + 逐层下潜；
> * [第 29 节](#29-商用化第十六轮横向滚动截屏方向自动判定) —— 横向滚动截屏
>   （转置复用纵向逻辑）；以及方向误判的两个来源（**空白区域任何位移都匹配**、
>   已经拼过之后不该再怀疑方向）；
> * [第 30 节](#30-商用化第十七轮ocr-的文本--位置双向联动) —— OCR 双向联动；
>   以及"**联动只在只读的逐行列表里做**"这条设计约束（可编辑文本会让行号失配）；
> * [第 31 节](#31-商用化第十八轮界面多语言zh-cn--en-us) —— 界面多语言；
>   以及为什么用"中文原文当键"、以及**表必须被测试兜住**（否则英文界面半中半英）。

---

## 0. 一句话现状

M1（截图核心链路）+ M2（吸附 / 手感 / 假开关）+ **M3 的 OCR 与滚动截屏**已完成，
**253 个测试全绿**（另 7 个真机测试用 `-- --ignored` 跑，同样全绿），clippy 零警告，
release 产物 **8.1 MB** 上下（含图标与版本资源）。
OCR 用系统自带的 `Windows.Media.Ocr`：零下载、零打包、离线可用，中文开箱即用，
实测对一张屏幕截图约 130 ms 出结果。滚动截屏用纯 Rust 的行签名匹配 + 增量拼接，
端到端实测长图首屏与普通截图**逐像素一致（平均偏差 0.000）**。

商用化第一轮（[第 12 节](#12-商用化第一轮配置项审计--假开关修复)）把**全量配置项**
对着代码搜了一遍，修掉 6 处「配置里写着、代码里不生效」的假开关，删掉 3 个从未实现的
占位字段。第二轮（[第 13 节](#13-商用化第二轮交付形态)）解决了「双击会留下一个控制台黑窗」
和「第二个实例抢走全局热键」。第三轮（[第 15 节](#15-商用化第三轮热键延迟)）把
「按热键 → 看到遮罩」**量了出来**（托盘模式下 min 79 / median 113 / max 188 ms，
"被读到"那一段只剩 2~13 ms），顺带查出一个会把设置窗口一起截进图里的真 bug（15.4）。
第四轮（[第 16 节](#16-商用化第四轮稳定性日志--panic--原子写入)）补上稳定性三件套：
**日志落盘（2 MB 滚动）+ panic 留现场 + 配置原子写入**。第五轮
（[第 17 节](#17-商用化第五轮交付形态exe-资源--便携版--自启)）补齐交付形态：
**exe 带版本与图标、便携版 zip（打包脚本自验）、开机自启开关、Inno Setup 安装包
（真的装/真的卸，`installer_e2e.ps1` 全绿）**；只剩代码签名（17.6）。
第六轮（[第 18 节](#18-商用化第六轮截图历史)）补上**截图历史**
（无损原图 + 面板 + 再复制/再贴图/另存为/删除，探针逐字节验证）。
审计表在 [docs/config-audit.md](config-audit.md)。

**待办**：代码签名；界面多语言、自动更新检查、托盘气泡通知；DXGI 捕获后端
（现在 53~105 ms 的整屏 `BitBlt` 是热键延迟的大头）；M3 只剩「在线 HTTP OCR 引擎」与
「OCR 文本 ↔ 图像位置双向联动」；M2 尾巴（控件级吸附、贴图圆角、拖拽落盘）。
详见 [第 8 节](#8-下一阶段)。

---

## 1. 项目定位与目标

同类工具大多是 Electron 实现，这里改用 **纯 Rust**：

| | Electron 系 | Cuter |
|---|---|---|
| 运行时 | Chromium + Node，150 MB+ | 无运行时，单 exe **7.4 MB** |
| 依赖 | Node + npm + WebView | 只有 cargo |
| 截图精度 | 受 WebView 缩放影响 | 强制 1:1 物理像素 |

**用户明确提出的三条硬约束**（一切取舍以此为准）：

1. 更偏向 Rust 开发（→ 选了 egui/eframe，不用 Tauri/WebView）
2. 高性能
3. 较小的存储占用

另外用户确认过：**Windows 优先，架构预留跨平台**。

---

## 2. 环境事实（新会话必须先确认这些）

| 项 | 值 |
|---|---|
| 工作目录 | `<repo>`（以下路径里的用户名与盘符按本机实际情况替换） |
| 工具链 | Rust **1.98.1**（`rust-toolchain.toml` 只钉了 `stable`），edition **2024**，target `x86_64-pc-windows-msvc`（只有这一个）；`Cargo.toml` 声明的 MSRV 是 **1.88** |
| 链接器 | MSVC 在 `C:\Program Files\Microsoft Visual Studio\2022\Community`，`cl`/`link` **不在 PATH**，但 cargo 能自动找到（已验证 `rustc` 可直接链接） |
| 依赖 crate | egui/eframe 0.36.2、windows 0.62.2、image 0.25、ab_glyph 0.2、arboard 3.6、global-hotkey 0.8、tray-icon 0.24 |
| 显示器 | **单屏 1920x1080 @ 125%**（物理 1920x1080 / 逻辑 1536x864） |
| 系统字体 | `C:\Windows\Fonts\msyh.ttc`（微软雅黑），已自动装载 |
| 保存目录 | `%USERPROFILE%\Pictures\Cuter` |
| 配置文件 | `%APPDATA%\Cuter\config.toml` |
| 系统光标 | **本机取不到真实光标位图**（`GetIconInfo` 的 `hbmColor` 为 NULL）→「包含鼠标指针」本机看不到效果 |

### 2.1 三个必须知道的测量陷阱（我已经各踩过一次）

1. **PowerShell 是 DPI 不感知的。** `GetWindowRect` 返回的是它自己进程 DPI 上下文里的**虚拟化坐标**：
   1920x1080 的窗口会被报告成 1536x864。必须用 `GetDpiForWindow(hwnd)`（见 `tools/probe/WinDpi.cs`）
   把缩放比乘回去，否则会得出「窗口尺寸错了」的错误结论（我为此白改了一轮代码）。
2. **`SetCursorPos` 的坐标也是虚拟化的**：传 (300,250) 实际把光标放到物理 (375,312)。
   验证方法：让 cuter 导出选区，看导出图的尺寸（物理像素），反推。
3. **绝不要用 PowerShell 改写 UTF-8 源码文件。** 我用 `Get-Content -Raw` + `Set-Content` 试图批量插入
   两行代码，把 `annotate.rs` 的中文注释写成了 GBK 乱码，只能 `git checkout` 恢复重做。
   **改代码一律用编辑工具，不要用 shell 重写文件。**

### 2.2 eframe / egui 0.36 的坑（都已解决，改动别退回去）

1. **`show_viewport_deferred` 必须在每一个 pass 里被调用**，否则 egui 会在本 pass 结束时判定
   「it was never used this pass」并**直接销毁窗口**。现象：遮罩/贴图窗口闪一下就消失。
   → 解法见 `CuterApp::declare_viewports()`，在 `logic()` 里每帧重新声明。
2. **隐藏的窗口收不到重绘事件**，所以「先隐藏 → 对齐 → 再显示」是死锁：视口回调永远不跑。
   → 窗口必须创建即可见。
3. **`eframe::App` 的 trait 是 `logic(ctx)` + `ui(ui)`，没有 `update`。** 而且窗口隐藏时
   **只会调用 `logic`**，所以托盘 / 热键 / 截图请求全部必须在 `logic` 里处理。
4. **单位换算链**：`egui-winit` 创建窗口时执行 `LogicalSize(zoom_factor * size)`（再乘显示器缩放），
   而 `ViewportCommand::InnerSize` 执行 `PhysicalSize(pixels_per_point * size)`。
   遮罩把 ppp 固定成 1.0（即 `zoom_factor = 1/系统缩放`），两个因子相消 ⇒ **builder 里直接写物理像素**。
   贴图窗口不固定 ppp ⇒ 目标要先除以系统缩放。见 `GeometryFix` 与 `apply_geometry_fix`。
5. `ColorImage` 的字面量需要 `source_size` 字段（epaint 0.36 新增）。
6. `Context` 没有 `screen_rect()`，是 `viewport_rect()`。
7. **隐藏窗口的重绘请求会被 eframe 推迟 100 ms**：
   `eframe::native::run::INVISIBLE_WINDOW_REPAINT_INTERVAL`，判据是
   `winit::Window::is_visible()`（就是 `IsWindowVisible`）。托盘模式下我们的窗口全是隐藏的，
   所以 `ctx.request_repaint()` 只能把延迟压到 100 ms 这一档，而不是"一帧以内"。
   这条直接决定了热键延迟的下限，见 [第 15 节](#15-商用化第三轮热键延迟)。

### 2.3 本机 rustup 工具链曾经坏过一次（怎么判断、怎么修）

现象：**任何** `cargo` 命令都失败，报

```text
info: syncing channel updates for stable-x86_64-pc-windows-msvc
info: downloading 3 components
info: rolling back changes
error: failed to install component: 'rust-std-x86_64-pc-windows-msvc',
       detected conflict: 'lib\rustlib\x86_64-pc-windows-msvc\lib\liballoc-....rmeta'
```

根因不是网络，而是 rustup 的**元数据丢了**：`~/.rustup/settings.toml` 不见了
（于是"no default toolchain is configured"），工具链目录里也少了
`lib/rustlib/components` / `multirust-channel-manifest.toml` 这些清单文件。
于是 `rust-toolchain.toml` 里声明的 `components` / `targets` 被当成"还没装"，
rustup 去装，又和已存在的文件冲突。

诊断与修法（都不破坏项目文件）：

```powershell
rustup default                       # "no default toolchain is configured" ← 就是它
# 应急：显式指定工具链名，绕过 toolchain 文件的解析
$env:RUSTUP_TOOLCHAIN='stable-x86_64-pc-windows-msvc'; cargo --version
# 连 RUSTUP_TOOLCHAIN 都救不了时，先 `rustup toolchain install stable --force`；
# 仍然报 conflict 就把整个工具链目录挪走再装（装坏了还能挪回来，不要先删）：
Move-Item "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-msvc" `
          "$env:USERPROFILE\.rustup\toolchains-broken\stable-x86_64-pc-windows-msvc"
rustup toolchain install stable      # 6 个组件，几分钟
rustup default stable
```

教训：**报错里出现 "syncing channel updates" 时先怀疑 `~/.rustup` 的元数据，
不要去查网络或代理。** 另外 `.cargo/registry` 的缓存当时也被清了一次，
所以第一轮 `cargo build` 会重新下载全部依赖（多花几分钟），不是代码问题。

---

## 3. 架构（三层单向依赖）

```
crates/
├── cuter-core/     平台无关：不依赖 egui，测试可完全无头
│   ├── geom.rs            整型像素几何（虚拟桌面坐标系允许负坐标）
│   ├── frame.rs           RGBA8 位图：裁剪 / 模糊(滑动窗口盒式) / 马赛克 / blend(直通alpha) / 编解码
│   ├── capture/
│   │   ├── mod.rs             ScreenCapture + WindowDetector 两个 trait、DesktopShot
│   │   ├── windows.rs         GDI BitBlt 屏幕捕获（CreateDIBSection，RAII guard）
│   │   ├── win_windows.rs     顶层窗口枚举（吸附用）
│   │   └── cursor.rs          光标位图提取 + alpha 合成
│   ├── config.rs          TOML 配置（损坏时备份并回退默认值，永不启动失败）
│   ├── scroll.rs          滚动截屏：行签名匹配 + 增量拼接（纯逻辑，可完全无头测试）
│   └── ocr/               OCR 接口与结果模型 + Windows.Media.Ocr 引擎
├── cuter-ui/       只依赖 egui + cuter-core，不碰任何平台 API
│   ├── overlay.rs         全屏冻结遮罩：状态机、框选、吸附、放大镜、快捷键
│   ├── snap.rs            吸附的纯几何（无 Win32，全部可单测）
│   ├── annotate.rs        标注模型 / 文档 / 撤回栈（分区像素快照）
│   ├── flatten.rs         标注的 CPU 栅格化（导出用，不依赖 GPU 回读）
│   ├── toolbar.rs         浮动工具条（图标全部矢量绘制，不引入图标字体）
│   ├── pin.rs             屏幕贴图窗口
│   ├── ocr_panel.rs       OCR 结果面板（左图右文）
│   ├── scroll_panel.rs    滚动截屏控制窗（缩略长图 + 完成/暂停/取消）
│   ├── theme.rs           配色与视觉常量
│   └── fonts.rs           系统中文字体探测（egui 自带字体没有 CJK 字形）
└── cuter-app/      应用层
    ├── app.rs             视口编排、截图流程、设置界面
    ├── save_dialog.rs     原生 IFileSaveDialog 封装（COM）
    ├── desktop.rs         全局热键 / 托盘 / 剪贴板
    └── icon.rs            运行时用代码画出应用图标
```

### 3.1 六条贯穿全局的设计决定

1. **坐标只有一种：物理像素。** 遮罩强制 `pixels_per_point = 1.0`，让 egui 的「点」等于屏幕像素。
2. **标注挂在冻结帧上，不是选区上。** 移动/拉伸选区无需改动标注；导出只需一次裁剪 + 一次平移。
3. **马赛克/模糊的撤回只存受损区域**（4K 下一次撤销几十 KB，而不是克隆 33 MB 整图）。
4. **捕获与界面解耦**：`ScreenCapture` / `WindowDetector` 两个 trait 把平台差异关在 core 里。
5. **吸附是纯几何**：`snap.rs` 不碰 Win32，边界情形全有单测。
6. **设置里的开关必须真的生效。** 没做的能力不给开关，写进路线图。

---

## 4. 已完成的功能与提交

```
8e9c390  chore: clippy 归零（chunks_exact → as_chunks，共 20 处）
e2284ed  feat: 让「配置里写着、代码里没有」的项全部失效或删除
1495fbd  test(m3): 滚动截屏端到端探针，以及「丢帧不丢内容」的逐像素单测
9ef66bb  feat(m3): 滚动截屏界面（控制窗、后台采集线程、配置项）
e589f86  feat(m3): 滚动截屏的拼接算法（行签名匹配 + 增量拼接 + 粘性页头）
ed45237  feat(m3): 接入 Windows 自带 OCR（截图即取词）
25eccb7  fix: 悬停高亮把窗口涂成纯黑
08bf1f9  feat(m2): 框选吸附、区域记忆、真实光标合成、原生另存为，并把设置里的「假开关」全部接上
7b5e858  fix: 遮罩窗口「闪一下就消失」
8c3ea9d  chore: 统一换行符（.gitattributes）
ee52359  feat: Cuter M1 —— 纯 Rust 截图工具的完整核心链路
```

### M1

- Windows GDI 捕获（`CreateDIBSection` + `BitBlt(CAPTUREBLT)`，一次抓完整虚拟桌面）
- Per-Monitor-V2 DPI 感知
- 全屏冻结遮罩：框选、八向控制点、方向键微调、双击确认、右键取消、Ctrl+A 全选
- 放大镜 8× + 取色器（HEX / RGB / 全局坐标）
- 标注：矩形、椭圆、箭头、画笔、荧光笔、马赛克、模糊、文字、序号；撤销/重做
- 标注 CPU 栅格化导出；浮动工具条；贴图窗口
- 托盘 + 全局热键 + 设置界面 + 诊断面板 + 运行时代码生成图标
- 无界面 CLI：`--full` / `--monitor` / `--region` / `--list-monitors`
- 系统中文字体探测；控制台切 UTF-8

### M2

- **框选吸附**：窗口与屏幕的边缘/中线；三种手势各自语义；参考线绘制；悬停高亮；**Alt 临时关闭**；
  工具条里有带勾选态的开关
- **区域记忆**：`Ctrl+R` 复原上次选区（与当前桌面求交，越界明确提示）
- **包含鼠标指针**：真的取光标位图并做 alpha 合成
- **延迟捕获**：`delay_ms` 生效（Waiting → Hiding → Capture 三阶段状态机）
- **原生「另存为」**：`IFileSaveDialog`（用 windows crate 直接调，没引入 rfd）
- **贴图**：鼠标穿透、真正的窗口不透明度、位置夹取到桌面内
- **`cuter --pin`**：把剪贴板图片贴到鼠标位置
- **撤回栈上限** `undo_limit` 生效
- **假开关全部接上**：包含鼠标指针、延迟捕获、吸附、区域记忆、自动保存、保存弹对话框、提示音

### M3

- **OCR**：`Windows.Media.Ocr`（系统自带，零下载零打包离线可用）、结果面板、自动复制、
  `Ctrl+O`；细节见 [第 10 节](#10-ocrm3-已完成)
- **滚动截屏**：`Alt+S` / 工具条按钮进入滚动模式，用户自己滚、程序增量拼接，
  控制窗有实时缩略长图与 完成/暂停/取消；细节见 [第 11 节](#11-滚动截屏m3-已完成)

### 商用化第一轮（配置项审计）

细节见 [第 12 节](#12-商用化第一轮配置项审计--假开关修复) 与
[docs/config-audit.md](config-audit.md)。一句话：**把每个配置字段都对着代码搜了一遍**，
修掉 6 处假开关、删掉 3 个占位字段、clippy 归零。

- **保存质量**：`save.quality` 真的进编码器（`ImageFormat::with_quality`），
  设置界面只在有损格式下显示滑块
- **贴图**：`wheel_zoom` / `zoom_within_window`（只放大内容）/ `shadow`（外部投影）生效；
  右键菜单的「始终置顶」真的改窗口层级；菜单里改过的值写回 `config.pin`
- **贴图窗口的「识别文字」**：接上 OCR（原来是一句「M3 接入」的占位提示）
- **`close_to_tray`**：关闭设置窗口退到托盘而不是退出程序
- **`ocr.paragraph`**：面板原来写死 `plain_text(true)`，现在跟随配置
- **新增设置分区**：「通用」（主题 / 启动即托盘 / 关闭到托盘）、
  「贴图」（8 个贴图参数的默认值）
- **`ThemeKind::Auto`**：真的读 `ctx.system_theme()`，不再是"永远深色"

---

## 5. 怎么验证（照着跑即可）

```powershell
cd <repo>

# 单测（226 个）+ 静态检查
cargo test --workspace
cargo clippy --workspace --all-targets      # 必须零警告（提交前用 -- -D warnings 再确认一次）

# 真机测试（真的截屏 / 真的枚举窗口 / 真的弹保存对话框）
cargo test --workspace -- --ignored

# 端到端
.\target\release\cuter.exe --list-monitors
.\target\release\cuter.exe --region 200,150,400,300 --out target\smoke\x.png
.\target\release\cuter.exe --capture          # 手动框选
```

**已验证过的硬证据（新会话可复现）：**

- 捕获正确性：`captured_pixels_match_gdi_ground_truth` 用屏幕 DC 的 `GetPixel` 逐点比对，7/7 逐字节一致
- 遮罩窗口几何：真实物理尺寸 1920x1080 @ (0,0)，t=5s/t=8s 持续存活
- 贴图像素级：贴图区域 vs 被截取源区域平均差 **5.34**，vs 无关区域 **66.62**，vs 它覆盖的桌面 **26.16**
- 窗口枚举：真实桌面 7 个候选，0.3–1.2 ms

### 5.1 探针工具（脚本化 UI 测试）

`tools/probe/` 下有本次会话积累的 C# 探针，用 `Add-Type -Path` 加载：

| 文件 | 用途 |
|---|---|
| `WinList.cs` | 列出可见顶层窗口（含前台标识），判断屏幕上是什么 |
| `WinDpi.cs` | 按 PID 枚举窗口并**用 `GetDpiForWindow` 还原真实物理尺寸**（绕开陷阱 2.1.1） |
| `WinEnum.cs` | 按 PID 枚举窗口，标题以 `\uXXXX` 转义输出（绕开控制台编码） |
| `CuterInputProbe.cs` | 键鼠注入（含滚轮）+ 窗口查找/摆放 + DPI 坐标系自检 |
| `scroll_e2e.ps1` | **滚动截屏端到端**：记事本 → Alt+S → 框选 → 滚轮 → 完成 → 像素级断言 |
| `overlay_client_probe.ps1` | 只查遮罩的「窗口矩形 vs 客户区原点」，用于定位 1 像素偏移 |
| `tray_close_e2e.ps1` | **关闭到托盘端到端**：真窗口 + 真 `WM_CLOSE`，断言 `close_to_tray` 的两条分支（true → 窗口隐藏但进程存活；false → 进程退出）。会临时改配置并在 `finally` 里还原 |
| `single_instance_e2e.ps1` | **单实例端到端**：跑起第一个实例 → 再跑 `cuter --capture` → 断言它退出且退出码为 0、第一个实例仍然活着、并且**真的弹出了遮罩**（按标题 `Cuter 截图` 找窗口）；最后还有一个对照用例：没有实例在跑时 `--capture` 必须能正常启动 |

典型用法：
```powershell
Add-Type -Path "tools\probe\WinDpi.cs"; Add-Type -Path "tools\probe\CuterInputProbe.cs"
$p = Start-Process .\target\release\cuter.exe -ArgumentList "--capture" -PassThru
foreach($i in 1..30){ Start-Sleep -Milliseconds 300; if([WinDpi]::Dump([uint32]$p.Id,'\u622A\u56FE').Count -gt 0){break} }
[CuterInputProbe]::Drag(300,250,900,650)      # 会真的操作鼠标，务必先确认遮罩已出现
[CuterInputProbe]::CtrlKey(0x53)              # Ctrl+S
```

⚠️ 三个新会话最容易再踩一次的坑（细节见 `tools/probe/README.md`）：

1. **`.ps1` 必须带 UTF-8 BOM**。Windows PowerShell 5.1 对无 BOM 的文件按 ANSI(GBK) 解析，
   中文注释的字节会**连换行符一起吞掉**，把下一行代码并进注释里 —— 症状是"某个变量莫名其妙
   是 `$null`"，且完全不报错。同一个文件在 `pwsh`(7) 下正常，在 `powershell`(5.1) 下出错。
   （本机**只有** PowerShell 5.1，没有 `pwsh` 7 —— 探针脚本请写成纯 ASCII 或加 BOM。）
2. **残留的 `cuter.exe` 会抢走全局热键**：新实例注册不上 `Alt+S`，热键进了旧实例，
   于是"新实例的遮罩永远不出现"。每轮测试前先清场，并用 `trap` 保证收尾。
3. **按 PID 枚举窗口时，必须先排除 `ConsoleWindowClass`**。`cuter.exe` 目前是控制台子系统
   程序，`Start-Process` 会给它一个**可见的、有标题的**控制台窗口（标题就是 exe 路径）。
   对它发 `WM_CLOSE` 关掉的是控制台，几秒后进程被一起带走 —— 看起来就像"程序没有退到托盘"。
   见第 12.6 节，这个假阴性花了整整一轮才定位。

---

## 6. bug：悬停高亮把窗口涂成纯黑（**已修复并提交**：`25eccb7`）

### 6.1 用户报告

「**选中的屏会变黑**」——鼠标移到某个窗口上时，那个窗口变成一整块黑色。

### 6.2 根因（一行代码）

`crates/cuter-ui/src/theme.rs`：

```rust
// 错误写法（修复前）
hover_tint: Color32::from_rgba_premultiplied(14, 14, 16, 255),
```

**`from_rgba_premultiplied` 的第 4 个参数就是 alpha，而 255 表示完全不透明。**
所以这不是「14% 灰的半透明遮罩」，而是**一块不透明的近黑色**。
传给它的 RGB 分量本应是**已预乘**的，这里传的是未预乘的颜色 —— 两者的差别在 alpha=255 时最致命。

触发条件（`overlay.rs` 的绘制第 1b 步）：

```rust
if self.selection.is_none() && self.drag == Drag::None && self.snap_allowed()
    && let Some(cursor) = self.cursor
    && let Some(rect) = self.snap_rects.iter().find(|r| r.contains(cursor)).copied()
{
    chrome.rect_filled(rect, ..., self.theme.hover_tint);   // ← 把整个窗口涂成实心黑
}
```

也就是说：**在还没开始拖拽、只是把鼠标移到某个窗口上时**，该窗口立刻被涂黑。
这正好对应「选中的屏会变黑」。

### 6.3 修复

1. `hover_tint` 改成真正的半透明薄纱：深色主题 `Color32::from_white_alpha(28)`（11% 白），
   浅色主题 `Color32::from_black_alpha(28)`
2. **同一类错误一并清理**：`crosshair` / `snap_line` 原本也是用
   `from_rgba_premultiplied(0,200,255,190)` 这类写法（RGB 分量大于 alpha，违反预乘不变量），
   改成 `from_rgba_unmultiplied(...)`
3. 删掉 `Theme::mask` —— 它是**从未被使用**的死字段，而且注释描述的「盖在桌面上的颜色」
   与实现实际采用的「乘算压暗」是两种不同技术，留着只会误导后来人
4. 加了回归测试 `hover_tint_is_a_translucent_veil`，断言两个主题的 `hover_tint.a()` 都 < 128。
   **这个测试如果早存在，这个 bug 根本不会出现。**

### 6.4 ⚠️ 给下一个会话的重要更正

我在排查过程中曾经测出「一块 1192x592 的纯黑矩形」（见下表的原始数据），并据此怀疑过捕获链路。
**那个矩形其实是我自己的 PowerShell 控制台窗口**被冻进了截图里（控制台窗口约 1239x648 @ (64,64)，
大小位置都对得上），属于**完全正常的现象**。请不要再去追这条线索。

保留下来的、当时测到的真实数据（供参考，勿再误读）：

| 采样点（物理） | 遮罩前 | 遮罩后 | 说明 |
|---|---|---|---|
| (100,200) | 227.3 | 6.0 | 落在控制台窗口里 |
| (100,700) | 240.3 | 107.7 | 正常压暗（≈0.488） |
| (600,400) | 248.3 | 12.0 | 落在控制台窗口里 |
| (600,750) | 229.3 | 229.3 | 选区内，全亮 ✓ |
| (1500,900) | 248.3 | 112.7 | 正常压暗 |

结论：**压暗系数 0.488 是正确的，「黑块」是深色窗口内容**。真正的问题只有 `hover_tint`。

### 6.5 修复后的验证

- 单测：`hover_tint_is_a_translucent_veil`、`guide_lines_are_slightly_transparent` 通过
- 实测（光标悬停在窗口上、遮罩开启）：屏幕平均亮度 87.6，**接近全黑(<30)的像素只占 1.01%**，
  且明亮内容区域落在 113–170（正常压暗）。修复前该区域会是 ~14（不透明黑）
- **无法做干净的 A/B 对照**：本机 ChatGPT 窗口铺满整个屏幕，两帧的「悬停窗口」是同一个，
  差值恒为 0。要真正 A/B 需要一个「不覆盖全屏、且不与其它窗口重叠」的窗口作为目标。

### 6.6 如果用户仍然看到变黑

按顺序排查（都建议用 `CUTER_LOG=debug` + `tools/probe/CuterInputProbe.cs` 脚本化复现）：

1. 在 1b 那段加 `log::debug!` 打出被高亮的 `rect`、`cursor`、以及 `hover_tint` 的最终 alpha —— 
   先确认画的到底是不是那一块
2. 检查 `snap_rects` 里是否存在**明显不该存在**的窗口（例如覆盖全屏的 `Progman`／桌面窗口），
   它们会让高亮覆盖整屏
3. 确认 ppp 确实为 1.0（`ctx.pixels_per_point()`），否则 `rect_filled` 的位置会整体偏移

---

## 10. OCR（M3 已完成）

### 10.1 用什么引擎，为什么

**Windows 自带的 `Windows.Media.Ocr`**（`cuter-core/src/ocr/windows_ocr.rs`）。

理由是本项目的硬约束「存储占用要小」：PP-OCR 的 det+rec+字典加推理运行时是几十 MB 级别，
而系统 OCR **零下载、零打包、完全离线**，中文只要装了语言包就能用。

本机实测：1400×700 的屏幕截图识别耗时 **130 ms**，中英混排都能读出来。

`config.ocr.engine` 保留三个取值：`Disabled` / `Builtin`（默认）/ `Http`（未实现）。

### 10.2 线程模型（改这里之前务必读）

**OCR 必须跑在自建的 MTA 后台线程上。**

WinRT 的 `RecognizeAsync` 返回异步对象，要拿到结果就得阻塞等待。而 winit 的事件循环
线程可能是 STA，在它上面阻塞等待「需要回到同一套间」的异步结果，会与套间消息泵互相
等待而死锁。

所以：`CuterApp::start_ocr` 里 `std::thread::spawn` 一个线程，`WindowsOcr::new` 内部
调用 `ensure_winrt_initialized()`（`CoInitializeEx(MTA)`），结果通过 channel 回传，
再用 `ctx.request_repaint()` 唤醒 UI 线程。

`windows-future` 0.3.2 **没有**阻塞式 `get()`（那是 0.2.x 的 API），所以
`windows_ocr.rs` 自己写了 `wait_for_async`：轮询 `IAsyncInfo::Status` 到终态再
`GetResults()`。5 ms 间隔，代价可忽略，换来完全不依赖套间消息泵。

### 10.3 面板

`cuter-ui/src/ocr_panel.rs`：左图右文。
- 左侧预览上叠文本块，悬停高亮并在图上浮出该行文字，**单击复制该行**；
- 右侧是**可编辑**的识别文本（OCR 必然有错字，校对后复制才好用）；
- 工具栏：复制文本 / 复制并关闭 / 关闭。

尚未做：文本光标位置反向映射回图上（只做了图 → 文本方向）。

### 10.4 怎么验证

```powershell
# 闭包测试：我们自己的栅格化器画字 → OCR 读回，逐字一致
cargo test -p cuter-ui --test ocr_windows -- --nocapture
#   → 原图 692x120，识别到 1 个文本块：Hello Cuter 12345
#   → 中文识别结果："截图工具测试"

# 端到端（脚本化框选 + Ctrl+O，然后看剪贴板）
# 见第 5.1 节的探针用法；实测输出：
#   开始 OCR：1675×875，语言 None
#   OCR 完成：16 行，348 个字符
#   剪贴板长度 493（内容为识别出的中英混排文本）
```

**踩过的坑**：只发 `Ctrl+O` 不会触发——遮罩需要先拿到键盘焦点，而焦点来自窗口的
`ViewportCommand::Focus` 或一次点击。脚本化测试里**先拖拽一次**（既建立选区又拿到焦点）
再发快捷键才可靠。

### 10.5 已知限制

* `OcrWord` 不暴露置信度，所以 `TextBlock::score` 一律填 1.0，
  配置里的 `min_score` 对内置引擎**实际不生效**；
* 只支持系统已安装 OCR 组件的语言，设置界面会列出实际可用的语言标签；
* 单边不得超过 `OcrEngine::MaxImageDimension`（本机 10000），超限会被明确拒绝而不是静默截断。

---

## 11. 滚动截屏（M3 已完成）

### 11.1 交互模型（**不是**程序帮你滚）

`Alt+S`（或设置界面、工具条上的滚动按钮）→ 遮罩进入滚动模式 → 框选 → 回车 →
弹出一个小控制窗，**用户自己在目标窗口里往下滚**，程序定时抓同一块区域并拼接。

选「用户自己滚」而不是「程序发滚轮」的理由：合成滚轮在很多程序里会被忽略或触发平滑
滚动，抓到的帧要么没动、要么动了一半；而且用户滚得比任何自动策略都准。

| 入口 | 位置 |
| --- | --- |
| 全局热键 `Alt+S` | `HotkeyCommand::ScrollCapture` → `CuterApp::request_scroll` |
| 设置界面按钮 / 工具条图标 | 同上；遮罩已开着时**只切模式，不重新截屏**（保留已有框选） |
| 遮罩里回车 | `Overlay::confirm_default()`：滚动模式优先级最高 |

控制窗（`cuter-ui/src/scroll_panel.rs`）显示实时缩略长图、帧数、尺寸与一行状态，
按钮是 完成 / 暂停 / 取消，键盘是 **回车 = 完成、空格 = 暂停、Esc = 取消**。
关掉窗口（X）等价于取消 —— 否则采集线程会在用户已经没有出口的情况下一直跑。

### 11.2 算法（`cuter-core/src/scroll.rs`）

- 逐行亮度签名（**全分辨率**，下标 = 行号），在候选位移上做平均绝对误差比较，取误差最小者；
  平局时取**较小**的位移。`min_offset` 以下算"没滚动"。
- `sticky_prefix_rows` 处理固定页头：顶部若干行在两帧里完全相同，拼接时只保留一次。
- `ScrollStitcher::push` 只追加"新露出来的 d 行"，**匹配始终以上一张被接受的帧为基准** ——
  因此抓帧漏掉几帧只是等价于一次更大的位移，**不会缺行也不会重复**
  （`dropped_frames_do_not_leave_gaps_or_duplicates` 逐像素验证了这一点）。

⚠️ 早期版本为了提速把行也抽稀了（签名下标 = `y / row_step`），那是错的：位移被量化到
`row_step` 的整数倍，**奇数位移根本表达不出来**，实测表现为"真实滚了 60 像素，却按 61 拼"。
现在抽稀只发生在比较循环内部。

### 11.3 控制窗为什么必须放在选区外

控制窗会一直挂在屏幕上；一旦它落在被采集的矩形里，就会被**逐帧抓进长图**，既污染画面
又让位移估计因为这块静止像素而失配。`scroll_control_position()` 依次尝试
右侧 / 左侧 / 上方 / 下方，先夹进显示器工作区再判断是否与选区相交；四个方向都不行
（典型情况是整屏框选）时退到工作区右下角**并明确告知用户**，不假装没问题。

### 11.4 ⚠️ 1 像素的客户区内缩（会误导所有像素级对比）

遮罩窗口虽然是 `with_decorations(false)`，Windows 仍然给了它 **1 像素的非客户区上边**：

```
窗口矩形  : (0,0) 1920x1080      ← GetWindowRect
客户区原点: (0,1) 1920x1080      ← ClientToScreen + GetClientRect
```

egui 全程用**客户区**坐标，所以脚本把光标放在屏幕 `y=170` 再按下，app 日志报的是
`区域 Rect { x: 130, y: 169, .. }`。这不是 bug：

* 选区方框、冻结帧、裁剪三者用的是同一套客户区坐标 ⇒ **用户框到什么就得到什么**；
* 只有"拿屏幕绝对坐标去和选区对比"的外部测量会差 1 行。

排查时靠的是"在候选偏移里找最小偏差"：`delta=1` 处平均偏差 **0.000**、`delta=0` 处 **7.1**，
一眼就能看出是整体错了一行，而不是内容不一致。复现工具：`tools/probe/overlay_client_probe.ps1`。

顺带一个**瞬时现象**（不是 bug，但会让人误判）：遮罩窗口刚创建时物理尺寸有时是
`2400x1350`（= 1920x1080 × 1.25，即 `zoom_factor` 还没被压到 0.8 的那一瞬），
几百毫秒内会被 `GeometryFix` 纠正回 `1920x1080`。用 250ms 轮询去抓刚出现的窗口就会抓到
这个瞬时值。**判断几何请等窗口稳定后再查**（`overlay_client_probe.ps1` 会先等 500ms，
实测输出 `window rect 0,0 1920x1080` + `client origin 0,1`）。

### 11.5 验证

```powershell
# 算法（含丢帧、超出搜索范围、真实截图往返）
cargo test -p cuter-core scroll

# 端到端：真记事本 + 真滚轮 + 真控制窗，然后做像素级断言
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\scroll_e2e.ps1
```

实测输出（`target\e2e\scroll\` 里留有 PNG 与 app 的 stderr）：

```
overlay: 1920x1080 @ (0,0)              dpi=120
scroll panel: (982,169) 431x585         与选区 overlap=False
saved: Cuter_20260927_001356.png = 840x978 (84709 bytes)
deviation(stitched top+inset vs plain capture) = 0.000   ← 首屏与普通截图逐像素一致
deviation(stitched tail vs plain capture)      = 16.892  ← 末尾是真正的新内容
PASS: stitched 840x978 starts with the exact selected content and is 1.63x taller
```

app 侧日志：`滚动截屏开始：区域 Rect { x: 130, y: 169, w: 840, h: 600 }` →
`滚动截屏完成：17 帧，结果 840×1032`。

### 11.6 已知限制

- **只支持向下滚动**；检测到回滚会提示"只支持向下滚动"，不会把往回滚的内容也拼上去；
- 内容整个换掉（切标签页、视频画面）时匹配会失败并提示"没识别出滚动"，不会乱拼；
- 完全周期性的内容（等距重复的横纹）按设计**无法判断位移**，会被报告为"没识别出滚动"；
- 控制窗的实际尺寸会被内容撑到 345x468 点（比请求的 330x430 大），不影响摆放逻辑，
  但"控制窗尺寸 == 配置值"这个假设不成立，别依赖它。

---

## 7. 已知限制（诚实清单）

- **多显示器与混合 DPI 未验证**：本机单屏 125%。检测到混合缩放时程序会提示改用「当前显示器」
- **吸附只到窗口边缘，不到控件**（需要 UI Automation）
- **界面外壳不随系统缩放放大**（1:1 像素与 DPI 缩放冲突，选了前者）
- **本机没有可见光标位图** ⇒「包含鼠标指针」在本机看不到效果（代码路径有合成光标的端到端测试覆盖）
- **「另存为」的确认保存路径未交互验证**（探针只走到取消分支；真窗口弹出已证实）
- **200 个可见顶层窗口时窗口枚举约 12.6 ms**（`DWMWA_EXTENDED_FRAME_BOUNDS` 单次 40–130 µs 是瓶颈）；
  真实桌面 7 个候选时约 1 ms
- **Release 仍带控制台窗口** —— 已修（见 [第 13 节](#13-商用化第二轮交付形态)）：
  双击不再留下黑窗，而 CLI 的等待 / 重定向 / 退出码仍然完好。残留的是
  `CreateProcess` 到 `FreeConsole` 之间极短的闪动，详见 13.1 末尾
- **没有多实例并发的概念**：第二个实例只做请求转交（`--capture` / `--pin` / 显示窗口），
  不提供"同时跑两个互不干扰的实例"。这是刻意的——全局热键本来就是独占资源（见第 13 节）

---

## 8. 下一阶段

M3 的 OCR 与滚动截屏都已完成（见 [第 10 节](#10-ocrm3-已完成) 与
[第 11 节](#11-滚动截屏m3-已完成)），商用化第一到第四轮也做完了
（配置审计 / 交付形态 / 热键延迟 / 稳定性，见 12、13、15、16 节）。
**完整清单在 [docs/roadmap.md](roadmap.md) 的 M5**，这里只列优先级最高的几条：

### 8.1 当前状态（商用化第六轮结束时）

路线图 64 项已完成 / 24 项未完成（`docs/roadmap.md` 里的勾选框）：

| 里程碑 | 状态 |
|---|---|
| M1 截图核心链路 | ✅ 15/15 |
| M2 手感与精度 | ✅ 主体 18/22（差控件级吸附、贴图圆角、拖拽落盘、多屏混合缩放） |
| M3 智能能力 | 🚧 13/18（OCR 与滚动截屏已完成；屏幕翻译、以图搜图未开始） |
| M4 跨平台 | 未开始（Windows 专用，架构上已隔离在 `cuter-core/src/capture/`） |
| M5 商用化 | 🚧 18/33 |

### 8.2 下一步（按优先级）

1. **DXGI 桌面复制后端**（唯一剩下的性能大头）。账已经量清楚，见第 19 节：
   整屏 `BitBlt` 33 ms 就是捕获的全部，预期换后端后落到 10~15 ms，
   即托盘模式热键中位数 113 ms → 约 90 ms。四个已知坑写在路线图 M5-D
2. **代码签名**：需要证书（取决于你），没有就只能在文档里写明 SmartScreen 提示
3. **用户可感知的功能**（挑一个）：贴图拖拽落盘、界面多语言、自动更新检查
4. **多显示器混合缩放**：每屏一个遮罩窗口。**本机单屏，做了也无法验证**——
   需要一块第二显示器或能改分辨率的虚拟机才能给出可信结论
5. **M3 收尾**：OCR 在线 HTTP 引擎、文本光标 ↔ 图上位置双向联动、横向滚动截屏
6. **M4 跨平台**：macOS（`CGDisplayCreateImage`）、Linux（X11 `XShmGetImage`；
   Wayland 需 portal，**无法做冻结遮罩**）


> ⚠️ 上一版这里写着「`Frame::thumbnail` 已有」——**那是错的**。
> `Frame` 只有 `resized` / `scaled`（`crates/cuter-core/src/frame.rs`），
> 名字叫 `thumbnail` 的方法在 `ScrollStitcher` 上（`scroll.rs`）。
> 做截图历史时直接用 `Frame::scaled()` 即可，别去找一个不存在的方法。

> ⚠️ 截图历史落盘的三个现实约束（先想清楚再动手）：缩略图要么存成小 PNG（几百 KB 级，
> 但每次截图多一次编码），要么存原图（用户会惊讶于磁盘占用）；缓存目录必须能一键清空；
> 历史记录里**不能**放剪贴板里的敏感图而不告知用户。

---

## 9. 工程约定与经验

### 9.1 提交前必做

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings   # 零警告是硬指标
cargo build --release                                    # 确认体积（当前 7.57 MB / 7,934,976 字节）
```

### 9.2 代码风格

- 注释与文档注释**一律简体中文**，模块头部写清「为什么这么设计」而不只是「做了什么」
- 每个 `unsafe` 块配 `// SAFETY:` 说明前提
- 平台相关的代码用 `#[cfg(windows)]` 隔离，纯逻辑抽成可单测的函数
- 公开接口写 `//!` / `///`，含单位与坐标系约定

### 9.3 并行子代理的用法（本次很有效）

本次会话用 5 个子代理并行完成了：GDI 后端、CPU 栅格化、光标提取、窗口枚举、原生保存对话框。
有效的模式是：

1. **先由主会话冻结接口**（trait / 函数签名 + 一个能编译的占位实现），再派发
2. 给每个子代理**只允许改一个文件**的硬约束，避免互相踩踏
3. 要求它们**用真实命令验证并粘贴真实输出**，明确禁止「假装验证过」
4. 要求报告「你无法验证的点」——本次每个子代理都诚实地列了出来，价值极高

### 9.4 提交状态

工作区里不应再有未提交的改动；开工前先 `git status` 确认（历史上这里曾留过一处
overlay 的未提交修复）。

### 9.5 踩坑速查

| 现象 | 原因 | 解法 |
|---|---|---|
| 遮罩/贴图窗口闪一下就消失 | `show_viewport_deferred` 只调了一次 | 每帧在 `logic()` 里重新声明 |
| 窗口一直是隐藏的 | 隐藏窗口收不到重绘事件，视口回调不跑 | 创建即可见 |
| 托盘模式下热键失效 | 窗口隐藏时只调用 `logic`，不调用 `ui` | 托盘/热键逻辑放 `logic` |
| 窗口尺寸总是差 1.25 倍 | 混淆了「点」与「物理像素」的换算链 | 见 2.2 第 4 条 |
| 中文日志变乱码 | 控制台代码页是 GBK | `SetConsoleOutputCP(65001)`（已做） |
| 界面里全是方框 | egui 自带字体没有 CJK 字形 | `fonts.rs` 装载系统字体（已做） |
| 源码文件变成乱码 | 用 PowerShell `Set-Content` 改写 UTF-8 文件 | **永远不要这么干** |
| `.ps1` 里某个变量莫名是 `$null` | 无 BOM 的 UTF-8 被 PS 5.1 当 GBK，换行被吞进注释 | `.ps1` 存成带 UTF-8 BOM（见第 5.1 节） |
| 按了 `Alt+S` 遮罩却不出现 | 残留的旧 `cuter.exe` 占着热键 | 测试前 `Stop-Process cuter` 清场 |
| 测出的滚轮位移恒为 0 | 注入的滚轮只送前台窗口，CLI 启动把控制台拉到前台 | 每次发滚轮前重新 `Activate` |
| 测试"通过"但设置全没生效 | TOML 里 `dir = "E:\x"` 转义非法 → app 回退默认值 | 用单引号字面量 `dir = 'E:\x'`，并断言 `config.toml.bak` 未出现 |
| 设置项改了没反应 | 字段有 `Default`、界面有控件，但**没有任何代码读它** | 按 [config-audit.md](config-audit.md) 的流程查一遍；新字段必须同时补表 |
| `cargo clippy --fix` 报 "errors present after applying fixes" | lint 建议在 `chunks_exact_mut` 上把 `&mut` 变成 `&`，改完编译不过 | 该 crate 的修复被整体回滚，按提示手工改（见第 12 节） |
| PowerShell 里 `git commit -F - <<'EOF'` 报语法错 | PowerShell 没有 heredoc | 先把提交信息写到文件（如 `target/commit-msg.txt`），再 `git commit -F` |
| `cargo check \| Select-String ...` 明明没输出却 exit 1 | 原生命令写 stderr 会被 PS 记成 NativeCommandError | 看 `$LASTEXITCODE` / 输出尾部，别被退出码误导 |
| 按文档设 `CUTER_LOG=debug` 却没有 debug 日志 | `env_logger::Env::default()` 只认 `RUST_LOG` | 已修：`init_logging` 现在两个变量都认（main.rs） |
| 给 PID 发 `WM_CLOSE` 后"程序没有退到托盘" | 关掉的是那个进程的**控制台窗口**，不是主窗口 | 按类名排除 `ConsoleWindowClass`（见 12.6） |
| `cargo` 任何命令都报 "syncing channel updates" + "detected conflict" | `~/.rustup` 的元数据丢了，不是网络问题 | 见 [2.3](#23-本机-rustup-工具链曾经坏过一次怎么判断怎么修) |
| 用秒表量"按热键 → 遮罩可见"恒 ≥530 ms | `CuterInputProbe::AltKey` 内部有 50+50+30+400 ms 的 `Sleep` | 用 `SendInput` 一次投递（`perf_probe.ps1`）；`INPUT` 在 x64 上必须 40 字节，否则**静默失败** |
| 热键延迟"改了没变快" | 隐藏窗口的重绘请求被 eframe 压到 100 ms 一次 | 用 `WM_PAINT` 唤醒，见 [第 15 节](#15-商用化第三轮热键延迟) |
| 截图里出现设置窗口的残影 | 按帧数/"样式位变假"当时间用，桌面合成器还没换帧 | 轮询 `IsWindowVisible` + `DwmFlush` + 150 ms 余量（15.4） |
| 遮罩里框选存出来的图尺寸不对（如 735x524） | 遮罩刚创建时是 `2400x1350` 的瞬时状态，拖拽被按错误的缩放换算 | 等窗口尺寸等于屏幕尺寸再拖（`hide_own_windows_e2e.ps1`） |
| 探针报"设置改了没生效"，但手工测又正常 | 探针**假定应用几秒内一定起得来**；本机冷启动实测 0.4~15 s（Defender/负载），应用还没跑到那步就被杀了 | 等到"应用真的跑了"再断言（stderr 出现内容或日志签名变化），窗口放宽到 60 s（`autostart_e2e.ps1`） |
| 探针连续几个用例一起失败，原因看不出来 | 上一个用例的进程没退干净，下一次启动被单实例守卫**转交**掉了，新进程什么都没做就退出 | 启动前断言没有别的实例、杀进程后等到真的没有进程（同文件） |
| 仓库根目录冒出 `cuter-*-config-*.toml` | `$env:TEMP` 取不到时 `Join-Path` 返回**相对路径**，备份落到当前目录（探针 `Set-Location` 到仓库根） | 用 `[System.IO.Path]::GetTempPath()` + 绝对路径断言；`.gitignore` 再兜一层 |
| 缓存写进了 `%APPDATA%`（漫游） | `dirs::data_dir()` 在 Windows 上是 **Roaming**；缓存应当用 `dirs::data_local_dir()` | `Config::data_dir()` 已改用 `data_local_dir()`，并有单测钉住（HANDOFF 18.3） |

---

> 想按「遇到问题怎么查」而不是按轮次读，看 `docs/MAINTAINER.md`（维护者手册）。

## 12. 商用化第一轮：配置项审计 + 假开关修复

目标不是加功能，而是**把"说了要做"和"实际做了"对齐**。这一轮的输入是
[docs/roadmap.md](roadmap.md) 的 M5 档和项目的硬约束
「设置里的开关必须真的生效」。

### 12.1 审计怎么做的（方法可以复用）

不要靠读 `config.rs` 猜。**用搜的**：把每个字段名（`general.theme`、`pin.shadow`、
`save.quality` …）在整个 workspace 里搜一遍，看有没有 `config.rs` 之外的命中。
本轮搜出来的问题分三类：

1. **完全没人读**：`general.close_to_tray`、`general.language`、`general.check_update`、
   `general.show_tray_notifications`、`save.quality`
2. **读了但被写死覆盖**：`ocr.paragraph`（面板里写死 `plain_text(true)`）、
   `pin.wheel_zoom` / `zoom_within_window` / `shadow`（`Pin::ui` 里根本没看这几个开关）
3. **读了但没接完整**：`pin.always_on_top`（菜单里的勾选框不发任何动作）、
   贴图窗口的 OCR 按钮（只显示一句占位提示）

全量对照表在 [docs/config-audit.md](config-audit.md)，**加字段时必须补那一行**。

### 12.2 三个可以复用的实现模式

- **配置出口统一覆盖，而不是在入口处传参**：`SaveConfig::image_format()` 里调用
  `ImageFormat::with_quality(self.quality)`，比给每个调用点加参数更不容易漏。
  配一条「真的编码一遍比对字节数」的测试——只断言字段相等是抓不到假开关的。
- **视口闭包拿不到 `&mut self`，就把改动排队回主线**：贴图右键菜单里的
  不透明度 / 穿透 / 置顶通过 `Shared<Vec<PinPrefPatch>>` 回到 `logic()` 里
  统一写配置（`CuterApp::apply_pin_prefs`）。同理，贴图窗口的 OCR 请求走
  `pin_ocr_requests` 队列。
- **窗口层级 / 鼠标穿透这类视口属性，让 `ViewportBuilder::patch` 去下发**：
  每帧从状态里取当前值重新声明 builder，它只在取值变化时发命令，
  于是菜单里一改，下一帧就自然生效，不用自己记「上次发过什么」。

### 12.3 `close_to_tray` 的正确做法（egui/eframe 0.36）

主窗口的关闭按钮不是"隐藏"，是**销毁根视口 = 退出程序**。
想让 X 变成"退到托盘"，必须在 `logic()` 里检查
`ctx.input(|i| i.viewport().close_requested())`，然后回
`ctx.send_viewport_cmd(ViewportCommand::CancelClose)` 再 `Visible(false)`。

- `ViewportCommand::CancelClose` 是**单元变体**（没有参数），
  在 `egui-0.36.2/src/viewport.rs` 里；`eframe` 的 `epi.rs` 文档明确写了这条路径
- 托盘菜单里的「退出」走 `self.quit`，必须在判断**之前**放行，
  否则"退出"也会被吃掉、程序永远关不掉
- 隐藏之后 `eframe` 只调 `logic`（见 2.2 第 3 条），所以托盘与热键仍然有效

### 12.4 clippy 归零（rustc 1.98 的新 lint）

`clippy::chunks_exact_to_as_chunks` 在 1.98 变成默认警告，全仓 20 处。
`cargo clippy --fix` 只修好了 `cuter-ui` / `cuter-app` 的 4 处，
`cuter-core` 的 16 处因为**建议本身不能编译**而整体回滚：

- 只读：`data.chunks_exact(4)` → `data.as_chunks::<4>().0.iter()`
- 要改像素：`data.chunks_exact_mut(4)` → `data.as_chunks_mut::<4>().0`（元素是 `&mut [u8; 4]`）
- 有一处 `pixels.chunks_exact_mut(4).zip(pattern)`，自动修复给的是
  `as_chunks_mut::<4>().0.iter()`（丢掉了 `mut`），必须手写成 `.iter_mut()`

`as_chunks` 自 **Rust 1.88** 稳定，与 `Cargo.toml` 里 `rust-version = "1.88"` 一致
（clippy 的 msrv 感知也是据此给出建议的）。

### 12.5 验证到什么程度

| 改动 | 证据 |
|---|---|
| `save.quality` | 单测（`image_format` 带质量、`save_as` 真按格式编码、低质量字节更少）+ **端到端**：把配置改成 `quality=10 / 95`，对同一块 1200×800 区域跑两次 `--out q.jpg`，得到 **43,106 vs 181,956 字节** |
| `Frame::save_as` | 单测：文件名叫 `.png` 也要按传入的 JPEG 编码（用 SOI 魔数断言） |
| `ocr.paragraph` | 单测覆盖 `plain_text(true/false)` 两条路径；连线为纯参数传递 |
| 贴图 `wheel_zoom` / `zoom_within_window` / `shadow` | 5 条纯几何单测（窗口尺寸、内容矩形、留白、窗口内缩放居中且溢出、中键复位） |
| `close_to_tray` | **端到端**：`tools/probe/tray_close_e2e.ps1`，真窗口 + 真 `WM_CLOSE`，两条分支都断言（见 12.6） |
| 贴图投影的"好不好看" | ❌ 没有做真人目视确认（只有几何与 alpha 的正确性） |
| 窗口层级切换（取消置顶） | ❌ 没有脚本化验证（`WindowLevel::Normal` 由 egui 的 `patch` 下发） |

### 12.6 一次假阴性：别把控制台窗口当成主窗口

`tray_close_e2e.ps1` 第一版是**失败**的：`close_to_tray = true` 时进程照样退出了。
但手工跑同样的步骤又"通过"。差别最后落到**枚举到的是哪个窗口**上：

```
=== cuter 进程的顶层窗口 ===
2361562 visible=True  class=Window Class              title=Cuter          ← eframe 的主窗口
2492676 visible=True  class=Winit Thread Event Target title=
2164072 visible=True  class=ConsoleWindowClass        title=E:\...\cuter.exe  ← 控制台！
984428  visible=False class=tray_icon_app             title=
1377550 visible=False class=global_hotkey_app         title=
```

`cuter.exe` 是**控制台子系统**程序，`Start-Process` 会给它一个可见、有标题的控制台窗口。
探针"可见 + 有标题"的过滤条件同时命中了它和主窗口，`EnumWindows` 先返回了控制台窗口，
于是 `WM_CLOSE` 关掉的是控制台；附着的进程会在几秒的宽限期后被系统一起结束 ——
**现象与"程序没有退到托盘"完全一样**。

修法：按类名排除 `ConsoleWindowClass`（`TitledWindows` 里那段注释写明了原因）。
顺便也解释了另一个现象：手工测试"通过"是因为 `Start-Sleep 3` 早于系统的 5 秒宽限期。

教训可以推广成一句话：**按 PID 找窗口时，先把这个进程到底有哪些窗口打出来看一眼**，
不要假设"可见 + 有标题"就等于"我要的那个窗口"。

### 12.7 遗留

- 上面 12.5 表里标 ❌ 的两项
- `pin.shadow` 的投影是**分层圆角矩形**（5 层，每层 alpha 12），不是高斯阴影；
  在浅色桌面上够用，如果嫌不够柔和，把 `SHADOW_LAYERS` 调大即可（代价只是几笔填充）

---

## 13. 商用化第二轮：交付形态

这一轮解决两个"用户第一眼就会下结论"的问题：**双击之后会不会留下一个黑框**、
**双击第二次会不会变成一个什么都做不了的进程**。

### 13.1 控制台窗口：为什么没改成 GUI 子系统

最直觉的做法是 `#![windows_subsystem = "windows"]`，但它会**打断无界面用法**：

| | 控制台子系统（现在） | GUI 子系统 |
|---|---|---|
| `cuter.exe --full --out a.png` 在 cmd / PowerShell 里 | shell **等它跑完** | shell 立刻返回，脚本竞态 |
| `> out.txt` 重定向 | 有效 | 拿不到（除非再 `AttachConsole`） |
| 退出码 | 能传递 | 能传递 |
| 双击 | 会有一个控制台窗口 | 干净 |

`tools/probe/*.ps1` 整条验证链路都依赖前两条（比如 `scroll_e2e.ps1` 会先
`cuter.exe --region ... --out reference.png` 再断言文件存在），所以**不能**简单换子系统。

采用的方案：**保留控制台子系统，启动第一件事判断这个控制台是不是"只属于我们"**。

```rust
let count = GetConsoleProcessList(&mut owners);   // 共享此控制台的进程数
if count == 1 { let _ = FreeConsole(); }          // 双击 / Start-Process：丢掉
                                                  // >= 2：是 cmd / PowerShell 的，保留
```

- 返回 0（调用失败）时什么都不做：宁可多一个窗口，也不能把 CLI 输出弄丢
- 实测：从 shell 直接跑 `--list-monitors` 输出正常、`> file` 有内容、
  而 `Start-Process`（等价于双击）在 200ms / 1s / 3s 三次采样里控制台窗口数都是 **0**
- 副作用（好的那种）：`tools/probe` 里"按 PID 枚举窗口会先命中控制台窗口"的坑
  也随之消失（见 12.6）

**残留**：控制台窗口是 CSRSS 在 `CreateProcess` 时创建的，我们只能在拿到 CPU 之后
才释放它，因此理论上仍有极短的闪动。要做到完全无闪，只能拆成两个二进制
（GUI 的 `cuter.exe` + 控制台子系统的 `cuter-cli.exe` 转发 stdio），
已记在路线图 M5-A 里，目前判断不值得为它多背一个 exe。

### 13.2 单实例：不是"禁止第二个"，而是"把请求交过去"

全局热键是进程独占的，第二个实例注册不上它 —— 用户看到的是「按 `Alt+S` 没反应」。
但简单地"发现已有实例就退出"是**更差的体验**：用户双击图标，什么都不发生。

所以做成**请求转交**：

```text
第一个实例：CreateMutexW(Local\cuter-single-<user>)  ← 拿到所有权 = 我是唯一实例
            CreateEventW(Local\cuter-req-show-<user>)     ┐
            CreateEventW(Local\cuter-req-capture-<user>)  ├ 三个自动重置事件
            CreateEventW(Local\cuter-req-pin-<user>)      ┘ 在 logic() 里非阻塞轮询
后来的实例：互斥体已存在 → OpenEventW + SetEvent 对应的那个 → 自己退出(0)
```

对应关系：双击图标 → `Show`（把设置窗口拿出来并抢焦点）；`cuter --capture` → `Capture`；
`cuter --pin` → `Pin`。

几个刻意的取舍：

- **用事件而不是命名管道**：管道要多开 `Win32_System_Pipes`（还得连带
  `Win32_System_IO` / `Win32_Storage_FileSystem`）三个 feature、一个后台线程，
  以及 `ConnectNamedPipe` 的阻塞语义；事件只用已经在用的 `Win32_System_Threading`，
  三种请求一个事件一个语义，反而更清楚。
- **`ERROR_ALREADY_EXISTS` 不等于"有人在跑"**：上一个实例崩溃后互斥体对象还在。
  所以拿到"已存在"之后要再 `WaitForSingleObject(mutex, 0)`，
  `WAIT_OBJECT_0` / `WAIT_ABANDONED`（持有者没释放就死了）都算"其实只有我们"，接管。
- **转发要重试 2 秒**：第二个实例可能在第一个刚建完互斥体、还没建事件的时候启动。
- **无界面子命令不参与**：`--full` / `--region` / `--list-monitors` 本来就可以并行，
  挡住它们只会让脚本莫名其妙地失败。守卫只在 `run_gui` 里。
- **守卫必须活到窗口关闭**：它被放进 `LaunchOptions → CuterApp`，
  绝不能"用完就 drop"，否则互斥体提前释放，第二个实例又能挤进来抢热键。
- **测试用独立命名对象**：`Guard::acquire_with(tag)` 让单测用自己的 tag，
  否则测试会和用户桌面上正在跑的 Cuter 抢同一个互斥体（那会随机失败）。

### 13.3 验证

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\single_instance_e2e.ps1
```

实测输出：

```
PASS: no instance running: `cuter` starts and keeps running
PASS: second instance exited instead of lingering
PASS: second instance exit code = 0 (expected 0)
PASS: first instance is still alive after the handover
PASS: running instance opened its overlay (1967786 1536x864 title='Cuter 截图')
PASS: with no instance running, `--capture` starts a full instance (guard does not block it)
RESULT: PASS - single instance guard + request handover verified
```

最后一条是**对照用例**：守卫不能把合法的新启动也挡掉。
另外 4 条单测覆盖了命名规则、tag 过滤、以及"转发真的能送达且只触发一次"
（`forward` 走的是独立的 `OpenEventW`，和守卫里的句柄不是同一个，
所以这条闭环测试是有意义的）。

### 13.4 踩到的构建坑

`CreateMutexW` / `CreateEventW` 在 windows crate 里被 **`#[cfg(feature = "Win32_Security")]`**
挡着——它们的签名里带 `SECURITY_ATTRIBUTES`（可空）。所以要在
`Cargo.toml` 的 `windows` features 里加 `Win32_Security`，否则报
"no `CreateMutexW` in `Win32::System::Threading`"，而源码里明明有这个名字（只是被 cfg 掉了）。

### 13.5 一次自己打自己脸的审计：`start_in_tray`

第一轮的 [config-audit.md](config-audit.md) 把 `start_in_tray` 标成 **"生效"**，
理由是"`main.rs` 里有 `with_visible(!start_in_tray …)`，`app.rs::restore_control` 会恢复"——
**那是读代码读出来的结论，不是量出来的。**

第二轮写单实例探针时，顺手断言了一句"托盘模式启动后不该有可见窗口"，结果是
**FAIL：`vis=True class=Window Class title=Cuter`**，t=1s / 2s / 4s 都是如此。
也就是说 `start_in_tray = true`（默认值！）**从来没生效过**：双击之后设置窗口照样弹在脸上。

根因在 eframe 里（`native/epi_integration.rs::post_rendering`）：

```rust
if core::mem::take(&mut self.is_first_frame) {
    // We keep hidden until we've painted something.
    window.set_visible(true);
}
```

**它画完第一帧就无条件把根窗口显示出来**，`ViewportBuilder::with_visible(false)` 直接被覆盖。
所以"启动即隐藏"只能在 `logic()` 里明确下发 `Visible(false)`，而且**必须补几帧**——
第一帧里下发的命令会被同一帧稍后的 `post_rendering` 盖掉。

修法与边界：

- `LaunchOptions::start_hidden`（= `start_in_tray || --capture`）→
  `CuterApp::start_hidden_frames = Some(3)`，`enforce_start_hidden` 每帧开头下发一次 `Visible(false)`；
- 一旦**用户明确要窗口**（托盘「设置…」、或另一个实例转达 `Show`）就立刻清空这个计数，
  绝不和用户抢窗口；
- 上一次截图结束后的 `restore_control` 也会清空它（否则 `--capture` 收工后窗口仍被按着）。

**结论写进规矩**：`config-audit.md` 的检查清单加了第 5 步——静态审计只能证明
"有人读这个字段"，行为对不对必须用真实进程量一次。这一条比这一轮的代码重要。

### 13.6 第二轮的验证清单

| 改动 | 证据 |
|---|---|
| 控制台黑窗 | `--list-monitors` 输出正常/exit=0、`> file` 有内容、`Start-Process` 后 200ms 起 `ConsoleWindowClass` 计数为 0 |
| 单实例转交 | `single_instance_e2e.ps1` 9 条断言全绿（第二条启动退出码 0 + stdout 有转交提示 + 第一个实例真的弹出设置窗口/遮罩 + 对照用例）；**加 `-Exe .\target\release\cuter.exe` 在真正要发布的产物上又跑了一遍，同样全绿** |
| 单实例纯逻辑 | 4 条单测：命名对象规则、tag 过滤、转发送达且只触发一次、无监听时如实失败 |
| `start_in_tray` | 同一条探针的前两条断言（可见窗口数 = 0），并由"第二次启动能弹出窗口"反证没有把窗口永久按住 |

---

## 14. 性能与内存：从"形容词"变成"数字"

项目的卖点是「毫秒级启动、小体积、内存小」。**这类说法不测就是错的**——
README 里原本写着「内存十几 MB 起」，实测是 **82 MB 工作集**。这一节把数字、量法、
以及为什么压不下去都记下来。

### 14.1 复现方法

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\perf_probe.ps1
# 也可以指定产物：-Exe .\target\release\cuter.exe
```

它量四类东西：托盘常驻内存、打开设置窗口后的内存、冻结遮罩打开时的峰值内存、
以及两条端到端延迟（`--full` 落盘、冷启动到遮罩可见）。

两个方法论上的坑，脚本里都做了处理：

1. **先逼出"一次性惰性开销"再采样**。托盘模式窗口是隐藏的，`ui()` 不跑，
   系统的 18.8 MB 中文字体也就**不会被解析**。第一次采样因此可能只有 48 MB，
   6 秒后变成 82 MB —— 看起来像"空闲时内存涨了 33 MB"，其实是字体刚被解析。
   现在脚本会先让第一个实例打开一次设置窗口，再开始计时。
2. **第一次 `--full` 可能慢到 6.5 秒**（本机实测：136 / 141 / **6580** ms）。
   那是 Defender 实时扫描 + 冷文件缓存，不是代码问题。所以脚本跑三次并报
   min / median / max —— 只报一个数就会被这种噪声带偏。

### 14.2 实测数字

本机：单屏 1920×1080 @125%，Intel UHD 630，`--release` 构建（7,934,976 字节）。

| 状态 | 工作集 | 私有 | 句柄 | 线程 |
|---|---|---|---|---|
| 托盘常驻（隐藏窗口） | **82.7 MB** | 52.5 MB | 293 | 10 |
| 托盘常驻 + 12 秒后 | 81.1 MB | 50.4 MB | 293 | 10 |
| 设置窗口打开 | 81.2 MB | 50.4 MB | 293 | 10 |
| 冻结遮罩打开（峰值） | **140.0 MB** | 75.3 MB | 301 | 10 |

- **空闲 6 秒内内存增长：-1.6 MB**（不涨，没有明显泄漏）
- `--full`（进程启动 → PNG 落盘）：min **136 ms** / median **141 ms** / max 6580 ms（见 14.1 第 2 点）
- 冷启动 → 冻结遮罩可见：**~486 ms**
- 用第二个进程 `--capture` 唤起遮罩：467 ms —— **这个数包含第二个进程自己的启动**，
  不是热键延迟（见 14.4）

参考：同一台机器上 `notepad.exe` 是 21.5 MB 工作集 / 5.1 MB 私有。

### 14.3 发现：18.8 MB 的字体被存了两份

第一次量到常驻 **119.5 MB 工作集 / 89.9 MB 私有**，比预期高一截。查下来是
`fonts::install_into_egui` 里这一行：

```rust
let data = egui::FontData::from_owned(font.bytes.as_ref().clone());   // 再拷一份！
```

epaint 的 `FontData` 内部是 `Cow<'static, [u8]>`，`from_owned` 会把整个字体文件**再拷一份**；
而 `LoadedFont` 自己还持有一份（导出时 CPU 栅格化要用）。微软雅黑 `msyh.ttc` 是 18.8 MB，
于是进程里常年躺着两份。

改法：`LoadedFont.bytes` 改成 `&'static [u8]`（装载时 `Box::leak` 一次），
egui 那边直接构造 `FontData { font: Cow::Borrowed(...), .. }`，
**egui 与导出共用同一份字节**。

| | 改前 | 改后 |
|---|---|---|
| 托盘常驻 | 119.5 MB WS / 89.9 MB 私有 | **82.7 MB / 52.5 MB** |

**直降 37 MB。** 代价写进了 `LoadedFont` 的文档注释：在设置里换字体会让旧字体文件的
字节留在进程里（换字体是低频操作，一次最多多占一个字体文件的大小）。

### 14.4 还没做的（都在路线图 M5-D）

- 热键延迟**已经在第三轮里量出来并压过一轮**，见
  [第 15 节](#15-商用化第三轮热键延迟)：托盘模式下 `Alt+C → 遮罩可见` 现在是
  **min 79 / median 113 / max 188 ms**（5 次），其中最不可控的那一段（唤醒）只剩
  **2~13 ms**，剩下的主要是 GDI 捕获本身（53~105 ms，随机器负载浮动）。想再往下压
  就得换捕获后端（DXGI 桌面复制），记在 M5-D。
- 托盘模式按需解析字体（不显示窗口就不解析）、遮罩关闭后主动释放纹理与 `Frame`
- 句柄回归：连续 200 次截图，句柄数不得增长（现在常驻 293）

---

## 15. 商用化第三轮：热键延迟

路线图 M5-D 里原本写着「把『按热键 → 遮罩可见』压到 150 ms 以内」，而当时连数字都没有。
这一轮先把它**量出来**，再按数据改。过程中查出一个会把自己的窗口截进图里的真 bug。

### 15.1 先说结论（本机、单屏 1920×1080 @125%、release 产物）

```text
Alt+C -> 遮罩可见（托盘模式，屏幕上没有我们自己的窗口）：
    min 79 / median 113 / max 188 ms（5 次）

同一份日志里把它拆开（`截图链路[...]：+N ms`）：
    [唤醒到处理]   2~13 ms    ← 热键线程的事件处理器 → UI 线程真的跑起来
    [进入捕获]     +0 ms      ← 请求一被读到就同帧开始捕获
    [捕获完成]     +53~105 ms ← GDI BitBlt 整屏 + 合成光标（这一段随机器负载波动，
                                  max 188 ms 就是这么来的）
    （剩下的 ~20~45 ms 是 eframe 建遮罩窗口 + GL 上下文）
```

两个结论：

* **"多久才被读到"这一段从 40~100 ms 压到了 2~13 ms**（改法见 15.3）；
* 现在的大头是**捕获本身**：一次整屏 GDI `BitBlt` + 8 MB 位图搬运要 53~105 ms，
  而且随机器负载上下浮动。想稳定压到 100 ms 以内得换捕获后端（DXGI 桌面复制），
  记在路线图 M5-D。

另外，**屏幕上真的有我们自己的窗口时**（设置窗口开着 / 有贴图），这一次要多等
约 200 ms 才能抓——那是"等窗口真的从屏幕上消失"的代价，见 15.4。

### 15.2 两个会把"测量本身"搞错的坑（都踩过）

1. **拿 `keybd_event` 量延迟是错的。** 原来的 `CuterInputProbe::AltKey` 在按键前后有
   50+50+30+400 ms 的 `Sleep`（给人眼观察留的），套在它外面的秒表量到的是**脚本自己的
   睡眠**，恒 ≥530 ms。现在改用 `SendInput` 一次性投递 4 个键盘事件、零睡眠。
2. **`INPUT` 结构体在 x64 上必须是 40 字节。** 联合体里只放 `KEYBDINPUT`（24 字节）时
   `Marshal.SizeOf` 得到 32，`SendInput` 会以 `ERROR_INVALID_PARAMETER` **静默失败**：
   现象是五次测量全部 `FAIL (no overlay within 10 s)`，看起来像 app 的热键坏了。
   联合体里必须放进最大的成员 `MOUSEINPUT`；`perf_probe.ps1` 现在启动时自检这个尺寸。

### 15.3 改了什么

1. **热键 / 托盘走事件处理器**（`desktop.rs`）：`global-hotkey` / `tray-icon` 的
   `set_event_handler` 在事件到达的瞬间就在自己的线程上回调，我们立刻把 UI 唤醒，
   不用等 `logic()` 里那 120 ms 的兜底轮询。代价是设了 handler 之后 `receiver()`
   收不到事件，所以自己建一条 `mpsc` 队列。
2. **`WM_PAINT` 唤醒**（`desktop::UiWake`）：`ctx.request_repaint()` 对**隐藏窗口**是
   没用的——eframe 会把它的重绘请求统一推迟到 `now + 100 ms`
   （`INVISIBLE_WINDOW_REPAINT_INTERVAL`）。所以除了 `request_repaint()`，还由热键线程
   给根窗口**投一条 `WM_PAINT`**：`winit` 的 `WM_PAINT` 分支是**直接派发
   `RedrawRequested`**（不是"请求重绘"），`eframe` 收到就直接 `run_ui_and_paint`，
   于是绕开了那条节流。句柄从 `eframe::Frame`（实现了 `HasWindowHandle`）取；
   拿不到就静默退回普通重绘。
3. **托盘模式不再无谓地"等两帧"**（`app.rs::begin_hiding_or_capture`）：
   托盘模式下窗口本来就是隐藏的，没有任何我们自己的像素会被截进去，
   而"等两帧"在隐藏窗口上一帧就是 100 ms。
4. **`poll_activation` 提到 `advance_request` 之前**：另一个实例转达过来的请求
   （`cuter --capture` / 双击图标）原来在 pass 末尾才登记，整整错过一帧；
   实测日志 `收到请求 → 无需等帧` 是 **+104 ms**，现在同帧。
5. **等"自己的窗口真的消失"改成问操作系统**，不是猜时长（见 15.4）。

### 15.4 ⚠️ 这一轮查出的真 bug：设置窗口会被截进图里

原来这里只有一句 `self.pending = Some(PendingCapture::Hiding(2))`（"再等两帧"）。
第 3 条把它改成"屏幕上真有我们自己的窗口时才等"，于是要**判断**有没有——第一版写成：

```rust
self.hide_all_windows(ctx);          // ← 它会把 control_visible 清成 false
if self.needs_hide_delay() { ... }   // ← 于是这里永远是 false，从来不等
```

**先隐藏、再问"需不需要等"**，判据自己把自己清掉了。后果是：设置窗口明明在屏幕上，
程序直接抓图，把它一起冻进了截图——一个"用户一看就发现"的 bug。

修法是先判断再隐藏。但紧接着第二个问题来了：**等多久才够？**

* 老代码是"等两帧"，而隐藏窗口一帧 100 ms ⇒ 实际等了 200~300 ms，**够长掩盖了真相**；
* 用 `WM_PAINT` 把帧加速之后，三帧只要 3 ms，`tools/probe/hide_own_windows_e2e.ps1`
  立刻报 `dev(saved, reference) = 16.951`（正常是 **0.000**）——窗口的残影还在画面里。

实测（`target/tmp/hide_timeline.ps1` 那次的输出，已删）给出的时间线：

```text
   0 ms  Alt+C 注入，设置窗口 visible=True
 114 ms  设置窗口 visible=False          ← 它才真的从屏幕上消失
 293 ms  遮罩窗口出现
```

**"下发隐藏"到"真的不见"隔了约 110 ms**：`ViewportCommand::Visible(false)` 要走完
eframe → `egui_winit::process_viewport_commands` → `winit::Window::set_visible`
（Windows 上还要经 `EventLoopThreadExecutor`）→ `ShowWindow`。这个数字在本机是 110 ms
量级，**在任何别的机器上都可能不一样**。

而且**"样式位不可见了"并不等于"屏幕上没了"**：有一版改成"轮询 `IsWindowVisible`，
一变假就抓"，结果存下来的图与"我们的窗口在场"的基准图仍然只差 26（干净时应该是 0）——
`ShowWindow(SW_HIDE)` 之后还要等**桌面合成器**把那一帧换掉。

所以最终写法是两段（`app.rs::advance_request` 的 `Hiding` 分支）：

```rust
// 第一段：每 ~5 ms 一问，问的是 IsWindowVisible(根窗口)。判据来自操作系统，不是猜时长。
// 一变假就 DwmFlush() 推一次合成器，开始计时。
// 第二段：再留 150 ms 余量等屏幕真的换帧；最长 900 ms，超时就如实警告后照抓。
let root_gone = !self.wake.root_is_visible();
```

**代价**：屏幕上真有我们自己的窗口时，这一次截图要多花约 200 ms
（实测 `--capture` 端到端 386 ms，其中"隐藏 → 进入捕获"是 202 ms）。
这是**正确性**，不是性能问题：不等的后果是截图里出现自己的窗口。
想要更快只能绕开 eframe 自己 `ShowWindow(hwnd, SW_HIDE)`，
那要额外承担 winit 内部状态不同步的风险，本轮没做。

### 15.5 这一轮新增的回归测试

| 测试 | 断言 |
|---|---|
| `tools/probe/hide_own_windows_e2e.ps1` | 记事本客户区铺满设置窗口 → 先证明"这块区域里确实有我们的窗口"（dev **160~200**），再 Alt+C 框选同一块 → 存下来的 PNG 与基准图 **dev 必须 ≈ 0**（实测 0.000），并断言 app 日志里走了"请求隐藏自身窗口"这条路 |
| `perf_probe.ps1` 的热键段 | 真注入 `Alt+C`，量"按键 → 遮罩窗口可见"，5 次报 min/median/max；并自检 `INPUT` 结构体尺寸 |
| `perf_probe.ps1` 的阶段日志段 | 打印链路各阶段耗时（唤醒 / 开始捕获 / 捕获完成），一眼看出时间花在哪 |

`hide_own_windows_e2e.ps1` 里有三条**必须保留**的写法：

1. **"teeth"断言**（`dev(with-window, reference) >= 60`）：没有它，区域里其实没有我们的
   窗口时测试也会"通过"。为此脚本**刻意把主题写成 `dark`**：浅色设置面板与白色记事本
   页面太像（dev 只有 25），深色面板的 dev 是 190 上下，判起"有没有漏"来一目了然。
2. **等遮罩几何稳定再拖拽**：遮罩刚创建时物理尺寸可能是 `2400x1350`
   （= 1920×1080 × 系统缩放，见 [11.4](#114--1-像素的客户区内缩会误导所有像素级对比)），
   这时拖拽会按错误的缩放换算，存出来的是 `735x524` 而不是 `882x628`。
   脚本现在会等到窗口尺寸等于屏幕尺寸才继续，并断言存下来的图尺寸对得上。
3. **额外抓一张 `after.png`**（整段流程结束、窗口已隐藏时再抓同一块）：
   `dev(after, reference)` 就是**背景自身的漂移**（实测 0.000）。
   没有这个数，一个 10~12 的残差到底是"漏了窗口"还是"背景变了"就说不清——
   本轮就在这里绕过一圈：残差其实是**文字形状**的、在区域右半边（空白页）**恰好为 0**，
   而我们的窗口是**整块均匀**覆盖的，两者一眼可分。

---

## 16. 商用化第四轮：稳定性（日志 / panic / 原子写入）

这一轮补的是"交给一个不认识它的用户"之前的底线：**出问题的时候，现场还在不在。**
蓝图在路线图 M5-B，三条都在这一轮做完。

### 16.1 日志落盘（`crates/cuter-app/src/logging.rs`）

`%APPDATA%\Cuter\logs\cuter.log`，启动时按 **2 MB** 滚动一次，最多保留
`cuter.log` / `.1` / `.2` 三份（`rotate_in` 是纯函数，用临时目录 + 小阈值做了 3 条单测）。

为什么非做不可：这是一个**常驻托盘、平时没有窗口**的程序。用户说"它没反应"时，
控制台早就被 `FreeConsole`（第 13 节）丢掉了，我们手里什么都没有。

几个刻意的决定：

* **控制台那一份格式一个字都不改**：`tools/probe/*.ps1` 会 grep ` ms$` 的行做断言，
  文件那一份只是把 env_logger 的 writer 换成文件（`Target::Pipe`），
  于是时间戳、级别、模块路径两边完全一致，对照着看不用翻译。
* **不做缓冲**（`File` 直接写，没有 `BufWriter`）：崩溃时最后一行必须还在盘上。
  代价是每条日志一次系统调用，对这种量级完全无所谓。
* **任何一步失败都只降级**：建不了目录 / 打不开文件 → 退回"只有控制台"，
  并在设置 → 诊断里显示"⚠ 日志没有落盘"，**绝不让程序因为写不了日志起不来**。
* 设置 → 诊断里加了**「打开日志目录」**：出问题该看哪儿，要在界面上直接找得到。

### 16.2 panic hook

托盘程序 panic 之后是"闪一下消失"：用户不知道发生了什么，我们也没有现场。现在：

* panic 写成 `ERROR` 日志，**带 `文件:行`**（`info.location()`），再补一条日志文件路径；
* **GUI 路径下**再弹一个 `MessageBoxW` 说明框（CLI 路径不弹，脚本里弹框会把自动化卡死）；
* hook **必须放行**原来的 panic（`previous(info)`），且不能把 panic 吞掉——
  单测 `panic_hook_does_not_swallow_panics` 锁住这一条。

**怎么验证**（没有交互桌面也能做）：

```shell
cuter --self-test-panic      # 隐藏开关，故意 panic
```

实测输出（日志文件里）：

```text
INFO  cuter] 自检：接下来会故意 panic，用来确认崩溃现场确实写进了日志文件
ERROR cuter::logging] 程序内部错误（crates\cuter-app\src\main.rs:421）：panicked at ...:
    Cuter 自检：这是一次故意的 panic（用 --self-test-panic 触发）
ERROR cuter::logging] 详细日志：%APPDATA%\Cuter\logs\cuter.log
```

### 16.3 配置原子写入

`Config::save_to` 不再直接 `fs::write`，而是：同目录临时文件 → `write_all` →
**`sync_all`** → `rename` 覆盖。

* 为什么：配置是**每次退出**、每次改设置都会重写的。直接覆写意味着"写到一半断电 /
  被杀进程"会留下半个文件。虽然 `load()` 会把损坏配置备份并回退默认值（程序仍然起得来），
  但用户所有设置会一起丢。
* **`sync_all` 不能省**：不把数据真的刷到盘上就 `rename`，断电后可能得到
  "文件名对、内容为空"的配置。
* 失败时**删掉临时文件**，不在用户目录里留垃圾；有单测断言保存两次之后
  `config.toml` 内容完整、且目录里没有任何 `.tmp`。

### 16.4 验证清单

| 改动 | 证据 |
|---|---|
| 日志落盘 | 真跑 `cuter` 4 秒后 `%APPDATA%\Cuter\logs\cuter.log` 有内容；`CUTER_LOG=debug` 时文件里出现 eframe/glutin 的 DEBUG 行 |
| 日志滚动 | 手工把日志填到 3 MB → 启动后 `cuter.log.1` = 3000000 字节、`cuter.log` 重新开始 |
| 日志滚动逻辑 | 3 条单测：未超阈值不动 / 依次后移并丢掉最老的 / 没有历史时不凭空造 `.2` |
| panic hook | `--self-test-panic` 后日志里有两条 ERROR（含 `文件:行` 与日志路径），进程照常以失败码退出；单测保证不吞异常 |
| 配置原子写入 | 单测：连续保存两次后读回内容完整、目录里无 `.tmp` |

---

## 17. 商用化第五轮：交付形态（exe 资源 / 便携版 / 自启 / 安装包）

这一轮解决的是"能不能把一个像样的东西交出去"：**exe 有没有版本与图标**、
**能不能不写系统目录地跑**、**能不能开机就位**、**能不能双击装上**。

### 17.1 exe 资源：版本号 + 图标（`crates/cuter-app/build.rs`）

之前 exe 的文件资源段是空的——属性页里产品名没有、版本号没有、图标只有默认的。
现在 `build.rs` 用 `winresource` 编进去：`FileVersion` / `ProductVersion`（来自
`Cargo.toml` 的 `version`，D 的语言包也一并写上）、`ProductName`、
`FileDescription`、`CompanyName`、`LegalCopyright`、`OriginalFilename`，以及图标。

**图标只有一份来源**：`src/icon.rs` 在运行时画给托盘与标题栏，`assets/cuter.ico`
是由同一个函数生成的——

```shell
cargo run -p cuter-app --example dump_icon -- crates/cuter-app/assets/cuter.ico
```

`.ico` 是手写的（`ICONDIR` + 7 个尺寸：16/24/32/48/64/128 用经典 32 位 DIB，
256 用内嵌 PNG，不然光那一张就 256 KB），**没有引入任何图像依赖**。
**改了 `icon.rs` 的图形记得重跑上面那条命令并提交新的 `.ico`**，否则资源管理器里的
图标会和托盘里的不一样。

验证（真实产物）：

```powershell
(Get-Item target\release\cuter.exe).VersionInfo   # 六项齐全，FileVersion 0.1.0
[System.Drawing.Icon]::ExtractAssociatedIcon("target\release\cuter.exe")   # 32x32，能取到
```

代价：exe 从 7.96 MB 涨到 **8.08 MB**（图标 125 KB）。

### 17.2 便携模式：一个标记文件，全局跟着走

`cuter.portable`（空文件）放在 exe 旁边 → `Config::config_dir()` 返回 exe 所在目录，
于是**配置 / 日志 / 数据 / 默认保存目录**（`Screenshots\`）全部落在 exe 旁边。

* **判据是标记文件，不是"目录能不能写"**：可写性会在 U 盘写保护、装在
  `Program Files`、目录只读时悄悄变化，而用户最容易被坑的正是这里——
  明明想"带着配置走"，配置却写回了 `%APPDATA%`。
* 列表与判据函数 `portable_root` 都是纯函数，有单测（有/没有标记、目录不存在）。

### 17.3 免安装 zip：`tools/package/portable_zip.ps1`

产出 `dist\cuter-<版本>-portable-x64.zip`（exe + 标记 + README，README 用 UTF-8 BOM 写），
然后**自己解压验证两件事**：

1. 运行解压出来的副本后，`logs\cuter.log` 出现在 **exe 旁边**（便携模式确实生效）；
2. `%APPDATA%\Cuter\logs\cuter.log` 的**大小与时间戳一个字都没变**。

实测：`3.65 MB` 的 zip（8.1 MB 的 exe 压下来），两条断言都通过。
打包脚本里"自己验证自己"是刻意的：**交付物没人验过就等于没有**。

### 17.4 开机自启：`general.autostart`

写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下的一条 `REG_SZ`：

```text
"C:\...\cuter.exe" --tray
```

* 用 HKCU 而不是 HKLM：**不需要管理员权限**，截图工具没有理由提权；
* **必须带 `--tray`**：否则用户把「启动即驻留托盘」关掉之后，开机就会蹦出一个设置窗口；
* 路径带引号：装在 `C:\Program Files\...` 这种带空格的目录下，不加引号会被拆成两个参数。

对账逻辑（`desktop::reconcile_autostart`，启动时跑一次）：

| 配置 | 注册表现状 | 动作 |
|---|---|---|
| 开 | 不是我们/不是当前路径 | 写成当前 exe（**换了目录也能自愈**） |
| 关 | **指向本 exe** | 删掉 |
| 关 | 指向别人（另一份副本、别的程序） | **一根汗毛都不动** |

最后一格很重要：便携版默认不开自启，而它与已安装版本用的是同一个值名，
如果"关"就无脑删，就会把用户装好的那份的自启项顺手删掉。

**配置是唯一的事实来源**（2026-09-29 修订）。中间试过一版"注册表里有指向我们的项就
采纳到配置"，理由是防升级竞态（见下），但它让"手工把 `autostart` 改成 `false`"
变成**关不掉**：探针 D 立刻抓出来了。现在的取舍是：宁可留一个可解释的小竞态，
也不要一个关不掉的开关。

> ⚠️ **已知限制（升级竞态）**：安装器在勾了自启时会调 `--enable-autostart`
> （写配置 + 注册表）。但如果**旧版本的实例当时正在运行**，它退出时会把内存里的
> 旧配置整份写回去（`autostart = false`），于是下一次启动按"配置说了算"把自启项删掉——
> 用户会觉得"装了但自启没生效"。设置界面里的勾选框会如实显示"未开启"，重新勾一次即可。
> 要彻底消除它，得让安装器先请正在运行的实例退出（或让它把请求转交给旧实例去写）。

验证：`tools/probe/autostart_e2e.ps1` 对**真实注册表**跑四种情形（外加一个自动备份/还原），
实测输出：

```text
A. config off, foreign entry   -> left alone            PASS
B. config on                   -> '"...\cuter.exe" --tray'  PASS
C. stale entry with config on  -> healed                PASS
D. config off, our entry       -> removed               PASS
RESULT: PASS - general.autostart drives HKCU\...\Run in all four cases
```

探针里还加了**两条防幻觉的护栏**（这一轮真被咬过一次）：开始前若还有别的 `cuter` 实例，
直接抛错——单实例守卫会把这次启动"转交"给旧实例，新进程立刻退出，
于是四个情形全部失败却看不出原因；启动 2 秒后再断言**我们的进程还活着**，
把"程序跑了但没做事"和"程序根本没跑起来"分开。

### 17.5 安装包：Inno Setup 6（编译器本身也是免安装的）

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\package\get_innosetup.ps1     # 一次性
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\package\build_installer.ps1
# → dist\cuter-0.1.0-x64-setup.exe （4.81 MB）
```

**本机原来没有 Inno Setup / WiX**，而这两个是路线图点名的方案。这里的选择是：
下载官方安装包（`innosetup-6.7.3.exe`，**先校验 Authenticode 签名：Pyrsys B.V.**），
用 `/PORTABLE=1` **免安装**解到 `%LOCALAPPDATA%\Cuter\tools\innosetup`——
既拿到了编译器，又**没有**往 Program Files / 开始菜单 / 卸载列表里加任何东西。
`get_innosetup.ps1` 幂等，`build_installer.ps1` 发现缺工具链时会自己调它；
顺带从 Inno 官方仓库拉一份**官方中文语言包** `ChineseSimplified.isl`
（它不在默认安装集里），所以向导是中文的。

`tools/package/cuter.iss` 里的几个刻意决定：

* **按用户安装**：默认 `{localappdata}\Programs\Cuter`，`PrivilegesRequired=lowest`；
  想装到 `Program Files` 的用户可以在向导里选（`PrivilegesRequiredOverridesAllowed=dialog`）；
* **版本号从 exe 的版本资源里读**（ISPP 的 `GetFileVersionString`），
  所以安装包的版本号不可能和程序对不上；
* **不带 `cuter.portable` 标记**：安装版就该用 `%APPDATA%`（探针会断言这一点）；
* **自启不直接写注册表**，而是调用程序自己的 `--enable-autostart`：
  自启是"配置 + 注册表"两处状态，必须由同一个作用点写，否则第一次启动就会被对账改回去。
  卸载同理走 `--disable-autostart`（`RunOnceId` 必须有，否则 Inno 会警告）；
* **卸载保留用户配置**：`ConfirmUninstall` 与完成页都明说"设置、日志与截图会保留"。

验证：`tools/probe/installer_e2e.ps1`（真的装、真的卸，装在临时目录里，全程备份/还原）：

```text
installing (silent) into C:\...\Temp\cuter-installer-e2e-20260929-192654
installed exe exists: True
start menu shortcut: True
desktop shortcut:    True
autostart entry: "...\cuter.exe" --tray
config autostart = True            ← 注册表与配置两侧都到位（天真的安装器只会写一边）
installed program says: cuter 0.1.0
uninstalling (silent)
install dir gone: True
autostart after uninstall:         ← 自己那条也被删掉了
user config kept: True
RESULT: PASS
```

### 17.6 还没做的：代码签名

`dist\` 里现在是三种交付物：`cuter.exe`（裸的）、便携版 zip、安装包。
**三者都没有代码签名**，所以用户第一次运行会看到 SmartScreen 的"未知发布者"提示。
这需要一张证书（OV 约 $200/年，EV 更贵）；没有证书之前只能在文档里把提示写清楚，
不要假装没有这回事。

---

## 18. 商用化第六轮：截图历史

路线图 M5-C 的头号用户价值项。截图工具最常见的动作是"刚才那张再给我一次"
（粘错了地方、想换个输出方式），没有历史就只能重新截。

### 18.1 为什么历史要存**原图**（以及代价怎么交代）

HANDOFF 第 8 节留过三个约束，实现时逐条对应：

| 约束 | 这一轮的处理 |
|---|---|
| 只存缩略图不行 | 每次交付存一份**无损 PNG 原图**，面板里的「复制 / 贴图」用的是它。探针用 **SHA256 逐字节比对**证明"历史的原图 == 同一次截图的另存文件" |
| 只存内存不行 | 内存里只有 egui 纹理（按需解码 + 缓存），30 张 4K 不会变成 1 GB |
| 存原图会吓到用户 | 面板顶部常驻「共 N 张 · X MB」，设置里也显示张数/占用/目录，「清空历史」一键可达（两次点击确认），`limit` 可调、`0` = 关闭 |

目录布局（`%LOCALAPPDATA%\Cuter\history`，便携模式跟 exe 走）：

```text
index.toml        条目清单（id / 时间 / 尺寸 / 字节数），原子写入
images/<id>.png   原图
thumbs/<id>.png   缩略图（宽 <= history.thumb_width）
```

**清单与文件不一致时以文件为准**：清单指向的图没了就把这条丢掉（用户手动删过、
上次断电写了一半）。反过来不去扫孤儿文件——那会让"打开面板"变成一次目录遍历。

### 18.2 面板：一行一张，而不是网格

网格更好看，但"这张占多大、什么时候截的"没地方放，而这个面板的第二个作用恰恰是
**让用户看得见磁盘占用**。所以：每行 = 缩略图 + 时间 + `尺寸 · 占用` + 四个按钮
（复制 / 贴图 / 另存为… / 删除），不做"先选中再操作"（少一步，也不会出现
"看着选中的是 A、操作的是 B"）。

入口有三个：托盘菜单「截图历史…」、`cuter --history`（会转交给正在跑的实例）、
设置 → 截图历史。说明文字里明写**这里只有你自己的截图**，剪贴板贴进来的图不会进来。

### 18.3 两个当场踩到的坑

1. **`dirs::data_dir()` 在 Windows 上是 Roaming，不是 Local。**
   历史最早写进了 `%APPDATA%\Cuter\history`，而探针按 `%LOCALAPPDATA%` 找，
   一条都找不到（现象是"日志里明明有 `历史 +1`，目录里空空如也"）。
   已改成 `dirs::data_local_dir()`（缓存不该跟着漫游配置文件同步，域账户下那是几十 MB），
   并加了单测 `data_dir_is_local_not_roaming` 钉住。
2. **纹理的所有权**：`Image::new(&texture)` 需要一个**借用**的纹理句柄，而
   `load_texture` 返回的是拥有所有权的 `TextureHandle`，得 `.clone()` 之后才能画；
   直接把 `HashMap::entry().or_insert_with()` 的结果拿来用会一路和借用检查器打架。

### 18.4 验证

| 项 | 结果 |
|---|---|
| 单测 | `history.rs` 10 条（写盘/淘汰/清空/缺文件/损坏清单/缩略图尺寸/字节统计/原图往返）+ 配置 2 条 |
| 端到端 | `tools/probe/history_e2e.ps1`：`limit=2` 连截 3 张 → 2 条记录、旧文件真的被删、记录字节数与实际一致、**原图与另存文件 SHA256 相同**、`limit=0` 一个文件不写、`--history` 后面板窗口真的出现（734x598） |
| 回归 | `hide_own_windows_e2e.ps1`（dev 0.000）、`autostart_e2e.ps1`（四情形）、真机测试 7 条，全绿 |

---

## 19. 捕获耗时拆解（下一轮 DXGI 的依据）

第 15 节把"按热键 → 遮罩可见"压到中位数 113 ms 之后，最大的那块就是**整屏捕获**。
商用化第六轮把这条链路**拆开逐项量了一遍**——"捕获慢"这句话对优化没有指导意义：
到底是 `BitBlt` 慢，还是我们自己在 Rust 侧拷贝/换通道慢，决定了完全不同的做法。

跑法（只在手动调优时跑，它要独占屏幕）：

```powershell
cargo test -p cuter-core bench_capture_breakdown --release -- --ignored --nocapture
```

1920x1080（每次搬运 7.9 MB）实测：

| 步骤 | 耗时 |
|---|---|
| `BitBlt` + `CAPTUREBLT` | **33.3 ms** |
| `BitBlt` 不带 `CAPTUREBLT` | **33.2 ms** |
| `GdiFlush` | 约 0 ms |
| `to_vec`（8 MB 拷贝） | 3.0 ms |
| `from_bgrx`（BGRA 转 RGBA） | 5.1 ms |
| `capture_rect_impl` 合计 | **33.7 ms** |
| `capture_desktop` 合计 | **33.2 ms** |
| 窗口探测 `visible_windows()` | 0.2 ms |
| `monitors()` | 0.01 ms |

三条结论，其中两条**推翻了旧猜测**：

1. **`CAPTUREBLT` 完全不是负担**（33.3 对 33.2 ms）。以前"去掉它省一截"的猜测是错的，
   而它换来的是分层窗口（部分工具提示、半透明 UI）不出空洞——所以必须留着。
2. **软件搬运那两步（拷贝 + 通道交换，共约 8 ms）在合计里被完全掩盖**：
   `capture_rect_impl` 合计 33.7 ms 约等于 `BitBlt` 一项。也就是说"去优化 Rust 侧的
   `to_vec` / `from_bgrx`"没有意义，即使优化到 0 也省不下几个毫秒。
3. **窗口探测不是开销**（0.2 ms），不必去"优化"它。

所以剩下的唯一杠杆是**换捕获 API**：DXGI 桌面复制拿的是 DWM 已经合成好的表面，
省掉的正是 `BitBlt` 那 33 ms（预期落在 10~15 ms，瓶颈转到 GPU 到 CPU 的回读）。
难度与坑写在路线图 M5-D 里（每个输出只允许一个 duplication、静态桌面会超时、
RDP 不支持、多屏要拼接）。

注意：这段是**下一步的账**，DXGI 后端**尚未实现**。

---

---

## 20. 商用化第七轮：用户实测反馈（六条）

用户装好便携版实际用了一圈，提了六条。逐条记下**真因**与处理，因为其中两条是
"看起来像小毛病、其实是真 bug"，还有一条推翻了我对默认值的假设。

### 20.1 「截图设置里点『重新截图』，整屏变白」——真 bug，已修

复现：截一张 → 打开工具条的「更多」→ 点「重新截图」，整屏变成遮罩自己的底色
（探针截图里放大镜读到的颜色就是遮罩背景 `#F3F3F5`）。

真因在"截图前要不要先把自己藏起来"的判断上：

* `needs_hide_delay()` 只看**主窗口**（`control_visible`）和贴图列表，**没算遮罩自己**；
* `hide_all_windows()` 也只给主窗口和贴图发 `Visible(false)`，**没给遮罩发**。

于是「重新截图」时：主窗口本来就隐藏、没有贴图 → 判定"无需等帧" → 立刻抓图，
而遮罩正开在屏幕上 → **把遮罩自己冻进了新的一帧**。日志里那句
`截图链路[无需等帧，直接捕获]` 就是证据。

修法（三处，缺一不可）：

1. `desktop` 增加 `UiWake::window_with_title_visible(标题)`：按**标题 + 进程 id**
   枚举顶层窗口，问操作系统"我们自己的某个窗口还在屏幕上吗"——不看 egui 那边的
   视口状态（egui 回收视口与 winit 销毁窗口之间隔着好几步）；
2. `needs_hide_delay()` / `hide_seen()` 都把遮罩（和贴图）算进去；
3. `hide_all_windows()` 给遮罩补一条 `Visible(false)`。

注意第 3 条要**先确认它真的在屏幕上**再发。无脑发会挂到那个视口 id 上，
于是**几帧后新建的遮罩一开始就是隐藏的**——表现成"按热键遮罩再也不出现"。
我改完第一版就踩了这个（连第一次截图都没有遮罩），所以 `perform_capture` 里
补了一句显式的 `Visible(true)`。

### 20.2 「碰到窗口会自动确认边界，但左键点一下不出选区」——真 bug，已修

两层原因，第二层是没料到的：

1. 单击（按下→松开几乎没移动）之前**什么都不做**：`Drag::Create` 拉出一个 0 像素的
   框，松手时被"太小就丢弃"清掉。现在：**按下时记住当时高亮的窗口，松手时若移动
   小于 4 点，就把那个窗口作为选区**（并提示"（单击选中窗口）"）。
2. 为什么用户觉得"点了没反应"：**默认工具是「矩形」**（`AnnotateConfig::default()`），
   所以单击其实画了一个 1 像素的矩形。截图工具的第一步永远是"选哪里"，
   默认工具已改成**「选择」**（想"选完立刻画框"的人在设置里改回去即可）。
   高亮矩形取**面积最小**的那个：最大化浏览器上压着小对话框时，用户点的是看得见的那个。

### 20.3 「鼠标放在左上角怎么还能拉动临时截图」——改成"选区内拖动 = 重新框选"

原来的规则是"选区内拖动 = 移动选区"。用户的实际感受是"我明明想重新框一块，
结果整个选区被我拖着跑"。现在：

* **选区内拖动 = 重新框选**（默认行为）；
* **Ctrl + 拖动 = 移动选区**（方向键微调仍然可用，缩放仍然走八个控制点）。

实现上有个必须记住的细节：**不能一按下就把旧选区清掉**——那样"选区内双击确认"
会失效（第二次点击时已经看不到 `inside_selection`）。改为让
`update_selection_drag` 用新矩形覆盖旧的：只点一下 → 旧选区留着（有高亮窗口则改选它），
真的拖了 → 变成新框选的矩形。

### 20.4 「设置界面太丑」——拆成两部分，本轮先做工具条弹窗

设置界面要做成"左侧分类导航 + 右侧卡片"（Win11 风格），这是下一轮的主任务。

本轮先把**截图遮罩里的工具选项弹窗**重做了（原话是"选择线粗的弹框很不友好"）：
原来是固定两行——第一行线宽滑块、第二行「填充图形 + 马赛克块大小」——
三个不相干的东西挤在一个叫「线宽」的面板里。现在按**当前工具**决定内容：

| 当前工具 | 弹窗内容 |
|---|---|
| 矩形 / 椭圆 / 箭头 / 画笔 / 荧光 / 序号 | 标题=工具名 + **4 个线宽预设点** + 滑块 + **实时预览线**（矩形/椭圆多一个「填充图形」） |
| 马赛克 | 块大小 + 一块**效果预览**（直接看到块多大） |
| 模糊 | 半径 + 渐变预览 |
| 文字 | 字号 |
| 选择 | 提示"在图上拖出一块选区…" |

顺带修掉一个"改了就丢"：弹窗里的颜色/线宽/填充/马赛克块/字号现在会
**写回配置**（`OverlayResult` 那条通道会销毁遮罩，所以走旁路
`Overlay::take_style_change()`，由宿主每帧取走落盘）。

### 20.5 「参考同类工具」

已做：**托盘悬浮提示显示快捷键**（`Cuter —— 截图 Alt+C · 滚动截屏 Alt+S · 贴图 Alt+V`）、
**托盘菜单每项后面显示快捷键**（Win32 菜单的 `\t` 技巧，用户改键位后菜单跟着变）、
**隐藏/显示所有贴图**（新热键 `Shift+F3`，只藏窗口不销毁，再按一次原样回来；
托盘菜单也有这一项）。

下次做：贴图分组 / 多张贴图管理。

顺带修掉热键表里三处过期文案（「滚动截屏（未实现）」「识别文字（未实现）」
「贴图并识别（未实现）」——它们早就实现了）。

### 20.6 验证

| 项 | 结果 |
|---|---|
| 新增端到端探针 | `tools/probe/overlay_select_e2e.ps1`：拿一个**自己摆好位置的记事本窗口**当标尺，用"应用真正保存下来的 PNG 尺寸"当判据——单击窗口 → **784×492**（窗口 800×500，差值是不可见边框）；拖 240×160 → 得到 **240×160**；在选区内再拖 380×230 → 得到 **380×230**（若是"移动"就会是 240×160），全绿 |
| 「重新截图」修复 | 同一套点击流程复现 + 修后截图：冻结帧变成真实桌面（放大镜读到 `#81858C`，修前是遮罩底色 `#F3F3F5`），日志从 `无需等帧，直接捕获` 变成 `请求隐藏自身窗口` |
| 工具弹窗 | 实机截图确认：标题=当前工具、4 个预设点、滑块、**红色预览线**（当前颜色 + 实际线宽）、勾选框只在形状工具出现 |
| 其它 | 257 个单测 + clippy 零警告；用户配置在探针期间被覆盖过，已从备份还原并确认能正常加载（不产生 `config.toml.bak`） |


---

## 21. 商用化第八轮：设置界面重做（Win11 风格）

用户反馈第 4 条"设置界面太丑，完全没有现代化软件的简洁美观和便捷性"，
他自己选的方案是**左侧分类导航 + 右侧卡片**。

### 21.1 结构

```
Cuter                                        [框选截图] 当前显示器 整个桌面 … 退出
v0.1.2 · Windows GDI (BitBlt) · 纯 Rust / egui
按 Alt+C 开始截图                       ← 最近一次动作的状态（有反馈时用强调色）
┌──────────┬────────────────────────────────────────┐
│ 通用     │ ┌ 外观与启动 ──────────────────────────┐ │
│ 快捷键   │ │ 主题  [深色 v]                        │ │
│ 截图     │ │ [x] 启动后只驻留托盘…                 │ │
│ 保存     │ │ [ ] 关闭这个窗口时退到托盘…           │ │
│ …        │ └──────────────────────────────────────┘ │
│ 关于     │                                          │
└──────────┴────────────────────────────────────────┘
```

12 个分类：通用 / 快捷键 / 截图 / 保存 / 标注 / 贴图 / 识别文字 / 滚动截屏 /
截图历史 / 字体 / 诊断 / **关于**（新增：版本、后端、许可、快捷键速查表——以前找键位
要在一个长折叠列表里翻半天）。

只有一个按钮是强调色实心（「框选截图」= 主操作），其余中性：一排名词按钮全蓝非常吵。

### 21.2 两个 egui 视觉上的坑（都实测踩过）

1. **主题里 `widgets.inactive.bg_fill` 是 `TRANSPARENT`**。那是给工具条图标定的
   （图标自己画底），但设置界面直接用这套视觉会**"复选框只剩一个对勾、框看不见"**。
   现在 `settings_visuals()` 只在设置界面的作用域里把控件底色补回来。
   第一次补的时候用了 `surface_hover`/`surface_active`，结果**一排查选框变成蓝色圆点**、
   看着像单选按钮——所以用独立的 `control_fill()`（中性灰），强调色只留给选中态。
2. **卡片底色必须与控件底色不同**。两者都是 `theme.surface` 时控件会"融进"背景，
   现在 `card_fill()` 按主题明暗各偏一点（深色偏亮 1.12、浅色偏暗 0.97）。

### 21.3 一条工程约束：别随手跑 `cargo fmt --all`

本项目**不按 rustfmt 默认风格**排版（手工对齐的表格化注释很多）。
这一轮误跑了一次 `cargo fmt --all`：它重排了 **25 个文件**（+876/-388），
而且因为在 `app.rs` 上解析失败而中断。纯格式化的改动已全部 `git checkout` 回滚，
只保留功能改动。**要格式化就单个文件手工整理，不要整仓跑 fmt。**

### 21.4 验证

| 项 | 结果 |
|---|---|
| 静态 | 258 个单测全绿、clippy 零警告 |
| 实机截图 | 左栏选中态（蓝底 + 左侧强调条）、卡片边框与底色、**方形复选框可见**、下拉框可见、主按钮为唯一强调色按钮——逐个确认过 |
| 回归 | 默认页=通用；所有原有开关/输入框都在（只是从折叠列表搬进了卡片），config-audit 里那些"必须有真实作用"的字段一个没丢 |


---

## 22. 商用化第九轮：贴图分组

用户从同类工具的做法里挑的一项：把"参考资料"和"正在照着看的东西"分成几套，
切一下整体换掉，而不是一张张关。

### 22.1 做法

* 配置：`pin.groups`（默认 2，1-9）、`pin.active_group`（当前在第几组，**持久化**——
  下次启动新建的贴图落在同一组）
* 热键：`toggle_pins`（默认 **Alt+H**）隐藏/显示当前组；`next_pin_group`
  （默认 **Alt+G**）切到下一组
* 托盘菜单两项都带上键位；设置 → 贴图 里有一张「贴图分组」卡片，显示每组的张数、
  当前组、"切到下一组"、"隐藏/显示本组"
* 新增两个**可脚本化**的入口：`cuter --toggle-pins` / `cuter --next-pin-group`
  （走单实例转交，不碰键盘——探针就是靠它做确定性验证的）

### 22.2 关键实现决定：可见性靠"不声明视口"，不靠 `Visible(false)`

贴图视口是**每帧重新声明**的，`ViewportCommand::Visible(false)` 对它不生效——
实测 `Shift+F3` 只是改了个状态，窗口照样在屏幕上；连"截图前藏起自己的窗口"
对贴图也失效（要等满 900 ms 超时，还会把贴图截进去）。

现在判据只有一条：`pin_should_show(entry) = !pins_suppressed && !pins_hidden && entry.group == pin_group`，
`declare_viewports` 按它决定**要不要声明**这个视口（不声明 = 窗口被收掉，语义与遮罩 /
OCR 面板一致）；想再显示时重新声明即可，`Pin` 的状态（缩放 / 透明度 / 标注）都还在。
截图期间的临时隐藏用独立的 `pins_suppressed`，抓完图由 `restore_control` 清掉。

### 22.3 这一轮踩到的三个坑（都不是功能本身写错）

1. **别用别人占着的键位。** `Shift+F3` / `Ctrl+F3` 常被常驻的截图类程序拿去做
   「隐藏所有贴图」「切换贴图组」，这类程序在用户机器上常常就在跑，`RegisterHotKey` 直接失败
   （日志：`HotKey already registered`），于是功能"看起来实现了、按下去没反应"。
   默认键位已改成 `Alt+H` / `Alt+G`。**顺带加了一条常驻日志**：启动时打印
   `已注册热键 N 个：…`（这一轮排查全靠它）。
2. **同一个表达式里对同一把 std `Mutex` 加两次锁 = 死锁。**
   `log::info!("…", lock(&self.pins).len(), self.pins_in_group(..))` —— 第一个临时
   `MutexGuard` 活到整条语句结束，第二个 `lock` 直接卡死 UI 线程
   （现象：`Responding=False`、连截图请求都不再处理、日志停在"剪贴板贴图"那一行）。
   统计数字**先算好再打日志**。
3. **"按了没反应"的探针结论要能区分环境问题。** 这一轮的探针最初用 Alt+V / Alt+G / Alt+H，
   在"VM 抢键盘 / Windows 搜索浮出抢焦点"时四步全红，看起来像四个功能 bug。
   现在这个探针**完全不用键盘**（走 `--pin` / `--next-pin-group` / `--toggle-pins`），
   只有真正依赖热键的探针才需要那套 SKIPPED 守卫。

### 22.4 验证

`tools/probe/pin_groups_e2e.ps1`（键盘无关，判据是"可见的 `Cuter 贴图` 窗口数量"）：

```
pin from clipboard (group 1)     visible pins = 1
switch to group 2                visible pins = 0     ← 上一组被隐藏（没销毁）
another pin (now in group 2)     visible pins = 1
back to group 1                  visible pins = 1     ← 切回来东西还在
hide this group                  visible pins = 0
show again                       visible pins = 1
RESULT: PASS
```

另有单测：`pin_groups_are_clamped_consistently`（组数夹到 1-9、当前组永远落在组数内、
手改 0 组也不会除零）、热键表 9 项一致性。258 个单测 + clippy 零警告。


---

## 23. 商用化第十轮：DXGI 桌面复制捕获后端（实测快 3~5 倍）

动机来自第 19 节的实测结论：整屏 `BitBlt` 就是捕获的全部成本（33.3 ms），
Rust 侧的拷贝与通道交换都被它盖住。想再快只能换 API。

### 23.1 结果（本机 1920×1080 @125%，Intel UHD 630，逐像素与 GDI 对照）

| 场景 | GDI BitBlt | DXGI 桌面复制 | 倍率 |
|---|---|---|---|
| 整屏 1920×1080 | 42 ~ 107 ms | **11 ~ 15 ms** | 约 **4×** |
| 区域 960×540 | 21 ~ 26 ms | **4.1 ~ 5.0 ms** | 约 **5×** |
| 建立后端（设备 + 桌面复制） | 0.00 ms | **137 ms**（优化前 1761 ms） | —— |

**像素完全一致**：逐像素比对平均偏差 `0.00`、`≤8` 的占比 `100.0%`；偶尔出现的
最大偏差 49~75 是两次抓取之间时钟/动画真的变了（只占极少数像素，均值仍是 0.00）。

一键复跑：

```powershell
cargo test -p cuter-core dxgi_vs_gdi --release -- --ignored --nocapture
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\dxgi_backend_e2e.ps1
```

### 23.2 三个坑（都不是"API 用错了"这么简单）

1. **`D3D11CreateDevice` 传了适配器就必须用 `D3D_DRIVER_TYPE_UNKNOWN`。**
   文档里写着 `HARDWARE` 只能与"适配器为 NULL"搭配；写错时返回
   `E_INVALIDARG (0x80070057)`，而报错只有一句"参数错误"，完全看不出跟适配器有关。
   为了让它可查，`DxgiCapture::new` 现在会把**每个输出失败的具体原因**收集进错误消息
   （`Error::Unavailable`，新变体：区分"这会儿环境不行"与"这个平台没实现"）。
2. **建立复制通道后的第一帧是空帧（整屏纯黑）。**
   `dxgi_vs_gdi` 那次只比对了"第二帧之后"的抓取（第一轮恰好没比像素），
   于是这个 bug 躲过了单测，是端到端探针抓到的：DXGI `mean 0.0 / 1 种颜色`，
   同期 GDI `mean 251 / 36 种颜色`。修法是丢掉 `primed` 之前的那一帧。
   现在**整屏比对的第 1 轮也逐像素查**，这类"只在首帧出现"的错再也躲不掉。
3. **建立后端要 137 ms（优化前 1761 ms），所以不能放在捕获路径上。**
   优化前的原因很蠢：给**每个适配器**都建一遍 D3D11 设备，而本机 3 个适配器里
   只有 1 个接了显示器（先 `EnumOutputs` 看有没有输出，再建设备）。
   即便如此，137 ms 也不该让用户等：GUI 路径在启动时**挂后台线程**建（见
   `app::spawn_dxgi_backend`），建好之前所有截图走 GDI，建好之后一直用它。
   也正因为这个成本，**不能"用完就释放去让别的录屏程序用"**——下次再建又是 137 ms。

### 23.3 配置与默认值

* `capture.backend = "gdi" | "dxgi"`，**默认仍是 `gdi`**：DXGI 依赖桌面复制，
  RDP 会话没有、被别的录屏程序独占时也没有，而默认值必须是最不容易出事的那个。
* 选 `dxgi` 时：GUI 在后台预热，拿不到就回退 GDI 并留日志；设置 → 截图 页有下拉框，
  下面那行如实显示**当前实际在用**哪个后端。
* **无界面（`cuter --full`）是一次性进程**，没有预热机会：配 `dxgi` 会为这次捕获
  付 137 ms 的建立成本（实测整跑 1.29 s vs GDI 0.17 s，主要差在这里）。
  一次性脚本截图用 GDI 更快，连续截很多张才轮到 DXGI 赢——日志里会打印建立耗时。
* 无界面路径也跟随配置（以前写死 GDI，那会让"配置了 DXGI 却没变快"变成查不出的问题）。
* 手改配置写错值（如 `not-a-real-backend`）→ 配置解析失败回退默认值，捕获照常工作，
  探针里专门测了这一条。

### 23.4 已知取舍（写进文档，不藏着）

持有桌面复制期间，**别的程序可能拿不到同一个输出的复制通道**（Windows 同一时刻
只允许一个 `DuplicateOutput`）。默认关闭 DXGI 也就默认避开了这个副作用；
主动打开的用户应当知道这一点。


---

## 24. 商用化第十一轮：把"用户真正感受到的延迟"量出来（结论：瓶颈已经不在捕获）

### 24.1 补上缺失的那一段

在这之前，链路日志到 `+捕获完成` 就断了，于是"按下热键到屏幕上真出现遮罩"这中间
（eframe 建窗口 + 建 GL 上下文 + 首帧上传）一直是黑盒——DXGI 把捕获从 45 ms 压到 11 ms，
总量却几乎没变，正好说明这一段才是大头。

现在补上了两个点：

* `截图链路[遮罩首帧可见]：+N ms` —— 由遮罩视口闭包在**第一个 pass** 落下时刻、
  由 `logic()` 落日志。**一次性开关必须是 `AtomicBool::swap(false)`**：
  第一版用"槽为空就写"判断，而 `logic()` 每帧把槽取走，于是下一帧又写一次，
  日志每 133 ms 刷一条、耗时一路涨到 1.2 s，全是假数据。
* 新探针 `tools/probe/latency_probe.ps1`：按 Alt+C → 等遮罩 → Esc，重复 N 次，
  从**应用自己的阶段日志**里算出每一段。

### 24.2 实测（本机 1920×1080 @125%，各 8 次，取中位）

| 阶段 | GDI | DXGI |
|---|---|---|
| 唤醒到处理 | 1 ~ 4 ms | 1 ~ 2 ms |
| 捕获完成 | **+45 ms**（37~60） | **+11 ms**（10~14） |
| **遮罩首帧可见（用户看到的总延迟）** | **251 ms**（218~285） | **235 ms**（127~457） |
| 其中"捕获完成 → 首帧可见" | **204 ms** | **224 ms** |

**结论：捕获只占 4~18%，剩下 ~200 ms 全在建那个视口窗口（winit 窗口 + GL 上下文 +
首帧纹理上传）。** DXGI 那 4 倍捕获提速只值 35 ms，压在 250 ms 的总量上几乎看不出来。
所以：

* "再优化捕获"已经没有意义（这一轮的账就是证据）；
* 想让工具"点下去就出来"，要做的是**让遮罩视口窗口活着复用**，而不是每次新建。
  这是下一轮的第一件事，判据现成：`latency_probe.ps1` 的中位数从 ~240 ms 降到 100 ms 以内，
  同时 `overlay_select_e2e` / `pin_groups_e2e` / `hide_own_windows_e2e` 三个探针不回退。

顺带排除的一个猜想：把 `ViewportBuilder::with_active(true)` 改成 `false` 实测没有改善
（中位 270 ms vs 235 ms，纯噪声），**不是激活窗口的锅**。

### 24.3 这一轮顺手修掉的两个"把测量变成谎话"的坑

1. **探针手改 TOML 会把配置写坏。** `latency_probe` 第一版用正则往 `[capture]` 后面插
   `backend = "dxgi"`，某次写出了一个**重复的 `[capture]` 表**，应用解析失败后**静默回退默认值**，
   于是"dxgi 那一次"其实在测 GDI——数字看着合理，结论完全错。现在：
   探针**整份写一个最小配置**（与 `pin_groups_e2e` 一致）而不是动用户的文件；
   并且**一旦日志里出现 `TOML parse error` 就直接失败**。
2. **一次性开关用错原子语义会刷屏。** 见 24.1，`swap` 而不是 `if is_none`。

教训一句话：**"测量工具自身出错"比"被测对象出错"更危险，因为它会给出看似合理的数字。**


---

## 25. 商用化第十二轮：把"热键 → 遮罩出现"从 ~250 ms 压到 ~116 ms

### 25.1 背景与结论

第 24 节量出"捕获只占 4~18%，~200 ms 在建遮罩视口窗口"，于是这一轮的目标就是那一段。
最后**真正见效的是一行"催帧"**，而"让窗口常驻复用"这条路**在 eframe 0.36 上走不通**，
两条都记下来，后者能省掉下一次重走。

| | 第 24 节（本轮之前） | 本轮之后 |
|---|---|---|
| **热键 → 遮罩首帧可见（中位）** | GDI 251 ms / DXGI 235 ms | **GDI 156 ms / DXGI 116 ms** |
| 其中捕获 | 45 / 11 ms | 39 / 12 ms |
| 其中"捕获完成 → 首帧可见" | 204 / 224 ms | **118 / 103 ms** |

（DXGI 10 次：min 100 / median 116 / max 232 ms。本轮数字一律以 **`begin_request` 那一刻**
为起点；第 24 节的日志其实晚记了一帧，所以旧数字偏大——新日志由遮罩闭包在**首帧当场**落。）

### 25.2 见效的那一行：托盘模式下要显式催帧

托盘模式下根窗口是隐藏的，eframe 把重绘节流到 **~100 ms**；而遮罩从"声明视口"到
"闭包真的跑起来"需要**两帧**，于是白等两个节流周期。实测逐帧日志：

```
[已请求遮罩窗口] +11 ms → 遮罩等首帧 +135 ms → [遮罩首帧可见] +267 ms    （帧间隔 ~125 ms）
```

修法是在等首帧期间**每帧都戳一次 `UiWake::wake`**（它往根窗口投 `WM_PAINT`，
是第 7 节就写好的机制，之前只用在"热键进来的第一帧"）。之后帧间隔 ~17 ms：

```
[已请求遮罩窗口] +14 ms → [遮罩首帧可见] +116 ms
```

**必须有上限**：万一遮罩一直画不出来（窗口创建失败等），催帧会以 60 fps 空转，
用户看不见但费电。现在最多催 40 帧（约 0.7 s），之后停手并留一条 warn。

### 25.3 走不通的那条路：让遮罩窗口常驻复用（否决）

动机很直白：每次截图都要新建窗口 + 建 GL 上下文。做法是把窗口停在桌面之外
（-32000，尺寸保持满屏）留着，捕获时只改几何 + 重绘。实测**不可行**，三条都有证据：

1. **eframe 对不可见的 deferred 视口直接不跑 pass。**
   `glow_integration.rs`：`show_ui = is_visible || is_viewport_or_descendant_visible(...)`，
   `is_visible` 来自 `ViewportInfo::visible()`（由 winit 的窗口可见性更新）。
   窗口停在桌面之外时它被算作不可见 → 不跑 pass → **`visible` 这个状态被缓存住**，
   把窗口移回屏幕也不会重新开画。现象：**窗口能被探针找到（`IsWindowVisible` 为真），
   画面却永远空白**，日志里连一条"遮罩首帧可见"都没有。
   （`ctx.request_repaint_of(overlay_id)` 也救不了：可见性判断在跑 pass 之前。）
2. **`ViewportBuilder::active` 一变就重建窗口。** egui `viewport.rs`：
   `if new_active.is_some() && self.active != new_active { recreate_window = true }`。
   所以"停靠时 active=false / 截图时 true"等于每次都在重建，白忙一场——
   HWND 每轮都变（探针打印 HWND 抓到的）。
3. **`ViewportCommand::Visible(false)` 对每帧重新声明的视口不生效**（第 22 节在贴图上实测）。

**顺带纠正一条上一轮的判断**：停靠尺寸用 1×1 并不比满屏好——1×1 时每次捕获要把交换链
重新分配到满屏，实测多花约 70 ms（A/B：中位 184 ms vs 116 ms）。当时之所以觉得 1×1 更好，
正是因为旧日志晚记了一帧。

要再往下压只剩两条路，都超出"改改参数"的范围，暂不做：
* **少一帧**：热键一到就先声明一个透明遮罩窗口，抓到图再画（窗口创建与捕获重叠）。
  风险是用户会先看到一瞬间的实时桌面，而且遮罩状态要分"空壳/就绪"两态。
* **换窗口策略**（自建 winit 窗口 / 换渲染后端），收益 ~100 ms，成本是整个 UI 层。

### 25.4 这一轮的净收获

* 端到端延迟 **~250 ms → ~116 ms**（DXGI）/ 156 ms（GDI），是"手感"上能感觉到的变化。
* 日志与探针现在量的是**真正的端到端**（首帧当场落，不再晚一帧），
  并且逐帧可查（`遮罩等首帧`）。
* 一条明确的死路 + 三条 eframe 约束，写在这里省掉下次重走。


---

## 26. 商用化第十三轮：贴图拖出到别的程序（顺带挖出一个"点一下就卡死"的老 bug）

### 26.1 功能

贴图上 **Ctrl + 左键拖动** 或 **中键拖动** → 把这张图拖到资源管理器 / 微信 / 浏览器里。

* 两个手势都给：`Ctrl+左键` 顺手，但它依赖贴图窗口有键盘焦点（egui 的 `modifiers`
  来自焦点窗口的按键事件），而用户从托盘热键贴出来的图**是**有焦点的、鼠标点过别处之后就没有了；
  **中键不依赖修饰键**，所以一直能用。中键**单击**仍然是"恢复 100%"（按移动距离区分，
  与右键菜单一样是先按下、超过阈值才算拖）。
* 实现：把贴图落成 `%TEMP%\Cuter\drag\cuter-pin-<时间戳>.png`，再用 OLE 拖放把它交给目标程序
  （`desktop::drag_files`）。数据对象**用 shell 自己造的**（`SHCreateDataObject` + 文件 PIDL），
  我们只实现拖放源 `IDropSource`（两个方法）；手写 `IDataObject` + `IEnumFORMATETC` +
  `HGLOBAL` 所有权又长又容易错。
* 为什么落文件而不是走 `CF_DIB`：**所有目标程序都认文件**（资源管理器、微信/QQ、浏览器、VSCode…），
  而内存格式要自己实现 `IStream`，兼容面反而更窄。用 `DROPEFFECT_COPY`，目标程序会复制，
  不会把我们的临时文件搬走；`%TEMP%\Cuter\drag` 里超过一天的会自动清掉。

### 26.2 顺手挖出的老 bug：贴图的动作处理会**自锁死**

```rust
match lock(&pin).ui(&ctx) {                 // ← 这把 MutexGuard 是 scrutinee 的临时值
    Some(PinAction::Copy) => { lock(&pin)... }   // ← Rust 会把它活到整个 match 结束
```

Rust 里 **match 的 scrutinee 临时值活到整个 match 结束**，而下面每个分支又都要 `lock(&pin)`
（`set_status`、`frame()`…）——于是**贴图的全部动作处理都自锁死**：
右键菜单点复制/另存、`Ctrl+C`、`Ctrl+S`、识别文字、改不透明度、以及这次的拖出，
全都会让程序卡住不动（日志停在动作那一刻，之后一条都没有——实测抓到的现象）。

修法一行：先 `let pin_action = lock(&pin).ui(&ctx);` 再 `match pin_action`。
探针里加了回归：拖出之后对贴图按 `Ctrl+C`，断言**进程仍在响应**且剪贴板里出现了图片。

### 26.3 验证到什么程度（说清楚，不夸大）

`tools/probe/pin_drag_out_e2e.ps1`：起真程序 → 热键贴一张 240×160 的图 →
用**另一个进程**里的真 `IDropTarget` 接收 → 两个手势各拖一次。断言：

* 目标进程**每个手势都收到了拖放**（`DragEnter` 两次）；
* 数据对象**确实带着 `CF_HDROP`**（`QueryGetData(CF_HDROP)` 返回 S_OK）——
  这正是资源管理器/聊天软件读文件用的格式；
* 每次拖出都真的写出了 240×160 的 PNG；
* `Ctrl+C` 回归不卡死。

**没有自动验证的**：把文件从数据对象里**读回来**。探针那个 C# 接收端在 PowerShell 5.1
的旧编译器 + COM 互操作下 `GetData` 返回 `DV_E_FORMATETC`（`EnumFormatETC` 那条路也不可靠），
所以"文件内容确实到达目标程序"这半步留给了手动确认：**拖进资源管理器或微信窗口**。
哪天要补自动化，写一个 C++ 的最小接收端比在 PowerShell 里跟 COM 编组较劲更划算。

### 26.4 踩到的两个坑（记下来省事）

1. **`DragAcceptFiles` + `WM_DROPFILES` 收不到 `SHCreateDataObject` 造的数据。**
   一开始的接收端用的是这条老路，OLE 报 `DRAGDROP_S_DROP` 但 `effect=0`（"放下了没人接"）。
   换成自己实现 `IDropTarget`（真实程序的做法）立刻收到。
2. **`IDropSource::QueryContinueDrag` 不能只看左键。** 第一版写死 `MK_LBUTTON`，
   于是**中键那次拖放在起点就立刻结束**（OLE 认为"键已松开"）。
   现在只要**任意一个**鼠标键还按着就继续拖。


---

## 27. 商用化第十四轮：界面字体按需装载（常驻内存 86 MB → 57 MB）

### 27.1 账是怎么算的

托盘常驻是这个软件的常态。**根窗口隐藏时 eframe 连一次 egui pass 都不跑**
（`show_ui = false`，见第 25 节），也就是说**根本没有文字要画**——可我们启动时就把
18.8 MB 的 `msyh.ttc` 读进内存并 `Box::leak` 了一份（第 16 节为了避免与导出路径各存一份）。
于是纯常驻状态下那 19 MB 纯属白占。

### 27.2 做法与实测

启动时不读字体（`font = None`），每一帧开头问一句 `needs_font()`
（有控件窗/遮罩/贴图/历史/OCR/滚动窗要画吗），需要才 `ensure_font()`：

```rust
if self.needs_font() { self.ensure_font(ctx); }
```

| 状态 | 之前 | 现在 |
|---|---|---|
| 托盘常驻、从没截过图 | WS 86.0 MB / 私有 48.9 MB | **WS 57.2 MB / 私有 28.6 MB** |
| 截图一次之后（字体 + 遮罩纹理会常驻） | 同上 | WS 99.5 MB / 私有 69.6 MB（**用到了才花**）|

* **-29 MB 工作集、-20 MB 私有内存**，而这是"根本没用上"的那部分。
* 代价：第一次真的要画界面时多读一次字体，实测 **22 ms**（`界面字体已按需装载：…（耗时 22 ms）`
  这条日志就是给它留的），只在进程生命周期里付一次。
* 判定点放在"每帧开头"而不是各条打开路径上：这样**同一个 pass 里就已经有字体了**
  （`logic()` 先跑、`paint_control`/视口 pass 后跑），不会出现"第一帧是方框"。

### 27.3 验证

* `perf_probe.ps1`：托盘常驻数值见上（注意它自己的延迟环节会先截一次图，
  所以要看"从没截过图"那一段得单跑，我是单独量的）。
* **肉眼确认没变成方框**：把设置窗口与遮罩各截一张图看过——设置页的分类
  （通用/快捷键/截图/保存/标注/贴图/识别文字/滚动截屏/截图历史/字体/诊断/关于）、
  按钮、勾选项文字全部正常。
* 字体装载日志只出现一次；没有中文字体时的告警挪到了装载之后（原来在启动时判定，
  现在启动时还没装载，判定没有意义）。


---

## 28. 商用化第十五轮：控件级吸附（UI Automation）

窗口级吸附只能吸到整窗边界，想截"那个按钮""那个输入框"还得手动描。这一轮接上系统
自带的 UI Automation，能吸到光标下最深那个控件。

### 28.1 关键坑：`ElementFromPoint` 只会返回**遮罩自己**

第一版直接调 `IUIAutomation::ElementFromPoint`，结果每次都返回"类型 50032（Window）、
矩形 (0,0)-(1920,1080)"——**就是我们自己的全屏遮罩**。这不是 bug，是命中测试的定义：
它按 z 序找最上层，而遮罩恰恰是最上层的全屏窗口。

正确做法两步（`crates/cuter-core/src/capture/uia.rs`）：

1. **自己按 z 序找"遮罩底下"那个顶层窗口**：`GetTopWindow` → `GetWindow(GW_HWNDNEXT)`
   逐层往下，跳过本进程、不可见、最小化、以及不含该点的；
2. `ElementFromHandle(那个窗口)`，再**逐层下潜**：每层只 `FindAll(TreeScope_Children)`
   找"矩形包含光标、面积最小"的子元素，直到没有更深的。

第 2 步刻意不写成一次 `FindAll(TreeScope_Descendants)`：浏览器那种窗口的直接后代可能上千，
逐层走每层只有个位数个元素——实测一次探测 1~20 ms（见下）。

### 28.2 顺带要过滤的与要节流的

* **过滤"窗口的一部分"**：Window / Pane / **TitleBar** 一律返回 `None`，退回窗口级吸附。
  不滤掉 TitleBar 的话，鼠标停在标题栏上会吸到那条窄边——把"点一下选整窗"这个顺手操作弄坏。
* **节流**：一次探测 5~8 ms（跨进程 COM），所以遮罩里**只在悬停时问、拖拽中不问**
  （拖拽要跟手），并且按"位置移动 > 3 px 且距上次 > 150 ms"才重新问一次
  （`CONTROL_PROBE_*`）。同一位置重复使用缓存。
* **配置开关** `capture.snap_controls`（默认开）：老机器 / 远程桌面觉得卡可以关掉；
  设置 → 截图 页有勾选项。探针里专门验证了关掉之后同一个点击回到"选整窗"。

### 28.3 行为变化（有意为之）

**点一下 = 选中高亮的那个东西**，而高亮现在优先是控件。所以：

* 鼠标停在按钮上点一下 → 精确选中那个按钮（实测 275×80，与控制真实尺寸**逐像素一致**）；
* 停在窗口空白处点一下 → 还是整窗（实测 682×516 = DWM 扩展边框）；
* **不想要控件级时**：按住 Alt（遮罩里本来就有的"临时关闭吸附"）或关掉开关。

`overlay_select_e2e.ps1` 里那条"点一下选整窗"的用例因此显式关掉了 `snap_controls`——
它测的是选择机制本身，控件级由新探针负责。

### 28.4 验证

新探针 `tools/probe/control_snap_e2e.ps1`（靶子是 `ControlWindow.cs` 里一个位置已知的
原生 BUTTON）：

| 场景 | 期望 | 实测 |
|---|---|---|
| A. 点按钮 | 按钮的精确矩形 | **275×80 = 按钮实测尺寸** |
| B. 点客户区空白 | 整窗（DWM 边框） | **682×516 = 窗口实测边框** |
| C. `snap_controls=false` 再点按钮 | 回到整窗 | **682×516** |

判据用的是"应用真的存出来的 PNG 尺寸"，所以它同时证明了吸附结果**真的落到了交付物**上。
顺带确认没有拖慢热键链路：`latency_probe` 中位 118 ms（上一轮 116 ms，噪声内）。


---

## 29. 商用化第十六轮：横向滚动截屏（方向自动判定）

滚动截屏原来只会往下拼；网页、表格、代码编辑器经常是横着滚的。这一轮把横向补齐，
而且**方向不用用户选**：他滚哪边，程序就往哪边拼。

### 29.1 做法：把每一帧转置，复用纵向那套逻辑

`ScrollStitcher` 的匹配、接缝、长度上限都在"行"上做文章，是最容易写错、也测最狠的部分。
横向不必重写一套——**把每一帧转置之后再喂进去**，在转置坐标系里"横向滚动"与"纵向滚动"
就是同一件事；取图和缩略图时再转置回来。新增的就是：

* `Frame::transposed()`（43 行，单测验证"转置两次等于没转"）；
* `ScrollStitcher::transposed()`（数据与"上一帧"一起转置，宽高互换）；
* `DirectedStitcher`：对外只暴露 `push / image / thumbnail / size / axis`，
  横向时在出入口各转置一次。

### 29.2 方向自动判定，以及它差点造成的事故

判定逻辑：当前方向接不上时，把两边都转置再试一次；能接上就整体切换。
朴素版立刻出了问题——**一次纵向滚动被判定成横向**，840 宽的长图拼成了 1032 宽
（是 `scroll_e2e.ps1` 抓到的，见下）。两个原因，都堵上了：

1. **空白区域在任何位移下都"匹配"**（记事本里没字的行、纯色背景）。
   于是"换个方向试试"必然成功。现在要求重叠区 `band_has_texture()`：
   抽样看这一段是不是整块同色，是同色就判定误匹配、保持原方向。
2. **只在"还没拼进任何内容"之前允许换方向**（`inner.frames() > 1` 就不再换）。
   已经开始拼的方向是对的；此时某两帧接不上通常只是"这一帧滚得太远"
   （滚轮一次跳几屏就会这样），不该因此怀疑方向。

### 29.2b 改完之后**又**翻了一次车，才算把判定收紧到位

上面两条堵完之后，纵向探针仍然偶尔失败（840 宽被拼成 1019 宽）。原因是我漏了一种情况：
纵向探针连续 14 次滚轮让某些帧**完全没有重叠**，而记事本内容里那些**等长的横线**
在转置坐标系里能凑出匹配。现在的完整判据是四条，缺一不可：

| 判据 | 拦住什么 |
|---|---|
| `frames() > 1` 不换 | 已经拼过内容后再怀疑方向 |
| 重叠区 `band_has_texture()` | 纯色区域任何位移都"匹配" |
| 位移 ≥ `min_offset` | 静止画面（两帧一模一样）被当成一次投票 |
| **连续两次**且**步长一致**（±25%） | 周期性内容凑出来的偶然匹配 |

第 4 条的两个细节都踩过坑：

* 第二次投票必须拿**第一次投票那一帧**当参照，不能拿"上一次成功拼接的帧"——
  后者会让位移一帧帧攒大，很快超过帧高，于是**永远**匹配不上（单测里横向四帧的内容
  只在第一帧之后就再也接不上了，方向自然也就切不过去）；
* 确认时要把**第一次投票那一帧补进拼接**：它才是紧挨着画布末尾的那一帧，少了它
  画布与当前帧之间多出一个空档，位移超出帧高——切了方向也拼不进去。

真横向滚动每帧步长都差不多，两帧之内必然确认；周期性内容凑出来的步长是随机的，
过不了"步长一致"。两个探针各连跑两轮：纵向 840×978 / 840×1005（宽度始终等于选区），
横向 760×320 / 760×320。

单测把这四种情形都钉住了：转置往返、纵向保持纵向、横向自动切换（含取图/缩略图朝向）、
纯色不改方向、拼过之后不改方向。

### 29.3 顺带修掉一个"没人跑的探针"

`scroll_e2e.ps1`（纵向）其实**从第 8 轮起就已经是坏的**：那一轮把控件级吸附设成默认开启，
拖动选区时边缘会被吸附，于是"实际选区"与探针期望的差了几十像素，断言 `width mismatch`。
教训：探针也要有"每次改动都跑一遍"的习惯——它不动，坏掉是不会自己喊的。
现在该探针显式关掉吸附（它测的是拼接几何，不是吸附），并恢复正常。

### 29.4 验证

| 探针 | 断言 | 实测 |
|---|---|---|
| `scroll_horizontal_e2e.ps1`（新） | 横滚之后长图**变宽**、高度等于选区高 | **730×320**（选区 700×320） |
| `scroll_e2e.ps1`（纵向回归） | 长图**变高**、宽度等于选区宽 | **840×978**（选区 840×600） |

靶子是一个原生 EDIT 控件装一长行伪随机字母的窗口（`tools/probe/ScrollTarget.cs`）——
不自己画像素：第一版自绘窗口在 PowerShell 的 C# 编译器 + 封送器下会把进程搞崩
（`PAINTSTRUCT` 里那个 `ByValArray` 未初始化 → 0xC0000417），索性交给系统控件。
横向滚轮必须用 **`SendInput`**：`mouse_event` 会忽略 `MOUSEEVENTF_HWHEEL`
（症状是该滚的目标纹丝不动）。

另外：`scroll.max_height` 现在语义是**长度上限**（纵向=高度、横向=宽度），配置审计那一行已更新。


---

## 30. 商用化第十七轮：OCR 的「文本 ↔ 位置」双向联动

同类工具的标志性体验之一：识别完之后，鼠标指到哪一行文字，图上就高亮哪一块；
反过来在图上点一块，右边就落到对应那一行。原来只有**一半**（图 → 文：悬停框有高亮和气泡），
文 → 图完全没有。

### 30.1 关键设计：联动只在"逐行对照"里做

右侧原来是一个**可编辑**的全文框。行号与文本块的对应关系在可编辑文本里是不成立的
（用户改两个字，第 3 行就不再是第 3 块了），硬做联动只会指错地方。所以面板现在有两个模式：

* **编辑全文**（默认，行为不变）：校对、整段复制用；
* **逐行对照**：只读的行列表，行号 ↔ 文本块严格一一对应——联动只在这里成立。
  悬停某行 → 图上高亮对应块；点某行 → 复制该行并把它标为选中；
  在图上点某个框 → 选中它并让列表**滚动到那一行**（几十行的识别结果，不滚过去根本看不到对应关系）。

切模式除了点那两个字，还能按 **`L`**——顺手，也让探针不用去猜按钮坐标。

### 30.2 命中测试抽成纯函数

原来命中测试写死在 `preview()` 里（内联的四个比较）。现在抽到
`OcrResult::block_at(point)`（`cuter-core/src/ocr/mod.rs`），并且带单测：

* 重叠时给**面积最小**的那个块——OCR 引擎偶尔会把一行里的一小段单独报成块，
  用户指着那一小段时想要的就是那一小段；
* 边界左闭右开，与 `Rect::contains` 一致。

### 30.3 验证

新探针 `tools/probe/ocr_linkage_e2e.ps1`（顺带补上了 OCR 这条链路**从来没有过**的端到端测试）：

1. 记事本写三行内容 → `Alt+C` → 框选这三行 → **`Ctrl+O`**（遮罩里的"识别文字"）；
2. 断言识别面板出现（标题 `Cuter 识别结果`）；
3. 按 `L` 切到逐行对照，截一张图；把鼠标移到第 1 行，再截一张；
4. 断言两张图**不一样**（说明悬停真的改变了画面 = 文 → 图这一半是活的）。

两张截图留在探针的临时目录里供人复核。**我逐张看过**：面板标题显示
`zh-Hans-CN · 2 行 · 55 ms`，左侧图上两个识别框都画了蓝框，其中**第 1 行那个框填充了半透明高亮**，
右侧列表里第 1 行也同时高亮——联动确实指到了正确的块。

**顺带如实记两件事**：

* 这次识别把前两行并成了一段（`ter OCR 绗 尊琛?second line abcdef`），
  中文与英文混排的小字号文本，Windows OCR 的效果就是这个水平。联动本身没问题，
  认字质量是引擎的事。
* 这也说明**探针不能只看"没报错"**：如果只断言"面板出现了"，上面这个混行根本发现不了。


---

## 31. 商用化第十八轮：界面多语言（zh-CN / en-US）

界面文案一共 **282 条**（按钮、标签、勾选项、提示、状态、托盘菜单），现在都能切英文。

### 31.1 为什么用"中文原文当键"

给每条文案起符号名（`SETTINGS_TITLE_CAPTURE` 之类）的话：改文案要同时改几百个符号，
而且**漏改的地方编译不会报错**（符号还在，内容旧了）。这里反过来——中文原文本身就是键：

```rust
tr!("关闭")                  // 中文界面 → "关闭"；英文界面 → "Close"
fill!(tr!("已保存到 {}"), path)   // `format!` 只吃字面量，所以另有一个 fill!
```

* 语言是中文、或者表里查不到 → **原样返回中文**：界面永远不会变成空白或 `???`，
  所以这个改造可以**逐块进行**，中间状态始终可用；
* 语言是英文且表里有 → 英文。

`tr` 也是函数名、`tr!` 是宏：宿主侧要翻**运行时**字符串（`cuter-core` 的
`ThemeKind::label()` 之类）时得写 `cuter_ui::i18n::tr(...)`——写成 `tr(...)` 会被宏遮蔽。

### 31.2 表必须被测试兜住（否则英文界面半中半英）

运行时查不到只会静默显示中文，编译不报错。所以 `i18n.rs` 里有四个测试：

| 测试 | 钉住什么 |
|---|---|
| `every_tr_key_has_a_translation` | 扫 `cuter-ui` / `cuter-app` 源码里每个 `tr!("…")`，都有条目 |
| `no_dead_translations` | 表里没有"没人用"的死条目；顺带检查来自 `cuter-core` 枚举的 18 条 |
| `table_is_sorted_for_binary_search` | 表按**运行时**字符串排序（见下） |
| `placeholders_survive_translation` | 每条译文里 `{}` 的**个数**与原文一致 |

排序那条踩了坑：源码里的 `\n` 是**两个字符**（反斜杠 + n，0x5C 0x6E），运行时是换行
（0x0A）。按源码字面量排，`\n\n详细日志…` 会排到 `[第 …` **后面**——于是二分查找找不到
那几条，测试直接报"没有英文翻译"。生成器现在按还原后的字符串排序。

### 31.3 282 条是怎么被"找出来"的

界面文案分散在十几处，人工列全不现实，所以写了一套脚本（`tools/i18n/`）：

1. `wrap_ui_strings.py`：把**用户看得见**的调用点里的中文字面量包成 `tr!(…)`。
   跳过日志、注释、测试模块（项目约定：注释与日志一律中文，不翻译）。
2. `i18n_convert.py` / `fix_multiline_format.py`：把 `format!(tr!("…"), 参数)`
   改写成 `fill!(tr!("…"), 参数)`。**必须按括号配平解析**——第一版用正则，
   把 `{:.0}`、具名捕获 `{count}`、嵌套括号全切坏了，还把 `RichText::new(...)` 的
   右括号吃掉。
3. `extract_keys.py` → `en_table.py` → `splice_table.py`：抽键、对照、原地替换表体
   （必须幂等，前面手工拼接把 `];` 拼重过一次）。
4. `fix_fill_args.py`：修 `fill!` 的参数形态（数组包一层、多余的 `&`）。

漏掉的三类后来补上了：**跨行调用**（`ui.checkbox(` 换行写标签）、**匹配臂**
（`Self::General => "通用"`，设置页左侧分类名）、**自定义 UI 助手**（`settings_card(`）。

### 31.4 配置与验证

* 新配置项 `general.language`：`auto`（跟随系统，默认）/ `zh-CN` / `en-US`。
  `auto` 由宿主探测（`cuter-app` 的 `GetUserDefaultLocaleName`）——
  `cuter-ui` 的硬约定是**不碰任何平台 API**，所以探测放在宿主侧。
  设置 → 通用 → 语言可即时切换，不用重启。
* 探针 `tools/probe/i18n_e2e.ps1`：写 `language = "en-US"` 起一个进程、截图、
  **断言日志里没有 `i18n-missing` 行**（应用渲染时每缺一条翻译就打一行），
  再用 `zh-CN` 跑一遍并断言两张截图不同。两种语言各 0 条缺失。
* 我逐张看过截图：左侧分类、工具条、勾选项、说明段落全是英文。


---

## 32. 一个真实的商业化拦路虎：Defender 把本程序判成"持久化"威胁

这一轮收尾时，**Windows Defender 在构建完成后把 `cuter.exe` 判成威胁并拦截执行**：

```
ThreatID      2147737394
ThreatName    Behavior:Win32/Persistence.A!ml
检测文件       target\release\cuter.exe
              dist\verify-portable\cuter-0.1.11-portable-x64\cuter.exe
报错          Operation did not complete successfully because the file
              contains a virus or potentially unwanted software.
```

### 32.1 为什么会这样（这是误报，但不是"无缘无故"）

`Behavior:Win32/Persistence.A!ml` 是**机器学习型的"持久化行为"判定**。本程序确实做了
两件会被这个模型盯上的事：

1. **开机自启**：往 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 写一条指向自己的命令；
2. **全局热键 + 常驻托盘**：注册 7 个全局热键、长期驻留后台、隐藏窗口。

对截图工具来说这两件事都是**正常功能**（同类工具都这么干），但把它们
组合起来、再加上**可执行文件没有数字签名**，机器学习的判定就没有任何"信任来源"可以依据，
只能按行为模式判。多个探针反复读写注册表（`autostart_e2e` / `installer_e2e`）
大概也让它在短时间内看到了更多"持久化"行为。

### 32.2 为什么这件事必须记在路线图上

`docs/roadmap.md` 里"代码签名"原来写的理由是"减少 SmartScreen 警告"。现在有了**直接证据**：
没有签名，杀软可以**直接拦掉执行**——用户双击图标什么都没发生，或者像这里一样被静默隔离。
对一个"截图工具"来说这是致命的：它的核心能力之一（常驻 + 热键）恰好是启发式最敏感的行为。

签名解决不了全部问题（新证书也需要攒信誉），但它是**唯一能提供信任来源**的手段；
此外还可以考虑：给发布包做"提交误报"（Microsoft 的 false-positive 提交入口）、
在官网/README 里写明如何手动允许。

### 32.3 当时的验证状态（如实记录）

* **源码层面完全可信**：274 个单测 + clippy 零警告全绿；拦截发生后重跑
  `cargo test --workspace` 依然全绿（测试二进制在 `target\debug\deps\`，没有被拦）。
* **0.1.11 的产物在拦截之前完成了全部验证**：`portable_zip.ps1` 自带的可移植性检查
  （真的跑了一次 exe）、`installer_e2e.ps1`（静默安装 / 快捷方式 / 自启项 / 卸载）都是
  在 4:20 前后通过的；`i18n_e2e` 也在同一批产物上通过、两种语言各 0 条缺失翻译。
* **4:24 起，该二进制在本机无法执行**：所有运行时探针都会以
  "contains a virus" 失败。这不是代码回归，是杀软拦截。

**结论**：这一轮的功能已完成并通过验证；但"在这台机器上再跑一次探针"需要人工放行
（Defender → 保护历史记录 → 允许该项，或给 `target\` 加排除项）。
这是本会话里明确**没有**自动去做的机器级改动。

### 32.4 后续（0.1.12 的实测补充）

重新构建（0.1.12，与 0.1.11 同一份源码 + 反向滚动）之后，**新 exe 没有被拦**：
便携包的可移植性检查、`installer_e2e`、以及全部十个探针都跑通了。也就是说这次判定
更像是"针对那个具体文件的信誉/哈希"，而不是"识别出了我们的行为模式"。
**这不代表问题解决了**——对一个没签名的自启型工具，任何一次新构建都可能再次被拦，
所以路线图里"代码签名"依然是阻塞项，只是不再阻塞本会话的验证工作。


---

## 33. 商用化第十九轮：反向（往上）滚动截屏

原来用户往回滚，程序只会说一句"检测到往回滚了，只支持向下滚动"。这一轮把它做掉：
**往上滚同样拼接**，而且往回滚到已经拍过的内容时会正确识别"这段已经有了"，不重复。

### 33.1 关键概念：画布两侧的"余量"

拼接器只有一个不断变长的画布和"上一帧"。往上滚时新内容出现在当前帧**顶部**，
照着追加的逻辑对称写一个前插就完事了吗？不行——用户在**已经拍过的区间**里来回滚时，
那会长图里出现重复段落（单测里断言"来回滚之后总高度"时立刻暴露）。

所以给画布加两个量：

| 量 | 含义 |
|---|---|
| `head_room` | 画布在**当前帧上方**已经存了多少行 |
| `tail_room` | 画布在**当前帧下方**已经存了多少行 |

* 往下滚 `offset`：真正要追加的是 `offset - tail_room`；然后 `head_room += offset`、`tail_room -= offset`；
* 往上滚 `offset`：真正要前插的是 `offset - head_room`；然后 `tail_room += offset`、`head_room -= offset`。

差为 0 时返回 `Unchanged`（"画面没变化"），画布一点都不长。被长度上限截掉的部分仍然记在余量里，
所以之后接着滚还能继续接上。

### 33.2 顺带抓出一个真 bug：切换方向之后的帧没有转置

写这轮单测（要求**精确**宽度/高度）时暴露出上一轮的一个 bug：`DirectedStitcher` 在切换到横向
之后，仍然把**未转置**的帧喂给内层拼接器——尺寸对不上，于是每一帧都被丢掉。
症状是"横向长图只涨过切换那一次就不动了"；上一轮那个横向探针只断言"比选区宽"，
于是它"通过"了（760 = 700 + 切换那一次 60）。现在统一走 `for_inner()`，并且单测断言精确尺寸。

### 33.3 验证

`crates/cuter-core/src/scroll.rs` 里新增/加强的测试（**逐像素**与真值比对，不只看尺寸）：

| 测试 | 断言 |
|---|---|
| `stitcher_prepends_when_scrolling_backwards` | 前插 40 行，内容顺序"更早的在上"，与真值逐像素一致 |
| `reverse_vertical_scroll_prepends_content` | 四帧一路往上：120×250，与真值一致 |
| `reverse_horizontal_scroll_is_detected_and_prepended` | 往回横滚也能判出横向：250×120，与真值一致 |
| `scrolling_down_then_back_up_covers_the_whole_range` | 先下两帧再上四帧（越过起点）：恰好 120×340，无重复段落 |

（写这些测试时还踩了一次自己的坑：步长取成"正好一屏"时两帧毫无重叠，位移估不出来——
真实用户也不会这么滚，测试步长改成 60/100 视口高。）


---

## 34. 商用化第二十轮：把"冷启动"摊开量一遍（以及结论：瓶颈不在我们的代码里）

本项目的卖点之一是"轻快"，那启动耗时就不能靠感觉。这一轮先把**时间线打点**做进程序，
再拿数据决定要不要优化——测完的结论是：我们自己的部分已经很干净，剩下的大头在机器那边。

### 34.1 打点方式（关键：不能从 `main()` 开始量）

`main()` 执行时 CRT 与 exe 装载早就完成了，从 `main()` 里打点会**系统性漏掉**用户真正等待的
那一段。所以 `desktop::since_process_start_ms()` 问系统要**进程创建时间**
（`GetProcessTimes`），两个时间戳都走 `GetCurrentProcess`+`GetSystemTimeAsFileTime`
同一只时钟，不做时区/纪元换算。七个阶段各打一行：

```
启动时间线[main 进入] / [配置已读] / [单实例守卫就绪] / [交给 eframe（含窗口与 GL 上下文）]
/ [配置已读②/语言已定] / [热键注册完成] / [第一帧]
```

* 诊断页会显示 `启动耗时（进程创建 → 第一帧）：N ms` —— 用户能直接报出自己机器上的数字；
* `latency_probe.ps1` 断言时间线**存在且数值合理**（50 ms ~ 30 s），并按顺序打印
  `startup: phase N = +xxx ms`（只打 ASCII：阶段名是中文，控制台代码页会把它们变成乱码）。

### 34.2 实测（同一台机器，5 次，取中位）

| 阶段 | 中位耗时 | 说明 |
|---|---|---|
| 进程创建 → main + 日志 + 配置 + 单实例守卫 | 221 ms | 含 exe 装载与 CRT |
| eframe（窗口 + GL 上下文） | +126 ms | winit + 驱动，我们控制不了 |
| 托盘图标 + 7 个全局热键 | +1 ms | |
| 第一帧（egui 上下文 + 首次布局） | +16 ms | |
| **合计：进程创建 → 第一帧** | **364 ms** | |

同一批 5 次里还出现过 **1.5 s / 2.3 s** 的单次启动：那是杀软扫描（见第 32 节），
不是我们的代码。对照实验很直白——**这台机器上连 `cmd.exe /c exit` 都要 1172 ms 才跑完**，
而我们的 exe 走 `--help`（装载 + CRT + 日志 + 参数解析）热态只要 **85 ms**。
也就是说：如果用户在慢机器上抱怨启动慢，最该怀疑的是杀软，而不是这个程序。

### 34.3 试过但没用的（如实记录）

* `vsync: false`（想压"按下热键到看见遮罩"的延迟）：**eframe 0.36 的 `NativeOptions`
  已经没有这个字段**，编译不过；实测 `eframe → 第一帧` 稳定在 ~140 ms，
  那一段是 winit + GL 上下文创建，不是交换缓冲的等待，关 vsync 也压不到；
* `depth_buffer` / `stencil_buffer` / `multisampling`：eframe 的默认值**本来就是 0**，
  没有可省的空间（读源码确认 `epi.rs` 的 `Default`）。

### 34.4 结论

* 我们自己的启动路径：**装载 + CRT ≈ 85 ms，eframe ≈ 126 ms，托盘 + 热键 ≈ 1 ms，第一帧 ≈ 16 ms**；
* 想要再压，只能绕开 eframe/winit（等于重写窗口层），收益上限 ~126 ms，代价是全部 UI 代码 ——
  不值得；
* 真正会毁掉"看起来很快"的是杀软（可以到秒级），这在路线图里由**代码签名**解决。


---

## 35. 商用化第二十一轮：托盘待命时那 0.6% GPU 是怎么来的

用户在任务管理器里看到"Cuter"待命时**一直占着 0.6% GPU**（CPU 0%、内存 5 MB、电源"非常低"）。
0.6% 很小，但它是**持续**的——说明有东西在无休止地重绘。这一轮把它查清、修掉，并留下一个探针。

### 35.1 病灶：一句无条件的 `request_repaint_after(120 ms)`

`logic()` 结尾原来是：

```rust
// 兜底轮询：即使没有任何输入事件，也要定期检查热键与托盘队列
ctx.request_repaint_after(Duration::from_millis(120));
```

它诞生于"热键/托盘队列靠轮询"的年代。后来热键与托盘都在各自线程上 `request_repaint()`
**立即唤醒** UI（`desktop::UiWake`），OCR 结果也一样（worker 发完消息就唤醒），
于是这句心跳只剩下副作用：**每秒 8 帧，每帧都要清屏 + 交换缓冲**，
在任务管理器里就是那条"常驻 0.6% GPU"。

实测（新探针 `idle_gpu_probe.ps1`，托盘待命、15 秒采样本进程的 GPU 引擎计数器）：

| | GPU 均值 | GPU 峰值 | 15 秒 CPU |
|---|---|---|---|
| 改前 | **0.734 %** | 0.773 % | 0.281 s |
| 改后 | **0.079 %** | 0.080 % | 0.234 s |

**降了 9.3 倍**，任务管理器里从 0.6% 变成 0.0~0.1%。

### 35.2 但"完全不画"是错的（差点把功能改坏）

第一版把它改成"只在真的有活干时才请求下一帧"。结果 `pin_groups_e2e` 立刻红了：
贴图窗口一个都不出现。原因有两层，都值得记住：

1. **eframe 对不可见窗口的重绘，是靠"有一个待处理的重绘请求"来排队的**——
   一个请求都没有时，连热键那次唤醒都不一定能换来一次 pass；
2. 贴图窗口是**延迟视口**，它要好几帧才落地：窗口要下一帧才创建，
   而 `self.pins` 也要等那个闭包跑起来才变成非空——"非空"恰恰是继续出帧的条件之一（鸡生蛋）。

这不是猜的：把 0.1.13 的便携包解压出来，同一个探针在**旧二进制**上是 `1,0,1,1`（预期），
在新二进制上全是 `0`。定位问题的过程中还踩了自己两个坑，一并记下：
* 用 PowerShell 的字符串替换加诊断日志时**因为 CRLF 静默失配**，日志根本没编进去，
  于是"热键命令数为 0"这个结论是假的（后来改用可靠的编辑方式重做，
  实测新构建下 Alt+C 能正常取到 3 次命令）；
* 因此**先 A/B 再下结论**：拿旧产物跑同一个探针，是最快的"是回归还是环境"判据。

### 35.3 最终做法：心跳分两档

```rust
let heartbeat = if busy { 120 } else { 1000 };   // busy = 有视口/遮罩/捕获/状态在忙
ctx.request_repaint_after(Duration::from_millis(heartbeat));
// 另外：处理过托盘/热键命令之后 1.5 秒内，用 wake（post 消息）顶帧，
// 让贴图这类"要好幾帧才落地"的延迟视口及时出现。
```

"始终排一个下一帧"这条**保留**了（可靠性优先）；只是把"什么都不做"那一档从 120 ms 放到 1000 ms。

**这个改动对热键延迟有没有代价？** 做了两轮受控 A/B（同一个二进制，只改这一个常数，
各 8 轮，取中位）：

| 空转心跳 | 热键 → 遮罩中位（第一轮） | （第二轮） |
|---|---|---|
| 1000 ms | 403 ms | 394 ms |
| 120 ms | 378 ms | 386 ms |

差异在噪声内（今天这台机器上杀软很活跃，绝对值整体偏高——见第 34 节：
连 `cmd /c exit` 都要 1.1 s）。所以**这一档是白拿的**。

### 35.4 验证

* 新探针 `tools/probe/idle_gpu_probe.ps1`：托盘待命采样本进程 GPU 引擎计数器，
  断言均值 < 0.25%、峰值 < 2%；顺手打印 CPU 时间增量（两个角度看"它还忙不忙"）。
  驱动/计数器不可用时**明确 SKIPPED**（exit 2），不假装通过。
* `pin_groups_e2e` 连续两轮恢复为预期序列 `1,0,1,1,0,1`；
  `latency_probe`、`pin_drag_out`、`tray_close`、`single_instance`、`autostart`、
  `i18n`、`overlay_select`、`control_snap`、`ocr_linkage`、`leak` 全部 PASS。
* 277 个单测 + clippy 零警告。


---

## 36. 商用化第二十二轮：把"进度"本身弄准 + 托盘气泡通知

### 36.1 roadmap 与实现对齐（11 处）

`docs/roadmap.md` 的 M2/M3 段落里还挂着一批**早就做完但没打勾**的条目（控件级吸附、
拖拽落盘、OCR 双向联动、冷启动、字体按需装载…），而且有的同一件事被写了两遍。
"进度文档不准"本身就是一种技术债——它会让人重复劳动。

这一轮把它们逐条对齐（脚本 `tools/docs/align_roadmap.py`，每处替换都必须命中，
不然报 MISS，避免"看着改了其实没改"）。对齐后剩余未完成项 **12 条**，其中大部分
**卡在外部条件**而不是代码：

| 项 | 卡在哪 |
|---|---|
| 代码签名 | 需要证书；已被 Defender 拦过一次执行（第 32 节） |
| OCR 在线引擎 / 屏幕翻译 | 需要真实 HTTP 端点 |
| 多显示器混合缩放 | 开发机只有一块屏，做了也无法验证 |
| 自动更新检查 | 需要更新源 URL |
| 托盘气泡通知 | 本轮做掉了（见下） |
| 图片圆角、自动滚动模式、纹理释放、CI 化探针、quad 竖排 | 能做，只是还没做 |

### 36.2 新增 `CHANGELOG.md`

按版本倒序，只写**用户能感知的变化 + 实测数字**；每条都指向 `HANDOFF` 的对应小节。
数据来自 `git log`（版本号变更历史）与各轮文档，**不凭记忆**。
顺带把"还没做的"整理成一张表，写清每一条卡在哪。

### 36.3 托盘气泡通知（那个被删掉的假开关，现在是真的）

`general.show_tray_notifications` 曾经是"配置里有、代码里没有"的假开关，被删掉过；
这一轮真做掉了：**启动驻留托盘时打个招呼**（顺便把热键告诉用户——"装完了不知道按什么"
是这类工具最常见的抱怨），以及**每次成功保存后提示落在哪个文件**
（遮罩这时已经关掉、用户回到自己的程序里，气泡是唯一能告诉他"存哪儿了"的方式）。

实现上有个约束：`tray-icon` 只有图标/菜单/提示文字，**没有气泡接口**，
而且它的 `hwnd` 与 `uID` 都是私有的，没法往它那个图标上挂 `NIF_INFO`。
所以 `desktop::TrayNotifier` 注册一个**隐藏的辅助图标**（`NIS_HIDDEN` + 我们自己的
根窗口句柄 + 固定 `uID`）专门当气泡宿主：外观由系统画，现有托盘实现一行都没动。
失败只记日志——通知投放绝不能影响截图。

验证：新探针 `tray_notify_e2e.ps1` 跑两个方向——开关打开时启动问候被系统接受
（`delivered=True`），关掉时**一条通知日志都没有**（这才是"开关不是假的"的判据）。

**如实记账：这台开发机看不到气泡。** 我做了对照实验：用**可见**图标调
`Shell_NotifyIconW`（`added=True`、`shown=True`，API 全部成功）**同样什么都不显示**——
也就是说这台机器/这个会话整体抑制了托盘通知，与我们的实现无关。
所以探针断言的是"API 是否被接受 + 开关是否真的生效"，截图留给人看，
而我没有声称"我亲眼看到气泡了"。

### 36.4 收尾时发现的一个真探针 bug（以及两条今天跑不动的探针）

**真 bug：纵向滚动探针的配置漏了 `snap_controls = false`。**
控件级吸附是第 8 轮加的（默认开），它会**改拖框选区的边缘**；第 9 轮我给
`overlay_select_e2e` 关掉了它，却漏了 `scroll_e2e`。于是这个探针一直在"实际选区 ≠ 期望选区"
之间飘：有时报 `width mismatch 756 vs 840`，有时干脆给出别的尺寸——**症状看起来像随机抖动，
其实是配置漏了一项**。已补上（并加注释说明"这个探针要的是精确选区，两种吸附都得关"）。

**两条今天跑不动的探针（如实记录，不算通过）：**

| 探针 | 现象 | 证据 |
|---|---|---|
| `scroll_e2e.ps1` | `overlay did not appear after Alt+S` | 应用日志里热键**注册成功**（7 个，含"滚动截屏"），但整个运行期间**没有**任何"滚动截屏开始"记录 |
| `scroll_horizontal_e2e.ps1` | 落盘尺寸与选区不符（如 350×160 / 825×240） | 同上：注入的滚轮与 `Alt+S` 都没到达目标程序（探针自己打印的"target scroll counter 0 → 0"也是这个意思） |

判定"是环境还是回归"的依据：**同一时刻 `latency_probe` 用 `Alt+C` 连跑 4 轮全部通过**
（199~407 ms），说明热键通道本身是好的、`Alt+C` 能到；而 `Alt+S` 与合成滚轮在这台机器上
被吞掉。滚动截屏这个功能本身由 **31 个单元测试**（含逐像素比对、方向判定、反向拼接）覆盖，
并且今天早些时候在同一个二进制上端到端跑通过（840×978、760×320）。

所以这一轮的验证结论是：**13 / 15 个探针通过**，两条滚动探针因输入注入被吞而未能跑通，
功能本身由单测与更早的端到端结果兜住。这条不写成"通过"。

---

## 37. 商用化第二十三轮：文件名模板多两个占位符 + 贴图图片圆角

### 37.1 文件名模板：`{monitor}` 与 `{app}`

原来支持 `{date}` `{time}` `{datetime}` `{unix}` `{seq}` `{w}` `{h}`。这一轮加了两个
真正能帮上忙的：

| 占位符 | 含义 | 怎么来的 |
|---|---|---|
| `{monitor}` | 截图所在显示器序号（1 开始） | 按热键那一刻用**光标位置**判断（用户是在他正看着的那块屏上按的） |
| `{app}` | 按热键时前台窗口的**进程名**（不含扩展名） | `GetForegroundWindow` → pid → `QueryFullProcessImageNameW` |

`{app}` 必须**在按热键那一刻**取：等真正保存时前台已经是我们自己的遮罩/面板，
那时再问只会得到 `cuter`。所以 `begin_request()` 里就把它记进 `capture_app`。

顺带把渲染器的签名收了：参数从 `(pattern, now, seq, size)` 改成 `(pattern, &NameContext)`——
占位符每多一个就多一个位置参数，再多几个调用点就没法读了。CLI 路径用
`NameContext::minimal()`（无界面时没有"前台程序"这个概念，`{app}` 是空串）。

单测：`template_supports_monitor_and_app` 验证取值、空 `app` 不残留 `{app}` 字面量、
以及 `{app}` 里的非法字符照样被清掉（模板是用户写的，进程名干净不代表模板干净）。

### 37.2 贴图图片圆角（`pin.rounded_corners`，默认开）

阴影早就是圆角，图片本身还是直角，两者叠在一起会露出来。新增
`Frame::rounded_corners(frame, radius)`：**只改 alpha**、边缘按覆盖率做 1 像素抗锯齿、
半径夹到短边一半、`radius == 0` 走零开销快路径。贴图在**上传纹理时**裁一次
（之后每帧都是同一张纹理），半径按图片短边的 6% 自适应、夹在 2~24 px。
设置里改这个开关会立刻重建已有贴图的纹理，不用重新贴一次。

三个单测：四角透明 + 中心不动 + 颜色不变、`radius = 0` 原样返回与超大半径被夹住、
以及抗锯齿确实产生了半透明像素。

端到端用**像素**验证（`pin_rounded_probe.ps1`）：把一张纯红图贴上去，截贴图窗口，
比较四角与中心的颜色——

| `rounded_corners` | 中心 | 四角 | 结论 |
|---|---|---|---|
| `true` | `200,40,40` | `255,255,255` ×4（桌面透出来） | 4/4 角被裁掉 |
| `false` | `200,40,40` | `200,40,40` ×4 | 0/4，直角 |

**反向那一半才是"开关不是装饰"的判据。** 另外我把两张截图拼在一起看过：
左边圆角、右边直角，与预期一致。

---

## 38. 商用化第二十四轮：把界面字体在"没窗口要画"时卸掉（工作集 82 → 56 MB）

第 27 节把字体的**首次装载**改成了按需（托盘常驻 86 → 57 MB）。但那只解决了"第一次"：
字体一旦装进 egui 就一直挂在 `Context` 上，而 `LoadedFont` 还额外拿着 18.8 MB 的
`msyh.ttc` 原始字节——**开过一次设置窗口之后，它就再也不还了**。

### 38.1 实测（同一进程、WM_CLOSE 关窗、关窗后等 8 秒）

| 状态 | 工作集 | 私有 |
|---|---|---|
| 纯托盘（还没画过界面） | 57.3 MB | 28.5 MB |
| 设置窗口打开（字体已装载） | **82.1 MB** | 53.2 MB |
| 关窗 8 秒后（字体已卸载） | **55.8 MB** | 56.7 MB |

**工作集 82.1 → 55.8 MB（−26.3 MB）**，比"还没画过界面"时还低一点。

注意**私有内存没有回落**（53.2 → 56.7 MB）：Rust 把 `Vec<u8>` 还给了堆，但 Windows 会把
那几页**保持提交**、只是不再常驻（于是任务管理器里"工作集"下来了、"提交大小"还在）。
两个数字都写在这里，免得有人只看一个。

### 38.2 做法

`needs_font()` 早就算得出"现在有没有窗口要画字"（控件窗 / 遮罩 / 贴图 / 历史 / OCR /
滚动面板）。这一轮在它的反面加了一条**宽限 2 秒的卸载**：

```rust
if self.needs_font() {
    self.font_idle_since = None;
    self.ensure_font(ctx);
} else if self.font.is_some() {
    let since = *self.font_idle_since.get_or_insert_with(Instant::now);
    if since.elapsed() > Duration::from_secs(2) { self.release_font(ctx); }
}
```

`release_font` 里有一句是**必须的**：光把 `self.font` 置空不够，那份 `FontData` 的引用还在
`Context` 的字体表里——必须 `ctx.set_fonts(FontDefinitions::default())` 换回自带字体，
egui 才会真的把它丢掉（连带字形图集一起）。宽限 2 秒是防"关了又开"的抖动。

代价：下次要画界面时重装一次，实测 **29 ms**（与首次装载同价，见 27 节的 22 ms）。

### 38.3 验证（这次差点被自己的检查脚本骗过去）

* 日志链路：`界面字体已按需装载：…（耗时 38 ms）` → `界面字体已卸载（当前没有任何窗口需要画字）`
  → `界面字体已按需装载：…（耗时 29 ms）`，一装一卸一装，三种都出现过。
* **重新装载后中文没变方框**：我把第二次打开的设置窗口截了图逐字看过（分类导航、工具条、
  勾选项文案全是正常中文）。
* 中间踩的坑值得记：我第一版检查脚本用 `FocusDesktop()` + `Esc` 关窗，结果
  `control_visible` 一直是 `true`——**Esc 根本没送到窗口上**，于是"字体没卸载"这个结论是假的。
  换成已通过的 `tray_close_e2e.ps1` 用的 `PostMessage(WM_CLOSE)`（不依赖焦点）之后，
  卸载立刻出现。同一个坑我在 `perf_probe.ps1` 里也修了（它原来的"重新停放"阶段用的是
  `Esc`，数字不可信）。
* 那两个诊断日志（每帧打印 `needs_font` 的各项）已经把 `needs_font` 降成 `trace`。

---

## 39. 商用化第二十五轮：一条命令跑完全部探针（以及"遮罩关闭后还剩什么"的实测）

### 39.1 遮罩关闭后的留存：量清楚了，但**不值得再啃**

先量（同一进程，全屏截一张再 Esc 取消）：

| 状态 | 工作集 | 私有 |
|---|---|---|
| 基线：纯托盘，从未截图 | 58.0 MB | 28.8 MB |
| 遮罩打开（冻结帧 + 纹理都在） | 143.3 MB | 78.2 MB |
| 遮罩刚关闭 | 90.3 MB | 75.9 MB |
| 关闭 10 秒后 | 69.1 MB | 52.3 MB |

结论有三条，都要写清楚：

1. **冻结帧与它的纹理确实跟着遮罩一起没了**：143.3 → 90.3 那一下掉的就是它们。
2. 剩下的 +11 MB 工作集 / +23.5 MB 私有**不是我们的对象**：我查了日志，
   字体在这个循环里确实先装载（78 ms）、关闭后**确实卸载**了
   （`界面字体已卸载`）。也就是说残留来自 GL 驱动与 egui 纹理管理器的内部缓存，
   我们既看不见也控制不了。
3. 所以"遮罩关闭后主动释放纹理与 `Frame`"这条**没有可做的动作**——
   对象生命周期本来就是对的。与其写一段"看起来在释放"的代码，不如把这三个数字记下来。

### 39.2 本轮真正交付的东西：`tools/probe/run_all.ps1`

每次有意义的改动都应该把探针跑一遍，而"忘了跑某个探针"这件事在这个项目里**已经造成过真实损失**
（有一个探针连续三轮没人跑，坏着都没人知道）。现在是一条命令：

```
powershell -NoProfile -ExecutionPolicy Bypass -File .	ools\probe
un_all.ps1
powershell ... -File .	ools\probe
un_all.ps1 -Only pin      # 只跑名字含 pin 的
powershell ... -File .	ools\probe
un_all.ps1 -Quick         # 跳过慢的（perf/latency/leak/...）
```

它会自己发现 `tools/probe` 下所有 `*_e2e.ps1` / `*_probe.ps1`，逐个单独起进程跑（互不干扰，
各自带超时），最后打一张表并给出汇总：

* 退出码沿用各探针的约定：**0 = PASS，2 = SKIPPED，其它 = FAIL**；
* **SKIPPED 不算失败**，但会在表里明写 `SKIP`——它绝不能被当成"通过"；
* 只要有一个 FAIL（或超时），脚本自己以非零退出，可以直接挂进 CI 或提交前钩子。

这一轮就是用它跑的全量回归（同时算是对运行器本身的自测）。

### 39.3 第一次全量跑的结果：16 / 22 通过，以及"假阳性"长什么样

第一次全量（22 个探针）是 **16 通过 / 6 失败**。六个失败里：

* `control_snap_e2e` —— **纯级联干扰**：单独复跑立刻通过。原因是 22 个探针共用同一份用户配置
  和同一个已安装实例，前一个探针的应用进程还没退干净就会影响下一个。运行器因此加了
  "探针之间先杀掉残留 `cuter` + 停 1.5 秒"。
* `hide_own_windows_e2e` / `history_e2e` / `ocr_linkage_e2e` —— 需要**连续多次合成输入**
  （`history_e2e` 要连做四次截图），机器被压满时遮罩首帧从平时的 143~332 ms 掉到
  **2394 / 2360 ms**，探针的 2 秒等待窗口不够，于是报"遮罩还没出现"。
  我一开始怀疑是上一轮"字体卸载"引入的回归，于是做了**定向对照**：连续三轮
  "Alt+C → Esc → 等字体卸载 → 再来一次"，实测首帧 **766 / 546 / 215 ms**（其中含字体重装
  16~102 ms），与平时同量级 —— **排除**。
* `scroll_e2e` / `scroll_horizontal_e2e` —— 第 36.4 节记过的那两条（合成滚轮被吞）。

教训写在这里：**全量扫描要在一台"空"的机器上跑**，否则失败里混着环境噪声。
判据是——先看应用日志里的实测耗时（`遮罩首帧可见：+xxx ms`），
再看同一探针单独复跑是否通过；两个证据都指向环境，就不要去改代码。

---

## 40. 商用化第二十六轮：自动滚动模式（路线图上最后一个功能项）

### 40.1 它是什么，以及为什么默认关

滚动截屏一直是「程序抓帧、**用户自己滚**」。自动滚动把这半步也接管了：
程序自己合成滚轮事件，一帧一帧往下推，直到判定到底了为止。

**默认关**，有两个实打实的理由：

1. 合成滚轮**必须先把光标移到选区中央**（见下），用户会看到鼠标跳一下；
2. 一部分程序根本不理会合成滚轮（浏览器里的虚拟化列表、某些 Electron 应用）。
   遇到这种目标，自动模式会连着几帧没变化然后停下——这是**设计好的行为**，不是卡死。

面板上按 `A`（或点按钮）随时开关，`scroll.auto_scroll` 只是**新会话的初始值**。

### 40.2 为什么是 `SendInput` 而不是 `PostMessage`

`PostMessage(WM_MOUSEWHEEL)` 是「给某个窗口发一条消息」，现代程序大多不认；
`SendInput` 的事件会真正走一遍系统输入栈，落到**光标底下那个窗口**上。
代价就是那条：得先把光标移进选区，发完再放回原处（`GetCursorPos` / `SetCursorPos`）。

两个方向的增量都是**负**的，这不是笔误：`MOUSEEVENTF_WHEEL` 正数向上滚（我们要向下），
而 `MOUSEEVENTF_HWHEEL` 的符号约定是反的（正数向左滚、我们要向右）。clippy 会把它当成
「两个分支一模一样」，所以代码里写了注释说明。

### 40.3 「什么时候停」是一个能单测的状态机

`cuter_core::scroll::AutoScrollTracker`：

* 连续 `stall_frames`（默认 3）帧 `Unchanged` / `NoMatch` → **停**（到底了，或目标忽略滚轮）；
* `TooTall`（到长度上限）→ **停**；
* 转发次数到 400 → **停**（自保：绝不无限往别人窗口里灌事件）；
* 中间只要拼上了一帧，就把「没变化」的计数归零（动画、懒加载都会造成一两帧没变化）。

五个单测覆盖：一直有内容时继续、连三帧没变化时停、好帧会重置计数、
到长度上限与转发上限立刻停、`stall_frames = 0` 被当作 1。

### 40.4 端到端验证（新探针 `scroll_auto_e2e.ps1`）

探针只做三件事：打开一份 300 行的长文档 → `Alt+S` 框选 → 回车 → 按 `A`，
然后**读应用自己的日志**：

```
autoscroll-stop stalled=3 steps=35 height=4141（滚动截屏自动滚动结束）
steps=35  stitched height=4141
RESULT: PASS - the app scrolled the target itself and stopped on its own
```

三个数字就是整件事：`steps=35` 证明合成滚轮**真的滚动了目标**（被吞的话 3 步就停了）、
`height=4141` 证明拼接真的长上去了、`stalled=3` 证明它是**自己停的**而不是一直灌事件。
输入被环境吞掉时探针以 **exit 2 (SKIPPED)** 退出——不算通过。

顺带一个踩过的坑：探针脚本必须**纯 ASCII**，而这条日志本身是中文，于是给日志加了
`autoscroll-stop` 这个 ASCII 标记（PowerShell 5.1 会把无 BOM 的中文脚本按 GBK 读并解析失败）。

---

## 41. 商用化第二十七轮：贴图纹理预算（切走的贴图不再占纹理）

### 41.1 一次测量说明了一切

贴 3 张 1920×1080 的图，然后切到另一组贴图（`Alt+G`）：

| 状态 | 工作集 | 私有 |
|---|---|---|
| 基线（还没贴图） | 57.0 MB | 28.7 MB |
| A 组 3 张贴图（纹理已建） | 222.0 MB | 116.2 MB |
| **切到 B 组（A 组隐藏）** | **146.9 MB** | 116.0 MB |
| 切回 A 组（纹理重建） | 223.3 MB | 116.1 MB |

应用日志同时给出：

```
release-hidden-pin-textures count=3 mb=23（隐藏的贴图不再占纹理）
```

**−75 MB 工作集**，其中 23 MB 是我们的纹理（3 × 1920×1080×4）、其余是那些贴图窗口
本身的绘图表面。切回去 +76 MB —— 说明纹理是**按需重建**的，行为上没有任何变化。

（私有内存不动，还是那个老规律：释放的堆页保持提交、只是不再常驻。两个数字都写在这里。）

### 41.2 为什么"隐藏就该释放"是安全的

贴图纹理本来就是**惰性**创建的（`Pin::texture` 第一次要画时才 `load_texture`），
所以"丢掉纹理"只需要把 `Option<TextureHandle>` 置空：`TextureHandle` 一 drop，
egui 就会在下一帧把 GPU 纹理还掉。原始 `Frame` 一直留着（贴图本身没变小、没降质），
切回来时只是一次重新上传。

释放点就两个：`toggle_pins`（整组隐藏）与 `switch_pin_group`（切组），
两处都调 `release_hidden_pin_textures`，判据复用**唯一的可见性函数** `pin_should_show`
——不新造第二套"什么算隐藏"的规则。

### 41.3 超预算只提醒，不降质、不拒绝

`pin.texture_budget_mb`（默认 256，夹 32~4096）：纹理总量超过它就用托盘通知提醒一次
（掉回预算内会重置，"下次再超还能再提醒"）。

**为什么不强制降质**：4K 截图一张的纹理就是 33 MB，十几张几百 MB 很正常，
但把用户的图偷偷缩小、或者拒绝再贴，都是把工具的问题变成用户的损失。
所以只把事实告诉用户（"已占用 X MB / 上限 Y MB，关掉一些可以释放"），
而真正能自动省下来的那部分（隐藏的贴图）已经在 41.2 里悄悄省掉了。

---

## 42. 商用化第二十八轮：把"提交前必须过的检查"做成一条命令 + 一份能跑的 CI

### 42.1 本地门禁：`tools/ci/check.ps1`

以前"跑 clippy / 跑测试 / 查 i18n"是三条要记住的命令。**忘掉其中一条**在这个项目里
已经造成过真实损失（有个探针坏着连续三轮没人发现）。现在是一条：

```
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\ci\check.ps1
powershell ... -File .\tools\ci\check.ps1 -WithProbes      # 再跑一遍桌面探针（-Quick）
```

四步，每步一行、各自计时，**任何一步失败就非零退出**：

| 步骤 | 为什么它是门禁 |
|---|---|
| `cargo clippy --workspace --all-targets -- -D warnings` | 这个项目一直零警告，所以 `-D warnings` 就是标准 |
| `cargo test --workspace` | 348 个单测，全部无头，CI 也能跑 |
| i18n 表完整性 | 有缺失译文就拦下（`en_table.py` 还会报 UNUSED，那是已知误报：主题名走运行时字符串，所以只把 MISSING 当致命） |
| `cargo build --release` | 真正会被打包的就是它 |

踩到的两个坑都值得记：cargo 把进度写到 **stderr**，在 `$ErrorActionPreference = 'Stop'`
下 PowerShell 会把它当致命错误（必须临时改成 `Continue`，判定只看 `$LASTEXITCODE`）；
以及用 `cmd /c` 包一层，否则每次跑都刷一屏 `NativeCommandError` —— **一个没人愿意看的门禁
等于没有门禁**。

### 42.2 CI：`.github/workflows/ci.yml`

Windows runner 上跑上面四步里的前三步 + 构建。**探针故意不在 CI 里跑**，理由写在
workflow 的注释里：`tools/probe/` 里的东西会合成真实键鼠输入、创建真实顶层窗口、再去读
应用日志——它们需要一个**交互式桌面会话**，而托管 runner 没有；一个"静默跳过"的探针
比没有探针更糟。

也就是说，这套东西的分工是：**CI 保证代码是干净的、能编译、测试全绿；
真机上的 `check.ps1 -WithProbes` 保证它作为一个程序仍然是对用户可用的。**

---

## 43. 商用化第二十九轮：设计系统 v1 落地（主题 + 设置窗口外壳 + 通用页）

这一轮不是加功能，是**把界面按 `docs/design/DESIGN-SYSTEM.md` 重做**。用户的原话是
"现在的界面太丑"。

### 43.1 先做底座：主题令牌（`theme.rs`）

**保留全部既有字段名，只换取值 + 新增字段**（`surface_raised` / `surface_sunken` /
`accent_subtle` / `on_surface_faint` / `outline_strong` / `success` / `warning` /
`accent_hover`）。这样整个程序（遮罩、工具条、所有面板）一次性换成新配色，既有调用点
不会因为改名字而集体报错；然后才逐屏精修布局。深色与浅色**两套都更新**，不是只改深色。

`visuals()` 里把 egui 的默认值整组覆盖掉（含 `extreme_bg_color`、各种圆角、浮层阴影）——
egui 默认深色是**发黄的灰**，只改几个字段会出现"我们的卡片是冷的、原生控件是暖的"的廉价感。

### 43.2 设置窗口外壳

* 侧栏：12 项分 4 组、每项**矢量图标**（`painter` 画，不依赖图标字体）、
  选中 = `accent/subtle` 底 + 左侧 3px 强调条 + 强调色文字；版本号移到底部；
* 头部：从"四行都在说我是 Cuter + 7 个按钮横铺"变成 **页面标题 + 一句说明 +
  一个实心「开始截图」+「⋯」溢出菜单**；
* 卡片：明确的一档颜色（`bg/raised`）+ 1px 边框 + 10px 圆角 + 15px 标题。

### 43.3 这一轮踩到的两个真问题

1. **托盘模式下根窗口隐藏 → 它的 `logic()` 不跑 → `apply_pending_theme` 里的
   `set_visuals` 不执行**。后果是：系统是浅色、配置选了深色时，设置窗口的上半部分
   是浅色的、和深色卡片拼在一起。修法是在设置窗口**自己的绘制路径**里设一次视觉主题。
2. **`egui::Checkbox` 在深色下画出来是"浅底浅勾"**：方块的底色取自 `widgets.*.bg_fill`、
   勾取自 `visuals.selection`，两者撞在一起几乎看不见勾。改成自绘 18px 圆角方块
   （选中 = 强调色底 + 白勾），反而更短也更贴稿子。

另外我自己在改代码时犯过一次"按位置删除"的错（`const ALL` 是数组，结尾是 `];` 而我搜的是
`};`，一次删掉了 200 行），回滚后改成**精确文本替换**。这个教训值得记：**在这个仓库里，
删除代码不要用索引，用精确字符串 + 断言。**

### 43.4 通用页按稿落实

52px 行结构（标签 13px + 说明 11px 三级色 + 控件右对齐 + 行间 1px 分隔线）、
下拉框（30px、"凹"底、圆角 6、选中文字用正文色）、自绘开关、状态行移到底栏。

对照图：`target/tmp/design-vs-real.png`（上：设计稿；下：实际程序同机截屏）。

### 43.5 还没做的

其余 11 个设置页、OCR 面板、遮罩工具条、应用图标（`corner` 概念已定为 9 尺寸 `.ico`）、
浅色主题实测。零件（行 / 开关 / 下拉 / 卡片）已经就位，逐页推进即可。

---

## 44. 商用化第三十轮：应用图标重做（双皮肤）+ 设置窗口换掉系统标题栏

用户这一轮的要求（原话）："图标再加一个薄荷奶白为浅色主题，设置不同皮肤主题颜色，
图标替换为不同主题。请定期使用cargo clean 命令及时清理run时产生的缓存。"
再往前一轮（附图）指出窗口上方**系统自带的白条标题栏**与整体不和谐，要求一并统一。

### 44.1 图标：从「C 裁切角」换成「甘」形取景框，并做成两套皮肤

`crates/cuter-app/src/icon.rs` 里新增 `pub enum IconSkin { Purple, Mint }` 与私有
`struct Palette { bg_top, bg_bottom, glow, glow_alpha: [f32; 2], line, dot }`；
`render_for(skin, size)` 是唯一的绘制入口（`render` 只是 `render_for(skin(), size)` 的壳）。
几何一字未改，全部按 `docs/design/icons/viewfinder-badge.svg` 的 256 网格等比换算：

* 圆角方块 `radius = 58 * k`，底色从上到下渐变；
* 两圈同色系半透明描边当辉光 / 柔影（浅底上"发光"看不出层次，改成柔影才托得住方框）；
* 「甘」形框 `(inset, side, radius, width) = (76, 104, 18, 15) * k`，四角刻度宽 `13 * k`；
* 中心焦点半径 `14 * k`。

两套配色：**曜夜紫** 底 `#2A1650 → #0B0618`、线 `#8CF8FF`、焦点白，辉光 `#7DF9FF`
（α 0.10 / 0.18）；**薄荷奶白** 底 `#F6FCF9 → #D6EFE6`、线 `#14A88E`、焦点 `#0B5F52`，
辉光 `#0C8A78`（α 0.14 / 0.22）。

**深浅两套必须成对存在**：深紫底挂在亮任务栏上是一块脏斑，奶白底挂在深色任务栏上糊成一团。

### 44.2 皮肤怎么跟主题联动（以及踩到的启动时序）

`fn resolve_theme(kind, ctx)`（`crates/cuter-app/src/app.rs` 第 4838 行附近）解析出主题的同时
调 `icon::set_skin(...)`：浅色 → `Mint`，深色/跟随系统但系统说深色 → `Purple`。
`fn apply_pending_theme` 里比较 `self.icon_skin` 与当前皮肤，**变了才**换图标
（窗口图标发 `ViewportCommand::Icon`，托盘走新加的 `desktop::Tray::set_skin`）。

坑：**开机时 `main.rs` 建窗口和托盘都早于 `resolve_theme`**，两处拿到的都是默认紫色。
所以 `icon_skin` 初值固定写成 `IconSkin::Purple` 当哨兵，让浅色主题在**第一帧**就被判定为
"皮肤变了"，从而把任务栏与托盘图标换成奶白。这一条没有单测能覆盖（要真窗口），
靠"浅色主题启动后托盘图标是奶白"的肉眼确认。

托盘句柄原本叫 `_icon: TrayIcon`（`crates/cuter-app/src/desktop.rs` 第 1147 行）被主动丢掉，
这轮改名成 `icon` 并加了 `set_skin`——**要换图标就必须留着句柄**。

### 44.3 浅色主题配色改成"薄荷奶白"

`crates/cuter-ui/src/theme.rs` 第 103 行 `Theme::light()` 的强调色从蓝 `#3A7AE8` 换成薄荷青
`#14A88E`，并同步 `surface_raised / surface_sunken / accent_subtle / accent_hover /
handle_stroke / snap_line / outline`。理由：图标是薄荷、控件是蓝会显得割裂。
深色主题保持原来的蓝——两套皮肤因此一眼可辨（如果希望深色也换成紫色调，只动这一处即可）。

### 44.4 自绘标题栏 + 修掉头部「⋯」

* `crates/cuter-app/src/main.rs` 第 434 行附近的 `ViewportBuilder` 加 `.with_decorations(false)`，
  `with_inner_size([780, 596])`、`with_min_inner_size([560, 416])`。
* `app.rs` 的 `paint_control` 开头调 `paint_title_bar` 与 `paint_resize_border`：36px 标题栏 =
  16px 应用标记（`paint_app_mark`，按皮肤换色）+ 「Cuter」+ 右侧三个 46×36 按钮
  （`paint_title_button_glyph` 用线段/矩形矢量画 ─ / ☐ / ✕）。
* **去掉系统装饰就没有系统缩放边框**，所以自己按光标离客户区边缘 6px 内做八向命中测试，
  发 `ViewportCommand::BeginResize(dir)`；空白处 `drag_started` → `StartDrag`、
  双击 → 切最大化。
* 头部的「⋯」从裸 `ui.menu_button` 换成 `egui::containers::menu::MenuButton::from_button`
  包一个 32×32 幽灵按钮，并在 `right_to_left` 里**先加 ⋯ 再加主按钮**（先加的在最右），
  修掉上一版"⋯ 在主按钮左边、还是个空心方块"。

截图验证的坑：非 DPI 感知的 PowerShell 进程里 `GetWindowRect`/`ClientToScreen` 返回被
虚拟化的坐标（截图整体偏约 55px）；先 `SetProcessDpiAwarenessContext(-4)` 才拿到真实
客户区 `975×745`（逻辑是 780×596，系统 125% 缩放）。`Graphics.DrawImage` 用两个 `Rectangle`
的 4 参数重载会抛 `无法将"System.Object[]"…转换为"System.UInt32"`，要用
`DrawImage($src, 0, 0, $srcRect, [System.Drawing.GraphicsUnit]::Pixel)`。

### 44.5 i18n 这轮才真正全绿

跑 `cargo test -p cuter-ui i18n` 发现三项检查里有 14 条界面文案缺英文、5 条旧文案已无人使用、
1 条 `cuter-core` 枚举文案缺译。补 11 条、删 6 条（含 1 条因主题提示改文案而变成死条目的）。
排序判据也修对了一次：表是按**运行时字符串**升序（`\n` 是换行符，不是反斜杠），
用源码文本排序会把 `\n\n详细日志已经写到：\n{}` 排到 `[第 {} 组 {} 张]` 后面，测试会拦下来。

### 44.6 实测数字

* `assets/cuter.ico` 重新导出：7 个尺寸（16/24/32/48/64/128/256），113,736 字节；
  用 `[System.Drawing.Icon]::ExtractAssociatedIcon` 从 `cuter.exe` 取回来与 `icon.rs`
  的 32px 帧逐像素比对：**最大通道差 0，差异区域 None**（资源确实重新嵌入了）。
* `cargo test -p cuter-app`：60 passed / 0 failed / 1 ignored；`cargo test -p cuter-ui`：全绿。
* 磁盘：这次动手前 `target` 已经 **31.87 GB / 71000 个文件**（E: 只剩 36.9 GB），
  按用户要求在本轮末尾跑了 `cargo clean`。

---

## 45. 商用化第三十一轮：截图页按设计稿改造（卡片 + 行）

第 43 轮把设计系统的零件（`settings_row` / `settings_switch` / `settings_switch_row` /
`settings_select` / `settings_card`）做出来了，但当时只把「通用」「保存」两页换过去，
「截图」页还是一串原生 `ui.checkbox`：标签里塞着长括号解释，控件右对齐但行高参差。
这轮把这一页补齐，样式与前面两页完全一致。

### 45.1 改法

`crates/cuter-app/src/app.rs` 的 `fn section_capture`（原 3932 行起）整段重写：

* **捕获后端**一行：`settings_select`（`GDI` / `DXGI（桌面复制）`），说明行写
  「拿不到桌面复制时会自动回退 GDI」——原来这句是塞在标签里的括号。
* **7 个 `settings_switch_row`**：包含鼠标指针、显示放大镜与取色、截图前隐藏本程序窗口、
  吸附到单个控件（按钮 / 输入框）、框选吸附到窗口边缘、记住上次框选区域、冻结屏幕时播放提示音。
  每一项的"代价"从标签里挪到下面那行小字（例如「走系统 UI Automation，老机器上可以关掉」）。
* **延迟捕获**：`egui::DragValue`，`0..=5000`、`.suffix(" 毫秒")`。
* 卡片底部保留 `capture_backend_name()` 的「当前实际在用：{}（DXGI … 会回退 GDI）」。

### 45.2 零件在哪（都在 `crates/cuter-app/src/app.rs`）

| 零件 | 位置 | 作用 |
|---|---|---|
| `settings_row(ui, theme, label, hint, control)` | 326 行 | 52px 行高 + 底部 1px 分隔线，左标签/说明、右控件 |
| `settings_switch(ui, theme, &mut bool)` | 372 行 | 18px 自绘开关（不是系统复选框） |
| `settings_switch_row(...)` | 409 行 | 上面两个的组合 |
| `settings_select(ui, theme, id, current, options, render)` | 428 行 | 30px 凹底下拉 |
| `settings_card(ui, theme, title, body)` | 460 行 | 卡片外壳 |
| `section_general` | 3582 行起 | **调用范式**，新页照抄它 |

### 45.3 拆分带来的 i18n 增删

UI 文案从"标签 = 长句子"拆成 label + hint 两个短句后，旧的长键就没人用了：
`no_dead_translations` 立刻报出来。这轮删 4 条旧文案、新增 13 条英文，表内共 362 条，
7 个 i18n 测试全绿。批量改表用一次性脚本 `target/tmp/i18n_capture.py`——**按解码后的
中文键排序**写回（源码里 `\n` 是两个字符，排序判据见 44.5）。

### 45.4 编译坑

`settings_select` 的 `render` 闭包签名是 `impl Fn(T) -> String`，而
`cuter_ui::i18n::tr(backend.label())` 返回 `&'static str`，必须 `.to_owned()`；
忘了会在 `app.rs:3960` 报 `error[E0308]: mismatched types`。

### 45.5 抓图验证

`target/tmp/page_capture.ps1`：DPI 感知（`SetProcessDpiAwarenessContext(-4)`）、
点侧栏「截图」行的客户区坐标 `(100, 222)` 切页、抓客户区 → `target/tmp/page-capture.png`。
图里能直接核对：卡片「截图」、捕获后端行 + 下拉「GDI（BitBlt，兼容性最好）」、
7 行开关（每行标签 + 小字 + 右侧开关）、52px 行高与分隔线。

## 46. 商用化第三十二轮：设置窗口所有页按设计稿改完（卡片 + 行）

### 46.1 改了什么

把 `crates/cuter-app/src/app.rs` 里剩下的 10 个设置页从「原生 checkbox / horizontal 堆叠」全部换成卡片 + `settings_row` 行式布局：快捷键（`section_hotkeys`）、标注（`section_annotate`）、滚动截屏（`section_scroll`）、识别文字（`section_ocr`）、贴图（`section_pin`）、截图历史（`section_history`）、字体（`section_font`）、诊断（`section_diagnostics`）、关于（`section_about`），加上此前已改的通用 / 保存 / 截图，12 页至此统一。

每行统一「左侧标签 + 灰色说明，右侧控件」，行高 52px、底部 1px 分隔线。

### 46.2 零件与范式（都在 `crates/cuter-app/src/app.rs`）

- `settings_card(ui, theme, title, body)`：第 460 行，卡片外壳。
- `settings_row(ui, theme, label: &str, hint: &str, control)`：第 326 行，**返回 `()`**。要累加变更必须在控制闭包里写 `changed |= ui.add(...).changed();`；`hint` 传 `&fill!(...)` 或 `&format!(...)`（String 不取引用会报 `error[E0308]: mismatched types: expected &str, found String`）。
- `settings_switch_row(...) -> bool`：第 409 行，自绘开关，可以直接 `changed |= ...`。
- `settings_select<T: PartialEq + Copy>(ui, theme, id, current, options: &[(T, &str)], render)`：第 428 行。options 里的 `&str` 是下拉项文字，`render` 是收起时显示的文字，且必须返回 `String`（`tr(...)` 要 `.to_owned()`）。
- 参考实现：`section_general`（第 3582 行起）。

### 46.3 i18n 生成器有个一直没生效的坑

`tools/i18n/en_table.py --write` 生成的是 `crates/cuter-ui/src/i18n.table.rs`，但 `crates/cuter-ui/src/i18n.rs` 里一直留着一份旧的内联 `TABLE`，生成物根本没被编译进去——英文表长期停在旧版本，本轮补齐 84 条新文案、删掉 24 条死条目。

修法两处：`en_table.py` 写出的文件改成**单个表达式**（`&[ … ]`），`i18n.rs` 里改成 `static TABLE: &[(&str, &str)] = include!("i18n.table.rs");`。

另一个坑：`en_table.py` 的排序必须按 **Rust 解码后的字节序**。EN 里的键是源码字面量写法（`\n` 在 Python 里是两个字符），Rust 会把它解码成真新行，于是 `sorted(EN)` 的顺序和 Rust 里 `TABLE` 的字节序不一致，二分查找会漏条目（现象：`every_tr_key_has_a_translation` 报 `{}` 没有翻译、`table_is_sorted_for_binary_search` 失败）。见 `en_table.py` 里的 `_rust_sort_key`。

### 46.4 `tr!` 必须写成单行

`tools/i18n/extract_keys.py` 的正则 `tr!\(\"((?:[^"\\]|\\.)*)\"\)` 只认**单行** `tr!("…")`：写成三行会在提取时静默消失（既不进表也不被检查，英文界面继续显示中文）。查多行用 `tr!\(\s*$`（文件是 CRLF，`$` 前要留 `\s*`）。

### 46.5 抓图核对

`target/tmp/pages_capture.ps1`（开发辅助，不是探针）：一次启动，按侧栏行的物理客户区坐标逐个点击并抓客户区，输出 `target/tmp/page-<名字>.png`。侧栏行坐标（975×745 物理像素）：快捷键 149 / 标注 297 / 滚动截屏 335 / 识别文字 417 / 贴图 455 / 截图历史 515 / 字体 552 / 诊断 589 / 关于 665。

抓图顺手改掉两个真问题：诊断页的显示器明细原来是没有行的裸 label（现在每台一行：名字 / `1920×1080 @ 125%` / 主屏标记）；关于页的「界面」行值原来错写成 `capture_backend_name()`（现在「界面框架与渲染后端 → egui / eframe（glow 渲染）」）。

### 46.6 实测数字

`cargo test -p cuter-app` 60 passed / 0 failed / 1 ignored；`cargo test -p cuter-ui` 88 + 6 passed；i18n 表 429 条、0 MISSING、16 UNUSED（全是运行时字符串查表的误报：工具名、主题名、捕获后端名）。

## 47. 商用化第三十三轮：遮罩浮动工具条按设计稿对齐

### 47.1 设计依据

- `docs/design/mockups/overlay-toolbar.png`（700×300 高保真稿）：工具条高 48、按钮 34、圆角 12、实际宽 573；分组顺序 **9 种标注工具 | 线宽·颜色更多 | 撤销·重做 | 完成（强调色，唯一的实心按钮）**；数字键 1-9 切工具、Enter 完成、Esc 取消，快捷键写进 tooltip 不占界面。
- `docs/design/DESIGN-SYSTEM.md`：圆角档位 输入框 6 / 按钮 8 / 卡片 10 / 窗口与弹层 12；「工具条上只有一个实心按钮『完成』。实心 = 唯一强调，其余都是描边或纯色块」。

### 47.2 改了什么（都在 `crates/cuter-ui/src/toolbar.rs`）

- 按钮计数：`STYLE_BUTTONS` 2 → 3（颜色 + 线宽 + 更多），`ACTION_BUTTONS` 7 → 1（只剩完成），`SEPARATORS` 仍是 3。`desired_size()` 因此从 21 个按钮收到 16 个：34×16 + 9×3 + 6×2 = 583px。
- 「更多」从原来的第 4 组挪到样式组（颜色 / 线宽之后），第 4 组只剩一个实心按钮。
- `icon_button`：选中态由 `theme.surface_active` 实心底 + `Color32::WHITE` 图标改成 `theme.accent_subtle` 淡底 + `theme.accent` 图标，圆角改用 `metrics::CORNER_BUTTON`（设计系统的按钮 8）。
- 新增 `fn done_button(ui, theme, enabled, tooltip) -> egui::Response`：accent 实心（hover 用 `accent_hover`）、白色勾、点击 = `ToolbarAction::Copy`（与 Enter 同效）。
- `enum Icon` 新增 `Done` 与对应的 `draw_icon` 分支（两条 `line_segment` 画的勾，2.2px）。
- `fn more_popup` 高度 246 → 400，顶部加进 5 行：保存 (Ctrl+S) / 复制并完成 (Enter) / 贴到屏幕上 (Ctrl+T) / 识别文字 (Ctrl+O) / 滚动截屏 (Alt+S)。
- 工具按钮 tooltip 由 `tool.label()` 改成 `format!("{}（{} {}）", tr(tool.label()), tr("数字键"), index + 1)`。

### 47.3 坑

- `tr!` 宏只吃**字面量**：写成 `tr!(tool.label())` 会报 `error: no rules expected tool`，运行时要翻译得用函数 `crate::i18n::tr(tool.label())`。
- 改完必须重编再抓图：	arget/tmp/pages_capture.ps1 启动的是 debug 版，	arget/tmp/toolbar_capture.ps1 启动的是 **release** 版。只编了 debug 就去抓工具条，	arget/release/cuter.exe 还是旧的，抓出来的画面和改动前一模一样（吃过一次）。

### 47.4 实测

- `cargo build -p cuter-app`（debug 58.4s）+ `cargo build --release`（3m48s）均通过。
- 抓图核对（`target/tmp/toolbar_capture.ps1` + `target/tmp/crop_toolbar.ps1` 放大 2 倍）：分组、选中态淡底、末端实心勾都对上设计稿。
- i18n：英文条数 429 → 443（补 14 条），`tools/i18n/en_table.py --write` 重新生成 `crates/cuter-ui/src/i18n.table.rs`。

## 48. 商用化第三十四轮：选区提示合成一个胶囊

### 48.1 设计依据

`docs/design/mockups/overlay-toolbar.png` 里选区上方的提示是**一个**胶囊：「840 × 600　(130, 170)」（尺寸 + 坐标同框，中间一个全角空格），而实现里画的是两个——左下角尺寸、右上角坐标。

### 48.2 改了什么（都在 `crates/cuter-ui/src/overlay.rs`）

- `fn paint_size_chip`：两次 `chip(...)` 合成一次，文本用 `format!("{} × {}\u{3000}({}, {})", w, h, x, y)`（`\u{3000}` 是全角空格，和设计稿的间距一致）。
- 同一函数里 `let above = rect.top() - 22.0` 改成 `- 27.0`：原来胶囊底边正好压住左上角的控制点，现在留出 5px 空隙。
- `fn chip` 的胶囊圆角 `CornerRadius::same(4)` → `same(6)`（设计系统的「输入框/复选框 6」档）。

### 48.3 验证

- `target/tmp/toolbar_capture.ps1` 抓真机 → `target/tmp/crop_pill.ps1` 放大 3 倍：显示为「690 × 460　(430, 319)」单个胶囊，左上角控制点露在外面。
- `tools/probe/overlay_select_e2e.ps1` **RESULT: PASS**（单击选窗口 784x492 / 拖拽 240x160 / 框内重画 380x230），说明改提示不影响选框交互。

## 49. 商用化第三十五轮：OCR 结果面板按设计稿对齐

### 49.1 设计依据

`docs/design/mockups/ocr-panel.png`（980×620）：顶栏「识别结果」+ 元信息胶囊 + 右上「编辑全文 / 逐行对照」；左列冻结帧预览画编号框 + 底部说明；右列「识别文本」，每行「编号徽标 + 文本 + 复制」+ 提示框；底栏「识别 2 行，用时 55 ms」+「重新识别」「复制全部」（强调色实心，全面板唯一）。

### 49.2 改法（`crates/cuter-ui/src/ocr_panel.rs`）

- `OcrAction::Retry` 新增；`ui()` 开头处理 Esc → `Close`、Ctrl+Enter → `CopyAndClose`。
- 新辅助函数：`chip_label`、`mode_button`（选中 = `accent_subtle` 底 + `accent` 文字）、`primary_button`（`accent` 实心 + 白字）、`badge`（行号徽标）、`footer`（分隔线 + 行数耗时 + 两个按钮）。
- 逐行列表每行是一个 `egui::Frame`（圆角 8、`Margin::symmetric(8, 5)`），行本身 `interact(Sense::click())` 维持 hover 高亮与点击复制。
- 预览里每个文本块左上角画 accent 徽标；新增 `pub fn frame(&self) -> &Frame`，宿主 Retry 复制完图再调 `start_ocr`（避免 `error[E0502]` 借用冲突）。
- i18n：+7 / −3，表 447 条。

### 49.3 布局坑（两次都只能靠抓图发现）

1. 正文用 `ui.available_size().y` 铺满剩余高度，会把底栏顶到面板窗口之外（底栏直接看不见）。修法：`let body_height = (available.y - 80.0).max(80.0);`，两列都用它（先试 44.0 不够，还要算标题栏与分隔线）。
2. `ui.allocate_ui_with_layout(size, …)` 只按子 Ui 的**实际用量**占位，不会预留 `size`。所以每个正文块里要显式 `ui.set_min_height(body_height);`，底栏才会稳定贴底——空结果分支同样处理。

### 49.4 验证

- `tools/probe/ocr_linkage_e2e.ps1` **RESULT: PASS**（面板 1243×822；截图确认顶栏胶囊、视图切换、行徽标 + 复制、编号框、hover 联动；底栏贴底）。
- 注意：这个探针偶尔会因为这一次 OCR 出 **0 行**而误判（没有行可悬停），属于探针抖动，不是面板回归——复跑一次即 PASS。
- 抓图归档：`docs/design/shots/0.1.30-ocr-panel.png`。

## 50. 商用化第三十六轮：设置页三处收尾

### 50.1 改了什么（`crates/cuter-app/src/app.rs`）

1. **保存页质量行**：原来整行 `if is_lossy()` 才画，切格式时下面整片会跳。现在行始终占位，无损格式时右侧给灰字「不适用（当前格式无损）」。保留原来那条原则——不摆「拖了没反应」的假开关。
2. **关于页许可行**：`settings_row(ui, &theme, tr!("许可"), tr!("GPL-3.0-or-later"), |ui| …)`，右侧补一条灰字「GNU GPL v3 或更新」（原来是空的控制区）。
3. **字体页按钮位置**：`settings_row` 的控制区是**右对齐**排布的，**先加的控件靠右**。原来先加 `TextEdit`、后加按钮，渲染出来是「按钮 输入框」；把按钮那段移到前面，才变成设计要的「输入框 按钮」。

### 50.2 判定为「不是毛病」的一处

快捷键页「格式：修饰键 + 主键…」与识别文字页「使用 Windows 自带的 OCR…」是卡片内的整行说明。OCR 面板的设计稿本身就有同类的底部说明（`docs/design/mockups/ocr-panel.png` 左列底部注释），所以这两处保留，不塞进行内 hint。

### 50.3 i18n

新增 `不适用（当前格式无损）` / `GNU GPL v3 或更新`，EN 447 → 449。

### 50.4 验证

- `target/tmp/pages_capture.ps1` 重抓并逐页看图：字体页「输入框 … 重新探测」、关于页「GPL-3.0-or-later / GNU GPL v3 或更新」、保存页「JPEG 质量 / 不适用（当前格式无损）」全部符合预期；截图归档 `docs/design/shots/0.1.31-settings-save.png`、`docs/design/shots/0.1.31-settings-font.png`、`docs/design/shots/0.1.31-settings-about.png`。
- 顺手把「保存」页加进了抓图脚本（`'11-save' = 260`）。

## 50. 商用化第三十六轮：遮罩毛玻璃底板

### 50.1 设计依据

Figma 稿（`docs/design/mockups/overlay-toolbar.png` 那一批）写的是：**面板矩形取冻结帧像素，做两遍盒式模糊，再叠白 10%**，全程用我们自己的像素，不依赖系统合成器——Windows 上的 `DwmEnableBlurBehindWindow` / Acrylic 在不同版本上要么没有、要么掉帧，截图工具不能赌。

### 50.2 改了什么

- 新文件 `crates/cuter-ui/src/frost.rs`：`pub fn frost(base: &Frame, factor: f32, radius: u32, passes: u32, white: f32, dim: f32) -> Frame`。
  - 先 `base.scaled(factor)`（取 0.25），再 `out.blur(out.bounds(), radius, passes)`（radius 6、两遍），最后逐像素 `v = v * dim + (255 - v * dim) * white`。
  - 缩到 1/4 再糊是关键：贴回面板时放大插值刚好补足「奶玻璃」的柔，而代价只有几十毫秒。
  - 带 3 个单元测试：`blur_kills_high_frequency_detail`（128 棋盘格方差掉到 5% 以下）、`white_overlay_brightens_by_the_requested_ratio`（黑底叠白 10% ≈ 25.5）、`dim_darkens_by_the_requested_ratio`（白底压暗 50% ≈ 127.5）。
- `crates/cuter-ui/src/annotate.rs`：`Document` 加 `frost: Option<egui::TextureHandle>` 与 `pub fn frost_texture(&mut self, ctx: &egui::Context) -> &egui::TextureHandle`（懒加载，`TextureOptions::LINEAR` 让放大平滑）；常量 `pub const FROST_DIM: f32 = 115.0 / 255.0;` 与遮罩压暗层 `DIM_TINT = Color32::from_gray(115)` 对齐。
- `crates/cuter-ui/src/overlay.rs`：工具栏绘制前，用 `toolbar::desired_size()` + `toolbar::position()` 算出面板矩形，`expand(6.0)` 后画进 `egui::Order::Middle` 层（在 `Background` 的压暗层之上、`Foreground` 的工具栏之下）。
- `crates/cuter-ui/src/toolbar.rs`：主条 `.fill(theme.surface.gamma_multiply(0.82))`；「更多」弹层保持不透明。

### 50.3 两个坑

- **底板一开始看不见**：我先写成 `shrink(4.0)`，结果毛玻璃完全被工具条盖住，抓图上看不出任何变化。改成 `expand(6.0)` 露出一圈模糊边才成立——设计稿里那层本来就是「比控件大一圈」的垫子。
- **取纹理的借用顺序**：`frost_texture` 要 `&mut self.doc`，所以必须先算完 `panel`/`uv`（只读 `self.doc.size()`），再取纹理、再画；顺序颠倒会撞上 `error[E0502]`。

### 50.4 验证

- `cargo test -p cuter-ui`：91 + 6 passed（含 3 个 frost 测试）。
- `tools/probe/overlay_select_e2e.ps1` **RESULT: PASS**（单击选窗口 784x492 / 拖拽 240x160 / 框内重画 380x230）——毛玻璃是纯绘制层，没有抢输入。
- 抓图 `target/tmp/ovl-toolbar-zoom.png`（放大 2 倍）可见工具条外圈多了一层模糊亮边；归档为 `docs/design/shots/0.1.32-overlay-frost.png`。

## 51. 商用化第三十七轮：贴图右键菜单按设计稿重做

### 51.1 设计依据

先在 Figma 出稿（文件 `MYNbSQBWmSO7DVaRS7jY6t`「界面稿」页，画板 `38:230`「贴图右键菜单 · 设计稿」，菜单本体 `38:233`），设计系统 `docs/design/DESIGN-SYSTEM.md` 的弹层档位：**圆角 12、内边距 6、1px `border/subtle`、行高 32、hover 用 `accent/subtle`、破坏性/退出项用 `danger`**。

### 51.2 改了什么（都在 `crates/cuter-ui/src/pin.rs`）

- 新常量：`MENU_WIDTH = 200` / `MENU_HEIGHT = 288` / `MENU_ROW_HEIGHT = 32` / `MENU_SEPARATOR_HEIGHT = 9` / `MENU_PAD_X = 10` / `MENU_PADDING: i8 = 6` / `MENU_CORNER: u8 = 12` / `SLIDER_WIDTH = 72` / `SLIDER_HINT_GAP = 34`。
- 三个自由函数：`menu_row(ui, theme, label, hint, danger) -> (Rect, Response)`（左标签 + 右快捷键，hover 画 `accent_subtle` 圆角 8）、`menu_toggle(ui, theme, row, on) -> bool`（28×16 胶囊 + 白钮，`ui.interact` 点击翻转）、`menu_separator(ui, theme)`（1px `outline`）。
- `menu_ui` 重写：`let theme = self.theme;` 先取出（避免和 `&mut self.opacity` 撞借用）；行序为 复制 / 保存为… / 识别文字 → 分隔 → 不透明度（72px 滑杆 + 百分比）→ 鼠标穿透 → 始终置顶 → 分隔 → 尺寸信息 → 分隔 → 关闭贴图。
- 滑杆用 `ui.put(slider_rect, egui::Slider::new(&mut self.opacity, 0.1..=1.0).show_value(false))`；开关用 `menu_toggle`。
- i18n：补 `保存为…` / `关闭贴图`，删掉因改版而失效的 `复制 (Ctrl+C)` / `识别文字 (OCR)` / `关闭 (Esc)`（表 449 → 448）。

### 51.3 已知限制（下一轮修）

贴图窗口（约 250×130）比菜单（200×288）矮，**菜单下半截会被窗口边界裁掉**——这是老问题（老菜单 186 高同样会被裁），这一轮菜单变高之后更明显。两条修法：① 菜单打开时先发 `PinAction::Resize` 把窗口撑到装得下（关闭时还原，`PinAction::Resize` 在宿主里只改目标尺寸、不会缩放贴图）；② 把菜单画到独立视口。

### 51.4 验证

- `cargo test -p cuter-ui`：91 + 6 passed；`cargo test -p cuter-ui i18n` 7 passed。
- 真机抓图 `target/tmp/pin-menu-live.png`（开发辅助脚本 `target/tmp/pin_menu_capture.ps1`：备份配置 → 深色主题 → 剪贴板放图 → `cuter --pin` → 在贴图中心右键 → 截图 → 还原配置）显示前几行排版正确：行高 32、快捷键右对齐、hover 行 `accent/subtle`。

## 52. 商用化第三十八轮：贴图菜单不再被窗口裁掉

### 52.1 现象与实测数字

贴图窗口是按图片尺寸算出来的（240×120 的图 → 窗口 264×144 物理像素，含 12px 阴影留白），而右键菜单是 200 宽 × 288 高，比窗口还高 → 菜单下半截被窗口边界裁掉，真机只能看见「复制 / 保存为… / 识别文字」三行 + 分隔线。

### 52.2 改法（都在 `crates/cuter-ui/src/pin.rs`）

- 新字段：`menu_extra: Vec2`（为菜单额外撑出的尺寸，与 `window_size` 同单位；关掉归零）、`menu_restore: Option<Vec2>`（撑大之前的窗口尺寸，物理像素）。
- `fn grow_for_menu(&mut self, ctx, at)`：菜单画在 `at` 处，所以窗口至少要装得下 `at + 菜单尺寸 + 10`；不够就把 `base_window_size` 抬到 `need * ctx.pixels_per_point()`，并记下 `menu_extra = need - 当前窗口尺寸`、`menu_restore = 原尺寸`。已经撑过（`menu_restore.is_some()`）就直接返回，避免反复撑。
- `fn shrink_after_menu(&mut self, action)`：`menu_restore.take()` 存在就还原 `base_window_size`、`menu_extra` 归零，并发 `PinAction::Resize`。它在 `ui()` **帧首**、`menu_open == false` 时调用——放在这里才不会挤掉菜单行自己的动作（`action` 同一帧只有一个）。
- `fn content_rect()`：把 `window` 改成 `self.window_size - self.menu_extra`——**这一步是关键**，否则撑出来的空间会把贴图拉长。原点不变，所以贴图也不会移位。
- 右键分支里 `menu_open` 变成 `true` 时调 `grow_for_menu` 并发一次 `PinAction::Resize`；宿主那边只更新 `target_size`（`crates/cuter-app/src/app.rs` 的 `PinAction::Resize` 分支），不会缩放贴图。

### 52.3 验证

- 开发辅助脚本 target	mp 下的 pin_menu_capture2.ps1（备份配置 → 深色主题 → 剪贴板放 240×120 图 → `cuter --pin` → 在贴图中心右键 → 截窗口 + 截全屏 → 还原配置；写完必须补 UTF-8 BOM）。
- 实测窗口：264×144 → **330×180**，可见内容从 3 行变成 4 行（多出「不透明度」滑杆那行），抓图 target\tmp\pin-menu-live.png。贴图贴着屏幕右下角时，宿主会把撑大的尺寸按屏幕工作区夹住，所以增幅不是请求的 428×463。

### 52.4 仍待解决

贴图紧贴屏幕边缘（下方/右方没有空间）时窗口撑不开，菜单仍会看不全。彻底修法是**把菜单画到独立视口**（和 OCR 面板同一套 `show_viewport_deferred`），已经记在 roadmap 里。

## 53. 商用化第四十轮：截图历史面板改成卡片 + 行

### 53.1 设计依据

`docs/design/DESIGN-SYSTEM.md` 里历史面板还没出 Figma 稿，但设计系统本身给了全部零件：卡片圆角 10 / 按钮 8、hover 用 `accent/subtle`、破坏性操作用 `danger`、列表一条一条卡片。本轮按这套零件重排，不改任何行为。

### 53.2 改了什么（`crates/cuter-ui/src/history_panel.rs` 整份重写）

- 公开接口一字未动：`WINDOW_TITLE`、`HistoryAction { Copy, Pin, SaveAs, Delete, Clear, OpenDir, Close }`、`HistoryPanel::new(store, theme)`、`ui()`；宿主不需要改。
- 顶栏：标题 + 两个胶囊（`{} 张`、`human_bytes(total)`）+ 右侧「清空历史 / 打开目录 / 关闭」，点「清空历史」先变成「确定清空？ 取消」。
- 列表：每条一张 `row_card`（圆角 10、内边距 12/10），内含缩略图（44px 高、圆角 6）、时间、`尺寸 · 占用`，右侧四个按钮——「复制」实心强调，其余 28 高描边，删除用 `danger`。
- 空列表：一张 `surface_raised` 卡片给提示。
- 新私有零件：`chip` / `row_card` / `outline_button` / `primary_button` / `danger_button`；`row_card` 用 `ui.interact(rect, id.with(("history-row", rect.top() as i32)), Sense::hover())` 画 hover 描边。

### 53.3 i18n 的三个坑

1. `tools/i18n/en_table.py` 里的键是 **Rust 源码字面量形式**（中文写成 `\uXXXX`、换行写成 `\n`），所以**不能拿中文字面量去文件里搜**——用 `'共 {'` 搜一条都匹配不上。必须先 `codecs.decode(key, 'unicode_escape')` 再比。
2. `python -c "...中文..."` 在 Windows 控制台码页下传进去就坏了（中文比较恒不成立）；PowerShell 的 `Select-String` 用中文当模式同样不可靠。**一律写成脚本文件再跑**。
3. 改版留下的死条目要删：`共 {}`、`截图历史（{} 张）`，以及「这里只有你自己的截图（剪贴板里贴进来的图不会进来）。上下限与缩略图大小在「历史」设置里。」，否则 `no_dead_translations` 直接红。最终 EN 表 447 条，`{} 张` 补成 `{} shots`。

### 53.4 验证

- `cargo check -p cuter-ui` 无错无警告；`cargo test -p cuter-ui` 91 + 6 passed。
- `tools/probe/history_e2e.ps1` **RESULT: PASS**（面板 734×598；截图确实进历史、原图就是原图、上限会裁剪、0 表示关闭）。

## 54. 商用化第四十一轮：修历史面板行被撑爆

### 54.1 现象与根因

0.1.35 交付后抓真机图，一张缩略图占了一整屏：缩略图一行、时间与占用一行、按钮一行还在最底下，右边的按钮跑出窗口。

根因：`crates/cuter-ui/src/history_panel.rs` 里 `row_card` 的闭包**直接写在竖向布局里**（`ui.label` / `ui.vertical` / `with_layout(right_to_left)` 各占一行），egui 的父布局是竖向，于是每样东西都独占一行。修法：闭包第一层包 `ui.horizontal(|ui| { … })`，缩略图、文字列、按钮组才落在同一行。

顶栏按钮被挤出窗口是同一个原因的连带现象（行内容太宽把内容宽度撑大）。

### 54.2 第二个错：多行 `tr!` 抽取器看不见

面板里那句「这里只有你自己的截图（剪贴板里贴进来的图不会进来）。上下限与缩略图大小在「历史」设置里。」原来写成

```rust
tr!(
    "这里只有…"
)
```

`tools/i18n/extract_keys.py` 的正则只匹配**单行** `tr!("…")`，所以这句在报告里算 MISSING，而它的英文条目在上一轮被当成死译文删掉了——英文界面会露中文。改成单行后再把译文补回 `tools/i18n/en_table.py`（键用真实中文即可，Python 两种写法等价）。

### 54.3 别被「窗口大小」骗了

抓图一开始总看到内容比窗口宽一圈：面板窗口被上一次探针 `MoveWindow` 设成了 **734×598 物理像素**，而 egui 的 `with_inner_size([720.0, 560.0])` 是**逻辑点**——125% 缩放下窗口实际只有 587×478 点，内容自然溢出。抓图脚本里先 `MoveWindow(hwnd, 120, 120, 900, 700, true)`（= 720×560 点）再看，就正常了。

**教训**：抓 UI 图之前先把窗口尺寸设成「逻辑点 × 缩放」的物理值；egui 的窗口尺寸会被持久化，探针跑过之后可能留下一个不一致的尺寸。

### 54.4 验证

- `cargo test -p cuter-ui i18n` 7 passed；`cargo build --release` 2m08s 通过。
- 抓图 `docs/design/shots/0.1.36-history-panel.png`（900×700 物理 = 720×560 点）：顶栏标题 + 两个胶囊 + 右侧关闭/打开目录/清空历史；每张截图一条 60 像素高的行，右侧「复制（实心）/ 贴图 / 另存为… / 删除（danger）」。

## 55. 商用化第四十一轮：滚动截屏控制面板设计稿

### 55.1 为什么先出稿

roadmap 里「滚动截屏面板」一直标着「需要先出设计稿」：功能在 `crates/cuter-ui/src/scroll_panel.rs` 里已经齐了（`ScrollPanelInput` 有帧数、尺寸、方向、状态、暂停、自动、上限、预览），但没有设计基准，照代码改版面无据可依。本轮按设计系统出稿，代码对齐放下一轮。

### 55.2 稿子在哪

Figma 文件 `MYNbSQBWmSO7DVaRS7jY6t` 的「界面稿」页，画板 `40:230`「滚动截屏控制面板 · 设计稿」（1180×700，39 个子节点）。仓库里存导出图 `docs/design/mockups/scroll-panel.png`，同图另存 `docs/design/shots/0.1.36-scroll-panel-design.png`。

### 55.3 稿子定了什么

- 两个状态并排：A 进行中（面板 460×420）、B 等第一帧 / 已暂停 / 已达上限（面板 460×300）。
- 顶栏：20px 粗体「滚动截屏」+ 三个 6px 圆角胶囊（`{} 帧`、`{}×{}`、`竖滚`）+ 右侧「自动滚动：开 (A)」（accent 文字）与「取消」。
- 中间：拼接预览（圆角 8、等比缩放）；B 状态退化成两行提示文案。
- 底部：11px 状态行（`正在自动滚动…按 A 停下`）+ 键盘提示（`回车完成 · 空格暂停/继续 · Esc 取消`）；右下角唯一实心按钮「完成 (Enter)」（accent 底、白字）。
- 配色全部取设计系统：page `#1B1D22`、panel `#262931`、border `#3A3F4B`、raised `#2E323C`、text `#E8EAED`、dim `#9AA1AD`、faint `#6C7480`、accent `#4C8DFF`。

### 55.4 坑

- Figma 里子节点的 `x`/`y` 是**相对父 frame** 的；我按「绝对坐标」又减了一次画板原点，画板边界膨胀到 8448×1769，用一次 `use_figma` 把 39 个子节点 `+= board.x / board.y` 才复原。
- 后建的图形会盖住先建的文本：两个「完成 (Enter)」标签被 accent 按钮盖住，要 `board.appendChild(label)` 把它们提到最上层。
- 胶囊底色在 `#262931` 面板上用 `#2E323C` 几乎看不见，提亮到 `#3A3F4B`。

## 56. 商用化第四十二轮：贴图右键菜单挪窗（修 0.1.33 的已知限制）

### 56.1 现象与实测

0.1.33 把贴图右键菜单改成设计稿版面（200 × 288），但**贴图窗口比菜单矮**：240 × 120 的图窗口只有 264 × 144（含左右各 12px 阴影留白），菜单只能露出「复制 / 保存为… / 识别文字」三行。0.1.34 试过「把窗口撑大」，可贴图贴在屏幕右下角时窗口撑不开（宿主按工作区夹住），只到 330 × 180、能看到四行。

### 56.2 改法

- `crates/cuter-ui/src/pin.rs` 新增摆放计算：`struct MenuPlacement`、`fn menu_placement(local, outer, original, monitor) -> MenuPlacement`、`fn axis_fit(...)`，调用方只关心「这次要不要挪、挪到哪、挪完窗口多大」。
- 新增公开查询 `pub fn menu_shift(&self) -> Vec2`（内容补偿用的位移）、`pub fn menu_target_origin(&self) -> Option<Pos2>`（贴图希望的内框左上角，**egui 点**）、`pub fn menu_target_size(&self) -> Vec2`；`fn need_for_menu(&self, at: Pos2) -> Vec2`、`fn send_menu_shift(&mut self, ctx, origin, shift)`、`fn shrink_after_menu(&mut self, ctx, action)` 负责下发与还原。
- 挪窗走 `ViewportCommand::OuterPosition`；内容**补偿绘制偏移**（`content_rect` 用 `window_size - menu_extra` 一类算法），所以贴图看起来不动。
- 宿主 `crates/cuter-app/src/app.rs` 新增 `fn sync_menu_shift(pin: &Arc<Mutex<Pin>>, fix: &mut GeometryFix)` 与 `GeometryFix.pos_owned`：贴图自己负责位置时，宿主的 `apply_geometry_fix` **只对齐尺寸、不再下发位置**，否则两边同帧互相覆盖，实测最终位置偏掉半个位移、菜单又被裁成一行。归一化用贴图给的绝对内框左上角，避免逐帧累加跑飞。
- 装不下时按可用工作区降级（`menu_placement_degrades_when_the_screen_is_too_short` 覆盖）。

### 56.3 验证

- `cargo test -p cuter-ui` **96 passed**，其中本轮新增 5 个摆放测试：`menu_placement_keeps_a_roomy_window_in_place`、`menu_placement_grows_the_window_in_place_at_the_bottom_right_corner`、`menu_placement_degrades_when_the_screen_is_too_short`、`content_rect_stays_put_when_the_window_does_not_move`、`content_rect_compensates_the_window_move`。
- 真机抓图（`target\tmp\pin_menu_capture2.ps1`：`--pin` 起一个 240 × 120 的贴图再右键）：窗口 264 × 144 → **395 × 444**，菜单十行全部可见（复制 / 保存为… / 识别文字 / 不透明度 + 滑杆 + 100% / 鼠标穿透 / 始终置顶 / 240 × 120 · 100% / 关闭贴图），贴图本体停在原地没动。证据图 `docs/design/shots/0.1.37-pin-menu-live.png`。

## 57. 商用化第四十三轮：贴图手感收口（穿透逃生门 / 整行开关 / Ctrl+滚轮不透明度）

### 57.1 用户报的三个缺陷与复现结论

用户在 0.1.37 上手后报（原话，`m04887`，此处把工具名换成「同类工具」）：「第一：设置鼠标穿透后，之后贴图就不能对贴图进行右键单击。
第二：贴图不可移动。第三：点击始终顶置的关闭整体卡住不可操作。建议贴图功能这点去调研一下同类工具，
尤其喜欢鼠标滚轮就可以操控贴图放大缩小和透明度控制。」（第三处的错字按实测理解成「始终置顶」的开关）

探针 `target\tmp\pin_bug_probe.ps1` 与 `target\tmp\pin_fix_verify.ps1`（都用
`Add-Type -Path tools\probe\CuterInputProbe.cs`，屏幕物理坐标 = 逻辑点 × 1.25）实测：

- ① 真 bug：穿透打开后窗口带 `WS_EX_TRANSPARENT (0x20)`，收不到任何鼠标消息，菜单里那颗开关自己也点不到——
  而提示文案还写着「在菜单里可关闭」，是一句无法兑现的承诺。
- ② 没复现：图片区和阴影留白那一圈都能拖（`drag(image): moved=-160,-120`，与请求一致）。用户遇到的
  「不能移动」是 ① 的连带现象：穿透态下既不能右键也不能拖。
- ③ 没复现：点「始终置顶」开关后 `exstyle` 由 `0xC0138` → `0x40110`（`WS_EX_TOPMOST` 已清掉），
  连打 5 次 `WM_NULL` 都是 `responding=True`、0~1 ms。用户看到的「卡住」是**取消置顶后被别的窗口盖住**。
- 附加证据：`target\tmp\pin-bug1-passthrough.png` 里那句「已开启鼠标穿透（在菜单里可关闭）」
  正好压在「关闭贴图」行上（提示画在窗口底边，菜单展开后窗口底边落在菜单里）。

### 57.2 改法

- `crates/cuter-ui/src/pin.rs`：`fn menu_toggle(ui, theme, row, row_response, on)` 的命中判定改成
  `if switch.clicked() || row_response.clicked()`（原来行标签的 response 被丢掉，只有右侧 28×16 胶囊能点）；
  新增 `fn adjust_opacity(&mut self, scroll: f32) -> bool`（步长 `OPACITY_WHEEL_STEP = 0.05`、
  夹在 `MIN_OPACITY = 0.1..=1.0`）与输入段 `if ctrl && scroll.abs() > 0.1 { … } else if self.zoom_by(scroll) …`；
  `pub fn set_status` 换成 `pub fn toast(&mut self, ctx, text)`（`status_until = now + TOAST_SECONDS(1.8)`、
  到期自动清、`menu_open` 时不画），新增 `pub fn set_click_through(&mut self, enabled: bool)`。
- `crates/cuter-app/src/app.rs`：6 处 `set_status` 全改 `toast`；新增
  `fn clear_pin_click_through(&mut self, ctx: &Context)`（先 `collect` 成 `Vec<Shared<Pin>>` 再逐个上锁，
  避免与「先锁贴图再读表」的路径死锁；同时清 `config.pin.click_through` 并 `persist_config()`）。
- `crates/cuter-app/src/desktop.rs`：`TrayCommand::ClearPinClickThrough` + 托盘菜单项
  「取消所有贴图的鼠标穿透」（托盘文案本来就是硬编码中文，不走 `tr!`）。
- i18n：`target/tmp/i18n_0138.py` 补 7 条、删 3 条死文案，entries 447 → 451，MISSING 0。

### 57.3 验证

- `cargo test -p cuter-ui` **98 passed + 6 passed**，新增 `opacity_wheel_steps_and_clamps`、
  `opacity_wheel_does_not_zoom`（默认不透明度 0.95 见 `crates/cuter-core/src/config.rs` 的 `opacity: 0.95`）。
- 真机 `target\tmp\pin_fix_verify.ps1`（`--pin` 起 240×120 贴图 → 在 `948,528` 右键）：
  `menu open rect=948,528,395,444` → **点「始终置顶」行标签 `topmost True -> False`，再点回到 `True`**
  （0.1.37 只有胶囊能点）→ 右击锚点左上收菜单 `rect=948,528,330,180`（`shrink_after_menu` 正常）→
  **`Ctrl+滚轮` 下滚 6 挡后开菜单读数 `240 × 120 · 70%`**（100% → 70%）。证据图
  `docs/design/shots/0.1.38-pin-row-toggle.png`（行内高亮 + 开关变灰）与
  `docs/design/shots/0.1.38-pin-ctrl-wheel.png`（菜单读数 70%）。
- 探针坑（复用价值高）：① 先 `Drag` 再右键，第一次点击会被挪窗循环吞掉；
  ② 脚本里调 `SetForegroundWindow` 会让之后所有合成点击失效（0.1.37 那个能用的老脚本恰好没调它）；
  ③ 贴图只从**自己的**键盘事件里学 Ctrl，`keybd_event` 发给的是前台窗口（终端），
  要验证 Ctrl+滚轮必须用 `PostMessage` 把 `WM_KEYDOWN(VK_CONTROL)` + `WM_MOUSEWHEEL` 直接投给贴图窗口；
  ④ pwsh 7 没有 `System.Drawing`，截图/裁剪得用 `powershell.exe -File` 走 5.1。
- 未机器验证：托盘「取消所有贴图的鼠标穿透」（托盘图标菜单不便合成点击）——代码路径复核 + 配置清理逻辑
  已读，作为下一轮的补验项写进 HANDOFF。

## 58. 商用化第四十四轮：滚动截屏控制面板按设计稿重写

### 58.1 依据与结构

设计稿是 Figma 画板 `40:230` 的导出图 `docs/design/mockups/scroll-panel.png`：460 × 420、圆角 12、
内边距 16，顶栏 20px 粗体标题 + 三颗胶囊（帧数 / 拼接尺寸 / 方向）+ 右侧 `自动滚动：开 (A)` 与 `取消`，
中间等比缩放的拼接预览，底部状态行 + 键盘提示 + 唯一实心按钮 `完成 (Enter)`。

`crates/cuter-ui/src/scroll_panel.rs` 整份重写，公开接口一字未动（`ScrollAction` / `ScrollPanelInput` /
`ScrollPanel::new` / `ui` / `WINDOW_TITLE`）。私有零件照抄截图历史面板那一套（`chip` / `outline_button` /
`primary_button` / `text_button`），尺寸常量走 `crate::theme::metrics`——**不是** `Theme` 的关联常量，
第一次写 `Theme::TEXT_LABEL` 是编译不过的。

窗口尺寸在 `crates/cuter-app/src/app.rs` 的 `SCROLL_PANEL_SIZE`：设计稿 460，实测真机上「滚动截屏」+
`12 帧` + `840 × 1113` + `竖滚` + `自动滚动：开 (A)` + `取消` 需要约 446 点，而 460 点减去左右内边距只剩
428 点，所以放宽到 **500 × 420**。

### 58.2 只有真机截图才看得见的两个版面错

1. **右侧开关骑在胶囊上**：`ui.horizontal` 里先排标题与胶囊、再用
   `ui.with_layout(Layout::right_to_left(..))` 排右侧，右侧那组拿到的是「已被左侧吃掉」的剩余矩形，
   字一宽就重叠。改成右侧先排（`right_to_left`），标题与胶囊嵌在它里面的 `left_to_right` 里。
2. **底部整块被推出窗口**：`ui.with_layout(Layout::bottom_up(..))` 是在**当前光标**处再自下而上排，
   而那时光标已经落在窗口底部，于是状态行与「完成」按钮都跑到窗口外面半行。改成普通自上而下，
   用「预览高度 = 可用高度 − 底部预留」保证不溢出（底部预留从 56 提到 72，无预览时的提示区 48）。

这两条都不是单元测试或探针能发现的：探针只看窗口出现与长图尺寸，`cargo test` 更看不到。

### 58.3 视口主题要钉死

滚动截屏面板是独立视口，有自己的 egui `Context`。`eframe` 默认跟随系统主题，只调 `set_visuals`
会被每帧覆回去——系统浅色 + 配置深色时，面板底色是浅的（走 visuals），而标题文字用的是
`self.theme` 的颜色（浅色），真机截图里「滚动截屏」几乎看不见。修法与主视口一致
（`crates/cuter-app/src/app.rs` 里已有同样的注释）：`set_theme(Light/Dark)` 与 `set_visuals`
两个都设，并把 `CentralPanel` 的 fill 换成 `theme.surface`。

### 58.4 i18n

面板文案换了一轮：补 7 条（`{} 帧`、`{} × {}`、`竖滚`、`横滚`、`已暂停 · 空格继续`、
`已达长度上限，新的滚动内容不会被再追加`、`长度到上限时会提示「新的滚动内容不会被再追加」`），
删 2 条死条目（`{} 帧 · {}×{} · {}`、`已暂停`）。`tools/i18n/en_table.py` 里的键是源码转义形式
（`\uXXXX`），比对前必须解码，写回时用 `json.dumps(..., ensure_ascii=True)` 保持同一种风格；
条目数 451 → 456，先跑 `extract_keys.py` 再跑 `en_table.py --write`。

### 58.5 验证

- `cargo test -p cuter-ui` → 101 passed + 6 passed（新增 `the_axis_chip_is_short`、
  `the_status_line_tells_what_is_happening`、`the_status_color_does_not_shout`）；
- 真机截图 `docs/design/shots/0.1.39-scroll-panel-live.png`（`Alt+S` 框选后抓面板本身）；
- `tools/probe/scroll_e2e.ps1` → `PASS: stitched 840x1113 starts with the exact selected content and is 1.86x taller`；
- 顺带修了这条探针自己的一个坑：遮罩刚出现时它还在被搬到工作区上，此时按下的合成拖拽会被记成
  一个很小的选区（实测拖 840 × 600 只换来 140 × 100，日志 `滚动截屏开始：区域` 的 x 还落在拖拽终点附近）。
  加 900ms 等待 + 「面板没出现就重做选区并按回车」的重试后稳定通过。

## 59. 商用化第四十五轮：每个 deferred 视口钉死主题 + 浅色主题真机核对

### 59.1 现象与根因

贴图窗口、识别结果面板、截图历史面板、滚动截屏面板都是 `show_viewport_deferred` 开的**独立视口**，
每个视口有自己的 egui `Context`。eframe 会按**系统主题**每帧把 visuals 覆写回去，所以
「只调 `set_visuals`」是不够的：`crates/cuter-app/src/app.rs` 主视口那段注释（"白底蓝字 + 深色标题栏"）
说的就是这个坑。

先踩到的是滚动截屏面板（0.1.39）：系统浅色 + 配置深色时，面板底色走 visuals（浅），
文字走 `self.theme`（深色主题的浅色文字），结果标题几乎看不见。

### 59.2 改法

在 `crates/cuter-app/src/app.rs` 里加两个自由函数：

* `fn pin_viewport_theme(ui: &egui::Ui, theme: &Theme, light: bool)`：`set_theme` + `set_visuals` 两个都设；
* `fn light_mode(kind: cuter_core::config::ThemeKind, ctx: &Context) -> bool`：`Auto` 时跟随系统。

四个视口（贴图 / 识别结果 / 截图历史 / 滚动截屏）的回调第一件事都是调 `pin_viewport_theme`，
带 `CentralPanel` 的三个再把 frame fill 显式设成 `theme.surface`。

### 59.3 验证

真机（配置 `theme = "dark"`、系统浅色 = 最容易串主题的组合）逐屏抓图：

* 贴图右键菜单：滑杆与开关按深色主题渲染，不再出现浅色控件 → `docs/design/shots/0.1.40-pin-menu-dark-on-light-system.png`；
* 截图历史面板：深色底 + 可读文字，`docs/design/shots/0.1.40-dark-history.png`；
* 滚动截屏面板：与 0.1.39 一致（那一轮已经修过）；
* **没拿到识别结果面板的真机图**：`Alt+O` 在遮罩里没有把面板调出来（截图停在选区状态，
  见 `target/tmp/dark-ocr.png`）。这一处的改动与另外三个视口完全同构，本轮只有编译与代码复核，
  真机图列为补验项。

`cargo test -p cuter-ui` 101 + 6 passed；`tools/ci/check.ps1` 全绿。

### 59.4 浅色主题核对

设计系统文档 §11 把「浅色主题全套」列为未覆盖。本轮在浅色（薄荷奶白）下真机抓了设置窗口与
截图历史面板：文字、胶囊、按钮、开关对比度都正常，没有发现需要修的对比度问题
（`docs/design/shots/0.1.40-light-settings.png`）。

## 60. 商用化第四十六轮：鼠标穿透的第二个出口 + 真机验证

### 60.1 为什么要加

开穿透的贴图窗口带 `WS_EX_TRANSPARENT`，它收不到任何鼠标消息 —— 菜单里那颗「鼠标穿透」开关自己也点不到，
用户一旦打开就只能靠**窗口外**的出口。原来只有托盘菜单一项（0.1.38 加的），但那一项一直没做过真机验证：
托盘图标在通知区域，合成点击要先精确点到图标，风险高、不稳定。

于是补一条命令行出口：`cuter --clear-pin-click-through`。它和托盘菜单那一项调用**同一个**函数
（`App::clear_pin_click_through`），所以验证这条链路等价于验证托盘那一项的处理逻辑。

### 60.2 改法

* `crates/cuter-app/src/single_instance.rs`：`Activation` 加 `ClearPinClickThrough`（`ALL` 从 6 项变 7 项）
  + `slug()` 加 `"clear-pin-click-through"`；
* `crates/cuter-app/src/main.rs`：新 CLI 开关 `--clear-pin-click-through`（进 `--help`）、
  转交分支映射到新 `Activation`；**本进程就是活着的实例**时顺手把配置里的 `pin.click_through` 清掉；
* `crates/cuter-app/src/app.rs`：`Activation::ClearPinClickThrough => self.clear_pin_click_through(ctx)`；
* 单测 `clear_pin_click_through_is_a_gui_activation`（走 GUI 路径而不是无界面子命令，否则转交不出去）。

### 60.3 真机读数（`target/tmp/escape_hatch_probe.ps1`）

```
1) started with click_through = true: exstyle = 0xC0138  (WS_EX_TRANSPARENT 0x20 set: True)
   pin rect = 945,0,864,1137
2) right-click while click-through is on: rect = 945,0,864,1137  (menu opened: False)
3) cuter --clear-pin-click-through -> Cuter 已经在运行（托盘里），已把这次启动转交给它。
4) exstyle now = 0x40118  (WS_EX_TRANSPARENT cleared: True)
5) right-click after the escape hatch: rect = 945,0,1080,1421  (menu opened: True)
6) config now says: click_through = false
```

读法：① 穿透开着时窗口确实收不到鼠标消息（右键菜单打不开，窗口尺寸不变）；② 一条命令之后
`0x20` 位被清掉；③ 同一个位置再右键，菜单又能打开（窗口按菜单需要撑大）；
④ 配置也被写回 `false`。

**仍然没验证的一点**：托盘菜单里那一行本身（点托盘图标 → 菜单 → 这一项）。它的处理逻辑与命令行
完全同一条，缺的只是"托盘菜单能不能点到"这一层；要真机验证得先精确点到通知区域图标。

`cargo test -p cuter-app` 61 passed；`tools/ci/check.ps1` 全绿。

## 61. 商用化第四十七轮：托盘右键菜单改自绘面板

### 61.1 为什么动它

托盘菜单是最后一块还在用系统原生控件（`muda` 的 `Menu`/`MenuItem`）的界面：字号、行高、圆角、
选中色都由 Windows 决定，和设置窗口、贴图菜单、历史面板不是一套观感；设计系统里也一直缺这一屏。

### 61.2 改法

* 新模块 `crates/cuter-ui/src/tray_panel.rs`：`TrayPanel` / `TrayPanelInput` / `TrayAction` /
  `PANEL_SIZE = (268.0, 407.0)`；行高、分隔、内边距都是常量，`Esc` 关菜单，10 行 3 分隔；
* `crates/cuter-app/src/desktop.rs`：`Tray::new` 不再建 `muda` 菜单，只留图标 + `TrayIconEvent`
  （左键抬起 = 框选截图，右键抬起 = `TrayCommand::OpenMenu`）；`TrayCommand` 加 `OpenMenu`；
  删掉因此没人用的 `fn with_hotkey`（留着会被 clippy 当死代码）；
* `crates/cuter-app/src/app.rs`：加 `tray_menu_viewport` / `tray_menu_actions` / `tray_menu_focused` /
  `pending_start_tray_menu` 四个状态；`open_tray_menu`（贴鼠标弹出、夹进工作区、`Visible` + `Focus`）、
  `close_tray_menu`、`handle_tray_action`（`TrayAction` → `TrayCommand` 再走 `handle_tray`，
  「关闭菜单」只收面板）；`Activation::TrayMenu`；
* `crates/cuter-app/src/main.rs`：新 CLI 开关 `--tray-menu`（进 `--help`、隐藏主窗）；
  `single_instance::Activation` 加 `TrayMenu`（`ALL` 从 7 项变 8 项）；
* i18n：`tools/i18n/en_table.py` 补 9 条（456 → 465 条）。

### 61.3 三个坑（都只有真机才暴露）

1. **探针必须声明 DPI 感知**：不调 `SetPhysicalDpiAwareness()` 时 `GetWindowRect` 给的是虚拟化逻辑
   坐标，而 `SetCursorPos` 要物理坐标，点击整体偏 1.25 倍（面板 335×509 被读成 268×407，点位也全错）；
2. **窗口句柄早于第一帧**：winit 先建 HWND、egui 之后才画，窗口一出现就截图会拍到空窗；
   探针改成"轮询找到窗口后再等 ~1 秒"；
3. **`App::needs_font()` 漏了托盘菜单视口**：`--tray-menu` 场景下主窗、遮罩、贴图、历史、OCR、滚动
   全是假 → 字体从不装载 → 面板上所有中文变成方块（每个 deferred 视口是独立的 egui Context）。
   加第七项 `self.tray_menu_viewport.is_some()` 后正常。

### 61.4 验证

* `cargo test -p cuter-ui` 104 + 6 passed；`cargo test -p cuter-app` 61 passed；
* 面板真机图 335×509 物理 = 268×407 点（没有溢出），中文全部正常：
  `docs/design/shots/0.1.42-tray-menu-live.png`；
* 点击「截图历史…」行（窗口顶 +374 物理像素，`设置…` 在 +340）→ 历史窗口出现（918×747）、
  菜单关闭、日志 `托盘菜单：面板返回动作 OpenHistory` → `托盘菜单：收到动作 OpenHistory`；
* 探针 `target/tmp/tray_menu_probe.ps1`：
  `RESULT: PASS - the self-drawn tray menu opened, a click chose an item, and the menu closed`。

**这一轮顺带补上了上一轮留下的缺口**：0.1.41 时"HANDOFF 第 60 节"里写着托盘菜单那一行点击没验证过，
现在菜单是自己画的了，这一层由上面的探针覆盖。

## 62. 商用化第四十八轮：放大镜取色读数区

### 62.1 为什么动它

`docs/design/DESIGN-SYSTEM.md` 第 11 节列出的"还没覆盖的界面"里，遮罩内的放大镜与取色器是最后一块
还看得出拼接感的：放大镜框本身是规范里的做法（`label_bg` 垫底 + `magnifier_border` 描边 + `accent`
当前像素格 + `crosshair` 十字线），可它底下的读数是"18 点色块 + 两个 `chip()` 胶囊"
（`#RRGGBB rgb(r,g,b)` 与 `(x, y)`），三者宽度各不相同，竖着码在一起像三块碎片。

### 62.2 改法

* `crates/cuter-ui/src/overlay.rs`：`paint_magnifier` 末尾改成调新的
  `fn paint_readout(&self, painter: &Painter, anchor: ERect, screen: ERect, px: [u8; 4], global: Point)`
  （`pixel()` 返回的是 RGBA，所以是 `[u8; 4]`，只取前三位画颜色）；
* 卡片与放大镜框**等宽**（`anchor.width().max(metrics::MAGNIFIER)`，高 76 = 内边距 8 + 色块 22 + 6 + 两行 32 + 8），
  `surface_raised` 底 + 1px `outline` + 圆角 `metrics::CORNER_PANEL`；22×22 色块（圆角 `CORNER_INPUT`、
  1px `outline_strong`）与 `#RRGGBB`（`TEXT_LABEL` 12px、`on_surface`、`Align2::LEFT_CENTER`）同一行，
  下面两行 `rgb(r, g, b)`（`on_surface_dim`）与 `(x, y)`（`on_surface_faint`），都是 `TEXT_CAPTION` 11px；
* 位置抽成纯几何 `fn readout_rect(anchor: ERect, screen: ERect) -> ERect`：默认挂在框下 `READOUT_GAP`
  （6 点），屏幕底部放不下就翻到框上方，两个方向都越界时按屏幕边缘夹住 —— 按仓库惯例，能测的逻辑
  不埋在 `paint_*` 里。

### 62.3 验证

* `cargo test -p cuter-ui` **107 passed**（新增 `the_readout_card_hangs_below_the_magnifier`、
  `the_readout_card_flips_up_near_the_bottom_edge`、`the_readout_card_stays_on_screen_in_the_corner`）+ 6 passed；
* 真机前后对比（`target/tmp/magnifier_shot.ps1` 起 `cuter --capture`、鼠标停在 (760,520)、截整屏再裁 380×380）：
  改前 `docs/design/shots/0.1.43-magnifier-before.png`（色块 + 两块胶囊），改后
  `docs/design/shots/0.1.43-magnifier-after.png`（一张卡片：色块 + `#FFFFFF`、`rgb(255, 255, 255)`、`(760, 519)`）。

### 62.4 两处明确不改（已写进设计系统第 11 节）

* **panic 提示框保持原生 `MessageBoxW`**：`crates/cuter-app/src/logging.rs` 的 `show_error_dialog`
  故意不走 egui —— panic 时 egui 未必可用，自绘窗口恰恰在最需要它的时候最容易失败；
* **托盘气泡不改**：本机被系统抑制（`docs/roadmap.md` 已记），拿不到真机图就无法按规范核对。

### 62.5 工具坑（这一轮踩到的）

* `target/tmp/crop_one.ps1`：`New-Object System.Drawing.Bitmap($w, $h)` 带变量时参数表会被当成类型名的
  一部分，报 `[System.String] does not contain a method named 'Dispose'` → 改用 `::new(...)`；
* **PowerShell 变量大小写不敏感**：局部变量不能叫 `$src` —— 那是 `[string]` 类型的参数 `$Src`，
  赋值会被强制转回字符串（错误信息只说"String 没有 Dispose"，很绕）→ 改名 `$img`；
* `magnifier_shot.ps1 -Out` 传相对路径时会被脚本里的 `Join-Path` 二次拼接成 `target\tmp\target\tmp\...`
  → 抓完整屏图自己再裁一次更稳。

## 63. 在线 OCR 引擎（0.1.44）

### 63.1 为什么自己写 HTTP

`config.ocr.http_endpoint` 与 `OcrEngine::Http` 从第一版就在配置里，但一直是"由宿主自己接"。
要接就得选一条路：拉 `reqwest`（连 tokio + TLS 一整套）还是自己写。这个项目的硬约束是体积，
而这里的需求窄到不能再窄——**发一个 POST，读一个响应体**：

* 请求：`multipart/form-data`，一个字段 `image`，内容是 PNG 的 base64（PaddleOCR 的
  `read_image` 见到字符串会按 base64 解，这是它服务端最常见的用法）；
* 响应：`{"results": [[[text, score, [[x, y] × 4]], …]], "status": "000"}`。

于是用 `std::net::TcpStream` 写了 `crates/cuter-core/src/ocr/http_ocr.rs`，base64 与 JSON
解析分别是它和同目录的 `json.rs` 里手写的。**代价是只支持 `http://`**：没有 TLS 实现，
填 `https://` 时 `split_endpoint` 会直接给出理由（而不是连上去握手失败）。

顺带处理了几件"不该自己写但必须写"的事：`Content-Length` 与 `Connection: close`、
读/写超时（15 s）、响应体上限（8 MB）、`Transfer-Encoding: chunked` 的解码
（Flask 常分块）、非 200 时把状态码与正文前 200 字放进错误信息。

### 63.2 解析回包

`parse_paddle_response` 认两种箱子写法：`[text, score, [[x,y] × 4]]`（PaddleOCR 原样）与
把四个点摊平在后面的 `[text, score, [x,y], [x,y], [x,y], [x,y]]`（有些封装这么给）。
`status != "000"` 时报错并带上 `msg`；`min_score` 在这里终于**真的生效**了——内置引擎拿不到
逐块置信度，所以那个设置在那边一直是摆设。

### 63.3 顺手修的一条老账：OCR 热键其实是死的

写端到端探针时先试的是"遮罩里按回车"（`ocr.auto_after_capture = true`），但合成输入到遮罩
一直不稳（Enter 有时根本没进去，日志里连"开始 OCR"都没有）。于是改用全局热键——结果发现
**热键列表里的「识别文字」(Alt+O) 与「贴图并识别」压根没接线**：`handle_hotkey` 里它们落进
`other =>`，只印一句"规划在 M3 实现"。接上之后那个 `other` 分支变成不可达（编译器的
`unreachable pattern` 警告就是证据），说明**九条热键至此全部有实现**。

* `Ocr`：识别剪贴板里的图（不贴图）；
* `OcrClipboard`：先按名字把图贴到屏幕上，再识别同一张。

### 63.4 验证

* 单测：`cargo test -p cuter-core` 149 项通过，其中 `http_ocr` 10 项包含一个**本机假服务端**
  的端到端用例（`TcpListener::bind("127.0.0.1:0")`，断言请求里带 `name="image"` 与这张 PNG
  的 base64）；base64 用了 RFC 4648 的 7 个标准向量。
* 真机：`tools/probe/ocr_http_e2e.ps1` + `tools/probe/fake_ocr_server.py`。探针把
  `%SystemRoot%\Web\Screen\img100.jpg`（3840×2160）放进剪贴板，配置成
  `engine = "http"` + `ocr_clipboard = "Alt+Shift+O"`，按热键后：
  * 假服务端收到 `path=/ocr bytes=11781024`、`has_image_field=True`、`has_png_base64=True`；
  * `Cuter 识别结果` 面板打开（1243×822），显示服务端返回的「在线接口识别成功 / HTTP OCR online」；
  * 截图归档 `docs/design/shots/0.1.44-ocr-http-live.png`（这张正好补上了"识别结果面板深色主题
    真机图"这个悬了几轮的缺口）。
* 探头踩坑：`-notmatch 'name="image"'` 判断的是**日志内容**而不是请求体，而日志里写的是
  `has_image_field=True`——断言第一版写错了，改成断言日志行本身。

### 63.5 还没做的

* **没有对着真的 PaddleOCR 服务验过**：只验了协议形状（multipart 字段名、base64、PaddleOCR
  回包结构），服务端是假的。
* 竖排 / 旋转文本（`TextBlock.quad`）仍然只有在线引擎会给出四个不一样的点，内置引擎只给
  轴对齐矩形（roadmap 第 114 行）。

## 64. 遮罩瞬时状态收口 + 长截图进度条（0.1.45）

### 64.1 先真机核对，再决定改什么

`DESIGN-SYSTEM.md` 第 11 节最后一条留的是"遮罩其余瞬时状态（拖拽中尺寸提示 / 吸附预览线 /
长截图进度）"。逐个核对的结果是**前两个本来就与令牌一致**，所以这一轮的价值主要在"证实 + 收口"，
而不是重写：

* 拖拽中尺寸提示：`paint_size_chip`（`crates/cuter-ui/src/overlay.rs`）把尺寸与坐标合成一个胶囊
  （`540 × 331　(760, 522)`），锚在选区左上角上方 27 点（22 → 27 是给左上角控制点让位），
  创建/移动/缩放三种手势下都在；
* 吸附预览线：`active_snap.is_snapped()` 时画 1px `theme.snap_line` 贯穿整屏（横竖各一条），
  与令牌表里的 `snap/line #4C8DFF` 一致。**参考线永远与选区某条边共线**（吸附的定义就是这样），
  所以截图里它看起来"只是选区边框的延长线"——要拍到它，得让选区小一点，让线跑到选区外面去
  （探针就是这么做的：`target/tmp/overlay_states_shot.ps1`）。

核对用的探针：`target/tmp/overlay_states_shot.ps1`（打开记事本当吸附靶子 → `MoveWindow` 到
`200,160 900x700` → 起 `--capture` → 按住左键中途截图 → 松手再拖一个小选区吸到记事本左上角
截图 → 松手截图）。注意 `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)` 报的可见框边是
`208,168`（不含 8px 的隐形缩放边），所以"吸到 200"实际落在 208。

### 64.2 长截图进度：4px 轨道

面板上的「N 帧」「W × H」是快照，回答不了"离长度上限还有多远"。新增
`fn progress_ratio(height, max_height) -> f32` 与 `fn progress_bar(ui, theme, ratio)`
（`crates/cuter-ui/src/scroll_panel.rs`）：

* 轨道 `theme.outline`、已拼部分 `theme.accent`，高 4、圆角 2（4px 网格）；
* 比例很小时也留 4px 的小节——否则"已经开始了"在屏幕上根本看不见；
* `max_height == 0`（不设上限）时不画，免得白占一行；画的时候底栏预留从 72 加到 92，
  否则状态行与「完成」会被顶出窗口（0.1.39 踩过一次同样的坑）。

### 64.3 验证

* `cargo test -p cuter-ui` → **108 + 6 通过**（新增 `the_progress_bar_reads_the_length_cap`：
  `0/0` 与 `x/0` 都给 0、`2000/8000 = 0.25`、到顶与超过都夹在 1.0）。
* 真机：`target/tmp/scroll_shot.ps1`（起记事本 300 行 → Alt+S → 框选 → 回车 → 滚 6 格 →
  截图）拍到预览框下方那条轨道与最左端的蓝色小节；`docs/design/shots/0.1.45-scroll-progress.png`。
* 遮罩三张：`docs/design/shots/0.1.45-overlay-drag.png`、`-snap.png`、`-selection.png`。

### 64.4 一处有意偏离（记下来，免得以后当成 bug）

第 8 节写的是"选区尺寸胶囊浮在工具条上方 8px"，实现里锚在**选区左上角上方 27px**。
跟着选区走更利于辨认"这是哪一个选区"，也不会和工具条的位置计算互相牵制。

## 65. 旋转 / 竖排文本框（0.1.46）

### 65.1 quad 一直有，只是没人用

`TextBlock.quad` 从第一版就在（字段注释写着"支持旋转文本，顺序为左上→右上→右下→左下"），
但**全仓没有一处读它**：结果面板的高亮框、命中测试、编号徽标统统只看轴对齐的 `bounds`
（`ocr_panel.rs` 里 `block.bounds.x/w` 那几行 + `OcrResult::block_at` 里的
`block.bounds.contains(point)` 与 `w * h`）。

之所以一直没露馅：系统引擎（`WindowsOcr`）本来就只产生轴对齐四边形。0.1.44 接上在线引擎之后，
PaddleOCR 那种"斜着排的文字"会回真正的四个点，问题立刻变得肉眼可见——高亮画成了包围盒，
斜框外面那四个"空白角"也被涂上蓝色；鼠标指在框外，那一行照样被点亮。

### 65.2 改法

`crates/cuter-core/src/ocr/mod.rs`：

* `TextBlock::is_axis_aligned()`：四个角是否正好落在包围盒的四个角上（决定怎么画）；
* `TextBlock::area()`：鞋带公式，重叠时用它挑最小的块（轴对齐时与 `w * h` 相同）；
* `TextBlock::contains(point)`：**先用 `bounds.contains` 挡一道**——保住"左闭右开"的边界语义
  （老测试断言 `block_at(Point::new(200, 20))` 是 `None`，因为右边界不算在内），再用四个叉积
  判断点是否在凸四边形内；
* `TextBlock::top_left_corner()`：编号徽标挂哪个角（最上面那个，同高取最左）；
* `OcrResult::block_at` 改用 `contains` + `area`。

`crates/cuter-ui/src/ocr_panel.rs`：轴对齐时照旧 `rect_filled` + `rect_stroke`（圆角 2 更好看）；
非轴对齐时 `Shape::convex_polygon`（填充）+ `Shape::closed_line`（描边），徽标挂在
`top_left_corner()` 换算出来的屏幕点。

### 65.3 验证

* 单测：`cuter-core` **152 通过**（149 → 152），新增
  `an_axis_aligned_quad_reports_its_area_and_corner`、
  `a_rotated_quad_is_hit_only_inside_the_polygon`（45° 菱形：包围盒里但四边形外的点不算命中）、
  `a_rotated_box_does_not_steal_a_hit_from_its_bounding_corner`；
  `cuter-ui` 108 + 6 通过。
* 真机：`tools/probe/fake_ocr_server.py` 支持 `POST /ocr?rotate=1`，会回一个斜框与一个竖排框；
  `tools/probe/ocr_http_e2e.ps1` 加了 `-Endpoint`（把 `?rotate=1` 塞进配置里的接口地址）、
  `-Hotkey`（`-Hotkey ocr` 走"只识别不贴图"那条热键，否则贴图窗口会盖住结果面板）、
  `-Shot`（**抓整屏**——按面板矩形抓会在应用还在搬窗口时抓到它背后的东西）。
  读数：`path=/ocr?rotate=1 bytes=11781024 rotated=True`，面板 3 行、197 ms，
  证据图 `docs/design/shots/0.1.46-ocr-rotated-quad.png`（斜框与竖排框都画成了多边形）。

### 65.4 还没做的

* **竖排文本的阅读顺序**：列行顺序仍按引擎给的原样，没有按"从右到左的列"重排；
* 真实 PaddleOCR 服务上的旋转框没试过（仍是假服务端，与 0.1.44 的限制一致）。

## 66. 自动更新检查（0.1.47）

### 66.1 先钉死"不做什么"

这一条在路线图上挂了很多轮，每次都绕开，原因是它天然会变成"往用户机器上写东西"的功能。
本轮定的边界是：

* **只检查**：启动时（可关）GET 一份清单，对比版本号；
* **只提示**：状态栏一句 + 「设置 → 关于」卡片里一行结果；
* **只帮打开下载页**：一个按钮，交给系统浏览器/`explorer`，程序自己不下载、不落盘、不替换；
* **没有渠道就闭嘴**：清单地址留空（默认）＝一个请求都不发。

之所以要在文档里写这么重，是因为 `general.check_update` 这个字段**以前出现过**：某一轮配置审计
把它当"配置里写着、程序里没有"的假开关删掉了。这次它是带着真正的实现和一个 `update_endpoint`
一起回到表里的——所以关于页那句"只查、只提示，不会自动下载或替换文件"是承诺，不是文案。

### 66.2 零新依赖怎么发 https 请求

仓库有一条硬约束：不引新依赖（`Cargo.toml` 里连 `serde` 都没有，JSON 是自己写的
`crates/cuter-core/src/ocr/json.rs`）。发 http 好办——0.1.44 的在线 OCR 引擎已经有
`TcpStream` + 手写请求行 + `split_response`（还处理 chunked）；https 就麻烦了，TLS 不是几百行能
对付的东西。

选的办法是**用系统自带的 WinHTTP**（`#[link(name = "winhttp")]`，底层 schannel，跟系统更新同一条
通路），只声明用到的那几个函数：

```
WinHttpOpen(agent, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY=4, …)     // 走系统代理设置
WinHttpSetTimeouts(…, 15s)                                        // 与 DEFAULT_TIMEOUT 一致
WinHttpConnect(host, port)
WinHttpOpenRequest(…, WINHTTP_FLAG_SECURE=0x0080_0000)
WinHttpSendRequest / WinHttpReceiveResponse
WinHttpQueryHeaders(WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER)
WinHttpQueryDataAvailable / WinHttpReadData                        // 超过 MAX_BODY 就报错
WinHttpCloseHandle                                                 // 用 RAII 壳保证不漏
```

代价是这段只在 Windows 上编译（非 Windows 直接返回"只用 Windows 的 WinHTTP 实现"），
好处是一个字节的依赖都没加、也不用管证书链——系统信任谁就信任谁。

### 66.3 代码落点

`crates/cuter-core/src/update.rs`（新模块，`lib.rs` 里 `pub mod update;`）：

* `UpdateManifest { version, url, notes }` + `parse()`：清单就是一个 JSON 对象，`version` 必需；
* `is_newer(current, candidate)`：按 `.` / `-` / `+` / 空格切段逐段比数字，前导 `v` 忽略、缺段当 0，
  所以 `0.1.46-rc1` **不会**被当成比 `0.1.46` 新；
* `split_url()` → `Target { tls, host, port, path }`，只认 http/https、默认 80/443；
* `fetch(url, timeout)`：`tls` 走 `fetch_tls`（WinHTTP），否则 `fetch_plain`（自写 socket），
  两者都受 `MAX_BODY = 1024 * 1024` 限制。

`crates/cuter-core/src/config.rs`：`GeneralConfig` 加 `check_update`（默认 `true`）与
`update_endpoint`（默认空）。

`crates/cuter-app/src/app.rs`：沿用 OCR 那条"后台线程 + channel + `ctx.request_repaint()`"的老路——
`start_update_check()` 起线程，结果通过 `UpdateOutcome { Newer, Current, Failed }` 回到 `logic()`
的 `update_rx.try_recv()`，进 `handle_update_outcome()` 写状态栏与日志；
「关于」页新增一张卡片（开关 / 地址输入框 / 立即检查 / 结果行 / 打开下载页）。
日志行统一带 `[update]` 这个 ASCII 前缀，探针就是靠它判断"这一轮到底跑没跑"。

顺带加了 `cuter --about`：与 `--tray-menu` 同类的"给探针用的确定性入口"，
直接把设置窗口停在关于页（否则探针没法保证拍到页面切换后的样子）。

### 66.4 验证

* 单测：`cuter-core` **156 通过**（`newer_versions_win`、`the_manifest_needs_a_version`、
  `urls_split_into_host_port_and_path`、`the_plain_leg_reads_a_manifest_from_a_local_server`）；
  另有 `the_tls_leg_reaches_a_real_https_site` 标了 `#[ignore = "需要外网"]`，
  手工 `cargo test -p cuter-core update:: -- --ignored` 实测 **0.98 秒**取回 `https://example.com/`。
* 全量：`cargo test --workspace` 绿（`cuter-app` 61 + 1 ignored、`cuter-core` 156 + 10 ignored、
  `cuter-ui` 108、集成 6、doctest 1）。
* 真机：`tools/probe/fake_update_server.py`（假清单服务端，日志里记 `GET /update.json` 与
  `UA=Cuter/<版本>`）+ `tools/probe/update_check_e2e.ps1` 四案例：
  A 有新版本（服务端真收到请求、应用日志出现 `[update]` 与 `9.9.9`）、B `check_update = false`
  一个请求都不发、C 地址留空同样不发、D 真实 `https://example.com/` 会失败但不会挂死。
  证据图 `docs/design/shots/0.1.47-about-newer.png`、`0.1.47-about-check-failed.png`。
* 两个"真机才看出来"的小修：① 只有 `Newer` 会刷新状态栏，`Current` / `Failed` 时那句
  "正在检查更新…"会一直挂在下边骗人（截图里一眼就看到了），现在两种情况都各自写一句；
  ② `parse` 的 JSON 报错包了一层「清单不是合法 JSON：…」——否则用户看到的是解析器的
  "第 N 字节处的数字看不懂"。顺带把探针里的窗口拉高到 1100×960：默认高度下更新卡片的下半截
  （「打开下载页」那一行）在折叠线以下，截不到。
* 「打开下载页」走的是与「打开项目主页」同一条 `explorer` 路径（那条在前面的轮次里已经验过），
  本轮没有再单独验一次点击。

### 66.5 还没做的

* **没有真实发布渠道**：清单要自己写、自己放，程序里不内置任何地址；
* **不下载、不校验、不换包**：`url` 只是拿给浏览器/`explorer` 打开；
* 没有"每天查一次" "失败重试" "跳过这个版本" 之类的策略，只有"启动时一次 + 手动点一次"。

## 67. 竖排阅读顺序（0.1.48）

### 67.1 问题只在"整页竖排"时才露出来

0.1.46 把高亮框改成认四边形之后，竖排文本框画对了；但**读的顺序**还是引擎给的。
系统自带引擎大致按从上到下给，问题不明显；在线引擎（PaddleOCR 一类）给的是检测顺序，
三列竖排的中文常常是乱的——面板编号 1、2、3 跳着走，复制出来的纯文本也跟着乱。

### 67.2 判"竖排"不能看 quad[0] → quad[1]

假服务端那个竖排框是 `[[470,60],[505,52],[540,250],[505,258]]`：`quad[0] → quad[1]` 的
dx=35、dy=-8，是框的**顶边**，横的。引擎给四个角的顺序是"沿文字方向走"的，但一个本来就
又高又窄的框，那条边就是顶边——按它判会把竖排判成横排。

正解是看**四边形的长边**：长边竖直 ⇒ 这一列字是竖着的；长边水平 ⇒ 那一行字是横着的。
并且要求长边**明显**更竖直（`dy.abs() > dx.abs() * 2`）：单个字的框接近正方形，按长短边判
会随机倒向一边，那种块不该把整页顺序翻过来。

### 67.3 重排 blocks 一处就够

`crates/cuter-core/src/ocr/mod.rs`：

* `fn is_vertical(block) -> bool`——长边两倍竖直判定；
* `fn vertical_columns(blocks) -> Option<Vec<usize>>`——不是"全部竖排"或少于两块就返回
  `None`；否则按水平中线从右到左扫，x 范围有重叠的归同一列，列内按 y 升序，最后平铺成顺序；
* `OcrResult::in_reading_order(self) -> Self`——按这个顺序重排 `blocks`，其余不动。

因为面板编号（`crates/cuter-ui/src/ocr_panel.rs` 里的 1-based 下标）、右侧文本列表、
`plain_text()`（复制 / 「编辑全文」）全都**按 `blocks` 顺序**来，重排一处全都跟着走。
调用点在两个引擎的构造处：`http_ocr.rs` 的 `parse_response` 与 `windows_ocr.rs` 的
`recognize`，都是 `Ok(OcrResult { … }.in_reading_order())`。

### 67.4 为什么横竖混排不动

真实混排样本（竖排标题 + 横排正文、竖排正文里夹一行横排注释）还没验过。乱排一遍比不排更糟，
所以只做"整页竖排"这一种确定的情况，混排保持引擎原序。

### 67.5 验证

* 单测（`crates/cuter-core/src/ocr/mod.rs` 的 `mod tests`）：三列竖排按右→左、单列按上→下、
  混排保持原序、近正方形块不算竖排、重排会传到 `plain_text()`；`cuter-core` 从 156 条涨到 161 条。
* 真机：`tools/probe/fake_ocr_server.py` 加 `?vertical=1`（三列 `左列/中列/右列`，
  引擎顺序故意给成左→右→中），`tools/probe/ocr_http_e2e.ps1 -Endpoint '…?vertical=1'`
  断言服务端日志里有 `vertical=True`，面板里读成「右列 / 中列 / 左列」、编号 1 在最右边那列；
  证据图 `docs/design/shots/0.1.48-ocr-vertical-order.png`。

## 68. 托盘右键的真机链路（0.1.48 补验，不升版本）

第 61 节把托盘菜单改成自绘面板时，验证是从 `cuter --tray-menu` 走的——也就是说
**"系统托盘图标 → `tray-icon` → `TrayCommand::OpenMenu`"这一跳从来没被真机验证过**。
这一轮补的就是它。没有改产品代码，所以没升版本号。

### 68.1 UIA 那条路走不通

先想当然地想用 UIA 找到托盘图标再右键，结果：

* 枚举 `Shell_TrayWnd` 的全部后代（116 个元素），只有 `TrayNotifyWnd`（1573,1042 347×38）、
  `系统升级的通知区域`（ToolbarWindow32 30×38）、`SysPager` + `用户提示通知区域`
  （ToolbarWindow32 90×38）以及输入指示/时钟/操作中心。**没有 Cuter 图标，也没有"显示隐藏的图标"箭头**；
* 溢出面板（点开之后）是 `NotifyIconOverflowWindow` + 一个 `ToolbarWindow32` 子窗，
  UIA 后代**只有 1 个**（就是那个 toolbar 自己）——Win11 的图标项不暴露给 UIA。

所以 UIA 拿不到图标位置，必须换 Win32 的接口。

### 68.2 `Shell_NotifyIconGetRect` + 点一下溢出箭头

`Shell_NotifyIconGetRect(NOTIFYICONIDENTIFIER{cbSize, hWnd, uID, guidItem})` 能直接给出
图标在屏幕上的矩形，**前提是图标当前真的在屏幕上**：

* 图标藏在溢出里时，uID 0..16 全部返回 `0x80004005`；
* 点开溢出面板（任务栏里那个 30×38 的 `Button`，`TrayNotifyWnd` 的第一个可见子 `Button`）之后，
  同一个调用立刻成功：`hwnd=<我们的 tray_icon_app> uid=2` → `1518,792 50×50`。

`tray-icon` crate 内部用的 uID 是 **2**（不是 1），所以探针扫 uID 0..16 而不是写死。
拿到矩形后 `MoveTo(中心)` + 右键（`RIGHTDOWN 0x0008` / `RIGHTUP 0x0010`，本轮给
`tools/probe/CuterInputProbe.cs` 补的 `RightDown/RightUp/RightClick`），面板就出来了。

### 68.3 探针

`tools/probe/tray_menu_e2e.ps1`（纯 ASCII，`powershell.exe -File` 跑）：

1. 备份并写入 `start_in_tray = true` + `theme = "dark"`，重启 `cuter.exe`；
2. 扫本进程所有窗口 × uID 0..16 找图标矩形；找不到就先点溢出箭头再扫一次；
3. 在图标中心右键 → 轮询 `FindWindow(pid, "Cuter")` 找面板 → `IsOnDesktop` 断言；
4. **等 1200 ms 再截图**——窗口是"先建后显示"的，立刻截会拍到淡入中的半透明帧
   （第一版就是这样，面板带着桌面和桌宠的重影、右下角还是圆角动画的斜切）；
5. 截面板周围 24px 留白（默认写到 `%TEMP%`，别让 `run_all.ps1` 覆盖归档的证据图；
   要刷新证据图就显式传 `-ShotDir docs\design\shots -ShotName 0.1.48-tray-menu.png`）；
6. `Esc` 后面板必须消失（`CountWindows` 归零），最后杀进程并还原配置。

探针按仓库约定区分三种结局：图标位置拿不到但 `tray_icon_app` 窗口在（系统不告诉我们）
→ `SKIP`（退出码 2）；连 `tray_icon_app` 都没有（我们自己没注册成功）→ `FAIL`；其余成功 `PASS`。

### 68.4 读数

两次运行覆盖了两条分支：

* 图标藏在溢出里：`the icon is not on screen; opening the overflow` → 点箭头 →
  `tray icon: window=… uid=2 rect=1518,792 50x50` → 右键 (1543,817) → 面板 `1543,533 335×509`；
* 图标已经在可见托盘区（上一轮点开溢出后系统把它提上来了）：直接
  `rect=1603,1042 30x38` → 右键 (1618,1061) → 面板 `1585,533 335×509`。

两次都 `RESULT: PASS`，`Esc` 都关掉了面板。证据图里能看到面板贴着任务栏上方弹出、
标题「Cuter / 版本 0.1.48」、10 行菜单与底部提示，中文没有方块。

**顺带确认的**：面板"点到别处就收起"的焦点处理（`crates/cuter-app/src/app.rs` 里
`has_focus` + 曾经聚焦过才生效的那段）一直在，这轮没有改动它。

## 69. 探针自己的两个竞态（0.1.48 后收尾，不升版本）

把 `tray_menu_e2e.ps1` 交给 `run_all.ps1 -Quick` 之后，它跑出的第一条红色不是产品的，
而是**探针的**：`run_all` 里 20 个探针有 2 个失败、1 个跳过，三个都定位到合成输入的时序，
不是应用行为。没有改产品代码，所以没升版本号。

### 69.1 `Drag` 按下时指针还没到位

`size` 对着的是 `scroll_horizontal_e2e.ps1`：探针要拖 `(380,330)-(1080,650)` 的 700×320 选区，
应用日志里记的却是

```
滚动截屏开始：区域 Rect { x: 496, y: 382, w: 584, h: 267 }
```

`496-380 = 116`、`382-330 = 52`：**拖拽的起点是上一次鼠标停的位置**，不是 `MoveTo` 的目标。
原因是 `tools/probe/CuterInputProbe.cs` 的 `Drag()` 在 `MoveTo(x0,y0)` 之后**立刻**按下左键，
而遮罩是从自己的输入队列里读取指针的——按下那一刻它还没处理那条移动消息，于是拿旧坐标当起点。
修法是按下之前 `Thread.Sleep(150)`（备注就写在函数里）。修完同一探针：

```
saved long image: 1000x320
RESULT: PASS - horizontal scrolling was detected and stitched into a wider long image
```

高度 320 与选区完全一致，宽度 1000 = 700 + 滚出来的 300。

### 69.2 遮罩没拿到键盘，拖拽和回车都进了记事本

`hide_own_windows_e2e.ps1` 报 `no PNG was auto-saved`：截图链路本身是好的（日志一路到
`截图链路[遮罩首帧可见]：+480 ms`），但拖拽与回车没有一条被遮罩接收——焦点还在记事本里，
那一下变成在记事本里选字。修法是在拖拽**之前**显式 `Activate($overlay)`：

```
saved: Cuter_20261017_140657.png = 882x628 (262607 bytes)
deviation(saved selection vs reference) = 17.227 (best dy=-1)
RESULT: PASS - our own window is gone from the shot (dev 17.227; 198.145 when it was visible)
```

17.2 是文字重绘的残差，198.1 是"我们自己的窗口还在画面里"的信号，两者的距离就是这条断言的余量。

### 69.3 顺带修掉的探针 bug

`hide_own_windows_e2e.ps1` 的失败文案原来用 `$saved.Width` 拼字符串，但那时 `$saved` 已经
`Dispose()` 了，于是打印成 `saved image is x, expected 882x628`——最难查的那种"证据自己坏了"。
现在宽高在读图时先存进 `$savedW/$savedH`。

### 69.4 自动滚动探针：三处都是探针的错

`scroll_auto_e2e.ps1` 之前一直 `SKIPPED`（"auto-scroll never reported a stop"），
把它跑通花了三次：

* 第一次 `FAIL`，应用日志 `autoscroll-stop stalled=3 steps=3 height=577`——滚轮根本没让页面动。
  原因是 `SendInput` 的滚轮落到**光标底下那个窗口**，而拖拽的终点 `(900,700)` 在默认大小的
  记事本外面。修法：开始收集之后把光标挪到选区中心 `(525,425)`；
* 第二次还是 `SKIPPED`，这次应用日志写着 `滚动截屏自动滚动：关（停机判据 3 帧）`——探针
  自己在配置里已经写了 `auto_scroll = true`，随后又按 `A` 把它**关掉**了。修法：不按 `A`
  （配置已经是开的时候，`A` 是反向开关）；
* 第三次 `PASS`：`autoscroll-stop stalled=3 steps=35 height=2413`。

### 69.5 教训

合成输入探针的失败**先怀疑时序**，别先怀疑产品：这轮四个"回归"（两个失败 + 一个跳过 +
一个文案坏掉）里没有一个是产品的问题。但也不能因此就把探针放宽——每个修法都是让
**输入更真**（等指针到位、先抢焦点、把光标放在该在的地方、别乱按开关），不是把断言调松。
`run_all.ps1 -Quick` 从 `17 passed / 1 skipped / 2 failed` 变成 **19~20 passed**
（详见下一节那一条还剩下的不稳定）。

### 69.6 还没解决的：`ocr_linkage_e2e` 的悬停高亮偶发不重绘

同一次构建、同一个探针，今天跑过 `PASS`（两次）也跑过 `FAIL`。失败时的现场：面板正常、
2 行文本、`panel-idle.png` 与 `panel-hover.png` **字节完全相同**，`%APPDATA%\Cuter\logs\cuter.log`
里没有任何异常。

已排查并排除：面板位置/尺寸（`Describe` 与截图一致，光标 x 落在行内）、鼠标移动本身
（`SetCursorPos` 成功）、行号假设（现在会依次试 `+120 / +165 / +210` 三个行高）。

剩下的怀疑点是**这一轮 OCR 结果本身**：失败那次识别出来的是"ter OCR 衍…"这种乱码、
只有 2 行，box 与行的对应关系退化到悬停不产生可见变化。这条留给下一次——它的前提是
先让探针能确定拿到"干净的两行文本"（例如换一个更大字号、纯 ASCII 的靶子窗口），
而不是继续在悬停坐标上加补丁。

## 70. 逐行列表的悬停联动（0.1.49）

### 70.1 症状与它藏了多久

`crates/cuter-ui/src/ocr_panel.rs` 的 `line_list` 文档注释写着「鼠标在哪一行，左边图上就高亮
哪个块——双向联动的另一半」。实际上只有「图 → 列表」这一半通：鼠标放到图上某个框上，
右边对应行会亮（`line_list` 用 `self.hovered` 决定行的底色）；反过来把鼠标放到行上，
图上的框不会变。

### 70.2 根因：左栏先画，把右栏刚设的值抹掉

每帧的调用顺序是 `preview()`（左栏，`crates/cuter-ui/src/ocr_panel.rs` 的 `ui()` 里
`if let Some(clicked) = self.preview(ui, left_width)`）→ `line_list()`（右栏，同一个 `ui()` 里
`if let Some(a) = self.line_list(ui)`）。而 `preview()` 里有一行

```rust
self.hovered = pointer.and_then(|p| ... self.result.block_at(...));
```

指针在右栏时 `response.hover_pos()` 是 `None`，于是这一行把 `self.hovered` 写成 `None`；
它又写在绘制叠框的循环之前，所以同一帧里 `line_list` 设进去的行号**从未被读到**。
共用一个字段加上固定的绘制顺序，等于这个方向永远失效。

### 70.3 修法：两个字段 + 取并集

- 新增 `row_hover: Option<usize>`。`line_list` 每帧先把上一帧的值存进局部变量再清空，
  按 `row.hovered()` 重新写入；行的底色用 `self.hovered == Some(i) || previous == Some(i)`
  （`previous` 是上一帧的 `row_hover`，一行代码换掉一个帧序问题）。
- `preview()` 照旧写 `hovered`，但叠框的命中判定改成
  `self.hovered == Some(i) || self.row_hover == Some(i)`。
- 指针离开整个面板时，两侧各自会把自己那一半清空，不需要额外的「离开」分支。

### 70.4 探针怎么把它逼出来的

原来的 `tools/probe/ocr_linkage_e2e.ps1` 用记事本当靶子、比整块面板的截图——列表行自己的
悬停底色也在变化，所以它一直是「PASS」。这轮把靶子换成自建的 `tools/probe/OcrTarget.cs`
（820×420、三行 70 px Consolas 纯 ASCII：`ALPHA LINE ONE` / `BRAVO LINE TWO` /
`CHARLIE LINE THREE`，行矩形由窗口自己报），断言收紧成只比面板左侧图片区，结果
**连跑三次三次失败**——产品 bug 就这样被逼出来。

### 70.5 验证

- `cargo test -p cuter-core -p cuter-ui`：161 + 108 + 6 通过。
- `tools/probe/ocr_linkage_e2e.ps1` 连跑五次全 PASS，每次都在图片区看到变化
  （命中行 y=336 / 304 / 272 / 336 / 336）。
- 证据图 `docs/design/shots/0.1.49-ocr-row-hover.png`：第 1 行在列表里发亮，图上 1 号框
  同时加粗（见 70.6：第一版框选边距太小，重拍过）。

### 70.6 那条「预览被裁掉左边界」是误判（已查清，产品没问题）

证据图第一版里 `ALPHA` 显示成 `LPHA`，我一度记成"面板预览在小选区时会裁掉左边"。
复查结论：**不是预览的问题**。探针当时框选的边距只有 12 px，而 70 px Consolas 的首字母
（`A`）左边距本身就有十几个像素，于是框选真的从 `A` 的右半边开始了——OCR 读出来就是
`LPHA LINE ONE`（列表里也是这两个字），说明缺的是图里的字，不是绘制裁切。把
`ocr_linkage_e2e.ps1` 的框选边距放大到 40/30 px 后，图上完整显示 `ALPHA LINE ONE`，
列表也读成 `ALPHA LINE ONE`，证据图重新拍过。

教训：OCR 结果里少了一个字形时，先怀疑"框选/截图范围"，再怀疑"绘制裁切"——前者看
探针算的矩形就能验证，后者要先量出"图与框错位"才有证据。

### 70.7 顺手记下的一条没修的

收尾跑 `run_all.ps1 -Quick` 是 **19 passed / 1 failed**：`scroll_horizontal_e2e` 报
`saved long image: 700x320` / `the stitched image is 700 px wide - horizontal scrolling was not
stitched (selection was 700)` / `target scroll counter: 0 -> 0 px`。同一个探针今天早些时候
是 PASS（长图宽 1000+），所以是探针侧的偶发——横向滚轮没有落到靶子上。本轮没动过滚动截屏
的任何代码（改动只有 `crates/cuter-ui/src/ocr_panel.rs` 的悬停状态与探针脚本），记在这里
作为下一轮的入口。

## 71. 横向滚动截屏的三个拼接 bug（0.1.50）

**入口**：0.1.49 收尾跑 `tools/probe/run_all.ps1 -Quick` 时，`scroll_horizontal_e2e` 报
`saved long image: 700x320`（选区也是 700 宽），20 个探针里唯一失败的一个。

### 71.1 探针先自己坏了三处，bug 才露出来

1. **靶子根本不能横向滚**：`ScrollTarget.cs` 原来用原生多行 EDIT（`WS_HSCROLL |
   ES_AUTOHSCROLL`）装 6000 个伪随机字母。多行 EDIT 即使给了 `ES_AUTOHSCROLL` 也按**控件宽度
   折行**、没有横向范围——`EM_LINESCROLL` 与横向滚轮都不可能让它动，而它自己
   `WM_MOUSEHWHEEL` 的累加器又从没被触发过，于是 `ScrollX()` 长期是 0，探针那行还被标成
   informational。现在换成「父窗口 + 一个 40000×60 的子 STATIC（`SS_LEFTNOWORDWRAP`）装整行
   长字符」，"滚动"就是 `SetWindowPos(_static, TEXT_X - _scrollX, ...)`，`ScrollPos()` 就是真值。
   （`SS_LEFT` 会在控件宽度处折行，必须用 `SS_LEFTNOWORDWRAP`；自绘那条路会踩
   `PAINTSTRUCT` 的封送器坑，整个 PowerShell 进程无声消失，别回头。）
2. **建窗线程没切 DPI 上下文**：125% 缩放下 `(320,260)+900×420` 实际落在 `(400,325)+1125×525`，
   选区与靶子对不齐。线程体开头加 `SetThreadDpiAwarenessContext(-4)`（照 `OcrTarget.cs`）。
3. **合成拖拽的按下锚点是旧的**：`CuterInputProbe.Drag()` 在 `MoveTo` 后立刻 `Press`，
   遮罩拿到的是**上一条输入消息**的位置。700×320 的拖拽实测变成 642×294（起点 438,355）。
   现在 `MoveTo → Sleep(250) → Press → Sleep(200) → 再 MoveTo(x0,y0) 零距离重锚 → 12 步插值`。

### 71.2 三个真 bug

- **空白行被当成新内容**（`ScrollStitcher::push`）：匹配只看"重叠区能对上"，而一片同样的底色
  在**任何**位移下都能对上。于是「顶上一条字 + 下面全白」的两帧被判成纵向滚了一行字的高度
  （24 px），追加 24 行空白，`frames > 1` 之后方向被 `axis_is_not_switched_after_the_first_append`
  锁死在纵向。修法：追加/前插的行必须先通过 `band_has_texture`，否则返回 `NoMatch`
  ——**注意是 `NoMatch` 而不是 `Unchanged`**：上层 `DirectedStitcher` 正是靠 `NoMatch` 才去试
  另一个方向，返回 `Unchanged` 会被它当成"当前方向是好的"，于是永远不翻（这条第一版就写错了，
  探针立刻证明：长图高度对了，宽度却一直不涨）。
- **`Unchanged` 清掉换方向的投票**（`DirectedStitcher::push`）：抓帧间隔 250 ms 比滚轮 350 ms 短，
  两次投票之间常夹着静止帧；它一清票，两次投票几乎永远排不到一起。现在 `Unchanged` 不清票
  （只清"当前方向真的拼进去了"那种情况）。
- **第二票的步长是两次滚轮的位移**：第一票 120、第二票 240，原来 `abs_diff <= offset/4` 直接判
  "步长不一致"。现在允许整数倍（`big % small <= tolerance`）：倍数关系同样说明是同一个滚轮
  在推同一段内容，而随机误匹配凑出整数倍的概率很低。

### 71.3 验证

- 单测：`cargo test -p cuter-core` **162 通过**，新增
  `a_match_that_would_append_blank_rows_is_not_a_scroll`（40×60 白底 + 顶上一条黑带，
  下一帧整片白 → 必须报 `NoMatch` 且长图高度不变）。
- 探针：`tools/probe/scroll_horizontal_e2e.ps1` 连跑 **4 次全 PASS**，每次
  `saved long image: 1900x320`（选区 700 + 横滚 1200 = 1900，高度严格等于选区）。
- 回归：`tools/probe/run_all.ps1 -Quick` **20 passed / 0 skipped / 0 failed**——
  这是第一次全部探针都绿（`scroll_e2e` / `scroll_auto_e2e` 的纵向逻辑没被
  `band_has_texture` 误伤：真实内容每一帧都有纹理）。

### 71.4 顺带记下的探针纪律

选区尺寸现在会从应用日志里**回读**（`滚动截屏开始：区域 Rect { x: … }`），不一致就当场抛错。
这样"产品拼错了"和"探针拖歪了"下次一眼就能分开——这条 flake 之所以藏了好几轮，
就是因为原来的失败文案两种可能都解释得通。

## 72. 配置审计表按代码重新对了一遍（0.1.51）

这一轮没有新功能，起因是审计表本身不可信。

### 72.1 表里到底错在哪

`docs/config-audit.md` 是 2026-09-28 那轮全量审计的产物。0.1.51 之前它有三类错，
而这三类错都会把后来的人带沟里：

1. **同一行复制了四遍**：`show_tray_notifications` 与 `language` 各出现四次，
   §1 读完仍然不知道哪些字段还没人管。
2. **把已经落地的字段写在"已删除"里**：`general.language`、`general.show_tray_notifications`、
   `general.check_update` 三个在 §2「本轮删除的字段」里，可它们分别早在 0.1.x 的语言/托盘通知
   和 0.1.47 的更新检查里就回来了。看表的人会以为"更新检查还没做"。
3. **"未实现但保留"的三个 `ocr` 字段早就有两个生效**：`http_endpoint` 与 `min_score`
   在 0.1.44 接通在线引擎后就是真的了；表里还写着"`app.rs::start_ocr` 会明确提示
   「在线 OCR 接口尚未实现」"——那句提示也一起被删了。

### 72.2 为什么会错：审计方法是手写 grep

旧方法写在文档第 13-20 行：

```powershell
Select-String -Path crates\cuter-core\src\config.rs -Pattern '^\s+pub [a-z_]+:'
grep -rn "general\.\|capture\.\|..." crates --include=*.rs
```

它按**字段名**搜整个 workspace，于是字段名出现在注释、文档字符串、测试名里都算"有人读"。
`check_update` 当年就是这么被算成生效的（其实那会儿它只是个没人读的占位字段，后来才真的实现）。
手写 grep 还有第二个毛病：**你得记得跑它**，而且跑完还要人肉判断 0 次和 1 次哪个算正常。

### 72.3 换成一条命令，并且能当回归用

新增 `tools/docs/audit_config.py`：

```powershell
python tools\docs\audit_config.py        # 每个字段被引用几次；有 0 次就以 1 退出
python tools\docs\audit_config.py --api  # cuter-core 里 app/ui 一个都没调用的 pub fn
```

它只统计 **config.rs 之外** 的引用次数，并且用 `\bname\b` 的完整标识符匹配，
所以注释里的字面量不会混进来。`--api` 那一半顺带解决了一个 `dead_code` 抓不到的问题：
**`pub` 项永远不报 dead_code**，一个从没被调用过的 `pub fn` 在 clippy 眼里是干净的。

### 72.4 这轮删掉的东西

| 删掉的 | 为什么 |
|---|---|
| `OcrConfig::model_dir` + `resolved_model_dir()` | 系统 OCR 与在线接口都不加载模型文件，函数一次都没被调用 |
| `geom::Point::to_f32` / `geom::Size::to_point` | 没有调用者；需要在浮点与像素之间换算的地方都是就地写 `as f32` |
| `geom::Rect::local_to_global` / `global_to_local` | 没有调用者；遮罩换算走的是 `overlay.rs` 自己的 `to_screen` 闭包 |
| `geom::Rect::bounding` | 只是 `union` 的别名，没有调用者 |
| `ToolKind::is_region_effect` / `AnnotateConfig::stroke_rgba` | 标注工具那边直接用 `matches!` 与就地构造 `[r,g,b,255]` |
| `HttpOcr::with_timeout` | 超时是 `DEFAULT_TIMEOUT` 常量，没有调用者去改它 |

删完 `cargo test -p cuter-core -p cuter-ui` 仍是 **162 + 108 + 6 + 1**，
`cargo clippy --workspace --all-targets -- -D warnings` 干净——说明这 8 个函数确实没人用，
也说明**测试没覆盖到它们**（覆盖了的话删掉会编译不过）。

### 72.5 教训

1. **审计文档会过期，而且过期的审计文档比没有更危险**：它给人一种"已经查过"的错觉。
   每次有字段从"未实现"变成"已实现"，都要顺手改表——这一条应该写进第 4 节的清单。
2. **"已删除"这个状态最难维护**：字段复活时没人会去翻旧表。所以 0.1.51 把 §2 收成一行
   当年真删掉的东西，其余全部按**现状**重写，不再记录"曾经删过什么"。
3. **静态审计只能证明"有人读"**：`start_in_tray` 就是反例（有作用点、但被 eframe 覆盖）。
   所以 §5 保留一句话：凡是"只读代码就判定生效"的，状态列必须写「代码路径成立，未实测」。

## 73. 真的接上了自建 PaddleOCR（0.1.52）

### 73.1 为什么自己起服务，而不是用 paddleocr 自带的 hubserving

`paddleocr` 的 3.x 已经不再附带 `hubserving`，而 2.x 的 `ocr.ocr()` 返回值正好就是老版
hubserving 的内层形状。于是最省事的做法是：装 **PaddleOCR 2.9.1 + paddlepaddle 2.6.2**，
用四十行 `http.server` 把 `ocr.ocr()` 包一层（`tools/probe/paddle_ocr_server.py`）。

3.x 在 Windows CPU 上跑不动，原因是 oneDNN + PIR：

    NotImplementedError: (Unimplemented) ConvertPirAttribute2RuntimeAttribute not support
    [pir::ArrayAttribute<pir::DoubleAttribute>] (at ...\onednn\onednn_instruction.cc:118)

设置 `FLAGS_enable_pir_api=0` 也无效。降到 2.x 之后一次就通了。venv 建在仓库外
（`%USERPROFILE%\paddle-ocr-venv`），这样 `cargo clean` 不会把它删掉。

### 73.2 `results` 是**一层嵌套**，摊平就变成 0 块

老版 hubserving 的形状是：

    {"status": "000", "results": [[[文字, 置信度, [[x,y],[x,y],[x,y],[x,y]]], ...]]}

注意 `results[0]` 才是"这一页的块列表"。我第一版把块直接摊在 `results` 里，客户端
`parse_paddle_response` 找不到 `results[0]` 数组，报的是

    OCR 接口的响应里没有 results[0] 数组（期待 PaddleOCR hubserving 形状）

——错误信息其实已经把话说清楚了，是服务端写错了，不是客户端挑剔。

### 73.3 探针的两处修正

- **剪贴板里必须是一张"有字"的图**。原来的探针放的是 Windows 自带壁纸（风景照），假服务端
  不在乎，真 OCR 老老实实回 0 块，面板显示"没有识别到文字"。现在先开 `OcrTarget`（三行
  `ALPHA / BRAVO / CHARLIE`）截它的窗口。
- **假服务端的文本日志断言不能套在真服务端上**。真服务端改用 `--log` 写 JSON 行
  （`path` / `bytes` / `blocks` / `elapsed_ms` / `texts`），探针断言 `"blocks": > 0`。

### 73.4 验证读数

真机一次运行（`-Endpoint http://127.0.0.1:8900/ocr`）：

    {"path": "/ocr", "bytes": 31836, "blocks": 11, "elapsed_ms": 1437,
     "texts": ["ocr-target", "口", "×", "ALPHA", "LINE", "ONE", "BRAVO", "LINE", "TWO", "CHARLIE", "LINE"]}

面板 11 行、每行都有编号徽标，图上是真实的词级四点框（PaddleOCR 是按词检测的）。
假服务端那条路径（不给 `-Endpoint`）同样 PASS。

### 73.5 教训：PowerShell 的变量名不分大小写

探针里有 `param([string]$Endpoint = '')` 和 `$endpoint = if ($Endpoint) { ... }`，
而 `$endpoint` 赋值会把 `$Endpoint` **一起覆盖掉**（同名不同大小写），于是后面那句
`$useFake = -not $Endpoint` 永远是 `$false`：假服务端根本不启动，探针却继续往下跑，
最后报的是"面板没出现 / 服务端没收到请求"。它不会报错，只会安安静静走错分支——
所以 `$useFake` 现在在覆盖之前就算好。

## 74. 真 PaddleOCR 探针化（0.1.53）

**入口**：0.1.52 用本机 PaddleOCR 2.9.1 验通了在线 OCR，但那次是手工敲的三条命令；
`docs/HANDOFF.md` 第 73 节写着"怎么做"，却没有任何东西能自动复查。

### 74.1 为什么值得做成探针

- 它覆盖的是**契约**而不是 UI：`parse_paddle_response` 认的字段、`results` 的那层嵌套、
  四边形坐标的顺序，全都只有真引擎才会照实回答。假服务端是照着客户端写的，验不了契约。
- 但真引擎**不能进套件硬依赖**：venv 约 1 GB、模型还要下载、机器上没装就不该算"回归"。
  所以结局必须是 **SKIP（退出码 2）** 而不是 FAIL——这一点和 `docs/MAINTAINER.md` 里
  "环境跑不了就诚实跳过"的规矩一致。

### 74.2 探针做了什么

`tools/probe/paddle_ocr_e2e.ps1`：找 venv → `Start-Process` 起 `paddle_ocr_server.py`
（端口 8917，避开手工调试常用的 8900）→ 轮询 `server.out` 等 `model ready`（默认最多
180 s，第一次跑要下模型）→ 调 `ocr_http_e2e.ps1 -Endpoint … -ServerLog … -Hotkey ocr`
→ 用正则从服务端日志里取 `"blocks": N`，小于 3 就 FAIL → `finally` 里杀掉服务端。
工作目录落在 `%TEMP%\cuter-paddle-<时间戳>\`，失败时可以直接翻 `server.err`。

### 74.3 踩到的两个坑（都是探针自己的）

1. **`-Hotkey ocr` 之前是死参数**：`ocr_http_e2e.ps1` 里热键是硬编码的 `Alt+Shift+O`
   （"贴图 + 识别"），而那条路会额外开一个置顶贴图窗口，正好盖住结果面板右下角。
   现在按名字查表：`ocr_clipboard = O`、`ocr = R`，两处都能用，取证图固定用后者。
2. **ASCII-only 不是洁癖**：`paddle_ocr_e2e.ps1` 第一版写了中文注释、文件没有 BOM，
   Windows PowerShell 5.1 按 GBK 解析，多字节序列把换行吃掉，报的是
   `Unexpected token '}'` + `MissingCatchOrFinally`——看起来像括号错，其实是编码。
   现在这个文件的非 ASCII 字节数是 0（有检查手段：`[IO.File]::ReadAllBytes` 数一遍）。

### 74.4 验证

- 装了 venv：`RESULT: PASS - a real PaddleOCR server answered the online OCR path`，
  读数 11 个块、约 1.9 s；日志在 `%TEMP%\cuter-paddle-20261018-010027\`。
- 把 `-Python` 指向不存在的路径：`SKIPPED: no PaddleOCR venv at …`，退出码 **2**。
- `run_all.ps1 -Quick` 里它自动被发现（文件名匹配 `_e2e`），本机 21 条探针全绿。

### 74.5 教训

"验过一次"和"能反复验"是两件事。手工验证留下的是一段历史记录（第 73 节），
探针留下的是一个能在下次改动后立刻告诉你"契约破了"的东西——而它的成本只有
一个 SKIP 分支。

## 75. 复测旧性能数字：先确认自己在量什么（0.1.54）

### 75.1 起因

0.1.53 收尾时把一直跳过的慢探针全跑了一遍（第一次在本机跑 `installer_e2e`、
`leak_probe`、`idle_gpu_probe`、`history_e2e`），27 条全过。顺带把
`latency_probe` / `perf_probe` 的读数与文档里记的对了一遍，对不上：

| 指标 | 文档旧值（第 25 节 / 第 34 节） | 0.1.54 实测（3 次） |
| --- | --- | --- |
| 热键 → 遮罩首帧（GDI） | 156 ms（中位） | 184 / 184 / 196 ms |
| 热键 → 遮罩首帧（DXGI） | 116 ms（中位） | 190 ms |
| 捕获段 `--full`（中位） | 141 / 136 ms | 137 / 168 ms |
| 冷启动 → 首帧 | 364 ms | 335 / 343 / 377 / 408 ms |
| 其中 capture-done → painted | —— | 97~120 ms（中位） |

### 75.2 判断

- **捕获段没变**（137–168 vs 136–141），冷启动也落在同一区间 → 不是"整体变慢了"。
- 变慢的是**遮罩绘制段**（`capture-done → painted`，窗口 + GL 上下文 + 首帧上传）：
  今天中位约 100–120 ms，而第 25 节把总延迟压到 156 ms 时这一段约 100 ms，差得不多，
  但没有当时的原始分项，无法逐段对账。
- DXGI 今天反而比 GDI 慢（190 vs 184–196 区间重叠），与第 25 节"DXGI 更省 35 ms"相反。
  这一次 DXGI 的 `handled` 段（按键到应用处理）抖动到 8–84 ms，怀疑是这台机器今天的
  合成器/电源状态，不是后端本身的差别。
- 结论：**本轮不下"性能回归"的判断，也不改第 25 节的历史记录**——历史记录是当时的事实。
  这次的数据作为"同机、同日、同探针"的复测记录留在这里。

### 75.3 为下一次比较做的改动

探针读数开始自带口径：

- `latency_probe.ps1`：`hotkey -> overlay painted [gdi]: min … / median … / max …`
  （它自己写配置，所以直接打印 `$backendValue`）。
- `perf_probe.ps1`：`--full` 与整屏捕获两行都带 `backend=<值>`（`--full` 走用户配置，
  所以先读一次配置里的 `backend`，读不到就写 `gdi (app default)`）。

理由很直白：GDI 与 DXGI 在这条链路上差着几十毫秒，而旧日志只写数字，
过两轮就没人说得清"那次 116 ms"量的是哪条路径——本轮我自己就先掉进去了。

### 75.4 教训

- **同一个数字，口径不同就不能比**。性能记录必须最少带三样：机器、后端、探针版本（或版本号）。
- **历史记录不要改**：它是"当时测到的事实"，改了就等于伪造证据。新数据另起一节，
  把差异和判断写在旁边。
- 慢探针长期不跑 = 盲区。`installer_e2e` 直到这一轮才第一次在本机跑通，
  而它验证的正是"用户拿到 setup.exe 之后会发生什么"。

### 75.5 顺手查出的产品缺口：自动滚动会滚错窗口

全量套件跑 `scroll_auto_e2e` 时偶发 `steps=4 height=577`（一帧都没长），单独跑必过。
按惯例这类"只在连跑时挂"的现象先算干扰，但这次顺着干扰查下去，发现是**产品行为**的缺口：

- 自动滚动的滚轮是 `SendInput` 发的（第 24 节记过为什么不用 `PostMessage`），
  它只会落到**光标底下那个窗口**；
- 探针按设计把光标停在选区中心，但那一块当时被别的窗口盖着（前一个探针残留的窗口），
  于是滚轮被别的窗口吃掉，帧永远不变 → `AutoScrollTracker` 判 `Stalled` 后停下；
- 真实场景里同样会发生：用户框完选区、切了个窗口再按"自动滚动"，滚的是别人。

修法最小：发滚轮之前 `WindowFromPoint` → `GetAncestor(GA_ROOT)` → `SetForegroundWindow`，
把那一块所在的顶层窗口提到前台。人手滚动之前做的本来就是这件事。任何一步失败都保持
原行为（直接发滚轮），不引入新的失败路径。

验证：`scroll_auto_e2e` 连跑 3 次全过；全量 27 条探针复跑仍然全绿。

## 76. README 的数字重新量了一遍（0.1.55）

### 76.1 起因

0.1.54 复量性能时顺手核对 README 的对比表，发现门面的数字与实测对不上：

| README 原文 | 0.1.55 实测（发出去的 0.1.54 便携包里的 exe） |
| --- | --- |
| 单个 exe **7.9 MB** | **8.6 MB**（9,025,536 字节；便携包 3.99 MB、安装包 5.05 MB） |
| 常驻托盘 工作集 **87~97 MB** / 私有 **51~52 MB** | **149.5~155.7 MB** / **69.4~77.3 MB**（句柄 380、线程 13） |
| 遮罩打开 工作集 **137~145 MB** / 私有 **75 MB** | **150.1 MB** / **69.4 MB** |
| 热键 → 遮罩 **min 79 / median 113 / max 188 ms** | **min 82 / median 147 / max 174 ms**（5 次，GDI） |
| `--full` 中位 **137~157 ms** | **137~168 ms**（今天的三次独立运行；单次可低到 121、高到 235） |
| 冷启动 → 遮罩 **~0.5 s** | **555~559 ms** |

结论：功能一路加过来（在线 OCR 客户端、更新检查、自绘托盘菜单、窗口标题 i18n 等），
**README 从 0.1.22 之后就没再跟着量**。

### 76.2 为什么用便携包里的 exe

那才是用户拿到手的东西：仓库里 `target\release\cuter.exe` 只是构建产物，
而发出去的 zip 经过打包脚本（剥离调试信息、写清单）之后大小与内存行为都可能不同。
量法：解压 `dist\cuter-0.1.54-portable-x64.zip` → `perf_probe.ps1 -Exe <解压出来的 exe>`。
`latency_probe.ps1` 目前只认 `target\release\cuter.exe`，所以那一项是等价的发布构建
（同一份源码、同一条 `cargo build --release`）。

### 76.3 内存增量不归因

这 60 MB 工作集里有 18.8 MB 是系统微软雅黑（README 早就写着），剩下的**没有逐项 A/B**：
没有"只关掉 OCR 客户端"的构建、也没有"去掉自绘托盘菜单"的构建，因此**不能**说
"是 OCR 吃掉的"。README 里因此只写了数字与"会随版本长胖"，不写结论。
要做真归因，得按功能分别出测量构建——那是另一轮的工作。

### 76.4 沿用下来的规矩

- 对比表里每个数字后面都要能追到一条复现命令；这次在表下面直接把两条命令写出来了。
- 数字变化时**改 README，不改 HANDOFF 的历史记录**（第 75 节的规矩）。
- 发布流程里加一句提醒：`docs/MAINTAINER.md` 的发布检查单里本来就要求"跑 perf_probe 并把
  表贴进 commit"，但 README 的数字没有被同一条规则覆盖——这个缺口现在补上了（第 76.1 节的表
  就是补旧的产物）。

## 77. 托盘闲置时把工作集还给系统（0.1.56）

### 77.1 起因

0.1.55 把 README 的数字重新量过：常驻托盘写着 149~156 MB 工作集。但那个数字是在
**设置窗口开着**的状态下量的（`perf_probe` 为了让字体装载、数字可比，会先请求一次
"打开设置"）。真正的"纯待命"是多少？量了一下：**工作集 58.8 MB / 私有 28.8 MB**。
也就是说 149 MB 里有约 90 MB 是"窗口开着 + 字体 + GL 表面"。

### 77.2 做了什么

纯待命（我们没有任何窗口可见）满 30 秒调用一次 `EmptyWorkingSet`：

- 工作集 **58.8 MB → 6.1 MB**；
- **私有提交 28.8 MB 不变**——这不是"省了 50 MB 内存"，只是把页换出去还给系统，
  下次访问时按需换回；
- 闲置之后按热键，`截图链路[遮罩首帧可见]` 从 **186 ms 变成 194 ms**（+8 ms），
  也就是说换回来的代价在这个场景里可以忽略；
- 一次闲置只收一次，任何窗口出现就重置，避免反复触发页错误。

### 77.3 收获：一个"永远不会执行"的写法

第一版把这段逻辑放在 `else if self.font.is_some() { ... }` 分支里（那里已经有"卸字体"的
闲置逻辑，看起来顺理成章）。但**纯托盘运行时字体从来没装载过**，`font.is_some()` 一直是
false，于是收紧工作集的代码一次都没跑——第一次实测 58.8 MB 一动不动、日志里也没有那行，
才发现挂错了分支。教训：闲置逻辑要挂在"是否需要窗口"这个**状态**上，不能挂在
"字体是否已装载"这种**副作用**上。

### 77.4 探针

`tools/probe/idle_trim_probe.ps1`（纯 ASCII，约 60 行）：

1. 备份配置，写一份"启动即进托盘"的最小配置；
2. 起进程，等 8 秒记基线，再等到 40 秒采样；
3. 断言工作集 ≤ 基线一半、**私有提交没有跟着掉**（掉了说明"省内存"的说法是假的）；
4. 按一次 `Alt+C`，断言遮罩照样弹出（页能换回来）；
5. 还原配置、杀掉进程。

它跑了约 55 秒，所以放在慢探针组里；默认套件（`-Quick`）不会跑它。

### 77.5 套件工具链的两处调整（同一轮）

1. **失败重试一次**：这些是 UI 探针，失败里有一类纯粹是"上一个探针刚把焦点/前台窗口弄乱"
   （`scroll_auto`、`pin_drag_out`、`tray_close` 都各中过一次，单独跑必过）。`run_all.ps1`
   现在会自动重试一次，但**必须在表里标出来**：`PASS*` + 汇总里单独一句
   "N of the passes needed a retry - they are not stable"。绿就是绿，稳定是另一回事。
2. **"滚不动"从 FAIL 改成 SKIP**：`scroll_auto_e2e.ps1` 滚的是桌面上的窗口
   （(150,150)-(900,700)），不是自己造的靶子。那块被别的窗口盖住时滚轮落到不认滚轮的窗口上，
   于是 `steps=4 height=占用选区高度` —— 这时它**什么都没测到**。真正的拼接 bug 表现为
   帧在动但拼不起来（高度会长）。所以这种情况报 SKIP，并打印 `steps` 与 `height` 供排查。

这一条和 0.1.54 的"自动滚动提前台"是同一类问题的两面：产品侧要保证滚的是选区内那个窗口，
探针侧要保证**自己确实测到了东西**，测不到就诚实跳过。

## 78. 热键延迟"回归"的最终结论：不是代码（0.1.57）

### 78.1 悬案的来历

- 0.1.54 复测：热键 → 遮罩中位 184~196 ms（GDI）/ 190 ms（DXGI）；
- 文档里记的是第 25 节（商用化第十二轮）的 **156 ms（GDI）/ 116 ms（DXGI）**；
- 当时没有旧二进制可对比，所以只能把差异记在案（第 75 节）。

### 78.2 这轮的做法：同机、同时刻、交替跑

    git worktree add --detach target\tmp\old-build 0f23034        # 0.1.51
    $env:CARGO_TARGET_DIR = "<repo>\target\old-target"
    cd target\tmp\old-build; cargo build --release

然后拿 0.1.51 的 exe 与**发出去的 0.1.56 便携包里的 exe** 交替跑
`perf_probe.ps1 -Exe <exe>`（同一个探针、同一份配置、同一台机器）：

| 构建 | min | p25 | median | p75 | max |
| --- | --- | --- | --- | --- | --- |
| 0.1.51 | 172 | 176 | **181** | 196 | 205 |
| 0.1.56 | 153 | 166 | **180** | 195 | 224 |

差 1 ms。**结论：不是回归。** 今天这台机器的底噪就是 ~180 ms 中位、快慢差别能到 0.4 倍
中位；文档里的 113/156 ms 是在另一段机器状态（或另一轮驱动/电源状态）下量到的，
任何构建今天都复现不出来。

### 78.3 顺带把尺子校准

第一次做这个 A/B 时用的是 5 次采样：**同一份二进制两次跑出 106 与 209 ms**，比要查的
差值还大。于是把 `perf_probe.ps1` 的热键段改成 9 次，报 min/p25/median/p75/max，
并打印"最快与最慢相差几倍中位"。这类"抖动比信号大"的量，只有分位数和次数才救得了。

### 78.4 教训

- **没有旧二进制就不要下"回归"的判断**。这次的成本只有一次 6 分钟的旧版构建，
  却把一条悬案彻底关掉；反过来，如果当初直接"优化"了遮罩绘制，可能把 100 ms 花在
  没有回归的地方。
- **抖动要写进读数里**。min/median/max 会让"某次 113 ms"看起来像一个可复现的指标，
  而实际上它可能只是 9 次里最快的那一次。

### 78.5 一个仍未解的 5 px（记录在案）

收尾跑套件时 `overlay_select_e2e` 的第 C 步稳定报
`expected the redrawn 380x230, got 380x235`：同一次运行里 A（点窗口）与 B（拖 240×160）
都对，只有 C（在已有选区里再拖 520,420 → 900,650）高度多了 5 px。今天早些时候同一个探针
跑过两次都是过的，所以不是我这两轮改动引进来的（0.1.56/0.1.57 只动了 idle trim 与探针脚本）。

已经排除的：探针配置里 `snap_controls = false` 是开着的；`auto_detect_windows` 必须为 true
（第 A 步就是靠它点中窗口），所以没有单独开关能关掉窗口边缘吸附。

没排掉的：这 5 px 是合成拖拽的落点偏了，还是吸附/取整在起作用。要定位得让探针在拖拽结束
后读一次指针位置——留给下一轮，先记在这里，避免下次又把它当成"产品坏了"。

## 79. overlay 探针的 5 px：合成输入自己的偏移（0.1.58）

### 79.1 现象

`overlay_select_e2e` 第 C 步（在已有选区里再拖 520,420 → 900,650）稳定报
`expected 380x230, got 380x235`；A（点窗口）与 B（拖 240×160）都精确。0.1.57 记在案（78.5）。

### 79.2 怎么查的

`RUST_LOG=debug` 跑一遍探针，读应用自己的日志（`overlay.rs` 里按下时会打）

    遮罩按下：pos=[520.0 419.0] 高亮窗口=... 工具=Select
    遮罩按下：pos=[500.0 399.0] ...

请求坐标是 520,420 与 500,400，应用收到的都是**高 1 px**。也就是说偏移发生在
"合成光标位置 → egui 指针位置"这一段，不是选区逻辑。松手时指针还会带几像素滞后
（`Release()` 紧跟在最后一帧移动之后），两者叠加就是那 5 px。

### 79.3 处理

- C 步判据从"像素精确"放宽到 **±6 px**，但**保留**最关键的断言：拖拽发生在已有选区**里面**
  时必须**重画**成新尺寸，而不是把旧选区整体挪走（那会得到 240×160、位置不同）。B 步仍要求
  像素精确——它是"干净拖拽"的基线。
- `CuterInputProbe.Drag` 在 `Release()` 前加 150 ms 沉降，减少"最后一帧移动还在队列里"的抖动。

### 79.4 教训

- **应用的 DEBUG 日志比猜测便宜**：这 5 px 我先后怀疑过松手时机、吸附、取整，最后是
  `遮罩按下：pos=...` 一行直接给出答案。
- 断言要区分"判据"与"精度"：C 步真正要证明的是"里面拖 → 重画"，而不是"像素一个不差"；
  把精度要求降到 ±6 px，判据一点没松。

## 80. 把真 PaddleOCR 的响应留成测试 fixture（0.1.58 之后，无版本变更）

### 80.1 为什么

0.1.52 用真 PaddleOCR 验通了在线 OCR，0.1.53 又把它做成探针。但**单元测试**里的
hubserving 响应一直是手写的——只能证明"我们以为的形状"，证明不了"真引擎给的形状"。
一旦哪天真实服务的字段变了（或者我们对嵌套的理解错了），单测照样绿。

### 80.2 做法

向本机 PaddleOCR 2.9.1 服务 POST 一张真实截图，把响应**原样**存进
`crates/cuter-core/tests/fixtures/paddle_real_response.json`（1.1 KB，14 个块），
再用 `include_str!` 在 `http_ocr.rs` 里加两条测试：

- `it_parses_the_response_a_real_paddle_server_sent`：块数、首块的文字/置信度/矩形、
  三行大字都在、中文与全角标点没被破坏；
- `a_low_min_score_filters_the_real_payload`：门槛抬到 0.99 时确实会掉块，且留下的都过线。

fixture 的来源、抓取参数、内容特征写在 `crates/cuter-core/tests/fixtures/README.md` 里——
测试数据也要能追溯，否则下一个接手的人不知道它是真是假。

### 80.3 顺带确认的事

- 真实响应里 `result[0]` 是 14 个块的列表（一层嵌套），与 0.1.52 的结论一致；
- 置信度最低 0.9559、最高 0.9996；
- 中文与全角标点原样通过（`单击图中的框可复制该行文字；切到「逐行对照」…`），
  说明 JSON 解析与 UTF-8 处理没问题。

这一轮没有版本号变更：只是测试与 fixture，用户看不到任何变化。

## 81. 配置读坏时：不再静默、备份不互相覆盖（0.1.59）

### 81.1 原来的行为

`Config::load()` 在解析失败时：记一条 warn 日志、把文件改名成 `config.toml.bak`、返回默认值。
功能上"永不失败"是对的，但有两个毛病：

1. **用户看不到**。设置全部变回默认，只有日志里有一行——这正是"这程序是不是在坑我"的瞬间。
2. **备份会互相覆盖**。固定名 `.bak`，第二次读坏就把上一次的备份顶掉；而那份往往是唯一
   能手工救回来的东西。

### 81.2 现在的行为

- 备份名带时间戳：`config.toml.broken-<unix 秒>`，坏几次留几份；
- 回退这件事被记进 `Config::last_recovery()`（`Recovery { error, backup }`），
  `App::new` 拿它拼一句状态栏文案，用户一进设置就能看到"已回退默认值，原文件在哪儿"；
- **文件不存在不算读坏**：首次运行没有配置文件是正常的，不该报错（这条是写测试时发现的——
  第一版会把"没有文件"也当成回退，于是新装的用户会看到一句莫须有的报错）。

### 81.3 可测性

`load_from_with_recovery(&path)` 是公开的"任意路径"版本，`load_with_recovery()` 只是传
`Self::path()`。这样测试可以在临时目录里造一个坏文件，断言：回退到默认值、备份存在且名字
带 `config.toml.broken-`、原文件已被移走、备份内容就是原来那份；再断言"没有文件"时
`recovery` 是 `None`。

### 81.4 教训

"永不失败"不等于"静默失败"。这个项目已经因为"配置项看着存在其实没接线"吃过一次教训
（第 72 节），这次是同一类问题的另一面：**失败被处理了，但没被说出来**。

## 82. 设置界面的六处打磨：五条同一个根因（0.1.60）

### 82.1 根因：`ui` 拿的是系统样式，不是主题

`App::ui()` 里做的是 `ctx.set_visuals(self.theme.visuals())`——**只设了 `Context`**。
而 eframe 传进来的那个 `ui` 是这一帧开始时用系统视觉样式建的，它的 `style()` 里躺着的
还是旧的无障碍/浅色默认值。于是所有从 `ui.visuals()` 派生颜色的地方（卡片里的输入框、
下拉框按钮、弹层、滑块）都按"系统浅色"在画：深色主题下看着发闷，浅色主题下直接变成
浅底浅字——这就是用户说的"下拉框还是浅色模式看不清字"。

修法是把主题视觉样式同时应用到 `ui` 上（`ui.set_visuals(self.theme.visuals())`），
`settings_visuals()` 也改成从主题重新取基线，而不是克隆 `ui.visuals()`。

### 82.2 输入框白底

`TextEdit` 的背景取 `visuals.extreme_bg_color`。主题里原来没设这一项，于是用了 egui 默认
的近白色。现在 `Theme::visuals()` 里显式设 `extreme_bg_color = surface_sunken`，
输入框回到"凹进去的一块"，和卡片底色的差别来自亮度而不是靠白。

### 82.3 滑块看不出轨道和手柄

egui 的滑块用 `widgets.inactive.bg_fill` 画轨道、`widgets.hovered.bg_fill` 画手柄，两者在
本主题里都接近卡片底色。给滑块单独一段作用域，显式指定轨道（`outline`）、已填充段
（`accent`）、手柄（`accent` 实心 + `surface` 描边）。浅色和深色都截图验过。

### 82.4 一体式分裂按钮

原来主操作是「开始截图」按钮，右边另挂一个 `⋯` 菜单按钮，两个控件之间有空隙、视觉上也
不像一组。现在自绘一个分裂按钮：一个圆角矩形，左半文字、右半「▾」，中间一条竖分隔线；
两半分别用 `ui.interact()` 判定点击，右半打开同一份菜单。箭头是 `painter` 画的两条线
（不用字符，避免字体缺字时变豆腐块——这一点在第 82.5 条的坑里被证明很有必要）。

### 82.5 豆腐块：字体装载晚于第一帧

`egui` 的字体是全局的，`ensure_font()` 在打开设置/托盘菜单时才装载。新视口的第一帧如果
早于装载，就用默认字体渲染中文 → 整片 □。截图里抓到过一次（托盘菜单整张变成方框）。
修法：`declare_viewports()` 一开始就确保字体已装，装完 `request_repaint()` 再画。
这个缺陷**无法稳定复现**，所以只按"消除时序窗口"处理，不宣称"已修复"。

### 82.6 托盘菜单收紧

行高 32 → 28、段间距 9 → 7、内边距 6 → 5、标题行 26 → 24、提示行 22 → 20，
整张菜单 **407 → 355 逻辑像素**（十行 + 三段分隔 + 提示）。本机 125% 缩放，
`tray_menu_e2e.ps1` 截图实测：0.1.59 的旧包 **509 px**、新包 **444 px**（物理像素）。。

## 83. 第二轮四条：对齐、气泡、折行、等宽（0.1.61）

### 83.1 右边贴边

内容列原来没有右边距，卡片和主按钮贴着窗口右边；左边侧栏却有 16 px。现在内容列
`set_max_width(available - 16)`，两边对称。

### 83.2 两个气泡

长文字被 `Label::truncate()` 截断时，egui 自己会弹一个"完整内容"的提示；我们又手写了
`.on_hover_text()`，于是叠成两个（一个在光标处、一个偏下）。改成折行后不再截断，
egui 那个自动提示自然消失，只留手写的一个。

### 83.3 说明只会截成「…」

行高原本固定 52 px，只够一行说明，长说明一律被截断。现在先用 `painter.layout()` 量出
说明在左列宽度下要占几行，行高按 `标签高 + 说明高 + 20` 计算并夹在 52~84 之间，
说明最多折两行；超过两行的部分用 `set_clip_rect` 裁掉，悬停看全文。

### 83.4 控件等宽

新增两个常量：控件列 `CONTROL_COLUMN = 240`、控件 `CONTROL_W = 172`。所有下拉框、单行
输入框统一用它；"输入框 + 行尾按钮"的组合是 172 + 6 + 56 ≈ 234，正好放得下。
另外引擎下拉里「内置离线（系统自带，零下载）」这句被缩成「内置离线」——它把下拉框撑成
了整行宽，解释本来就该写在下一行的说明里。

### 83.5 这四条的共性

都不是功能错误，而是**排版没有统一来源**：宽度、行高、底色、提示，各自写各自的数。
把它们收敛成常量和一处计算，比逐处打补丁更省事，也更容易在下一轮保持住。
## 84. 第三轮五条：待命耗电、控件拉齐、说明减负、贴边拖动（0.1.62）

用户的原话（2026-10-21）：

> 1. 内存为什么一直在涨，从初次启动或者截图十几秒后回归2.2M开始，每秒都在上涨，一直涨到
>    12.2M，另外一直占用0.1%的gpu是怎么回事？如果单体太大需要或者可以拆分单体变成dll的，都可以。
> 2. 统一一下输入框，和下拉框统一一下宽高。有些乱七八糟显得很吝啬，细狗一样。
> 3. 有些按钮更不明显，也是细狗一样。
> 4. 界面中哪些没必要额人机似的ai辅助解释说明就不用展示了，展示主体就可以。
> 5. 鼠标放到屏幕边界就会变成鼠标箭头，这时候左键移动鼠标就会把屏幕帧给移动。这个问题以前就反馈过。

### 84.1 「内存一直在涨」——是工作集在换入，不是泄漏

用新写的 `target/tmp/gpu_idle.ps1`（托盘待命，5 秒一采样，读
`\GPU Engine(*)\Utilization Percentage` 与进程 WS/私有字节）跑了 90 秒，数据：

| 指标 | 0.1.61 | 0.1.62 |
|---|---|---|
| 私有字节 | 29.5 MB → 29.4 MB（平线） | 29.2 MB → 29.1 MB（平线） |
| 工作集 | 60.2 MB → 12.5 MB（30 秒裁剪后爬到 12.5） | 60.3 MB → 8.8 MB |
| GPU 平均 | **0.094%** | **0.036%** |

结论：私有提交全程不涨，说明**没有分配增长**。工作集的"一直涨"来自 0.1.56 的
`EmptyWorkingSet` 把页还回去之后，1 fps 心跳每帧又要把代码页摸回来——所以它是
"裁剪→换入→再裁剪"的锯齿，最终在 16 MB 附近封顶（260 秒采样里 t=140 到 t=240 平在 16.3 MB）。

修法：把待命心跳从固定 1 s 改成**退避**（`idle_ticks`：1 s → 2 s → 4 s 封顶，
任何 `busy` 帧归零）。`crates/cuter-app/src/app.rs` 里那段注释解释了为什么不能直接
"零帧"：隐藏的延迟视口不跑自己的 pass，一旦完全没有待处理请求，贴图/面板就落地不了
（`pin_groups` 探针曾经从 1/0/1/1 掉成全 0）。所以退避是安全的那一档。

拆 DLL 对这个现象没有帮助：问题在页面换入节奏，不在代码体积；且 Rust 静态链接的
单体是刻意选择（无运行库依赖），没有拆。

### 84.2 控件拉齐：一处 `interact_size` 管住输入框与下拉框

egui 的 singleline `TextEdit` 高度和 `ComboBox` 按钮高度都取 `ui.spacing().interact_size.y`，
所以在 `paint_control` 开头加两行就够：

```rust
ui.spacing_mut().interact_size.y = Self::CONTROL_HEIGHT; // 28.0
ui.spacing_mut().button_padding = egui::vec2(10.0, 4.0);
```

放在 `paint_control`（设置窗口）而不是全局，遮罩工具条有自己的度量，不该被波及。

### 84.3 说明文字减负

共改 14 条（`crates/cuter-app/src/app.rs`），原则是**只留用户需要知道的结论，删掉实现细节**：

* 删：`走系统 UI Automation`、`一份含 version / url / notes 的 JSON`、`只写一条……HKCU 注册表项`
  这类"讲原理"的句子；
* 压：`使用 Windows 自带的 OCR：零下载、零打包、完全离线。语言取决于……` →
  `语言取决于系统里装了哪些 OCR 语言组件（见下方列表）。`
* 保留：会影响用户决策的那半句（`留空＝不检查`、`老机器上可以关掉`）。

i18n 表同步：`tools/i18n/en_table.py` 删 14 条旧键、加 14 条新键，`--write` 重生成
`crates/cuter-ui/src/i18n.table.rs`（489 条，UNUSED 仍是那 16 条已知的运行期查表误报）。

### 84.4 贴屏幕边拖动会把画面拖走——控制点压在了屏幕边界上

复现脚本 `target/tmp/overlay_edge.ps1`：`Alt+C` 开遮罩 → `Ctrl+A` 全选 → 指针移到
`(1918, 540)`（整屏选区 E 控制点就在 `(1920, 540)`）→ 按下左移 300 px。

同一个脚本、同一台机器，0.1.61 与 0.1.62 的日志对照：

```
0.1.61  遮罩松开：drag=Resize { handle: E, original: [[0.0 0.0] - [1920.0 1080.0]] }
0.1.62  遮罩松开：drag=Create { anchor: [1920.0 540.0] }
```

原因：`begin_drag` 的优先级是"控制点 > 选区内部 > 选区之外"，而整屏选区的控制点
正好落在屏幕边界上，指针贴边就会先命中它，于是拖动变成缩放整块选区——看起来就是
"画面被拖着走"。修法是新增 `handle_at_in_screen(rect, pos, screen)`：**贴着屏幕边的
控制点不参与命中**（`p.x > screen.min.x + 1.0 && p.x < screen.max.x - 1.0`，四个方向同理）。
贴边的边本来也无法再往外扩，跳过它不影响正向缩放；想缩小仍可从对面那条边拖。
单测 `handles_that_sit_on_the_screen_border_are_ignored` 覆盖了整屏、内缩、贴右三种情形。

### 84.5 这一轮新增的工具

* `target/tmp/gpu_idle.ps1`：待命 90 秒采样 GPU / WS / 私有字节，用来给"内存和 GPU"的话题
  一个可复现的数字（`Get-Counter` 的计数器名在中文系统上仍是英文，别翻译它）。
* `target/tmp/overlay_edge.ps1`：`-Exe` 可指定被测量的构建，用来做修前/修后的日志 A/B。

两者都在 `target/tmp`，属于开发期工具，不进发布包。

## 85. 第四条的真身：贴图窗口根本没有缩放手势（0.1.63）

§84.4 修的是遮罩的控制点，但用户 2026-10-21 又回来说「睁大眼睛好好看看第五条你解决没」。
重新查才定位到真正的对象：**贴图窗口**（`crates/cuter-ui/src/pin.rs`）。

### 85.1 两次误判，记下来省得再犯

| 误判 | 探针看到的事实 | 结论 |
|---|---|---|
| 以为是遮罩的控制点 | 整屏选区贴边按下，日志已经是 `Drag::Create` | 遮罩那条路径修对了，但不是用户说的地方 |
| 以为要"已有选区再贴边拖" | 画 `600,300→1919,700` 再抓右边，没有任何选区被移动 | 也不是 |

真正复现的脚本是 `target/tmp/pin_probe.ps1`：`Alt+C` 框选 → 回车复制 → `Alt+V` 贴图 →
读窗口 rect → 指针移到右边框内侧 2 px → 按下左拖 120 px。日志与截图给出两条硬证据：

```
贴图右边缘光标 = arrow            （没有任何缩放手势）
rect 700,520 424x246 → 580,520 424x246   （位置挪了 120 px，尺寸一点没变）
```

也就是说：贴图窗口只有"整窗拖动"一种手感，用户看到的"把屏幕帧搬走"完全成立。

### 85.2 实现：边/角命中 + 按位移缩放 + 位置钉住

* `PinEdge { N, S, E, W, Ne, Nw, Se, Sw }`，`cursor()` 给出对应的 `CursorIcon`；
  `edge_at_window(content, window, pos)` 先判"在不在窗口里"，再按**角优先、边其次**匹配，
  抓取范围 `EDGE_GRAB = 6.0` 点。注意必须把四周 12 点的阴影环算进来——
  `content_rect()` 是 `shrink(SHADOW_MARGIN)` 之后的内框，只按它判的话指针贴在最外圈
  会落到"移动窗口"分支（这正是第一版探针里 `arrow` 的原因）。
* `scale_after_drag(start_scale, edge, delta, start_content)`：用**按下时**的内容尺寸做分母，
  不能用"当前尺寸"——用当前尺寸会在缩小时自我放大，滚轮那边踩过同一个坑。
  写这条时实测到过一次"越拖越小、最后 44×35"的失控，原因就是分母用了 `content.size()`。
* 拖左边/上边时窗口要跟着挪：`at = outer.min + (start_size_pt - target_size_pt)`，
  单位必须统一——`PinAction::Resize` 的 `size` 是**物理像素**，`target_size` 是**点**，
  中间要除 `ui_scale`（125% 缩放下不除会一次跳 25%）。

### 85.3 两个连带缺陷

1. `desired_window_size()` 里 `.max(self.base_window_size)` 把"贴出来那一瞬的尺寸"当成了
   尺寸下限 → 窗口永远缩不回更小，滚轮缩小只缩内容、留一圈空边。删掉这个下限，
   只保留"内容 + 四周阴影"和菜单需要的额外高度。
2. `apply_geometry_fix` 每次对齐成功都发 `ViewportCommand::Focus`。贴图每拖动一帧都要
   重新对齐一次，于是每一帧都在抢焦点——抢焦点会打断拖动。新增参数 `focus_on_settle`：
   遮罩传 `true`（它要 Esc/回车），贴图传 `false`。

### 85.4 实测（`target/tmp/pin_probe2.ps1`，125% 缩放）

```
cursor at center:      arrow
rect after wheel-up:   700,520 448x259      （滚轮放大，窗口真的变大了）
cursor at right edge:  sizewe
rect after edge drag:  700,520 327x192      （位置没动，只缩尺寸）
cursor at left edge:   sizewe
rect after left drag:  753,520 274x163      （右边缘仍停在 1027）
```

证据图 `docs/design/shots/0.1.63-pin-edge-resize.png` 是同一块屏幕区域在拖动前后的裁切。


## 86. 屏幕最上面那一行从来不属于我们（0.1.64）

§85 修的是贴图窗口的缩放手势，用户 2026-10-21 直接回了一句「不是贴图，不是贴图，不是贴图」。
真身是**遮罩**：`Alt+C` 之后把指针放到**屏幕上沿**（含左右两个上角）按下左键拖动，
整块冻结帧跟着鼠标走；屏幕内部照常可以框选。

### 86.1 先证明"不是我们干的"

| 探针 | 读数 |
|---|---|
| `target/tmp/overlay_drag_probe.ps1`（边中、右边、左边各拖一次） | 窗口 rect 全程 `0,0 1920x1080`，松开也不变 |
| `target/tmp/shift_check.py`（`before` 与 `dragging` 两张全屏图比位移） | 最佳整体平移 `dx=0 dy=0`，残差 0.15（像素值），**画面一像素没动** |
| `target/tmp/overlay_top_probe.ps1`（上沿按下再拖） | 仍然只有选区在变，窗口不动 |

所以问题不在 `overlay.rs` 的 `begin_drag`，也不在 `ViewportCommand::StartDrag`（遮罩从不发它）。

### 86.2 真因：`HTCAPTION` 落在屏幕第一行

`target/tmp/overlay_hittest_probe.ps1` 直接向遮罩窗口发 `WM_NCHITTEST`：

```
(960,0)  → 2  HTCAPTION      ← 屏幕最上面一行
(960,1)  → 1  HTCLIENT
```

窗口样式 `0x16CB0000`，含 `WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX`；
`GetClientRect` 是 1920×1080，但客户区原点在 **(0,1)**——最上面那一行像素**不在客户区里**，
`DefWindowProc` 就把它当成标题栏。于是系统自己跑"拖标题栏"的模态循环，我们的进程
连按下事件都收不到，看起来就是"冻结帧被搬走"。

### 86.3 三条走不通的路（都试过）

| 改法 | 结果 |
|---|---|
| 摘 `WS_CAPTION` 等四个样式位 + `SWP_FRAMECHANGED` | 命中变成 `0 HTNOWHERE`，鼠标事件干脆掉到下面那个窗口去 |
| 再去掉 `WS_EX_WINDOWEDGE` | 仍然 `HTNOWHERE` |
| 改 `WS_POPUP` | 仍然 `HTNOWHERE` |

`HTNOWHERE` 比 `HTCAPTION` 更糟：遮罩自己的框选直接失效。

### 86.4 做法：只换窗口过程，不改样式不改几何

`crates/cuter-app/src/desktop.rs`：

* `overlay_wndproc`：`WM_NCHITTEST` 一律回 `HTCLIENT`，其余 `CallWindowProcW` 转给原过程；
* `own_hit_test(hwnd)`：`GetWindowLongPtrW(GWL_WNDPROC)` 存进 `OVERLAY_OLD_PROC` 再
  `SetWindowLongPtrW(hwnd, GWL_WNDPROC, ...)`；函数项转地址要 `as *const () as isize`，
  直接 `as isize` 会吃 `function_casts_as_integer`；
* `UiWake::take_over_hit_test(title)`：窗口还没建出来时返回 `false`，调用方留着下一帧再试
  （`App::overlay_hit_test_owned`，`open_overlay` 里重置）。

### 86.5 A/B 证据（同一个探针，两个可执行文件）

`tools/probe/overlay_top_edge_e2e.ps1`（退出码 0 通过 / 1 失败 / 2 跳过）同时问三件事：
命中码、拖动中的窗口 rect、松开后的 rect。

```
0.1.63（dist 里的包）   hit-test (960,0)/(2,0)/(1918,0) = HTCAPTION
                        rect before  0,0 1920x1080
                        rect dragging -120,144 1920x1080   ← 整窗被搬走
                        RESULT: FAIL
0.1.64（本次构建）      hit-test 三处 = HTCLIENT
                        rect before/dragging/after 全是 0,0 1920x1080
                        RESULT: PASS
```

### 86.6 教训

* "窗口铺满屏幕"不等于"整块都是客户区"：无装饰窗口仍可能带 `WS_CAPTION`，
  客户区还差一行；推断之前先 `SendMessage(WM_NCHITTEST)` 问系统。
* 截图对比只能证明"画面没动"，证明不了"系统把窗口搬走了"——后者要读窗口 rect。
* 换窗口过程比改样式安全：样式改坏了是 `HTNOWHERE`（事件丢失），换过程只改一个问题点。

## 87. 待命心跳的账：三支探针同时抓到（0.1.65）

0.1.62 为了省电把待命心跳从 1 s 放宽到 1 s / 2 s / 4 s 三档（第 84 节）。省电是真的
（GPU 0.094% → 0.036%），但"等下一帧"的路径全被拖慢了。全量套件（30 支）第一次跑出
**27 通过 / 3 失败**，三支失败原因完全一样：**根视口睡着了，而工作挂在"下一次出帧"上**。

### 87.1 三个症状与真实原因

| 探针 | 症状 | 真实原因 |
|---|---|---|
| `pin_groups_e2e` | 转发请求后窗口不该出现却出现了、该出现的晚一拍 | `logic()` 只在轮询里 `take_request()`，待命时 4 s 转一圈 |
| `tray_menu_e2e` | 按 `Esc` 后菜单窗口僵在屏幕上 3 s 以上 | 菜单是延迟视口，根视口睡着时它自己跑 pass；动作进了队列，没人收 |
| `update_check_e2e` | "假更新服务器起不来" | 本机 8901 已被别的开发服务占用并回 401 |

前两个是产品问题，第三个是**环境冲突被写成了产品失败**——探针不该假定端口空着。

### 87.2 修法

* **转发请求**：`crates/cuter-app/src/single_instance.rs` 的 `Guard` 起一条等待线程，
  `WaitForMultipleObjects(events, false, INFINITE)`；事件到了就把位图置上、投 `WM_PAINT`
  并 `ctx.request_repaint()`。`take_request_with()` 在起了线程之后改读位图（位图是
  `AtomicU64`，最多 64 种请求，`Activation::ALL` 现在 8 种）。
  线程里要自己重建 `HANDLE`：它是 `*mut c_void`，不是 `Send`（`error[E0277]`）。
* **托盘菜单**：`crates/cuter-app/src/app.rs` 的托盘视口闭包里，面板一返回动作就
  `send_viewport_cmd(Visible(false))` + `Close`（在**它自己的** pass 结束时立刻生效），
  再 `nudge()` + `request_repaint_of(ROOT)` 把根叫醒做记账。宿主侧 `close_tray_menu`
  也补了 `Visible(false)`：只发 `Close` 时窗口仍会多停 2 s 多。
* **探针端口**：`tools/probe/update_check_e2e.ps1` 增加 `Find-FreePort`，
  没显式传 `-Port` 就自己从 8901 往上找第一个能绑上的。

### 87.3 实测

* `tray_menu_e2e`：日志里"面板返回动作 Close"到"收到动作 Close"的间隔，修前
  **3.5 s**（`16:03:49.259` → `16:03:52.814`），修后同一帧内窗口就没了。
* `pin_groups_e2e`：修前稳定失败，修后 PASS。
* `update_check_e2e`：修前 `the fake update server did not come up`，修后
  `using port 8902` + PASS。

### 87.4 教训

* **省电的代价要按"最坏路径"算**：心跳退避影响的不只是热键延迟，还包括"队列里躺着
  一个动作，等根视口来收"。加一条 `busy` 条件不等于修好——根视口不被调度时，
  `busy` 根本没机会被求值。
* **延迟视口要能自己收尾**：`Visible(false)` + `Close` 必须在它自己的 pass 里发，
  指望宿主下一帧处理就是把菜单的生死挂在一个可能睡 4 s 的循环上。
* **探针不能假定端口空着**：失败了先看错误码，401 说明"有人在那儿"，不是"我们没起来"。

## 88. 方框字与"同一句日志写两遍"（0.1.66）

这一节记两件长期挂着的事：一件**修掉了根因**，一件**把判定做成了可执行的检查**。

### 88.1 方框字的根因：`set_fonts` 是"下一帧才生效"

用户两次反馈"概率性整片中文变方框"。0.1.62 之前做的是"把字体装载提到视口声明之前"，
只缩小了时间窗，没有消掉它——因为 `egui::Context::set_fonts` **要到下一帧开头**才重建
字体表。而 0.1.56 起为了省 19 MB，窗口全关 2 秒后 `release_font()` 会把字体卸掉
（`set_fonts(FontDefinitions::default())`），下次开窗口再装：

```
卸载（第 N 帧）→ 用户开窗口（第 N+k 帧）→ ensure_font 装回 → 这一帧仍用旧字体表 → 方框
```

修法：**装一次就不再卸**。`release_font` 整个删掉，`font_idle_since` 字段一起删。
任何窗口第一次打开时字体已经在 `Context` 里，"这一帧没有中文字形"这个状态不存在了。

代价（实测，`target\release\cuter.exe`，125% 缩放）：

| 场景 | 工作集 | 私有提交 |
|---|---|---|
| 从未开过窗口（纯托盘） | 59.2 → **6.2 MB**（闲置 40 s 收紧后） | **28.5 MB** |
| 开过设置窗口再关掉、闲置 42 s | 87.2 → **6.3 MB** | **51.5 MB** |

也就是说：**任务管理器里看到的内存没变**（工作集照样被 `EmptyWorkingSet` 收回去），
但私有提交会多留一份字体（约 23 MB），且是常量、不随使用时长增长。
这是拿"确定的 23 MB"换"不可能出现的方框字"，选择记录在此，改回去的门槛也写清楚：
若要省这 23 MB，就得把"开窗口"推迟一帧（字体装完、`request_repaint` 之后下一帧再
`show_viewport_deferred`），那是另一套改动。

### 88.2 日志每条写两遍：先把"能不能判定"补上

现象：`logs\cuter.log` 里偶尔出现**相邻两行消息体完全相同**（时间戳同毫秒），
且只在部分启动里出现；用 `-RedirectStandardError` 启动时不复现。

`MultiLogger` 本身没有问题（console + file 各写一次，`init()` 只在 `main` 调一次），
所以"写两遍"要么是消息被发了两遍，要么是同一文件落进了两个进程的内容——**没有进程号
就分不清**。所以这一版先做两件小事：

1. 每行日志加上 `pid=1234`（`crates/cuter-app/src/logging.rs` 的自定义 format）。
2. 新增探针 `tools/probe/log_integrity_e2e.ps1`：连续启动 3 次，每次断言
   "所有行同一个 pid" + "没有相邻重复行"。当前 3/3 通过。

这件仍**没有定位**：修的是"能不能发现"，不是"为什么"。下次再出现时，pid 会直接告诉我们
是"一条写两次"还是"两个进程各写一次"。

## 89. 贴图那圈"很宽的大边框"（0.1.67）

用户的原话是"贴图有一个很宽的大边框，我不喜欢这个边框"。这里说的不是选中态的高亮描边，
而是贴图窗口四周那圈投影留白——它在浅色桌面上读起来就是边框。

### 89.1 为什么它看着像边框

三个参数叠在一起的效果：

| 参数 | 0.1.66 | 看得出来什么 |
| --- | --- | --- |
| `SHADOW_MARGIN` | 12 px | 图片四周一圈 12 像素的"空"，窗口比图大一圈 |
| `SHADOW_LAYERS` × alpha | 5 层 × 12/255 | 叠出的是**匀色**灰边，中间没有渐变过渡 |
| 每层圆角 | 写死 `CornerRadius::same(6)` | 图片圆角 20+ 时，四角露出一小块**方**的灰边 |

单看每一项都合理：想要阴影、想要柔和、想要圆角。凑在一起就是"相框"——因为 12 像素宽
的匀色环，人眼第一反应是边框而不是投影。

### 89.2 现在是什么

| 参数 | 0.1.67 | 为什么 |
| --- | --- | --- |
| `SHADOW_MARGIN` | **7 px** | 留白越宽越像框；7 像素还能容下 6 像素的抓取带 |
| `SHADOW_LAYERS` × alpha | **9 层 × 5/255** | 层多、每层淡，累加 17% → 2% 才有渐变感 |
| `SHADOW_DROP` | **1.5 px** | 整体下移，读起来是"浮在桌面上"而不是"描了一圈" |
| 每层圆角 | `图片圆角 + 外扩量` | 四角不再露方边 |

边缘缩放用的是 `edge_at_window`，它把这圈留白也算进可抓范围；留白从 12 收到 7 之后
抓取带仍是满的（`EDGE_GRAB` 6 像素）。实测：右边缘拖 439×250 → 318×183，左边缘拖着走时
窗口 x 跟着变、右边缘不动。

### 89.3 教训

* **"看起来像什么"要按最坏背景判断**：深色桌面上那圈灰影挺好看，浅色桌面上就是边框。
* **圆角是三个地方的事**：图片、窗口、阴影。少改一处，四角就会露馅。
* 参数写在常量上、并且带一句"为什么是这个数"，下次才敢动。

## 90. 日志"每句写两遍"的根因：句柄数值被复用（0.1.68）

§88 记过这件事：日志文件里每条连着出现两遍，加 pid 之后排除了"两个进程各写一遍"，
但"为什么"一直没答上来。这一节是答案。

### 90.1 现状

修复前：约一半的启动会重复，每次 12 对，两条相隔 0~1 ms；重定向 stderr 到文件就干净。

### 90.2 定位过程（三次实验）

| 实验 | 做法 | 结果 |
| --- | --- | --- |
| 计数 | `MultiLogger::log` 里打到 stderr 一个自增序号 | 带重定向：12 条记录 12 行；不带重定向：36 行（12 序号 + 24 正文），序号也落在 **cuter.log 里** |
| 序号 + 线程 | 格式化时加 `seq=`（每次格式化自增）`tid=` | 重复的两条是 `seq=0` / `seq=1`、同一个 `ThreadId(1)`：**同一条记录被格式化两次，两份都写进同一个文件** |
| 变量隔离 | 固定版本连跑 5 次 | 3 次重复、2 次干净——与"句柄槽是否被复用"这种时序相关的现象一致 |

第二行是关键：`seq` 相差 1 说明两次格式化都发生了（控制台一份、文件一份），而**两份都落在
cuter.log**——所以不是"写了两次"，而是"控制台那份写错了地方"。

### 90.3 根因

`main()` 第一步 `detach_orphan_console()`：双击时 Windows 先给进程建一个控制台，
`GetConsoleProcessList()` 返回 1 说明只有自己在用，于是 `FreeConsole()` 丢掉它。

问题在 `FreeConsole` 之后：**它关掉了三个标准句柄，却没有清掉进程环境块里存的那三个数值**
（`SetStdHandle` 从来没被调用过）。随后 `logging::init()` 打开 `cuter.log`，Windows 分配
句柄时优先用**最小的空槽**——正好是刚被释放的那几个数值之一。于是：

```
GetStdHandle(STD_ERROR_HANDLE) -> 数值 3   （其实是那个文件）
```

控制台日志器把格式化结果写到"stderr"，系统就写进了 cuter.log。文件日志器再写一份。
两份内容、时间戳、pid 全都一样，因为它们是**同一条记录**的两次格式化。

为什么只有一半的启动会中：取决于打开 cuter.log 时那几个槽是不是最小的空槽，
而启动过程中还会开别的句柄（注册表、字体文件、事件）。

### 90.4 修法

```rust
let _ = unsafe { FreeConsole() };
// 环境块里的数值还在；之后 CreateFile 可能复用同一个数值
let none = windows::Win32::Foundation::HANDLE::default();
let _ = SetStdHandle(STD_INPUT_HANDLE, none);
let _ = SetStdHandle(STD_OUTPUT_HANDLE, none);
let _ = SetStdHandle(STD_ERROR_HANDLE, none);
```

清空之后 `GetStdHandle` 返回空，写 stderr 只会失败（那正是"没有控制台"该有的样子），
不会落到任何文件上。

### 90.5 验证

* `target\release` 连续启动 6 次：**0 处相邻重复**（修复前 6 次里 3 次重复）；
* `tools\probe\log_integrity_e2e.ps1` 连跑 3 次（每次 3 次启动，共 9 次）：全 PASS，
  且不再出现 WARN；
* 顺手给日志加了 `CUTER_LOG_SEQ=1`：每行多 `seq=`/`tid=` 两个字段，下次再遇到
  "写两遍"能一眼看出是两次格式化还是两次记录。

### 90.6 教训

* **"关掉了"不等于"忘了"**：Win32 的句柄是数值，关掉之后数值仍在环境块里，会被复用。
  凡是 `FreeConsole`/`CloseHandle` 之后还要用标准句柄的代码，都得显式清。
* **加一个自增序号**比加更多日志更有用：它把"同一个东西写两次"和"两个东西各写一次"
  这两种假设一次分开。
* 现象与"是否重定向 stderr"相关时，先怀疑**句柄指向哪里**，而不是"谁多写了一次"。

## 91. 更新检查的三条礼貌规则（0.1.69）

### 91.1 为什么要节流

0.1.47 做出更新检查时只写了"启动时查一次"。那意味着**每次双击都会往用户的服务器发一个
请求**——对一个本地工具来说，这个频率没有正当理由：用户几天才装一次新版，检查一天一次
就足够让他看见。所以加了 `general.last_update_check`（Unix 秒）：启动检查只在距上次
**成功**检查 ≥ 24 小时时才真的发请求，手点「立即检查」永远放行。

### 91.2 时间戳只记成功

失败也写时间戳会造出一个更糟的行为：用户某天开机时网络没起来 → 检查失败 → 时间戳被写成
"刚查过" → 接下来一整天都不再检查。所以只有 `Newer` / `Current` 两种结果才落盘，
`Failed` 只记日志。`sanitize()` 里把负数夹成 0：手改配置写成 `-1` 等于把自动检查关掉。

### 91.3 跳过名单只记一个版本

`general.skipped_version` 存"用户点过跳过的那个版本"，不存历史。理由是跳过是**针对版本**
的表态，不是永久的"别再打扰我"：下一个版本出来，用户有权重新被提示一次。命中的版本仍然
会去查（要能在「关于」页看到它），只是不再推到状态栏——日志里那行带 ASCII 标记
`[update] skip-hit`，探针就靠它判定，不用去比中文字符串。

### 91.4 探针怎么验

`tools/probe/update_check_e2e.ps1` 从 4 个分支扩到 6 个，全部用**假服务端**（`fake_update_server.py`）：

| 分支 | 断言 |
|---|---|
| A 有新版本 | 服务端收到 `GET /update.json` + `UA=Cuter/*`；配置里落下了 `last_update_check > 0` |
| B 开关关 | 服务端一个请求都没有，日志里没有 `[update]` |
| C 地址空 | 同上（这是"默认配置一个字节都不发"的承诺） |
| D 真 https | 请求走到 TLS 后失败（example.com 不是清单），不会卡死 |
| E 节流 | 时间戳是"刚查过" → 不发请求、日志有 `[update] throttled`；改成 25 小时前 → 重新发 |
| F 跳过名单 | 仍然发请求；日志有 `[update] skip-hit`；「关于」页显示"已跳过，不再提示" |

### 91.5 教训：证据图不能用截屏

第一版证据图靠 `CopyFromScreen`，结果三张全是**白板**——窗口被别的窗口盖住时，屏幕上那块
区域什么也没画，而应用自己的画面一直是好的（`PrintWindow` 一抓就有内容）。探针的截图函数
因此改成：先 `SetForegroundWindow`，再用 `PrintWindow` 抓**窗口自己的表面**，失败才退回截屏。
同理，看到"界面一片空白"先别怀疑渲染：先确认窗口是不是被挡住了。

另外 `Add-Type -Path` 不带额外引用，所以 `PrintWindow` 的封装不能塞进共享的
`CuterInputProbe.cs`（那会让所有探针都编不过，`System.Drawing` 找不到），它写在探针脚本里、
用自己的 `Add-Type -ReferencedAssemblies System.Drawing, System.Windows.Forms`。

---

## 92. 手动滚动截屏：为什么它值得一条探针

### 92.1 两种滚动，只有一种被测过

滚动截屏有两条路径：

* **自动滚动**：程序自己用 `SendInput` 往目标窗口送滚轮（`scroll_auto_e2e.ps1` 覆盖）。
* **手动滚动**：人自己滚，程序只负责"看画面动没动、动多少、拼上去"。

用户实际用的是第二种（他原话：滚动截屏我自己手动只要功能正常就可以）。可 0.1.69 之前，
第二种一条断言都没有——它只在演示视频里被"看着像是好的"。

### 92.2 探针怎么造出"人在滚"

`tools/probe/scroll_manual_e2e.ps1`：

1. 把配置写成 `[scroll] auto_scroll = false`——否则探针测到的还是自动模式。
2. Alt+S 进滚动截屏，拖一块 750×550 的选区，Enter 开始采集。
3. **探针自己** `Wheel(-3)` 十四轮、每轮间隔 450 ms。这个节奏是有意的：拼接判据靠"相邻帧
   的重叠行"，一轮滚太多（或滚太快）会让重叠区域小于阈值，看起来就像拼接失败。
4. 激活控制窗再 Enter 完成——滚动时焦点在目标窗口上，直接敲 Enter 会打空。

### 92.3 断言只认应用自己打的 ASCII 标记

```
scroll-done frames=23 w=750 h=3493
```

* `frames > 1`：首帧之后还在收帧（人在滚，程序看得到）。
* `h > 选区高度`：帧被拼进去而不是被丢掉。
* 滚轮没送到目标（`frames <= 1`）报 **SKIPPED**：那是环境吞了合成输入，没测到东西，
  SKIP 绝不能伪装成 PASS。

标记是 ASCII 的，因为 `.ps1` 必须是纯 ASCII（无 BOM 的脚本会被 PowerShell 5.1 按 GBK 读，
中文注释会连注释后的换行一起吞掉，见第 87、89 节）。

### 92.4 实测

第一次跑就通过：23 帧、750×550 的选区拼成 750×3493（高度是选区的 6.3 倍）。

### 92.5 教训

**"我看着是好的"不是验证**。这条路径从 0.1.44 就在，演示过、人工试过，但没有任何一行断言
在它上面。用户明确说"手动能用就行"的时候，最该做的不是写文档保证它能用，而是补一条探针
把它钉住——否则下一次重构拼接判据时，谁也不知道有没有把它弄坏。

---

## 93. 公开前的清理：去掉别人家的名字，去掉这一台机器的路径

### 93.1 第三方工具的名字

准备开源上传，第一件事是把"参考某某工具"这类字眼清掉：README、HANDOFF 的历史记录、
代码注释里一共有 39 处直接点名了别的截图工具（功能对照表、键位冲突的教训、
交互约定的来源）。全部改写成"同类工具"这类中性说法，**技术结论一条没丢**：

* 「别抄某某的键位」→「别用别人占着的键位」，仍然保留"`Shift+F3`/`Ctrl+F3` 常被常驻的截图
  程序占着，`RegisterHotKey` 会失败（日志 `HotKey already registered`）"这条实证；
* README 的对照表从"Electron 系（某工具）"改成"Electron 系同类工具"，数字不变；
* LICENSE 那行不再说"与参考项目一致"，只写 `GPL-3.0-or-later`。

### 93.2 这一台机器的绝对路径

顺手发现更该修的东西：多个脚本把本机路径写死在默认值里——`check.ps1` 的 Python、
`extract_keys.py`/`fix_fill_args.py` 的 `ROOT`、两个探针的 `$Python`。别人克隆下来直接跑，
门禁第一步就会因为找不到解释器而失败。

* `tools/ci/check.ps1` 与新增的 `tools/probe/python.ps1` 都改成"逐个候选验证"：
  `-Preferred` / `$env:CUTER_PYTHON` / PATH 上的 `python` / `py` / 用户目录安装，
  **每个都要跑一次 `--version` 并且退出码为 0 才认**。
* 为什么必须验证：Windows 上 PATH 里的 `python` 常常是应用商店别名存根——文件存在、
  执行无输出、退出码 **9009**。第一版只判断"文件存在"，门禁因此报
  `docs references ... FAILED in 0.1s (exit 9009)`，看起来像脚本坏了。
* Python 工具改用 `Path(__file__).resolve().parents[2]` 定位仓库根；文档里的示例路径换成
  `%APPDATA%` / `%USERPROFILE%` / `<repo>`。

### 93.3 顺手修掉的两个隐患

* `tools/probe/ocr_http_e2e.ps1` 里有四行中文注释——**违反 ASCII-only 规矩**（无 BOM 的
  `.ps1` 会被 PS 5.1 按 GBK 读，中文注释会吞掉后面的换行）。之前一直没炸是运气，
  这次改成英文注释。
* `tools/probe/log_integrity_e2e.ps1` 的注释还写着"根因还没定位"，实际上 0.1.68 已经定位
  并修好（第 90 节），一并更正。

### 93.4 验证

`check.ps1 -SkipBuild`：clippy 31.2 s、测试 61 + 168 + 112 + 6 + 1、i18n 495 条
（UNUSED 16）、文档引用 9 个文件 / HANDOFF 93 节，全绿；
`ocr_http_e2e.ps1` 与 `update_check_e2e.ps1` 改用共享解析器后各跑一次，都 PASS。

### 93.5 教训

**"能跑"的脚本不等于"别人能跑"**。写死本机路径的默认值在开发机上永远是对的，
所以它不会报错、不会引起注意，只有把仓库交给第二个人时才会暴露。
公开前该做一遍"把仓库换个路径、换个用户名再跑一遍"的检查——这次是靠搜绝对路径发现的。

---

