<#
.SYNOPSIS
  选区遮罩辅助功能真机冒烟（#9）：放大镜、按 C 把颜色值复制到系统剪贴板、手柄与方向键调整选区。

.DESCRIPTION
  先完整备份剪贴板（逐个格式读出；任何格式读不出就中止，不运行测试），再以
  BPS_SMOKE_REAL_CLIPBOARD=1 运行
    cargo test --release -p bps-app --test overlay_aids_smoke -- --ignored --nocapture
  测试按 C 后从系统剪贴板读回颜色值；结束后（无论成败）恢复剪贴板并核对格式。
  不启动常驻实例、不注册热键；遮罩在测试进程里显示约 3 秒，期间请勿操作电脑。
  多个工作树共用这台电脑时，先按约定拿到桌面锁再运行。
  访问剪贴板需要 STA：powershell -STA -File scripts/smoke-overlay-aids.ps1 [-Shots <目录>]

.PARAMETER Shots
  存放放大镜、手柄附近屏幕截图（PNG）的目录，供目视检查；不给就不存。
#>
param([string]$Shots)

$ErrorActionPreference = 'Stop'
if ([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') {
    throw '访问剪贴板需要 STA：用 powershell -STA -File 运行本脚本'
}
Add-Type -AssemblyName System.Windows.Forms
$clip = [System.Windows.Forms.Clipboard]

# 备份：把现有剪贴板的每个格式复制进一个新的 DataObject。读不出任何一个格式都中止。
$current = $clip::GetDataObject()
$backup = New-Object System.Windows.Forms.DataObject
$formats = @()
if ($null -ne $current) {
    foreach ($format in $current.GetFormats($false)) {
        $value = $current.GetData($format, $false)
        if ($null -eq $value) { throw "备份剪贴板失败：格式 $format 读不出内容，不运行测试" }
        $backup.SetData($format, $false, $value)
        $formats += $format
    }
}
'已备份剪贴板格式：' + ($(if ($formats) { $formats -join ', ' } else { '（空）' }))

$root = Split-Path -Parent $PSScriptRoot
$previousReal = $env:BPS_SMOKE_REAL_CLIPBOARD
$previousShots = $env:BPS_SMOKE_SHOTS
$exitCode = 1
try {
    $env:BPS_SMOKE_REAL_CLIPBOARD = '1'
    if ($Shots) {
        New-Item -ItemType Directory -Force $Shots | Out-Null
        $env:BPS_SMOKE_SHOTS = (Resolve-Path $Shots).Path
    }
    Push-Location $root
    try {
        & cmd /c 'cargo test --release -p bps-app --test overlay_aids_smoke -- --ignored --nocapture 2>&1'
        $exitCode = $LASTEXITCODE
    }
    finally { Pop-Location }
}
finally {
    $env:BPS_SMOKE_REAL_CLIPBOARD = $previousReal
    $env:BPS_SMOKE_SHOTS = $previousShots
    try {
        if ($formats) { $clip::SetDataObject($backup, $true, 10, 100) } else { $clip::Clear() }
        $after = $clip::GetDataObject()
        $restored = if ($null -ne $after) { @($after.GetFormats($false)) } else { @() }
        $missing = @($formats | Where-Object { $restored -notcontains $_ })
        if ($missing) { "警告：恢复后缺少剪贴板格式：$($missing -join ', ')" }
        else { '已恢复剪贴板' }
    }
    catch { "警告：恢复剪贴板失败：$($_.Exception.Message)" }
}
if ($exitCode -ne 0) { throw "冒烟测试失败（退出码 $exitCode）" }
'冒烟测试通过'
