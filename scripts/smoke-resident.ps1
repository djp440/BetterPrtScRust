<#
.SYNOPSIS
  常驻实例端到端冒烟：启动 BetterPrtSc.exe → 投递热键事件 → 检查剪贴板 → 退出。

.DESCRIPTION
  不模拟真实按键（单独按下再松开 Shift 可能切换输入法中英文），而是向常驻实例的
  隐藏窗口投递 WM_HOTKEY；热键是否注册成功看程序输出。
  -ExitVia Tray 时通过托盘菜单的“退出”结束（会在鼠标处闪现菜单约 1 秒），
  否则投递 WM_CLOSE。
  运行期间会占用剪贴板，结束时恢复原有的文本或图像内容（其他格式不恢复）。
  需要 STA 线程访问剪贴板，请用 Windows PowerShell 运行：
    powershell -NoProfile -STA -File scripts\smoke-resident.ps1 [-Exe <路径>] [-ExitVia Tray]
#>
param(
    # 默认使用 debug 构建（带控制台输出）。
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\debug\BetterPrtSc.exe'),
    # 默认热键绑定的标识：第 1 条绑定（Shift+PrtSc）为 1。
    [int]$HotkeyId = 1,
    [ValidateSet('Close', 'Tray')]
    [string]$ExitVia = 'Close',
    [string]$OutDir = (Join-Path $env:TEMP 'bps-smoke')
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class BpsSmokeUser32 {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowW(string className, string windowName);
    [DllImport("user32.dll")]
    public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);
}
'@
$WM_CLOSE = 0x0010
$WM_KEYDOWN = 0x0100
$WM_HOTKEY = 0x0312
$WM_RBUTTONUP = 0x0205
$WM_TRAY = 0x8000 + 1   # WM_APP + 1，见 platform-windows 的 event_loop.rs
$VK_RETURN = 0x0D
$VK_DOWN = 0x28

function Find-Window([string]$class) {
    [BpsSmokeUser32]::FindWindowW($class, [NullString]::Value)
}

function Exit-ViaTrayMenu([IntPtr]$hwnd) {
    [BpsSmokeUser32]::PostMessageW($hwnd, $WM_TRAY, [IntPtr]::Zero, [IntPtr]$WM_RBUTTONUP) | Out-Null
    $menu = [IntPtr]::Zero
    for ($i = 0; $i -lt 20 -and $menu -eq [IntPtr]::Zero; $i++) {
        Start-Sleep -Milliseconds 100
        $menu = Find-Window '#32768'
    }
    if ($menu -eq [IntPtr]::Zero) { throw '托盘菜单没有弹出' }
    '托盘菜单已弹出，选择“退出”'
    Start-Sleep -Milliseconds 500
    [BpsSmokeUser32]::PostMessageW($menu, $WM_KEYDOWN, [IntPtr]$VK_DOWN, [IntPtr]::Zero) | Out-Null
    [BpsSmokeUser32]::PostMessageW($menu, $WM_KEYDOWN, [IntPtr]$VK_RETURN, [IntPtr]::Zero) | Out-Null
}

New-Item -ItemType Directory -Force $OutDir | Out-Null
$stderr = Join-Path $OutDir 'stderr.log'
$pngPath = Join-Path $OutDir 'clipboard.png'

$clip = [System.Windows.Forms.Clipboard]
$backupImage = if ($clip::ContainsImage()) { $clip::GetImage() } else { $null }
$backupText = if ($clip::ContainsText()) { $clip::GetText() } else { $null }

$proc = Start-Process -FilePath $Exe -PassThru -WindowStyle Hidden -RedirectStandardError $stderr
$null = $proc.Handle  # 先取句柄，否则退出后读不到 ExitCode
$hwnd = [IntPtr]::Zero
try {
    for ($i = 0; $i -lt 50 -and $hwnd -eq [IntPtr]::Zero; $i++) {
        Start-Sleep -Milliseconds 100
        $hwnd = Find-Window 'BetterPrtSc.ResidentWindow'
    }
    if ($hwnd -eq [IntPtr]::Zero) { throw '未找到常驻实例的隐藏窗口' }

    $clip::Clear()
    $watch = [Diagnostics.Stopwatch]::StartNew()
    [BpsSmokeUser32]::PostMessageW($hwnd, $WM_HOTKEY, [IntPtr]$HotkeyId, [IntPtr]::Zero) | Out-Null
    $data = $null
    for ($i = 0; $i -lt 100; $i++) {
        Start-Sleep -Milliseconds 50
        $data = $clip::GetDataObject()
        if ($data -and $data.GetDataPresent('PNG')) { break }
    }
    if (-not ($data -and $data.GetDataPresent('PNG'))) { throw '5 秒内剪贴板没有出现 PNG' }
    "剪贴板在 $($watch.ElapsedMilliseconds) ms 内就绪，格式：$($data.GetFormats() -join ', ')"

    [IO.File]::WriteAllBytes($pngPath, $data.GetData('PNG').ToArray())
    $png = [System.Drawing.Image]::FromFile($pngPath)
    "PNG：$($png.Width)x$($png.Height)，已保存到 $pngPath"
    $png.Dispose()

    $dib = $data.GetData([System.Windows.Forms.DataFormats]::Dib).ToArray()
    $w = [BitConverter]::ToInt32($dib, 4); $h = [BitConverter]::ToInt32($dib, 8)
    $bits = [BitConverter]::ToInt16($dib, 14)
    "DIB：${w}x${h}，${bits} 位，共 $($dib.Length) 字节"

    if ($ExitVia -eq 'Tray') { Exit-ViaTrayMenu $hwnd }
    else { [BpsSmokeUser32]::PostMessageW($hwnd, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null }
    if ($proc.WaitForExit(5000)) { "常驻实例已经由 $ExitVia 退出" }
    else { throw "常驻实例没有在 5 秒内经由 $ExitVia 退出" }
}
finally {
    if (-not $proc.HasExited) {
        Stop-Process -Id $proc.Id -Force
        '警告：已强制结束常驻实例（托盘可能残留图标，鼠标移过即消失）'
    }
    $proc.WaitForExit()
    "常驻实例退出码：$($proc.ExitCode)"
    '--- 常驻实例输出 ---'
    Get-Content $stderr -Encoding UTF8
    if ($backupImage) { $clip::SetImage($backupImage) }
    elseif ($null -ne $backupText) { $clip::SetText($backupText) }
}
