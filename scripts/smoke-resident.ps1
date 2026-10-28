<#
.SYNOPSIS
  常驻实例端到端冒烟：便携模式首次运行 → 配置重新加载 → 热键 → 剪贴板和自动存储 → 退出。

.DESCRIPTION
  把 BetterPrtSc.exe 复制到临时目录并放上 portable 标记文件，以便携模式运行；子进程的
  APPDATA 指向一个空的临时目录，结束时确认它仍然是空的（便携模式不写用户目录）。

    1. 首次运行：程序目录里生成含 6 条默认绑定的 config.toml；
       投递第 1 条默认绑定（PrtSc → 区域 + 等待确认），应弹出选区遮罩；投递 Esc 取消，
       遮罩隐藏且不碰剪贴板。
    2. 改写 config.toml（带 BOM，同记事本），第 1 条绑定换成 Shift+PrtSc → 显示器 + 复制 +
       自动存储，存储目录覆盖为临时目录；不重启，投递热键事件后剪贴板应出现 PNG + DIB +
       存下的文件。
    3. 把 config.toml 改坏：应通知“配置文件有错误”，并且上一次有效的绑定继续生效。

  不模拟真实按键（单独按下再松开 Shift 可能切换输入法中英文），而是向常驻实例的
  隐藏窗口投递 WM_HOTKEY；热键是否注册成功看程序输出。
  运行期间默认热键（PrtSc 等）会被短暂注册，并弹出几条系统通知。
  截图只存到 -OutDir 下，不写“图片”文件夹。
  -ExitVia Tray 时通过托盘菜单的“退出”结束（会在鼠标处闪现菜单约 1 秒），
  否则投递 WM_CLOSE。
  运行期间会占用剪贴板，结束时恢复原有的文本或图像内容（其他格式不恢复）。
  需要 STA 线程访问剪贴板，请用 Windows PowerShell 运行：
    powershell -NoProfile -STA -File scripts\smoke-resident.ps1 [-Exe <路径>] [-ExitVia Tray]
#>
param(
    # 默认使用 debug 构建（带控制台输出，脚本靠它观察通知和配置重新加载）。
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\debug\BetterPrtSc.exe'),
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
    public static extern bool IsWindowVisible(IntPtr hwnd);
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
$VK_ESCAPE = 0x1B
$VK_UP = 0x26
# 第 1 条热键绑定的标识（热键标识 = 绑定下标 + 1）。
$FirstBinding = 1

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
    # “退出”是菜单的最后一项（前面是“设置”）：没有选中项时按上箭头选中最后一项。
    [BpsSmokeUser32]::PostMessageW($menu, $WM_KEYDOWN, [IntPtr]$VK_UP, [IntPtr]::Zero) | Out-Null
    [BpsSmokeUser32]::PostMessageW($menu, $WM_KEYDOWN, [IntPtr]$VK_RETURN, [IntPtr]::Zero) | Out-Null
}

# 常驻实例运行期间读取它的输出（文件仍被它打开着）。
function Read-Log {
    $stream = [IO.File]::Open($stderr, 'Open', 'Read', 'ReadWrite')
    try { (New-Object IO.StreamReader($stream, [Text.Encoding]::UTF8)).ReadToEnd() }
    finally { $stream.Dispose() }
}

# 等到输出里出现 $text。
function Wait-Log([string]$text) {
    for ($i = 0; $i -lt 50; $i++) {
        if ((Read-Log).Contains($text)) { return }
        Start-Sleep -Milliseconds 100
    }
    throw "5 秒内输出里没有出现：$text"
}

# 等选区遮罩（常驻窗口，平时隐藏）显示或隐藏。
function Wait-OverlayVisible([bool]$visible) {
    for ($i = 0; $i -lt 50; $i++) {
        $overlay = Find-Window 'BetterPrtSc.SelectionOverlay'
        if ($overlay -ne [IntPtr]::Zero -and [BpsSmokeUser32]::IsWindowVisible($overlay) -eq $visible) { return $overlay }
        if ($overlay -eq [IntPtr]::Zero -and -not $visible) { return $overlay }
        Start-Sleep -Milliseconds 100
    }
    $state = if ($visible) { '没有显示' } else { '没有隐藏' }
    throw "5 秒内选区遮罩$state"
}

function Send-Hotkey([IntPtr]$hwnd, [int]$id) {
    [BpsSmokeUser32]::PostMessageW($hwnd, $WM_HOTKEY, [IntPtr]$id, [IntPtr]::Zero) | Out-Null
}

# 等剪贴板出现 PNG，返回数据对象。
function Wait-ClipboardPng {
    for ($i = 0; $i -lt 100; $i++) {
        Start-Sleep -Milliseconds 50
        $data = $clip::GetDataObject()
        if ($data -and $data.GetDataPresent('PNG')) { return $data }
    }
    throw '5 秒内剪贴板没有出现 PNG'
}

if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
$appDir = Join-Path $OutDir 'app'
$fakeAppData = Join-Path $OutDir 'appdata'
$saveDir = Join-Path $OutDir 'saved'
New-Item -ItemType Directory -Force $appDir, $fakeAppData | Out-Null
Copy-Item $Exe (Join-Path $appDir 'BetterPrtSc.exe')
New-Item -ItemType File (Join-Path $appDir 'portable') | Out-Null
$config = Join-Path $appDir 'config.toml'
$stderr = Join-Path $OutDir 'stderr.log'
$pngPath = Join-Path $OutDir 'clipboard.png'

# Windows 路径写成 TOML 字面量字符串（单引号），反斜杠不转义。
$CopyAndSaveConfig = @"
# 冒烟测试：第 1 条绑定 Shift+PrtSc → 显示器 + 复制 + 自动存储到临时目录
[[bindings]]
hotkey = "Shift+PrintScreen"
mode = "display"
after = "copy_to_clipboard"
auto_save = true
save_dir = '$saveDir'
"@

$clip = [System.Windows.Forms.Clipboard]
$backupImage = if ($clip::ContainsImage()) { $clip::GetImage() } else { $null }
$backupText = if ($clip::ContainsText()) { $clip::GetText() } else { $null }

# 子进程继承环境变量：只在启动它的这一刻把 APPDATA 指向空目录。
$realAppData = $env:APPDATA
$env:APPDATA = $fakeAppData
try {
    $proc = Start-Process -FilePath (Join-Path $appDir 'BetterPrtSc.exe') -PassThru -WindowStyle Hidden -RedirectStandardError $stderr
}
finally { $env:APPDATA = $realAppData }
$null = $proc.Handle  # 先取句柄，否则退出后读不到 ExitCode
$hwnd = [IntPtr]::Zero
try {
    for ($i = 0; $i -lt 50 -and $hwnd -eq [IntPtr]::Zero; $i++) {
        Start-Sleep -Milliseconds 100
        $hwnd = Find-Window 'BetterPrtSc.ResidentWindow'
    }
    if ($hwnd -eq [IntPtr]::Zero) { throw '未找到常驻实例的隐藏窗口' }
    Wait-Log '常驻实例已启动'

    '--- 1. 首次运行 ---'
    if (-not (Test-Path $config)) { throw '首次运行没有在程序目录生成 config.toml' }
    $bindings = ([regex]::Matches((Get-Content $config -Raw -Encoding UTF8), '(?m)^\[\[bindings\]\]')).Count
    if ($bindings -ne 6) { throw "默认配置应有 6 条绑定，实际 $bindings 条" }
    "程序目录生成了 config.toml，含 $bindings 条默认绑定"
    $clip::Clear()
    Send-Hotkey $hwnd $FirstBinding
    $overlay = Wait-OverlayVisible $true
    '选区遮罩已显示'
    [BpsSmokeUser32]::PostMessageW($overlay, $WM_KEYDOWN, [IntPtr]$VK_ESCAPE, [IntPtr]::Zero) | Out-Null
    $null = Wait-OverlayVisible $false
    Start-Sleep -Milliseconds 300
    if ($clip::GetDataObject().GetDataPresent('PNG')) { throw '取消选区不应写入剪贴板' }
    'Esc 取消了选区，遮罩已隐藏，剪贴板未被写入'

    '--- 2. 修改配置后自动重新加载 ---'
    Set-Content -Path $config -Value $CopyAndSaveConfig -Encoding UTF8   # Windows PowerShell 写入带 BOM 的 UTF-8
    Wait-Log '已处理 ConfigFileChanged'
    $clip::Clear()
    $watch = [Diagnostics.Stopwatch]::StartNew()
    Send-Hotkey $hwnd $FirstBinding
    $data = Wait-ClipboardPng
    "剪贴板在 $($watch.ElapsedMilliseconds) ms 内就绪，格式：$($data.GetFormats() -join ', ')"

    [IO.File]::WriteAllBytes($pngPath, $data.GetData('PNG').ToArray())
    $png = [System.Drawing.Image]::FromFile($pngPath)
    "PNG：$($png.Width)x$($png.Height)，已保存到 $pngPath"
    $png.Dispose()

    $dib = $data.GetData([System.Windows.Forms.DataFormats]::Dib).ToArray()
    $w = [BitConverter]::ToInt32($dib, 4); $h = [BitConverter]::ToInt32($dib, 8)
    $bits = [BitConverter]::ToInt16($dib, 14)
    "DIB：${w}x${h}，${bits} 位，共 $($dib.Length) 字节"

    $savedFiles = @($data.GetData([System.Windows.Forms.DataFormats]::FileDrop))
    if ($savedFiles.Count -ne 1) { throw "剪贴板应附带 1 个文件，实际 $($savedFiles.Count) 个" }
    $saved = $savedFiles[0]
    if (-not (Test-Path -LiteralPath $saved)) { throw "剪贴板附带的文件不存在：$saved" }
    if ((Split-Path -Parent $saved) -ne $saveDir) { throw "文件不在配置的存储目录 ${saveDir}：$saved" }
    $img = [System.Drawing.Image]::FromFile($saved)
    "自动存储：$saved（$($img.Width)x$($img.Height)，$((Get-Item -LiteralPath $saved).Length) 字节）"
    $img.Dispose()
    Wait-Log '[通知] 截图已保存'

    '--- 3. 配置写错时保留上一次有效的配置 ---'
    Set-Content -Path $config -Value $CopyAndSaveConfig.Replace('"display"', '"dispaly"') -Encoding UTF8
    Wait-Log '[通知] 配置文件有错误'
    $clip::Clear()
    Send-Hotkey $hwnd $FirstBinding
    $null = Wait-ClipboardPng
    '配置有错误时通知了用户，原来的绑定仍然有效'

    if ($ExitVia -eq 'Tray') { Exit-ViaTrayMenu $hwnd }
    else { [BpsSmokeUser32]::PostMessageW($hwnd, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null }
    if ($proc.WaitForExit(5000)) { "常驻实例已经由 $ExitVia 退出" }
    else { throw "常驻实例没有在 5 秒内经由 $ExitVia 退出" }

    $written = @(Get-ChildItem -Recurse -Force $fakeAppData)
    if ($written.Count -gt 0) { throw "便携模式下 APPDATA 中出现了文件：$($written.FullName -join ', ')" }
    '便携模式：APPDATA 中没有写入任何文件'
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
    # 剪贴板可能正被其他程序占用：恢复失败只警告，不影响后面的清理。
    try {
        if ($backupImage) { $clip::SetImage($backupImage) }
        elseif ($null -ne $backupText) { $clip::SetText($backupText) }
    }
    catch { "警告：恢复剪贴板失败：$($_.Exception.Message)" }
}
