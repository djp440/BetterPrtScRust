<#
.SYNOPSIS
  常驻内存测量：启动常驻实例 → 空闲等待内存稳定 → 记录私有工作集等数值 → 让实例退出。

.DESCRIPTION
  判定指标是私有工作集（ADR 0005：任务管理器的「内存」列），目标 < 30 MB，
  MB 按任务管理器口径（1 MB = 1024 × 1024 字节）；工作集与私有字节一并记录作参考。
  每 IntervalMs 采样一次，最近 WindowSize 个样本的波动不超过 ToleranceKB 即视为稳定，
  取这个窗口内的最大值；TimeoutSeconds 内一直不稳定时取最后一个窗口的最大值并标为未稳定。

  为了不碰用户目录里的配置，先把 BetterPrtSc.exe 复制到临时目录并放上 portable 标记，
  以便携模式运行，结束后删除临时目录。同一会话里已有常驻实例时（单实例）无法测量，
  直接报错，不会动已在运行的实例。实例通过向它的隐藏窗口投递 WM_CLOSE 退出
  （注销热键、移除托盘图标），5 秒内没有退出才强制结束本脚本启动的那个进程。

  超出目标只给出警告（发布时如实记录数值）；测量本身失败时退出码为 1。
    pwsh -NoProfile -File scripts\release\measure-memory.ps1 [-Exe target\release\BetterPrtSc.exe] [-OutFile dist\memory.json]
#>
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\..\target\release\BetterPrtSc.exe'),
    [string]$OutFile,
    [double]$TargetMB = 30,
    [int]$IntervalMs = 500,
    [int]$WindowSize = 6,
    [int]$ToleranceKB = 512,
    [int]$TimeoutSeconds = 30
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Release.psm1') -Force

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class BpsMemoryProbe {
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessMemoryCountersEx2 {
        public uint cb;
        public uint PageFaultCount;
        public UIntPtr PeakWorkingSetSize;
        public UIntPtr WorkingSetSize;
        public UIntPtr QuotaPeakPagedPoolUsage;
        public UIntPtr QuotaPagedPoolUsage;
        public UIntPtr QuotaPeakNonPagedPoolUsage;
        public UIntPtr QuotaNonPagedPoolUsage;
        public UIntPtr PagefileUsage;
        public UIntPtr PeakPagefileUsage;
        public UIntPtr PrivateUsage;
        public UIntPtr PrivateWorkingSetSize;
        public ulong SharedCommitUsage;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool K32GetProcessMemoryInfo(IntPtr process, out ProcessMemoryCountersEx2 counters, uint cb);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr FindWindowExW(IntPtr parent, IntPtr childAfter, string className, string windowName);

    [DllImport("user32.dll")]
    static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);

    [DllImport("user32.dll")]
    public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);

    // 返回 [私有工作集, 工作集, 私有字节]，单位字节。
    // PROCESS_MEMORY_COUNTERS_EX2 需要 Windows 10 20348 / Windows 11 及以后的系统。
    public static long[] Read(IntPtr process) {
        ProcessMemoryCountersEx2 c;
        uint size = (uint)Marshal.SizeOf(typeof(ProcessMemoryCountersEx2));
        if (!K32GetProcessMemoryInfo(process, out c, size)) {
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        }
        return new long[] {
            (long)c.PrivateWorkingSetSize.ToUInt64(),
            (long)c.WorkingSetSize.ToUInt64(),
            (long)c.PrivateUsage.ToUInt64(),
        };
    }

    // 找属于指定进程、类名为 className 的顶层窗口；processId 为 0 时不限进程。
    public static IntPtr FindWindow(string className, uint processId) {
        IntPtr hwnd = IntPtr.Zero;
        while ((hwnd = FindWindowExW(IntPtr.Zero, hwnd, className, null)) != IntPtr.Zero) {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (processId == 0 || owner == processId) return hwnd;
        }
        return IntPtr.Zero;
    }
}
'@

$ResidentWindowClass = 'BetterPrtSc.ResidentWindow'  # 见 platform-windows 的 event_loop.rs
$WM_CLOSE = 0x0010

$Exe = (Resolve-Path -LiteralPath $Exe).Path
if ([BpsMemoryProbe]::FindWindow($ResidentWindowClass, 0) -ne [IntPtr]::Zero) {
    Write-CiMessage error '本会话里已有 BetterPrtSc 常驻实例在运行（单实例），请先退出它再测量'
    exit 1
}

$workDir = Join-Path ([IO.Path]::GetTempPath()) ('bps-memory-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $workDir | Out-Null
$appExe = Join-Path $workDir 'BetterPrtSc.exe'
Copy-Item -LiteralPath $Exe -Destination $appExe
New-Item -ItemType File (Join-Path $workDir 'portable') | Out-Null
$stderr = Join-Path $workDir 'stderr.log'

$proc = $null
$report = $null
$failure = $null
try {
    $proc = Start-Process -FilePath $appExe -WorkingDirectory $workDir -PassThru -RedirectStandardError $stderr
    $null = $proc.Handle  # 先取句柄，否则退出后读不到 ExitCode

    $hwnd = [IntPtr]::Zero
    $deadline = (Get-Date).AddSeconds(10)
    while ($hwnd -eq [IntPtr]::Zero -and -not $proc.HasExited -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 100
        $hwnd = [BpsMemoryProbe]::FindWindow($ResidentWindowClass, [uint32]$proc.Id)
    }
    if ($proc.HasExited) { throw "常驻实例启动后立即退出（退出码 $($proc.ExitCode)）" }
    if ($hwnd -eq [IntPtr]::Zero) { throw '10 秒内没有出现常驻实例的隐藏窗口' }
    Write-Host '常驻实例已就绪，开始采样'

    $privateWs = [Collections.Generic.List[long]]::new()
    $workingSet = [Collections.Generic.List[long]]::new()
    $privateBytes = [Collections.Generic.List[long]]::new()
    $stablePeak = $null
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ($null -eq $stablePeak -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds $IntervalMs
        if ($proc.HasExited) { throw "采样期间常驻实例退出了（退出码 $($proc.ExitCode)）" }
        $sample = [BpsMemoryProbe]::Read($proc.Handle)
        if ($sample[0] -le 0) { throw '读不到私有工作集：需要 Windows 10 20348 / Windows 11 及以后的系统' }
        $privateWs.Add($sample[0]); $workingSet.Add($sample[1]); $privateBytes.Add($sample[2])
        $stablePeak = Find-StableSample -Samples $privateWs.ToArray() -WindowSize $WindowSize -ToleranceBytes ($ToleranceKB * 1KB)
    }
    if ($privateWs.Count -lt $WindowSize) { throw "$TimeoutSeconds 秒内只采到 $($privateWs.Count) 个样本" }

    # 取最后一个窗口内的最大值（稳定时就是 Find-StableSample 的结果）。
    $windowMax = {
        param($list)
        ($list.GetRange($list.Count - $WindowSize, $WindowSize) | Measure-Object -Maximum).Maximum
    }
    $report = New-MemoryReport -PrivateWorkingSet (& $windowMax $privateWs) -WorkingSet (& $windowMax $workingSet) `
        -PrivateBytes (& $windowMax $privateBytes) -TargetMB $TargetMB -Stable ($null -ne $stablePeak)
    $report | Add-Member -NotePropertyMembers ([ordered]@{
            samples     = $privateWs.Count
            intervalMs  = $IntervalMs
            measuredAt  = (Get-Date).ToString('o')
            os          = [Environment]::OSVersion.VersionString
            exe         = $Exe
        })

    [BpsMemoryProbe]::PostMessageW($hwnd, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (-not $proc.WaitForExit(5000)) { throw '常驻实例没有在 5 秒内响应 WM_CLOSE 退出' }
}
catch {
    $failure = $_.Exception.Message
}
finally {
    if ($proc -and -not $proc.HasExited) {
        Stop-Process -Id $proc.Id -Force
        $proc.WaitForExit()
        Write-CiMessage warning '已强制结束本次启动的常驻实例（托盘可能残留图标，鼠标移过即消失）'
    }
    if ($failure -and (Test-Path -LiteralPath $stderr)) {
        Write-Host '--- 常驻实例输出 ---'
        Get-Content -LiteralPath $stderr -Encoding UTF8 | Write-Host
    }
    for ($i = 0; $i -lt 10 -and (Test-Path -LiteralPath $workDir); $i++) {
        try { Remove-Item -LiteralPath $workDir -Recurse -Force } catch { Start-Sleep -Milliseconds 200 }
    }
}

if ($failure) {
    Write-CiMessage error "常驻内存测量失败：$failure"
    exit 1
}

$verdict = if ($report.withinTarget) { '达标' } else { '超出目标' }
$stability = if ($report.stable) { "已稳定（$($report.samples) 个样本）" } else { "$TimeoutSeconds 秒内未稳定，取最后一个窗口的最大值" }
Write-Host ("常驻内存：私有工作集 {0} MB（目标 < {1} MB，{2}）；工作集 {3} MB；私有字节 {4} MB；{5}" -f
    $report.privateWorkingSetMB, $TargetMB, $verdict, $report.workingSetMB, $report.privateBytesMB, $stability)
if (-not $report.withinTarget) {
    Write-CiMessage warning "常驻内存（私有工作集）$($report.privateWorkingSetMB) MB 超出目标 $TargetMB MB"
}
if (-not $report.stable) {
    Write-CiMessage warning "常驻内存在 $TimeoutSeconds 秒内没有稳定"
}
Add-StepSummary @(
    "### 常驻内存（目标 < $TargetMB MB）", '',
    "- 私有工作集（判定指标）：$($report.privateWorkingSetMB) MB，$verdict",
    "- 工作集：$($report.workingSetMB) MB；私有字节：$($report.privateBytesMB) MB",
    "- $stability"
)
if ($OutFile) {
    $outPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutFile)
    New-Item -ItemType Directory -Force (Split-Path -Parent $outPath) | Out-Null
    [IO.File]::WriteAllText($outPath, ($report | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    Write-Host "已写入 $outPath"
}
exit 0
