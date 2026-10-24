<#
.SYNOPSIS
  结果面板（OCR）冒烟：生成一张含中英文文字的测试图交给 `BetterPrtSc.exe --result-panel`，检查识别、
  “复制”按钮、文本框 Ctrl+C / Ctrl+V 与系统剪贴板互通、“关闭”按钮，以及面板进程没有网络连接。

.DESCRIPTION
  直接运行结果面板子进程，截图经标准输入交给它（与常驻实例的做法相同）。依次验证：
    1. 面板窗口 10 秒内出现，标题是界面语言对应的文字
    2. 识别完成后点“复制”，剪贴板里是识别结果，含测试图里的中英文
    3. 点进文本框按 Ctrl+A、Ctrl+C，剪贴板里的文字与“复制”按钮的一致
    4. 剪贴板里放另一段文字，在文本框末尾另起一行按 Ctrl+V，再点“复制”：多出的正是这段文字
    5. 点“关闭”后面板进程 5 秒内退出（退出码 0）
    6. 过程中多次采样，面板进程没有 TCP 连接或 UDP 端点；另列出它加载的网络相关模块（仅供参考）
  -EndToEnd 再走一遍真实流程：临时配置只有一条“区域 + OCR”绑定（不复制、不存储、不检查更新），
  启动常驻实例并从它的输出确认加载的正是这份配置（1 条热键绑定）；临时显示一个置顶的测试窗口
  （内容即测试图），`bps capture region` 打开遮罩后向遮罩投递框选测试窗口与 Enter → bps 退出码 0、
  常驻实例打开结果面板子进程、剪贴板没有被改动、面板识别出测试图里的中英文；关掉面板与常驻实例。
  点击与按键是真实输入：先把面板切到前台，每次点击前确认面板在前台且点击位置在面板上、
  每次按键前确认面板在前台，否则中止（不把输入送进别的程序）；点击后鼠标移回原处。
  运行期间（约 30 秒）请勿操作键盘鼠标。面板截图（PrintWindow，只含面板自身）存在 -OutDir。
  -Previews 先用 `result_panel_preview` 示例（假的文字识别，不碰剪贴板）截下各状态的中英文界面，
  要先 `cargo build -p bps-ui --example result_panel_preview`。
  会读写剪贴板：开始前完整备份（任何一种格式备份失败就不运行），结束时恢复。
  子进程的 APPDATA 指向 -OutDir 下的临时目录，不碰真实的 %APPDATA%；-UiLanguage 不是 auto 时
  配置文件里写上界面语言。
  需要 STA 线程访问剪贴板，请用 Windows PowerShell 运行：
    powershell -NoProfile -STA -File scripts\smoke-result-panel.ps1 [-BinDir target\release] [-UiLanguage en] [-Previews] [-EndToEnd]
#>
param(
    # BetterPrtSc.exe 与 bps.exe 所在目录，默认 debug 构建。
    [string]$BinDir = (Join-Path $PSScriptRoot '..\target\debug'),
    [string]$OutDir = (Join-Path $env:TEMP 'bps-smoke-result-panel'),
    [ValidateSet('auto', 'zh-CN', 'en')][string]$UiLanguage = 'auto',
    [switch]$Previews,
    [switch]$EndToEnd
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class BpsPanelSmoke {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    public delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lParam);

    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string className, string windowName);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextLengthW(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr hwnd, ref POINT point);
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT point);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool BringWindowToTop(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool AttachThreadInput(uint attach, uint attachTo, bool doAttach);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT point);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] static extern uint MapVirtualKeyW(uint code, uint mapType);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);

    const uint GA_ROOT = 2;
    const uint MOUSEEVENTF_LEFTDOWN = 0x02, MOUSEEVENTF_LEFTUP = 0x04;
    const uint KEYEVENTF_EXTENDEDKEY = 0x01, KEYEVENTF_KEYUP = 0x02;
    const uint PW_RENDERFULLCONTENT = 2;

    // 本线程切到 Per-Monitor V2：窗口坐标、光标位置都按物理像素，不被系统按缩放比例换算；
    // 这期间创建的窗口也不被系统缩放。返回原来的设置，交给 Restore 恢复。
    public static IntPtr PerMonitor() { return SetThreadDpiAwarenessContext(new IntPtr(-4)); }
    public static void Restore(IntPtr previous) { SetThreadDpiAwarenessContext(previous); }

    // 进程 processId 的第一个可见、有标题的顶层窗口；还没有时为 IntPtr.Zero。
    // （OpenGL 初始化时还会有不带标题的辅助窗口，不算。）
    public static IntPtr FindVisibleWindow(uint processId) {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate (IntPtr hwnd, IntPtr unused) {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner == processId && IsWindowVisible(hwnd) && GetWindowTextLengthW(hwnd) > 0) { found = hwnd; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    // 进程 processId 的、窗口类为 className 的顶层窗口（可以是隐藏的）；没有时为 IntPtr.Zero。
    public static IntPtr FindClassWindow(string className, uint processId) {
        IntPtr hwnd = IntPtr.Zero;
        while (true) {
            hwnd = FindWindowExW(IntPtr.Zero, hwnd, className, null);
            if (hwnd == IntPtr.Zero) return hwnd;
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner == processId) return hwnd;
        }
    }

    public static string Title(IntPtr hwnd) {
        StringBuilder text = new StringBuilder(256);
        GetWindowTextW(hwnd, text, text.Capacity);
        return text.ToString();
    }

    // 窗口客户区在屏幕上的位置（物理像素）。
    public static RECT ClientOnScreen(IntPtr hwnd) {
        IntPtr previous = PerMonitor();
        try {
            RECT client;
            GetClientRect(hwnd, out client);
            POINT origin = new POINT();
            ClientToScreen(hwnd, ref origin);
            RECT rect;
            rect.Left = origin.X; rect.Top = origin.Y;
            rect.Right = origin.X + client.Right; rect.Bottom = origin.Y + client.Bottom;
            return rect;
        } finally { Restore(previous); }
    }

    // 虚拟桌面左上角（物理像素）。
    public static POINT VirtualScreenOrigin() {
        IntPtr previous = PerMonitor();
        try {
            POINT origin;
            origin.X = GetSystemMetrics(76);
            origin.Y = GetSystemMetrics(77);
            return origin;
        } finally { Restore(previous); }
    }

    // 投递窗口消息；鼠标消息里的坐标按物理像素。
    public static void Post(IntPtr hwnd, uint message, long wParam, long lParam) {
        IntPtr previous = PerMonitor();
        try { PostMessageW(hwnd, message, new IntPtr(wParam), new IntPtr(lParam)); }
        finally { Restore(previous); }
    }

    public static void PostClose(IntPtr hwnd) { PostMessageW(hwnd, 0x0010, IntPtr.Zero, IntPtr.Zero); }

    // 把窗口切到前台：临时把本线程的输入状态挂到当前前台窗口的线程上，SetForegroundWindow 才不会被拒绝。
    public static bool Activate(IntPtr hwnd) {
        IntPtr foreground = GetForegroundWindow();
        if (foreground == hwnd) return true;
        uint unused;
        uint foregroundThread = foreground == IntPtr.Zero ? 0 : GetWindowThreadProcessId(foreground, out unused);
        uint me = GetCurrentThreadId();
        bool attached = foregroundThread != 0 && foregroundThread != me && AttachThreadInput(me, foregroundThread, true);
        try { BringWindowToTop(hwnd); SetForegroundWindow(hwnd); }
        finally { if (attached) AttachThreadInput(me, foregroundThread, false); }
        Thread.Sleep(200);
        return GetForegroundWindow() == hwnd;
    }

    // 在窗口客户区 (x, y) 处单击；坐标以逻辑像素（96 DPI 下的像素）计，负数表示从右边 / 下边算起。
    // 窗口不在前台、或该处被别的窗口挡住时不点击，返回原因；点完把鼠标移回原处。
    public static string Click(IntPtr hwnd, double x, double y) {
        IntPtr previous = PerMonitor();
        try {
            if (GetForegroundWindow() != hwnd) return "窗口不在前台";
            double scale = GetDpiForWindow(hwnd) / 96.0;
            RECT client;
            GetClientRect(hwnd, out client);
            POINT point;
            point.X = (int)Math.Round(x < 0 ? client.Right + x * scale : x * scale);
            point.Y = (int)Math.Round(y < 0 ? client.Bottom + y * scale : y * scale);
            ClientToScreen(hwnd, ref point);
            if (GetAncestor(WindowFromPoint(point), GA_ROOT) != hwnd) return "点击位置被别的窗口挡住";
            POINT original;
            GetCursorPos(out original);
            SetCursorPos(point.X, point.Y);
            Thread.Sleep(100);
            mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, UIntPtr.Zero);
            Thread.Sleep(60);
            mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, UIntPtr.Zero);
            Thread.Sleep(100);
            SetCursorPos(original.X, original.Y);
            return null;
        } finally { Restore(previous); }
    }

    // 按住 modifiers 按一下 key；窗口不在前台时不按，返回原因。
    public static string Chord(IntPtr hwnd, byte[] modifiers, byte key, bool extended) {
        if (GetForegroundWindow() != hwnd) return "窗口不在前台";
        foreach (byte modifier in modifiers) Key(modifier, 0);
        uint flags = extended ? KEYEVENTF_EXTENDEDKEY : 0;
        Key(key, flags);
        Key(key, flags | KEYEVENTF_KEYUP);
        for (int i = modifiers.Length - 1; i >= 0; i--) Key(modifiers[i], KEYEVENTF_KEYUP);
        Thread.Sleep(150);
        return null;
    }

    public static void Key(byte vk, uint flags) {
        keybd_event(vk, (byte)MapVirtualKeyW(vk, 0), flags, UIntPtr.Zero);
        Thread.Sleep(30);
    }

    // 用 PrintWindow 截下窗口自身的内容（不含挡在它前面的其他窗口）存成 PNG。
    public static bool Capture(IntPtr hwnd, string path) {
        IntPtr previous = PerMonitor();
        try {
            RECT rect;
            GetWindowRect(hwnd, out rect);
            using (Bitmap bitmap = new Bitmap(rect.Right - rect.Left, rect.Bottom - rect.Top, PixelFormat.Format24bppRgb)) {
                bool ok;
                using (Graphics graphics = Graphics.FromImage(bitmap)) {
                    IntPtr hdc = graphics.GetHdc();
                    ok = PrintWindow(hwnd, hdc, PW_RENDERFULLCONTENT);
                    graphics.ReleaseHdc(hdc);
                }
                bitmap.Save(path, ImageFormat.Png);
                return ok;
            }
        } finally { Restore(previous); }
    }

    // 白底黑字的测试图，每行一段文字。
    public static Bitmap RenderText(string[] lines, int size) {
        Bitmap bitmap = new Bitmap(size * 24, size * 2 * lines.Length + size, PixelFormat.Format32bppArgb);
        using (Graphics graphics = Graphics.FromImage(bitmap))
        using (Font font = new Font("Microsoft YaHei UI", size, FontStyle.Regular, GraphicsUnit.Pixel)) {
            graphics.Clear(Color.White);
            graphics.TextRenderingHint = System.Drawing.Text.TextRenderingHint.AntiAliasGridFit;
            for (int i = 0; i < lines.Length; i++) {
                graphics.DrawString(lines[i], font, Brushes.Black, size / 2, size / 2 + i * size * 2);
            }
        }
        return bitmap;
    }

    // 结果面板标准输入的格式：魔数 "BPSIMG1\n"、宽、高（u32 小端）、RGBA 像素。
    public static byte[] Payload(Bitmap bitmap) {
        int width = bitmap.Width, height = bitmap.Height;
        BitmapData data = bitmap.LockBits(new Rectangle(0, 0, width, height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        byte[] bgra = new byte[width * height * 4];
        try {
            for (int y = 0; y < height; y++) {
                Marshal.Copy(new IntPtr(data.Scan0.ToInt64() + (long)y * data.Stride), bgra, y * width * 4, width * 4);
            }
        } finally { bitmap.UnlockBits(data); }
        byte[] magic = Encoding.ASCII.GetBytes("BPSIMG1\n");
        byte[] payload = new byte[16 + bgra.Length];
        Buffer.BlockCopy(magic, 0, payload, 0, 8);
        Buffer.BlockCopy(BitConverter.GetBytes((uint)width), 0, payload, 8, 4);
        Buffer.BlockCopy(BitConverter.GetBytes((uint)height), 0, payload, 12, 4);
        for (int i = 0; i < bgra.Length; i += 4) {
            payload[16 + i] = bgra[i + 2];
            payload[16 + i + 1] = bgra[i + 1];
            payload[16 + i + 2] = bgra[i];
            payload[16 + i + 3] = bgra[i + 3];
        }
        return payload;
    }
}
'@

$WM_MOUSEMOVE = 0x0200
$WM_LBUTTONDOWN = 0x0201
$WM_LBUTTONUP = 0x0202
$WM_KEYDOWN = 0x0100
$MK_LBUTTON = 0x0001
$VK_CONTROL = [byte]0x11
$VK_RETURN = [byte]0x0D
$VK_END = [byte]0x23
$VK_A = [byte]0x41
$VK_C = [byte]0x43
$VK_V = [byte]0x56
$clip = [System.Windows.Forms.Clipboard]
$residentExe = (Resolve-Path (Join-Path $BinDir 'BetterPrtSc.exe')).Path
New-Item -ItemType Directory -Force $OutDir | Out-Null

function Assert-That([bool]$condition, [string]$message) {
    if (-not $condition) { throw "失败：$message" }
    "通过：$message"
}

# 剪贴板可能正被别的程序占用：失败时稍等重试。
function Invoke-Clipboard([scriptblock]$action) {
    for ($i = 0; ; $i++) {
        try { return & $action }
        catch { if ($i -ge 9) { throw } ; Start-Sleep -Milliseconds 100 }
    }
}

# 完整备份剪贴板里的每一种格式；任何一种读不出来就抛出（备份失败就不运行）。
function Backup-Clipboard {
    $data = Invoke-Clipboard { $clip::GetDataObject() }
    $backup = New-Object System.Windows.Forms.DataObject
    $formats = @(if ($data) { $data.GetFormats($false) })
    foreach ($format in $formats) {
        $value = $data.GetData($format, $false)
        if ($null -eq $value) { throw "无法备份剪贴板里的 $format 格式，不运行" }
        $backup.SetData($format, $false, $value)
    }
    [pscustomobject]@{ Data = $backup; Formats = $formats }
}

function Restore-Clipboard($backup) {
    if ($backup.Formats.Count -eq 0) { Invoke-Clipboard { $clip::Clear() } }
    else { Invoke-Clipboard { $clip::SetDataObject($backup.Data, $true, 10, 100) } }
    $now = @(Invoke-Clipboard { $clip::GetDataObject().GetFormats($false) })
    $missing = @($backup.Formats | Where-Object { $now -notcontains $_ })
    if ($missing.Count -eq 0) { "已恢复剪贴板（$($backup.Formats.Count) 种格式）" }
    else { "警告：恢复后的剪贴板缺少格式 $($missing -join ', ')" }
}

function Set-ClipboardText([string]$text) { Invoke-Clipboard { $clip::SetText($text) } }

# 剪贴板里的文字，换行统一成 LF。
function Get-ClipboardText { (Invoke-Clipboard { $clip::GetText() }) -replace "`r`n", "`n" }

# 等进程 $p 出现可见的顶层窗口。
function Wait-Window($p, [int]$seconds) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        if ($p.HasExited) { throw "进程 $($p.Id) 提前退出（退出码 $($p.ExitCode)）" }
        $hwnd = [BpsPanelSmoke]::FindVisibleWindow([uint32]$p.Id)
        if ($hwnd -ne [IntPtr]::Zero) { return $hwnd }
        Start-Sleep -Milliseconds 100
    }
    throw "进程 $($p.Id) $seconds 秒内没有显示窗口"
}

# 向窗口 $hwnd（没有时用主窗口）发 WM_CLOSE，5 秒内没退出就强制结束。
function Close-Window($p, [IntPtr]$hwnd = [IntPtr]::Zero) {
    if ($p.HasExited) { return }
    if ($hwnd -ne [IntPtr]::Zero) { [BpsPanelSmoke]::PostClose($hwnd) } else { $p.CloseMainWindow() | Out-Null }
    if (-not $p.WaitForExit(5000)) {
        Stop-Process -Id $p.Id -Force
        "警告：已强制结束进程 $($p.Id)"
    }
}

# 面板被别的窗口抢走前台时，重新切回来再试一次；仍不行就中止。
function Click([IntPtr]$hwnd, [double]$x, [double]$y, [string]$what) {
    $reason = [BpsPanelSmoke]::Click($hwnd, $x, $y)
    if ($reason -and [BpsPanelSmoke]::Activate($hwnd)) { $reason = [BpsPanelSmoke]::Click($hwnd, $x, $y) }
    if ($reason) { throw "点击$what 失败：$reason" }
}

function Press([IntPtr]$hwnd, [byte[]]$modifiers, [byte]$key, [string]$what, [bool]$extended = $false) {
    $reason = [BpsPanelSmoke]::Chord($hwnd, $modifiers, $key, $extended)
    if ($reason -and [BpsPanelSmoke]::Activate($hwnd)) { $reason = [BpsPanelSmoke]::Chord($hwnd, $modifiers, $key, $extended) }
    if ($reason) { throw "按 $what 失败：$reason" }
}

# 把进程当前的 TCP 连接与 UDP 端点记进 $endpoints。
function Add-Endpoints([int]$processId, $endpoints) {
    Get-NetTCPConnection -OwningProcess $processId -ErrorAction SilentlyContinue |
        ForEach-Object { $endpoints.Add("TCP $($_.LocalAddress):$($_.LocalPort) -> $($_.RemoteAddress):$($_.RemotePort) $($_.State)") }
    Get-NetUDPEndpoint -OwningProcess $processId -ErrorAction SilentlyContinue |
        ForEach-Object { $endpoints.Add("UDP $($_.LocalAddress):$($_.LocalPort)") }
}

# 本次运行的 BetterPrtSc.exe 进程里，命令行带 `--result-panel` 的、由 $parentId 启动的那个。
function Find-PanelChild([int]$parentId) {
    Get-CimInstance Win32_Process -Filter "Name = 'BetterPrtSc.exe' AND ParentProcessId = $parentId" |
        Where-Object { $_.ExecutablePath -eq $residentExe -and $_.CommandLine -match '\s--result-panel(\s|$)' } |
        ForEach-Object { Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue } |
        Select-Object -First 1
}

# 结果面板布局（逻辑像素）：底部一排按钮，“复制”在最左，“关闭”在最右；文本框占据中间。
$copyButton = @(22, -15)
$closeButton = @(-24, -15)
$textArea = @(120, 90)
$sentinel = 'BPS-SMOKE-SENTINEL'
$pasteText = '粘贴测试 Paste 123'
$testLines = [string[]]@('BetterPrtSc 截图工具', 'Hello World 你好世界', '使用Windows识别文字，版本 2.0')
# Windows 的中文 OCR 对中英混排的识别并不完美（常把 l 认成 I、个别汉字认错），只检查稳定认得出的词。
$expectedWords = 'BetterPrtSc', '截图', '世界', 'Windows', '版本'

# 检查结果面板进程 $p（用 $name 区分输出文件）：窗口出现、标题对；识别完成后“复制”按钮把含测试图
# 中英文的识别结果放进剪贴板；$full 时再检查文本框的 Ctrl+A / Ctrl+C / Ctrl+V；最后点“关闭”。
# 期间采样面板进程的网络端点。调用前剪贴板里应是 $sentinel。
function Test-Panel($p, [string]$name, [bool]$full) {
    $endpoints = New-Object System.Collections.Generic.List[string]
    $samples = 0
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $hwnd = Wait-Window $p 10
    "$($name)：面板窗口在 $($watch.ElapsedMilliseconds) ms 后出现"
    Add-Endpoints $p.Id $endpoints; $samples++
    $title = [BpsPanelSmoke]::Title($hwnd)
    $titles = @{ 'zh-CN' = @('OCR 结果'); 'en' = @('OCR result'); 'auto' = @('OCR 结果', 'OCR result') }[$UiLanguage]
    Assert-That ($titles -contains $title) "$($name)：窗口标题是`“$title`”"
    Start-Sleep -Milliseconds 1000
    Assert-That ([BpsPanelSmoke]::Activate($hwnd)) "$($name)：面板切到了前台"

    # 识别完成前“复制”按钮不可用：点到剪贴板变了为止。
    Set-ClipboardText $sentinel
    $recognized = $null
    for ($i = 0; $i -lt 10 -and $null -eq $recognized; $i++) {
        Click $hwnd $copyButton[0] $copyButton[1] '“复制”'
        Start-Sleep -Milliseconds 300
        $text = Get-ClipboardText
        if ($text -ne $sentinel) { $recognized = $text } else { Add-Endpoints $p.Id $endpoints; $samples++; Start-Sleep -Milliseconds 500 }
    }
    [void][BpsPanelSmoke]::Capture($hwnd, (Join-Path $OutDir "$name-recognized.png"))
    Assert-That ($null -ne $recognized) "$($name)：识别完成后`“复制`”按钮把识别结果放进了剪贴板（面板出现后 $($watch.ElapsedMilliseconds) ms 内）"
    "识别结果：`n$recognized"
    [IO.File]::WriteAllText((Join-Path $OutDir "$name-recognized.txt"), $recognized)
    $squashed = $recognized -replace '\s', ''
    $missing = @($expectedWords | Where-Object { -not $squashed.Contains($_) })
    Assert-That ($missing.Count -eq 0) "$($name)：识别结果含 $($expectedWords -join '、')（缺 $($missing -join '、')）"
    Add-Endpoints $p.Id $endpoints; $samples++

    if ($full) {
        Set-ClipboardText $sentinel
        Click $hwnd $textArea[0] $textArea[1] '文本框'
        Press $hwnd @($VK_CONTROL) $VK_A 'Ctrl+A'
        Press $hwnd @($VK_CONTROL) $VK_C 'Ctrl+C'
        Start-Sleep -Milliseconds 300
        Assert-That ((Get-ClipboardText) -eq $recognized) "$($name)：文本框里 Ctrl+A、Ctrl+C 复制的文字与`“复制`”按钮的一致"

        Set-ClipboardText $pasteText
        Press $hwnd @($VK_CONTROL) $VK_END 'Ctrl+End' $true
        Press $hwnd @() $VK_RETURN 'Enter'
        Press $hwnd @($VK_CONTROL) $VK_V 'Ctrl+V'
        Start-Sleep -Milliseconds 300
        [void][BpsPanelSmoke]::Capture($hwnd, (Join-Path $OutDir "$name-pasted.png"))
        Set-ClipboardText $sentinel
        Click $hwnd $copyButton[0] $copyButton[1] '“复制”'
        Start-Sleep -Milliseconds 300
        $edited = Get-ClipboardText
        Assert-That ($edited -eq "$recognized`n$pasteText") "$($name)：Ctrl+V 粘贴进文本框的是剪贴板里的文字（复制得到：$edited）"
        Add-Endpoints $p.Id $endpoints; $samples++

        $p.Refresh()
        $networkModules = @($p.Modules | ForEach-Object { $_.ModuleName } |
            Where-Object { $_ -match '^(winhttp|wininet|webio|ws2_32|mswsock|dnsapi|winnsi|iphlpapi)\.dll$' })
        "$($name)：面板进程加载的网络相关模块（仅供参考）：$(if ($networkModules) { $networkModules -join ', ' } else { '无' })"
    }

    Click $hwnd $closeButton[0] $closeButton[1] '“关闭”'
    Assert-That ($p.WaitForExit(5000)) "$($name)：点`“关闭`”后面板进程退出"
    Assert-That ($p.ExitCode -eq 0) "$($name)：面板进程退出码 0（实际 $($p.ExitCode)）"
    Assert-That ($endpoints.Count -eq 0) "$($name)：面板进程没有网络连接（采样 $samples 次）$($endpoints -join '; ')"
}

$backup = Backup-Clipboard
"已备份剪贴板（$($backup.Formats.Count) 种格式）"
$realAppData = $env:APPDATA
$fakeAppData = Join-Path $OutDir 'appdata'
if (Test-Path -LiteralPath $fakeAppData) { Remove-Item -Recurse -Force -LiteralPath $fakeAppData }
New-Item -ItemType Directory -Force (Join-Path $fakeAppData 'BetterPrtSc') | Out-Null
$configPath = Join-Path $fakeAppData 'BetterPrtSc\config.toml'
$languageLine = if ($UiLanguage -ne 'auto') { "language = `"$UiLanguage`"`n" } else { '' }
[IO.File]::WriteAllText($configPath, $languageLine)
$env:APPDATA = $fakeAppData
$preview = $null
$panel = $null
$resident = $null
$form = $null
try {
    if ($Previews) {
        $previewExe = (Resolve-Path (Join-Path $BinDir 'examples\result_panel_preview.exe')).Path
        foreach ($language in 'zh-CN', 'en') {
            foreach ($scenario in 'recognized', 'slow', 'failed', 'no-language-pack') {
                $name = "preview-$scenario-$language"
                $preview = Start-Process -FilePath $previewExe -ArgumentList $scenario, $language -PassThru -NoNewWindow `
                    -RedirectStandardOutput (Join-Path $OutDir "$name.out.log") -RedirectStandardError (Join-Path $OutDir "$name.err.log")
                $hwnd = Wait-Window $preview 10
                Start-Sleep -Milliseconds 1200
                $printed = [BpsPanelSmoke]::Capture($hwnd, (Join-Path $OutDir "$name.png"))
                $title = [BpsPanelSmoke]::Title($hwnd)
                Close-Window $preview $hwnd
                "预览 $($name)：截图$(if ($printed) { '已存' } else { '可能不完整' })，窗口标题`“$title`”"
            }
        }
        $preview = $null
    }

    $bitmap = [BpsPanelSmoke]::RenderText($testLines, 32)
    $bitmap.Save((Join-Path $OutDir 'input.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    $payloadPath = Join-Path $OutDir 'input.bpsimg'
    [IO.File]::WriteAllBytes($payloadPath, [BpsPanelSmoke]::Payload($bitmap))

    $panel = Start-Process -FilePath $residentExe -ArgumentList '--result-panel' -PassThru -NoNewWindow `
        -RedirectStandardInput $payloadPath `
        -RedirectStandardOutput (Join-Path $OutDir 'panel.out.log') -RedirectStandardError (Join-Path $OutDir 'panel.err.log')
    $null = $panel.Handle
    Test-Panel $panel 'panel' $true
    $panel = $null

    if ($EndToEnd) {
        # 只有一条“区域 + OCR”绑定：不复制、不存储；检查更新与开机自启默认关闭。
        $binding = "[[bindings]]`nhotkey = `"Ctrl+Alt+Shift+F7`"`nmode = `"region`"`nactions = [`"ocr`"]`n"
        [IO.File]::WriteAllText($configPath, $languageLine + $binding)
        $residentLog = Join-Path $OutDir 'resident.err.log'
        $resident = Start-Process -FilePath $residentExe -PassThru -NoNewWindow `
            -RedirectStandardOutput (Join-Path $OutDir 'resident.out.log') -RedirectStandardError $residentLog
        $null = $resident.Handle
        $started = $null
        for ($i = 0; $i -lt 100 -and -not $started; $i++) {
            if ($resident.HasExited) { throw "常驻实例提前退出（退出码 $($resident.ExitCode)）" }
            $stream = [IO.File]::Open($residentLog, 'Open', 'Read', 'ReadWrite')
            try { $log = (New-Object IO.StreamReader($stream, [Text.Encoding]::UTF8)).ReadToEnd() } finally { $stream.Dispose() }
            $started = [regex]::Match($log, '常驻实例已启动.*')
            if (-not $started.Success) { $started = $null; Start-Sleep -Milliseconds 100 }
        }
        Assert-That ($null -ne $started) '常驻实例 10 秒内启动'
        Assert-That ($started.Value -match '，1 条热键绑定') "常驻实例加载的是测试配置（$($started.Value.Trim())）"

        # 测试窗口：内容即测试图，按物理像素显示（创建时本线程切到 Per-Monitor V2，系统不缩放它）。
        $previous = [BpsPanelSmoke]::PerMonitor()
        try {
            $form = New-Object System.Windows.Forms.Form
            $form.Text = 'BetterPrtSc 冒烟 OCR'
            $form.FormBorderStyle = 'None'
            $form.ShowInTaskbar = $false
            $form.StartPosition = 'Manual'
            $form.Location = New-Object System.Drawing.Point(200, 200)
            $form.ClientSize = $bitmap.Size
            $form.BackgroundImage = $bitmap
            $form.BackgroundImageLayout = 'None'
            $form.Show()
        }
        finally { [BpsPanelSmoke]::Restore($previous) }
        # 显示之后再置顶（显示前设的 TopMost 实测不一定生效）。
        $form.TopMost = $true
        for ($i = 0; $i -lt 25; $i++) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 20 }
        $bounds = [BpsPanelSmoke]::ClientOnScreen($form.Handle)
        $origin = [BpsPanelSmoke]::VirtualScreenOrigin()
        $overlay = [BpsPanelSmoke]::FindClassWindow('BetterPrtSc.SelectionOverlay', [uint32]$resident.Id)
        Assert-That ($overlay -ne [IntPtr]::Zero) '找到常驻实例的选区遮罩窗口'

        Set-ClipboardText $sentinel
        $bps = Start-Process -FilePath (Join-Path $BinDir 'bps.exe') -ArgumentList 'capture', 'region' -PassThru -NoNewWindow `
            -RedirectStandardOutput (Join-Path $OutDir 'bps.out.log') -RedirectStandardError (Join-Path $OutDir 'bps.err.log')
        $null = $bps.Handle
        $shown = $false
        for ($i = 0; $i -lt 250 -and -not $shown; $i++) {
            [System.Windows.Forms.Application]::DoEvents()
            $shown = [BpsPanelSmoke]::IsWindowVisible($overlay)
            if (-not $shown) { Start-Sleep -Milliseconds 20 }
        }
        Assert-That $shown 'bps capture region 打开了选区遮罩'
        # 遮罩铺满虚拟桌面，客户区坐标 = 屏幕坐标 - 虚拟桌面左上角。框选测试窗口内部（各边留 2 像素）。
        $left = $bounds.Left - $origin.X + 2
        $top = $bounds.Top - $origin.Y + 2
        $right = $bounds.Right - $origin.X - 3
        $bottom = $bounds.Bottom - $origin.Y - 3
        [BpsPanelSmoke]::Post($overlay, $WM_LBUTTONDOWN, $MK_LBUTTON, (($top -shl 16) -bor $left))
        [BpsPanelSmoke]::Post($overlay, $WM_MOUSEMOVE, $MK_LBUTTON, (($bottom -shl 16) -bor $right))
        [BpsPanelSmoke]::Post($overlay, $WM_LBUTTONUP, 0, (($bottom -shl 16) -bor $right))
        [BpsPanelSmoke]::Post($overlay, $WM_KEYDOWN, $VK_RETURN, 0)
        for ($i = 0; $i -lt 1000 -and -not $bps.HasExited; $i++) {
            [System.Windows.Forms.Application]::DoEvents()
            Start-Sleep -Milliseconds 20
        }
        if (-not $bps.HasExited) { $bps.Kill(); throw 'bps capture region 20 秒内没有结束' }
        $bpsErr = [string](Get-Content (Join-Path $OutDir 'bps.err.log') -Raw -Encoding UTF8)
        Assert-That ($bps.ExitCode -eq 0) "框选测试窗口并按 Enter：bps 退出码 0（实际 $($bps.ExitCode)）$bpsErr"
        $form.Close()
        $form.Dispose()
        $form = $null

        for ($i = 0; $i -lt 50 -and -not $panel; $i++) {
            $panel = Find-PanelChild $resident.Id
            if (-not $panel) { Start-Sleep -Milliseconds 200 }
        }
        Assert-That ($null -ne $panel) '常驻实例打开了结果面板子进程'
        $null = $panel.Handle
        Assert-That ((Get-ClipboardText) -eq $sentinel) '只有 OCR 动作时捕获不改动剪贴板'
        Test-Panel $panel 'end-to-end' $false
        $panel = $null

        $residentWindow = [BpsPanelSmoke]::FindClassWindow('BetterPrtSc.ResidentWindow', [uint32]$resident.Id)
        Close-Window $resident $residentWindow
        Assert-That ($resident.HasExited) '常驻实例已退出'
        $resident = $null
    }
}
finally {
    if ($form) { $form.Close(); $form.Dispose() }
    foreach ($p in @($preview, $panel)) { if ($p) { Close-Window $p } }
    if ($resident) {
        Close-Window $resident ([BpsPanelSmoke]::FindClassWindow('BetterPrtSc.ResidentWindow', [uint32]$resident.Id))
    }
    if ($bitmap) { $bitmap.Dispose() }
    [BpsPanelSmoke]::Key($VK_CONTROL, 2)
    $env:APPDATA = $realAppData
    Restore-Clipboard $backup
}
