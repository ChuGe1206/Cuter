# 单独验证：遮罩窗口（egui 多视口）的「窗口矩形」和「客户区」是否重合。
#
# 背景：端到端测试里发现，从屏幕 (130,170) 起拖，app 日志报告的区域是 (130,169)。
# egui 拿到的是**客户区**坐标，所以只要客户区比窗口矩形内缩了 1 行，就会正好差 1。
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root
Add-Type -Path (Join-Path $root 'tools\probe\CuterInputProbe.cs')
[CuterInputProbe]::SetPhysicalDpiAwareness() | Out-Null

$title = [string]([char]0x622A) + [char]0x56FE
Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300

$p = Start-Process (Join-Path $root 'target\release\cuter.exe') -ArgumentList '--capture' -PassThru
$hwnd = [IntPtr]::Zero
foreach ($i in 1..40) {
    Start-Sleep -Milliseconds 250
    $hwnd = [CuterInputProbe]::FindWindow([uint32]$p.Id, $title)
    if ($hwnd -ne [IntPtr]::Zero) { break }
}
if ($hwnd -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; throw 'overlay not found' }

Start-Sleep -Milliseconds 500
Write-Host "window rect  : $([CuterInputProbe]::Describe($hwnd))"
Write-Host "client origin: $([CuterInputProbe]::ClientInfo($hwnd))"
Write-Host "dpi          : $([CuterInputProbe]::DpiOf($hwnd))"

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
