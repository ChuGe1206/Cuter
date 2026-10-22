# 滚动截屏的端到端测试（无人交互）。
#
# ⚠ 本文件必须带 UTF-8 BOM 保存。Windows PowerShell 5.1 在没有 BOM 时会把 .ps1 当成
#   ANSI(GBK) 解析，中文注释的字节会被吞掉换行符，把下一行代码并进注释里——
#   症状是「某个变量莫名其妙是 $null」，极难排查。tools\probe\README.md 里有记录。
#
# 它做的是「一个真实用户会做的事」：
#   1. 开一个可滚动的目标窗口（记事本 + 300 行长文本）；
#   2. 用全局热键 Alt+S 让 Cuter 进入滚动截屏模式；
#   3. 在记事本里拖出一个选区，按回车开始采集；
#   4. 用真实滚轮事件把记事本往下滚；
#   5. 激活控制窗按回车「完成」；
#   6. 断言落盘的长图高度**远大于**选区高度，宽度等于选区宽度。
#
# 所有输出信息都用 ASCII，避免控制台代码页把中文写成乱码；标题片段一律用码点拼出来。
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root

Add-Type -Path (Join-Path $root 'tools\probe\CuterInputProbe.cs')

function Say($m) { Write-Host $m }

# 任何一步失败都要把进程和配置收干净：否则残留的 cuter 会占着 Alt+S 热键，
# 让下一次运行出现「遮罩永远不出现」这种毫不相关的假故障。
trap {
    Write-Host "FAILED: $($_.Exception.Message)"
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    # 这里的路径必须自己拼：脚本前半段抛错时，下面那些变量还没赋值，
    # trap 里引用它们会再抛一次异常，把真正的错误盖掉。
    $cfg = Join-Path $env:APPDATA 'Cuter\config.toml'
    if ($configBackup -and (Test-Path $configBackup)) { Copy-Item $configBackup $cfg -Force }
    exit 1
}

# 备份名带时间戳：**每次运行都留一份**，绝不覆盖上一次的备份。
# 固定名会串联覆盖——第一轮失败留下的默认配置会被第二轮当成"用户配置"备份下来，
# 于是用户真正的配置就被永久顶掉了（本轮真踩过，好在内容本来就是默认值）。
$configBackup = Join-Path $env:TEMP ("cuter-config-backup-{0}.toml" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))

# 尝试把本线程切成物理像素坐标系（成功率取决于宿主，后面会用 Awareness() 实测）
[CuterInputProbe]::SetPhysicalDpiAwareness() | Out-Null
$awareness = [CuterInputProbe]::Awareness()
Say "probe thread awareness = $awareness (0=unaware, 1=system, 2=per-monitor); screen=$([CuterInputProbe]::ScreenSize())"

# 目标窗口与控制窗的标题片段（由码点拼出，源码保持 ASCII-safe）
$titleShot = [string]([char]0x622A) + [char]0x56FE                       # overlay: "Cuter " + jietu
$titleScroll = [string]([char]0x6EDA) + [char]0x52A8 + [char]0x622A + [char]0x5C4F  # scroll panel title

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$outDir = Join-Path $root 'target\e2e\scroll'
$noteFile = Join-Path $env:TEMP 'cuter-scroll-target.txt'

# ---------- 0. 清场 + 备份旧配置 ----------
# 残留的 cuter 实例会抢走 Alt+S 热键：新实例注册不上，热键进了旧实例，于是
# 「按了 Alt+S 但新实例的遮罩永远不出现」——这个症状极具误导性，所以先清场。
$stale = @(Get-Process cuter -ErrorAction SilentlyContinue)
if ($stale.Count -gt 0) {
    Say "killing $($stale.Count) stale cuter process(es) - they would steal the Alt+S hotkey"
    $stale | Stop-Process -Force
    Start-Sleep -Milliseconds 500
}
Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

if (Test-Path $configPath) { Copy-Item $configPath $configBackup -Force; Say "config backed up to $configBackup" }
New-Item -ItemType Directory -Force -Path (Split-Path $configPath) | Out-Null
Remove-Item $outDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$config = @"
[general]
start_in_tray = true
close_to_tray = true

[capture]
hide_own_windows = true
delay_ms = 0
play_sound = false
include_cursor = false
# Snapping off (both kinds): this probe compares an EXACT selection against the saved image.
auto_detect_windows = false
snap_controls = false
show_magnifier = false

[save]
dir = '$outDir'
format = "png"
auto_save = true
ask_path_on_save = false
auto_copy = false
reveal_after_save = false

[ocr]
auto_after_capture = false
engine = "disabled"

[scroll]
interval_ms = 150
max_height = 20000
min_offset = 2
max_offset = 1200
"@
# Windows 路径必须写成 TOML 的「字面量字符串」（单引号）。用双引号时 `\C` 不是合法转义，
# 整份配置会被解析器拒绝——app 会把坏文件改名成 config.toml.bak 并回退默认值，于是测试
# 看起来「跑通了」，其实所有设置都没生效（第一次跑就是这样：没有任何 PNG 落盘）。
Set-Content -Path $configPath -Value $config -Encoding UTF8
Remove-Item (Join-Path (Split-Path $configPath) 'config.toml.bak') -Force -ErrorAction SilentlyContinue
Say "config written: $configPath"

# ---------- 1. 目标窗口：记事本 + 300 行 ----------
$lines = 1..300 | ForEach-Object { "SCROLL E2E LINE {0:D4} ----------------" -f $_ }
Set-Content -Path $noteFile -Value $lines -Encoding UTF8
$notepad = Start-Process notepad.exe -ArgumentList $noteFile -PassThru
Start-Sleep -Seconds 2
$npHwnd = [CuterInputProbe]::FindWindow([uint32]$notepad.Id, 'cuter-scroll-target')
if ($npHwnd -eq [IntPtr]::Zero) { $npHwnd = [CuterInputProbe]::FindWindow([uint32]$notepad.Id, 'Notepad') }
if ($npHwnd -eq [IntPtr]::Zero) { throw 'notepad window not found' }
# 摆到固定位置，位置与尺寸都用「探针单位」，和后面读回来的矩形同一套单位
[CuterInputProbe]::MoveWindow($npHwnd, 100, 100, 900, 700, $true) | Out-Null
Start-Sleep -Milliseconds 500
$npRect = @(([CuterInputProbe]::Describe($npHwnd)) -split '\|')
$npX = [int]$npRect[1]; $npY = [int]$npRect[2]; $npW = [int]$npRect[3]; $npH = [int]$npRect[4]
Say "notepad: hwnd=$($npRect[0]) x=$npX y=$npY w=$npW h=$npH"

# 选区：记事本客户区里的一块（避开标题栏和边框；比吸附容差 8px 远得多）
$selLeft = $npX + 30
$selTop = $npY + 70
$selRight = $npX + $npW - 30
$selBottom = $npY + $npH - 30
$selW = $selRight - $selLeft
$selH = $selBottom - $selTop
Say "selection(probe units): ($selLeft,$selTop)-($selRight,$selBottom) = ${selW}x${selH}"
if ($selW -le 0 -or $selH -le 0) { throw 'selection is degenerate - variable got clobbered?' }

# ---------- 1b. 基准图：同一块区域的普通截图 ----------
# 长图的前 $selH 行必须和它逐像素一致（闪烁的光标最多造成零点几的平均偏差），
# 否则说明拼接的起点就已经错了——只看「高度变大了」是查不出这种问题的。
$refPath = Join-Path $outDir 'reference.png'
& (Join-Path $root 'target\release\cuter.exe') --region "$selLeft,$selTop,$selW,$selH" --out $refPath | Out-Null
if (-not (Test-Path $refPath)) { throw "reference capture failed: $refPath" }
Say "reference capture: $refPath = ${selW}x${selH}"

# ---------- 2. 启动 Cuter，用 Alt+S 进入滚动截屏 ----------
# stderr 落盘：`start_scroll` 会把真实采集区域打进去，这是排查「选区与像素对不上」的硬证据
$logPath = Join-Path $outDir 'cuter-stderr.log'
$cuter = Start-Process (Join-Path $root 'target\release\cuter.exe') -PassThru -RedirectStandardError $logPath
Start-Sleep -Seconds 2

# 快速失败：app 拒绝我们的配置时会把它改名成 config.toml.bak 并回退默认值。
# 不检查的话，测试会「通过」但所有设置（尤其是 auto_save / 保存目录）都没生效。
$rejected = Join-Path (Split-Path $configPath) 'config.toml.bak'
if (Test-Path $rejected) { throw "app rejected the test config (found $rejected) - check TOML syntax/values" }
Say "config accepted by the app"

[CuterInputProbe]::Activate($npHwnd) | Out-Null

[CuterInputProbe]::AltKey(0x53)   # Alt+S
$overlay = [IntPtr]::Zero
foreach ($i in 1..40) {
    Start-Sleep -Milliseconds 250
    $overlay = [CuterInputProbe]::FindWindow([uint32]$cuter.Id, $titleShot)
    if ($overlay -ne [IntPtr]::Zero) { break }
}
if ($overlay -eq [IntPtr]::Zero) { throw 'overlay did not appear after Alt+S' }
$oRect = @(([CuterInputProbe]::Describe($overlay)) -split '\|')
Say "overlay: hwnd=$($oRect[0]) x=$($oRect[1]) y=$($oRect[2]) w=$($oRect[3]) h=$($oRect[4])"

# 探针单位 → 物理像素 的换算系数。不感知 DPI 时窗口坐标是被缩放过的。
$dpi = [CuterInputProbe]::DpiOf($overlay)
$factor = 1.0
if ($awareness -eq 0 -and $dpi -gt 0) { $factor = $dpi / 96.0 }
Say "overlay dpi=$dpi -> probe-unit-to-physical factor=$factor"
Say "overlay physical size = $([int]([int]$oRect[3] * $factor))x$([int]([int]$oRect[4] * $factor))"

# 遮罩的「窗口矩形」与「客户区」不重合：Windows 会给它一条 1 像素的非客户区上边，
# 而 egui 全程用**客户区**坐标。所以「光标停在屏幕 y=170」实际选中的是桌面帧的第 169 行。
# 这个内缩量必须实测，不能假设——它决定了基准图要和长图的哪一行对齐。
$clientInfo = @(([CuterInputProbe]::ClientInfo($overlay)) -split '\|')
$clientOrigin = @($clientInfo[0] -split ',')
$insetX = [int]$clientOrigin[0] - [int]$oRect[1]
$insetY = [int]$clientOrigin[1] - [int]$oRect[2]
Say "overlay client origin=($($clientOrigin[0]),$($clientOrigin[1])) size=$($clientInfo[1]) -> inset=($insetX,$insetY)"

# ---------- 3. 框选 + 回车开始采集 ----------
# 遮罩刚出现时还在被搬到显示器工作区上，这时候按下的合成拖拽会被它记成一个很小的
# 选区（实测选中 840x600 的拖拽只换来 140x100 的采集区域，日志里 `滚动截屏开始：区域`
# 的 x 还落在拖拽的**终点**附近）。所以先等它安顿好，再拖。
Start-Sleep -Milliseconds 900
[CuterInputProbe]::Drag($selLeft, $selTop, $selRight, $selBottom)
Start-Sleep -Milliseconds 400
[CuterInputProbe]::Key(0x0D)   # Enter
Say "dragged selection and pressed Enter"

$panel = [IntPtr]::Zero
foreach ($round in 1..3) {
    foreach ($i in 1..24) {
        Start-Sleep -Milliseconds 250
        $panel = [CuterInputProbe]::FindWindow([uint32]$cuter.Id, $titleScroll)
        if ($panel -ne [IntPtr]::Zero) { break }
    }
    if ($panel -ne [IntPtr]::Zero) { break }
    # 遮罩偶尔吞掉第一次拖拽或回车：重做一次选区，别让整条用例因为一次手抖白跑
    Say "scroll control window missing (round $round) - retrying the selection"
    [CuterInputProbe]::Activate($overlay) | Out-Null
    [CuterInputProbe]::Drag($selLeft, $selTop, $selRight, $selBottom)
    Start-Sleep -Milliseconds 400
    [CuterInputProbe]::Key(0x0D)
}
if ($panel -eq [IntPtr]::Zero) { throw 'scroll control window did not appear' }
$pRect = @(([CuterInputProbe]::Describe($panel)) -split '\|')
Say "scroll panel: hwnd=$($pRect[0]) x=$($pRect[1]) y=$($pRect[2]) w=$($pRect[3]) h=$($pRect[4])"

# 控制窗绝不能压在采集区域上，否则它会被逐帧截进长图（两个矩形同一套单位，可直接比）
$pL = [int]$pRect[1]; $pT = [int]$pRect[2]
$pR = $pL + [int]$pRect[3]; $pB = $pT + [int]$pRect[4]
$overlap = ($pL -lt $selRight) -and ($selLeft -lt $pR) -and ($pT -lt $selBottom) -and ($selTop -lt $pB)
Say "panel rect=($pL,$pT)-($pR,$pB) selection=($selLeft,$selTop)-($selRight,$selBottom) overlap=$overlap"
if ($overlap) { throw 'control panel overlaps the captured region' }

# ---------- 4. 真实滚轮滚动 ----------
[CuterInputProbe]::Activate($npHwnd) | Out-Null
[CuterInputProbe]::MoveTo($selLeft + 40, $selTop + 60) | Out-Null
foreach ($i in 1..14) {
    [CuterInputProbe]::Wheel(-1)
    Start-Sleep -Milliseconds 150
}
Say "scrolled 14 wheel clicks down"
Start-Sleep -Seconds 2

# ---------- 4b. reverse: scroll back up ----------
# The document starts at the top, so this cannot add rows above the first frame - what it
# proves is that the reverse path prepends *nothing wrong*: the stitched image must keep its
# exact width, still be much taller than the selection, and its first rows must still equal
# the baseline screenshot taken before any scrolling.
foreach ($i in 1..5) {
    [CuterInputProbe]::Wheel(1)
    Start-Sleep -Milliseconds 250
}
Say "scrolled 5 wheel clicks back up (reverse path)"
Start-Sleep -Seconds 2

# ---------- 5. 完成 ----------
[CuterInputProbe]::Activate($panel) | Out-Null
[CuterInputProbe]::Key(0x0D)   # Enter = finish
Start-Sleep -Seconds 3
Say "panel after finish: $([CuterInputProbe]::Describe([CuterInputProbe]::FindWindow([uint32]$cuter.Id, $titleScroll)))"

# ---------- 6. 内容断言 ----------
# PNG 的 IHDR 里宽高是大端 4 字节，直接从文件头解析，不依赖任何图像库。
$pngs = Get-ChildItem $outDir -Filter *.png -ErrorAction SilentlyContinue | Where-Object { $_.Name -ne 'reference.png' }
if (-not $pngs) { throw "no stitched png was written to $outDir" }
$png = $pngs | Sort-Object LastWriteTime -Descending | Select-Object -First 1
$bytes = [System.IO.File]::ReadAllBytes($png.FullName)
$w = [int]$bytes[16] * 16777216 + [int]$bytes[17] * 65536 + [int]$bytes[18] * 256 + [int]$bytes[19]
$h = [int]$bytes[20] * 16777216 + [int]$bytes[21] * 65536 + [int]$bytes[22] * 256 + [int]$bytes[23]
$expectW = [int]($selW * $factor)
$expectH = [int]($selH * $factor)
Say "saved: $($png.Name) = ${w}x${h} ($($png.Length) bytes)"
Say "expected physical selection: ${expectW}x${expectH}"

# 逐像素比对（System.Drawing 只有 Windows PowerShell 有，正好够用）
Add-Type -AssemblyName System.Drawing
$stitched = [System.Drawing.Bitmap]::FromFile($png.FullName)
$reference = [System.Drawing.Bitmap]::FromFile($refPath)

# 平均绝对偏差：在 (x,y) 采样，比较两张图相隔某个纵向偏移的同一位置
function Get-MeanDeviation($a, $b, $ay, $by, $rows) {
    $sum = 0.0; $n = 0
    for ($y = 0; $y -lt $rows; $y += 2) {
        for ($x = 0; $x -lt $a.Width; $x += 4) {
            $pa = $a.GetPixel($x, $ay + $y)
            $pb = $b.GetPixel($x, $by + $y)
            $sum += [math]::Abs($pa.R - $pb.R) + [math]::Abs($pa.G - $pb.G) + [math]::Abs($pa.B - $pb.B)
            $n += 3
        }
    }
    return $sum / [math]::Max($n, 1)
}

if ($h -ge $expectH) {
    # 长图的前 $expectH 行应当和基准图逐像素相同，只是整体错开一个客户区内缩量
    $devTop = Get-MeanDeviation $stitched $reference $insetY 0 $expectH
    Say ("deviation(stitched top+inset vs plain capture) = {0:N3}" -f $devTop)

    # 末尾那一段如果和基准图完全相同，说明「拼接」只是把最后一屏又抄了一遍
    $appended = $h - $expectH
    $devDup = -1.0
    if ($appended -gt 20) {
        $devDup = Get-MeanDeviation $stitched $reference ($appended + $insetY) 0 $expectH
        Say ("deviation(stitched tail vs plain capture) = {0:N3}  (appended {1} px)" -f $devDup, $appended)
    }
    $stitched.Dispose(); $reference.Dispose()

    if ($devTop -gt 3.0) {
        throw "stitched top band does not match the plain capture (dev=$devTop, inset=$insetY)"
    }
    if ($devDup -ge 0 -and $devDup -lt 3.0) { throw "stitched tail is a duplicate of the first screenful (dev=$devDup)" }
} else {
    $stitched.Dispose(); $reference.Dispose()
}

# 收尾：关进程、还原配置
Stop-Process -Id $cuter.Id -Force -ErrorAction SilentlyContinue
Stop-Process -Id $notepad.Id -Force -ErrorAction SilentlyContinue
if (Test-Path $configBackup) { Copy-Item $configBackup $configPath -Force } else { Remove-Item $configPath -Force -ErrorAction SilentlyContinue }

if ([math]::Abs($w - $expectW) -gt 3) { throw "width mismatch: stitched $w vs selected $expectW" }
if ($h -le $expectH) { throw "height did not grow: $h <= $expectH" }
Say "PASS: stitched ${w}x${h} starts with the exact selected content and is $([math]::Round($h / $expectH, 2))x taller"
