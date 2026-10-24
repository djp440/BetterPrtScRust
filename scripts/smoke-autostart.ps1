<#
.SYNOPSIS
  开机自启端到端冒烟：常驻实例按 config.toml 写入 / 移除 HKCU Run 项，卸载用的 --remove-autostart 能清理它。

.DESCRIPTION
  把 BetterPrtSc.exe 复制到临时目录并放上 portable 标记（便携模式，配置写在该目录，
  子进程的 APPDATA 也指向临时目录），然后依次验证：
    1. 配置 autostart = true 启动常驻实例：写入名为 BetterPrtSc 的 Run 项，内容是带引号的临时程序路径，
       并发出便携模式的提醒通知（debug 版在标准错误里输出 [通知] 行）
    2. 把配置改成 autostart = false：常驻实例立即移除 Run 项
    3. 再改回 autostart = true 后退出常驻实例，运行 BetterPrtSc.exe --remove-autostart：退出码 0，Run 项被移除
  配置里没有热键绑定，不注册任何热键。
  开始前如果已有名为 BetterPrtSc 的 Run 项（真实安装登记的开机自启）就中止，不碰它；
  结束时（包括失败时）结束本次启动的进程、删除指向临时程序的 Run 项和临时目录。
  不验证计划任务（以管理员身份运行）：那需要管理员权限，属于手动验收。
    powershell -NoProfile -File scripts\smoke-autostart.ps1 [-BinDir target\release]
#>
param(
    # BetterPrtSc.exe 所在目录，默认 debug 构建。
    [string]$BinDir = (Join-Path $PSScriptRoot '..\target\debug'),
    [string]$OutDir = (Join-Path $env:TEMP 'bps-smoke-autostart')
)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class BpsAutostartSmokeUser32 {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string className, string windowName);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
    [DllImport("user32.dll")]
    public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);
}
'@
$WM_CLOSE = 0x0010
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$ValueName = 'BetterPrtSc'
$source = (Resolve-Path (Join-Path $BinDir 'BetterPrtSc.exe')).Path

function Get-RunValue {
    (Get-ItemProperty -Path $RunKey -Name $ValueName -ErrorAction SilentlyContinue).$ValueName
}

# 等 $condition 成立，最多 $seconds 秒。
function Wait-Until([scriptblock]$condition, [double]$seconds, [string]$what) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while (-not (& $condition)) {
        if ((Get-Date) -gt $deadline) { throw "等待超时：$what" }
        Start-Sleep -Milliseconds 100
    }
}

# 只向属于 $processId 的常驻实例隐藏窗口投递 WM_CLOSE。
function Close-Resident([int]$processId) {
    $hwnd = [IntPtr]::Zero
    while ($true) {
        $hwnd = [BpsAutostartSmokeUser32]::FindWindowExW([IntPtr]::Zero, $hwnd, 'BetterPrtSc.ResidentWindow', [NullString]::Value)
        if ($hwnd -eq [IntPtr]::Zero) { return }
        $owner = 0
        [BpsAutostartSmokeUser32]::GetWindowThreadProcessId($hwnd, [ref]$owner) | Out-Null
        if ($owner -eq $processId) {
            [BpsAutostartSmokeUser32]::PostMessageW($hwnd, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
            return
        }
    }
}

if ($null -ne (Get-RunValue)) {
    throw "已有名为 $ValueName 的 Run 项（可能是真实安装登记的开机自启），为免误删，本冒烟中止"
}
if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
$programDir = Join-Path $OutDir '程序 目录'
New-Item -ItemType Directory -Force $programDir | Out-Null
$exe = Join-Path $programDir 'BetterPrtSc.exe'
Copy-Item $source $exe
Set-Content -Path (Join-Path $programDir 'portable') -Value 'smoke' -Encoding ASCII
$config = Join-Path $programDir 'config.toml'
$stderr = Join-Path $OutDir 'resident-stderr.log'
$expected = '"' + $exe + '"'

$previousAppData = $env:APPDATA
$env:APPDATA = Join-Path $OutDir 'AppData'
New-Item -ItemType Directory -Force $env:APPDATA | Out-Null
$process = $null
try {
    # 1. autostart = true 启动：写入 Run 项 + 便携模式提醒
    [IO.File]::WriteAllText($config, "autostart = true`n")
    $process = Start-Process -FilePath $exe -WorkingDirectory $programDir -PassThru -WindowStyle Hidden `
        -RedirectStandardError $stderr -RedirectStandardOutput (Join-Path $OutDir 'resident-stdout.log')
    Wait-Until { (Get-RunValue) -eq $expected } 15 '常驻实例写入 Run 项'
    Write-Host "1. 已写入 Run 项：$(Get-RunValue)"
    Wait-Until { (Get-Content $stderr -Raw -Encoding UTF8) -match '\[通知\] 已开启开机自启：便携模式下' } 5 '便携模式提醒通知'
    Write-Host '   已发出便携模式提醒通知'

    # 2. 改成 autostart = false：立即移除
    [IO.File]::WriteAllText($config, "autostart = false`n")
    Wait-Until { $null -eq (Get-RunValue) } 5 '常驻实例移除 Run 项'
    Write-Host '2. 关闭开机自启后 Run 项已移除'

    # 3. 再开启、退出常驻实例，然后用卸载流程的 --remove-autostart 清理
    [IO.File]::WriteAllText($config, "autostart = true`n")
    Wait-Until { (Get-RunValue) -eq $expected } 5 '重新开启开机自启'
    Close-Resident $process.Id
    if (-not $process.WaitForExit(5000)) { throw '常驻实例 5 秒内没有退出' }
    if ((Get-RunValue) -ne $expected) { throw '退出常驻实例不应移除开机自启' }
    $remove = Start-Process -FilePath $exe -ArgumentList '--remove-autostart' -PassThru -WindowStyle Hidden `
        -RedirectStandardError (Join-Path $OutDir 'remove-stderr.log') -RedirectStandardOutput (Join-Path $OutDir 'remove-stdout.log')
    # 先取一次句柄，否则 Windows PowerShell 5.1 拿不到退出码。
    $null = $remove.Handle
    if (-not $remove.WaitForExit(10000)) { $remove.Kill(); throw '--remove-autostart 10 秒内没有结束' }
    if ($remove.ExitCode -ne 0) { throw "--remove-autostart 退出码 $($remove.ExitCode)" }
    if ($null -ne (Get-RunValue)) { throw '--remove-autostart 后 Run 项仍在' }
    Write-Host '3. --remove-autostart 退出码 0，Run 项已移除'
    Write-Host '开机自启冒烟通过'
}
finally {
    $env:APPDATA = $previousAppData
    if ($null -ne $process -and -not $process.HasExited) {
        Close-Resident $process.Id
        if (-not $process.WaitForExit(5000)) { $process.Kill() }
    }
    # 只删除指向本次临时程序的 Run 项。
    if ((Get-RunValue) -eq $expected) {
        Remove-ItemProperty -Path $RunKey -Name $ValueName
        Write-Host '已清理遗留的 Run 项'
    }
    $approved = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
    if ($null -eq (Get-RunValue) -and $null -ne (Get-ItemProperty -Path $approved -Name $ValueName -ErrorAction SilentlyContinue)) {
        Write-Warning "StartupApproved\Run 下有名为 $ValueName 的值（不是本冒烟写的），未改动"
    }
    Remove-Item -Recurse -Force $OutDir -ErrorAction SilentlyContinue
}
