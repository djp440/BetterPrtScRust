<#
.SYNOPSIS
  bps CLI 与单实例端到端冒烟：bps 拉起常驻实例 → 捕获到剪贴板 → 再次启动不产生第二个实例 → 退出。

.DESCRIPTION
  要求开始时没有 BetterPrtSc 在运行（否则直接中止，不打扰正在使用的实例）。
  依次验证：
    1. 常驻实例未运行时 `bps capture display` 先拉起它，再把显示器截图复制到剪贴板（退出码 0）
    2. `bps capture virtual-desktop` 把整个虚拟桌面复制到剪贴板，PNG 尺寸等于物理像素的虚拟桌面
       `bps capture bogus` 是用法错误（退出码 2）
    3. 默认配置下 `bps capture region` 用 PrtSc 的“区域 + 标注”，标注未实现 → 退出码 3；
       在配置文件最前面加一条“区域 + 复制”绑定（Ctrl+Alt+Shift+F8）、等它重新加载后，
       `bps capture region` 打开选区遮罩并等待：向遮罩投递 Esc → 退出码 5、剪贴板为空；
       再次打开后投递拖拽 (100,100)→(299,199) 与 Enter → 退出码 0、剪贴板 PNG 为 200x100
       （遮罩会闪现两次，各不到 1 秒；期间请勿移动鼠标）
    3b. 同时加一条“窗口 + 复制 + 自动存储”绑定（命名模板含 {window_title}）。本脚本临时显示一个
       置顶的纯色测试窗口，`bps capture window` 打开遮罩后在测试窗口位置投递单击 → 退出码 0、
       剪贴板 PNG 尺寸等于窗口外框（物理像素）、中心是测试窗口的颜色、存下的文件名含窗口标题；
       Win11 上四角圆角以外透明。测试窗口随即关闭（遮罩再闪现一次）
    4. 再次启动 BetterPrtSc.exe 只激活已有实例并立即退出，进程数仍为 1
    5. 常驻实例已在运行时 `bps capture display` 直接转交（退出码 0）
  最后向隐藏窗口投递 WM_CLOSE 结束常驻实例，并恢复剪贴板原有的文本或图像。
  默认绑定（复制 + 自动存储）会在“图片\BetterPrtSc”里存截图：结束时删除本次存下的文件
  （以剪贴板附带的文件为准），目录是本次新建且已空时一并删除。
  本脚本启动的进程的 APPDATA 指向 -OutDir 下的临时目录，常驻实例首次运行生成的
  config.toml 写在那里，不碰真实的 %APPDATA%。
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
    [DllImport("user32.dll")]
    public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")]
    public static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hwnd);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("dwmapi.dll")]
    public static extern int DwmGetWindowAttribute(IntPtr hwnd, int attribute, out RECT rect, int size);
}
'@
$WM_CLOSE = 0x0010
$WM_KEYDOWN = 0x0100
$WM_MOUSEMOVE = 0x0200
$WM_LBUTTONDOWN = 0x0201
$WM_LBUTTONUP = 0x0202
$MK_LBUTTON = 0x0001
$VK_RETURN = 0x0D
$VK_ESCAPE = 0x1B
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

# 在后台启动 bps，返回进程对象；用 Wait-Bps 取结果。
function Start-Bps([string[]]$arguments) {
    $p = Start-Process -FilePath $bps -ArgumentList $arguments -PassThru -NoNewWindow `
        -RedirectStandardError (Join-Path $OutDir 'bps-stderr.log') `
        -RedirectStandardOutput (Join-Path $OutDir 'bps-stdout.log')
    $null = $p.Handle
    $p | Add-Member -NotePropertyName Watch -NotePropertyValue ([Diagnostics.Stopwatch]::StartNew())
    $p | Add-Member -NotePropertyName Arguments -NotePropertyValue $arguments
    $p
}

# 等 bps 结束，返回退出码、标准错误和耗时。
function Wait-Bps($p) {
    if (-not $p.WaitForExit(20000)) { $p.Kill(); throw "bps $($p.Arguments) 20 秒内没有结束" }
    [pscustomobject]@{
        Code   = $p.ExitCode
        Stderr = [string](Get-Content (Join-Path $OutDir 'bps-stderr.log') -Raw -Encoding UTF8)
        Ms     = $p.Watch.ElapsedMilliseconds
    }
}

# 运行 bps，返回退出码、标准错误和耗时。
function Invoke-Bps([string[]]$arguments) {
    Wait-Bps (Start-Bps $arguments)
}

# 常驻实例 $processId 的选区遮罩窗口（启动时就已建好并隐藏）。
function Find-Overlay([int]$processId) {
    $hwnd = [IntPtr]::Zero
    while ($true) {
        $hwnd = [BpsCliSmokeUser32]::FindWindowExW([IntPtr]::Zero, $hwnd, 'BetterPrtSc.SelectionOverlay', [NullString]::Value)
        if ($hwnd -eq [IntPtr]::Zero) { throw "找不到常驻实例 $processId 的选区遮罩窗口" }
        $owner = 0
        [BpsCliSmokeUser32]::GetWindowThreadProcessId($hwnd, [ref]$owner) | Out-Null
        if ($owner -eq $processId) { return $hwnd }
    }
}

# 等遮罩窗口显示（$visible）或隐藏，5 秒内没等到返回 $false。
function Wait-OverlayVisible([IntPtr]$hwnd, [bool]$visible) {
    for ($i = 0; $i -lt 250; $i++) {
        if ([BpsCliSmokeUser32]::IsWindowVisible($hwnd) -eq $visible) { return $true }
        Start-Sleep -Milliseconds 20
    }
    $false
}

# 遮罩客户区坐标 → WM_*BUTTON* / WM_MOUSEMOVE 的 lParam。
function Point([int]$x, [int]$y) { [IntPtr](($y -shl 16) -bor $x) }

# 投递时本线程临时切到 Per-Monitor V2：否则系统会按缩放比例换算鼠标消息里的坐标。
function Send-Overlay([IntPtr]$hwnd, [int]$message, [int]$wParam, [IntPtr]$lParam) {
    $previous = [BpsCliSmokeUser32]::SetThreadDpiAwarenessContext([IntPtr]-4)
    try { [BpsCliSmokeUser32]::PostMessageW($hwnd, $message, [IntPtr]$wParam, $lParam) | Out-Null }
    finally { [BpsCliSmokeUser32]::SetThreadDpiAwarenessContext($previous) | Out-Null }
}

# 物理像素的虚拟桌面尺寸：临时把本线程切到 Per-Monitor V2，避免系统按缩放比例虚拟化坐标。
function Get-PhysicalVirtualScreenSize {
    $previous = [BpsCliSmokeUser32]::SetThreadDpiAwarenessContext([IntPtr]-4)
    try { "{0}x{1}" -f [BpsCliSmokeUser32]::GetSystemMetrics(78), [BpsCliSmokeUser32]::GetSystemMetrics(79) }
    finally { [BpsCliSmokeUser32]::SetThreadDpiAwarenessContext($previous) | Out-Null }
}

# 窗口在屏幕上可见部分的外框（DWM 扩展边框，物理像素）。
function Get-WindowFrameBounds([IntPtr]$hwnd) {
    $previous = [BpsCliSmokeUser32]::SetThreadDpiAwarenessContext([IntPtr]-4)
    try {
        $rect = New-Object BpsCliSmokeUser32+RECT
        $hr = [BpsCliSmokeUser32]::DwmGetWindowAttribute($hwnd, 9, [ref]$rect, 16)
        if ($hr -ne 0) { throw "读取窗口外框失败：$hr" }
        $rect
    }
    finally { [BpsCliSmokeUser32]::SetThreadDpiAwarenessContext($previous) | Out-Null }
}

# 物理像素的虚拟桌面左上角。
function Get-PhysicalVirtualScreenOrigin {
    $previous = [BpsCliSmokeUser32]::SetThreadDpiAwarenessContext([IntPtr]-4)
    try { [pscustomobject]@{ X = [BpsCliSmokeUser32]::GetSystemMetrics(76); Y = [BpsCliSmokeUser32]::GetSystemMetrics(77) } }
    finally { [BpsCliSmokeUser32]::SetThreadDpiAwarenessContext($previous) | Out-Null }
}

# 等 bps 结束，期间处理本线程的窗口消息（测试窗口要能重画）。
function Wait-BpsPumping($p) {
    $deadline = [DateTime]::Now.AddSeconds(20)
    while (-not $p.WaitForExit(30)) {
        [System.Windows.Forms.Application]::DoEvents()
        if ([DateTime]::Now -gt $deadline) { $p.Kill(); throw "bps $($p.Arguments) 20 秒内没有结束" }
    }
    Wait-Bps $p
}

# 剪贴板里 PNG 格式图像的尺寸；没有 PNG 时返回 $null。
function Get-ClipboardPngSize {
    $stream = $clip::GetDataObject().GetData('PNG')
    if (-not $stream) { return $null }
    $image = [System.Drawing.Image]::FromStream($stream)
    try { "{0}x{1}" -f $image.Width, $image.Height } finally { $image.Dispose() }
}

function Assert-That([bool]$condition, [string]$message) {
    if (-not $condition) { throw "失败：$message" }
    "通过：$message"
}

# 剪贴板数据里附带的文件（本次自动存储的文件），结束时删除。
function Get-SavedFiles($data) {
    $files = $null
    if ($data -and $data.GetDataPresent([System.Windows.Forms.DataFormats]::FileDrop)) {
        $files = $data.GetData([System.Windows.Forms.DataFormats]::FileDrop)
    }
    # 逗号运算符防止单元素数组在返回时被展开成字符串。
    if ($files) { , [string[]]$files } else { , [string[]]@() }
}

if ((Get-Residents).Count -gt 0) { throw "已有 $resident 在运行，请先退出它再做冒烟测试" }

$clip = [System.Windows.Forms.Clipboard]
$backupImage = if ($clip::ContainsImage()) { $clip::GetImage() } else { $null }
$backupText = if ($clip::ContainsText()) { $clip::GetText() } else { $null }
$saveDir = Join-Path ([Environment]::GetFolderPath('MyPictures')) 'BetterPrtSc'
$saveDirExisted = Test-Path -LiteralPath $saveDir
$startedAt = Get-Date
$savedFiles = @()
# 子进程（bps 及它拉起的常驻实例）继承环境变量。
$realAppData = $env:APPDATA
$fakeAppData = Join-Path $OutDir 'appdata'
if (Test-Path -LiteralPath $fakeAppData) { Remove-Item -Recurse -Force -LiteralPath $fakeAppData }
New-Item -ItemType Directory -Force $fakeAppData | Out-Null
$env:APPDATA = $fakeAppData
try {
    $clip::Clear()
    $r = Invoke-Bps @('capture', 'display')
    Assert-That ($r.Code -eq 0) "未运行时 bps capture display 退出码 0（实际 $($r.Code)，$($r.Ms) ms）$($r.Stderr)"
    Assert-That ((Get-Residents).Count -eq 1) 'bps 拉起了一个常驻实例'
    $data = $clip::GetDataObject()
    Assert-That ($data -and $data.GetDataPresent('PNG') -and $data.GetDataPresent([System.Windows.Forms.DataFormats]::Dib)) `
        "剪贴板含 PNG 与 DIB（格式：$($data.GetFormats() -join ', ')）"
    $files = Get-SavedFiles $data
    $savedFiles += $files
    Assert-That ($files.Count -eq 1 -and (Test-Path -LiteralPath $files[0])) "与 Shift+PrtSc 一致，自动存储并附带文件：$files"

    $clip::Clear()
    $r = Invoke-Bps @('capture', 'virtual-desktop')
    $screen = Get-PhysicalVirtualScreenSize
    $png = Get-ClipboardPngSize
    Assert-That ($r.Code -eq 0 -and $png -eq $screen) `
        "bps capture virtual-desktop 退出码 $($r.Code)，PNG $png 等于物理像素虚拟桌面 $screen（$($r.Ms) ms）$($r.Stderr)"

    $r = Invoke-Bps @('capture', 'bogus')
    Assert-That ($r.Code -eq 2) "bps capture bogus 退出码 2（实际 $($r.Code)）"

    $r = Invoke-Bps @('capture', 'region')
    Assert-That ($r.Code -eq 3 -and $r.Stderr -match '标注') "默认配置下 bps capture region 退出码 3 且提示标注暂不支持：$($r.Stderr.Trim())"

    # 在常驻实例生成的配置文件最前面加一条“区域 + 复制”绑定，等它自动重新加载。
    $configPath = Join-Path $fakeAppData 'BetterPrtSc\config.toml'
    $regionBinding = "[[bindings]]`r`nhotkey = `"Ctrl+Alt+Shift+F8`"`r`nmode = `"region`"`r`nactions = [`"copy_to_clipboard`"]`r`n`r`n"
    $windowBinding = "[[bindings]]`r`nhotkey = `"Ctrl+Alt+Shift+F9`"`r`nmode = `"window`"`r`nactions = [`"copy_to_clipboard`", `"auto_save`"]`r`nfilename_template = `"{window_title} {yyyy-MM-dd HH-mm-ss}`"`r`n`r`n"
    $config = [IO.File]::ReadAllText($configPath)
    [IO.File]::WriteAllText($configPath, $regionBinding + $windowBinding + $config, (New-Object Text.UTF8Encoding $false))
    Start-Sleep -Seconds 2

    $overlay = Find-Overlay (Get-Residents)[0].Id
    $clip::Clear()
    $p = Start-Bps @('capture', 'region')
    Assert-That (Wait-OverlayVisible $overlay $true) "bps capture region 打开了选区遮罩（$($p.Watch.ElapsedMilliseconds) ms）"
    Send-Overlay $overlay $WM_KEYDOWN $VK_ESCAPE ([IntPtr]::Zero)
    $r = Wait-Bps $p
    Assert-That ($r.Code -eq 5 -and $r.Stderr -match '已取消') "Esc 取消后 bps 退出码 5 且提示已取消（实际 $($r.Code)）：$($r.Stderr.Trim())"
    Assert-That (Wait-OverlayVisible $overlay $false) '取消后遮罩已隐藏'
    Assert-That (-not $clip::GetDataObject().GetDataPresent('PNG')) '取消不产生剪贴板内容'

    $p = Start-Bps @('capture', 'region')
    Assert-That (Wait-OverlayVisible $overlay $true) "再次打开选区遮罩（$($p.Watch.ElapsedMilliseconds) ms）"
    Send-Overlay $overlay $WM_LBUTTONDOWN $MK_LBUTTON (Point 100 100)
    Send-Overlay $overlay $WM_MOUSEMOVE $MK_LBUTTON (Point 299 199)
    Send-Overlay $overlay $WM_LBUTTONUP 0 (Point 299 199)
    Send-Overlay $overlay $WM_KEYDOWN $VK_RETURN ([IntPtr]::Zero)
    $r = Wait-Bps $p
    $png = Get-ClipboardPngSize
    Assert-That ($r.Code -eq 0 -and $png -eq '200x100') `
        "框选 (100,100)→(299,199) 并按 Enter：退出码 $($r.Code)，剪贴板 PNG $png（应为 200x100）$($r.Stderr)"
    Assert-That (Wait-OverlayVisible $overlay $false) '完成后遮罩已隐藏'

    # 窗口模式：临时显示一个置顶的纯色测试窗口，在遮罩上单击它。
    $form = New-Object System.Windows.Forms.Form
    try {
        $form.Text = 'BetterPrtSc 冒烟 窗口模式'
        $form.TopMost = $true
        $form.StartPosition = 'Manual'
        $form.Location = New-Object System.Drawing.Point(160, 140)
        $form.Size = New-Object System.Drawing.Size(420, 300)
        $form.BackColor = [System.Drawing.Color]::FromArgb(30, 160, 90)
        $form.Show()
        for ($i = 0; $i -lt 25; $i++) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 20 }
        $frame = Get-WindowFrameBounds $form.Handle
        $origin = Get-PhysicalVirtualScreenOrigin
        $clickX = [int](($frame.Left + $frame.Right) / 2) - $origin.X
        $clickY = [int](($frame.Top + $frame.Bottom) / 2) - $origin.Y
        $clip::Clear()
        $p = Start-Bps @('capture', 'window')
        $shown = $false
        for ($i = 0; $i -lt 250 -and -not $shown; $i++) {
            [System.Windows.Forms.Application]::DoEvents()
            $shown = [BpsCliSmokeUser32]::IsWindowVisible($overlay)
            if (-not $shown) { Start-Sleep -Milliseconds 20 }
        }
        Assert-That $shown "bps capture window 打开了选区遮罩（$($p.Watch.ElapsedMilliseconds) ms）"
        Send-Overlay $overlay $WM_LBUTTONDOWN $MK_LBUTTON (Point $clickX $clickY)
        Send-Overlay $overlay $WM_LBUTTONUP 0 (Point $clickX $clickY)
        $r = Wait-BpsPumping $p
        $expectedSize = "{0}x{1}" -f ($frame.Right - $frame.Left), ($frame.Bottom - $frame.Top)
        $data = $clip::GetDataObject()
        $files = Get-SavedFiles $data
        $savedFiles += $files
        $png = Get-ClipboardPngSize
        Assert-That ($r.Code -eq 0 -and $png -eq $expectedSize) `
            "单击测试窗口：退出码 $($r.Code)，剪贴板 PNG $png（应为窗口外框 $expectedSize，$($r.Ms) ms）$($r.Stderr)"
        $bitmap = New-Object System.Drawing.Bitmap ($data.GetData('PNG'))
        try {
            $center = $bitmap.GetPixel([int]($bitmap.Width / 2), [int]($bitmap.Height / 2))
            $corner = $bitmap.GetPixel(0, 0)
        }
        finally { $bitmap.Dispose() }
        Assert-That ($center.R -eq 30 -and $center.G -eq 160 -and $center.B -eq 90 -and $center.A -eq 255) `
            "窗口截图中心是测试窗口的颜色（实际 $center）"
        Assert-That ($corner.A -eq 0 -or $corner.A -eq 255) "左上角圆角以外透明（Win10 无圆角时不透明）：alpha $($corner.A)"
        Assert-That ($files.Count -eq 1 -and (Split-Path -Leaf $files[0]) -like 'BetterPrtSc 冒烟 窗口模式 *.png') `
            "自动存储的文件名含窗口标题：$files"
        Assert-That (Wait-OverlayVisible $overlay $false) '完成后遮罩已隐藏'
    }
    finally { $form.Close(); $form.Dispose() }

    $second = Start-Process -FilePath $resident -PassThru -WindowStyle Hidden `
        -RedirectStandardError (Join-Path $OutDir 'second-stderr.log')
    $null = $second.Handle
    Assert-That ($second.WaitForExit(5000)) '再次启动的 BetterPrtSc.exe 在 5 秒内退出'
    $secondLog = [string](Get-Content (Join-Path $OutDir 'second-stderr.log') -Raw -Encoding UTF8)
    Assert-That ($second.ExitCode -eq 0) "再次启动的进程退出码 0：$($secondLog.Trim())"
    Assert-That ((Get-Residents).Count -eq 1) '仍然只有一个常驻实例'

    $clip::Clear()
    $r = Invoke-Bps @('capture', 'display')
    $data = $clip::GetDataObject()
    $savedFiles += Get-SavedFiles $data
    Assert-That ($r.Code -eq 0 -and $data.GetDataPresent('PNG')) `
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
    $env:APPDATA = $realAppData
    # 剪贴板可能正被其他程序占用：恢复失败只警告，不影响后面的清理。
    try {
        if ($backupImage) { $clip::SetImage($backupImage) }
        elseif ($null -ne $backupText) { $clip::SetText($backupText) }
    }
    catch { "警告：恢复剪贴板失败：$($_.Exception.Message)" }
    # 只删除本次运行存下的文件，以及本次新建且已空的存储目录。
    foreach ($f in $savedFiles) {
        if ($f -and (Test-Path -LiteralPath $f) -and (Get-Item -LiteralPath $f).CreationTime -ge $startedAt) {
            Remove-Item -LiteralPath $f
            "已删除本次存下的 $f"
        }
    }
    if (-not $saveDirExisted -and (Test-Path -LiteralPath $saveDir) -and
        -not (Get-ChildItem -LiteralPath $saveDir -Force)) {
        Remove-Item -LiteralPath $saveDir
        "已删除本次新建的空目录 $saveDir"
    }
}
