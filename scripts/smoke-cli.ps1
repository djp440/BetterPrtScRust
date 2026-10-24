<#
.SYNOPSIS
  bps CLI 与单实例端到端冒烟：bps 拉起常驻实例 → 捕获到剪贴板 → 再次启动不产生第二个实例 → 退出。

.DESCRIPTION
  要求开始时没有 BetterPrtSc 在运行（否则直接中止，不打扰正在使用的实例）。
  依次验证：
    1. 常驻实例未运行时 `bps capture display` 先拉起它，再把显示器截图复制到剪贴板（退出码 0）
    2. `bps capture region` 返回“暂不支持”（退出码 3），`bps capture bogus` 是用法错误（退出码 2）
    3. 再次启动 BetterPrtSc.exe 只激活已有实例并立即退出，进程数仍为 1
    4. 常驻实例已在运行时 `bps capture display` 直接转交（退出码 0）
  最后向隐藏窗口投递 WM_CLOSE 结束常驻实例，并恢复剪贴板原有的文本或图像。
  需要 STA 线程访问剪贴板，请用 Windows PowerShell 运行：
    powershell -NoProfile -STA -File scripts\smoke-cli.ps1 [-BinDir target\release]
#>
param(
    # bps.exe 与 BetterPrtSc.exe 所在目录，默认 debug 构建。
    [string]$BinDir = (Join-Path $PSScriptRoot '..\target\debug'),
    [string]$OutDir = (Join-Path $env:TEMP 'bps-smoke-cli')
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class BpsCliSmokeUser32 {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string className, string windowName);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
    [DllImport("user32.dll")]
    public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);
}
'@
$WM_CLOSE = 0x0010
$bps = (Resolve-Path (Join-Path $BinDir 'bps.exe')).Path
$resident = (Resolve-Path (Join-Path $BinDir 'BetterPrtSc.exe')).Path
New-Item -ItemType Directory -Force $OutDir | Out-Null

# 只认本次被测目录里的 BetterPrtSc.exe，不碰其他位置运行的实例。
function Get-Residents {
    @(Get-Process -Name BetterPrtSc -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $resident })
}

# 按类名枚举常驻实例的隐藏窗口，只向属于 $processId 的那个投递 WM_CLOSE。
function Close-Resident([int]$processId) {
    $hwnd = [IntPtr]::Zero
    while ($true) {
        $hwnd = [BpsCliSmokeUser32]::FindWindowExW([IntPtr]::Zero, $hwnd, 'BetterPrtSc.ResidentWindow', [NullString]::Value)
        if ($hwnd -eq [IntPtr]::Zero) { return }
        $owner = 0
        [BpsCliSmokeUser32]::GetWindowThreadProcessId($hwnd, [ref]$owner) | Out-Null
        if ($owner -eq $processId) {
            [BpsCliSmokeUser32]::PostMessageW($hwnd, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
            return
        }
    }
}

# 运行 bps，返回退出码、标准错误和耗时。
function Invoke-Bps([string[]]$arguments) {
    $stderr = Join-Path $OutDir 'bps-stderr.log'
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process -FilePath $bps -ArgumentList $arguments -PassThru -NoNewWindow `
        -RedirectStandardError $stderr -RedirectStandardOutput (Join-Path $OutDir 'bps-stdout.log')
    $null = $p.Handle
    if (-not $p.WaitForExit(20000)) { $p.Kill(); throw "bps $arguments 20 秒内没有结束" }
    [pscustomobject]@{
        Code   = $p.ExitCode
        Stderr = [string](Get-Content $stderr -Raw -Encoding UTF8)
        Ms     = $watch.ElapsedMilliseconds
    }
}

function Assert-That([bool]$condition, [string]$message) {
    if (-not $condition) { throw "失败：$message" }
    "通过：$message"
}

if ((Get-Residents).Count -gt 0) { throw "已有 $resident 在运行，请先退出它再做冒烟测试" }

$clip = [System.Windows.Forms.Clipboard]
$backupImage = if ($clip::ContainsImage()) { $clip::GetImage() } else { $null }
$backupText = if ($clip::ContainsText()) { $clip::GetText() } else { $null }
try {
    $clip::Clear()
    $r = Invoke-Bps @('capture', 'display')
    Assert-That ($r.Code -eq 0) "未运行时 bps capture display 退出码 0（实际 $($r.Code)，$($r.Ms) ms）$($r.Stderr)"
    Assert-That ((Get-Residents).Count -eq 1) 'bps 拉起了一个常驻实例'
    $data = $clip::GetDataObject()
    Assert-That ($data -and $data.GetDataPresent('PNG') -and $data.GetDataPresent([System.Windows.Forms.DataFormats]::Dib)) `
        "剪贴板含 PNG 与 DIB（格式：$($data.GetFormats() -join ', ')）"

    $r = Invoke-Bps @('capture', 'region')
    Assert-That ($r.Code -eq 3 -and $r.Stderr -match '暂不支持') "bps capture region 退出码 3 且提示暂不支持：$($r.Stderr.Trim())"
    $r = Invoke-Bps @('capture', 'bogus')
    Assert-That ($r.Code -eq 2) "bps capture bogus 退出码 2（实际 $($r.Code)）"

    $second = Start-Process -FilePath $resident -PassThru -WindowStyle Hidden `
        -RedirectStandardError (Join-Path $OutDir 'second-stderr.log')
    $null = $second.Handle
    Assert-That ($second.WaitForExit(5000)) '再次启动的 BetterPrtSc.exe 在 5 秒内退出'
    $secondLog = [string](Get-Content (Join-Path $OutDir 'second-stderr.log') -Raw -Encoding UTF8)
    Assert-That ($second.ExitCode -eq 0) "再次启动的进程退出码 0：$($secondLog.Trim())"
    Assert-That ((Get-Residents).Count -eq 1) '仍然只有一个常驻实例'

    $clip::Clear()
    $r = Invoke-Bps @('capture', 'display')
    Assert-That ($r.Code -eq 0 -and $clip::GetDataObject().GetDataPresent('PNG')) `
        "已运行时 bps capture display 退出码 0 且剪贴板有 PNG（$($r.Ms) ms）"
}
finally {
    foreach ($p in Get-Residents) {
        Close-Resident $p.Id
        if (-not $p.WaitForExit(5000)) {
            Stop-Process -Id $p.Id -Force
            '警告：已强制结束常驻实例（托盘可能残留图标，鼠标移过即消失）'
        }
    }
    "结束后常驻实例数：$((Get-Residents).Count)"
    if ($backupImage) { $clip::SetImage($backupImage) }
    elseif ($null -ne $backupText) { $clip::SetText($backupText) }
}
