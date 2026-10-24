# 一次性原型代码（ticket #2）：构建两个遮罩原型和测量工具，跑完整测量并汇总体积与环境。
# 用法：pwsh prototypes/overlay-latency/run-measure.ps1 [-ColdRuns 8] [-Triggers 12]
#        [-Strategies prewarm,ondemand,lazy] [-Protos native,egui] [-ShowMode flush] [-NoDrag]
# 运行期间屏幕会反复闪现遮罩、鼠标会被移动一次（拖拽验证），请勿操作电脑。
param(
    [int]$ColdRuns = 8,
    [int]$Triggers = 12,
    [string]$Strategies = "prewarm,ondemand,lazy",
    [string]$Protos = "native,egui",
    [string]$ShowMode = "",
    [switch]$NoDrag
)
$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
$env:CARGO_TARGET_DIR = Join-Path $root "target"
foreach ($c in "native-d2d", "egui-wgpu", "harness") {
    cargo build --release --manifest-path (Join-Path $root "$c\Cargo.toml")
    if ($LASTEXITCODE) { throw "构建 $c 失败" }
}
$bin = Join-Path $root "target\release"
$exe = @{ native = "$bin\overlay-native.exe"; egui = "$bin\overlay-egui.exe" }

$stamp = Get-Date -Format "yyyy-MM-dd HH-mm-ss"
$suffix = if ($ShowMode) { " $ShowMode" } else { "" }
$out = Join-Path $root "results\$stamp$suffix"
$hargs = @("--out", $out, "--strategies", $Strategies, "--cold-runs", $ColdRuns, "--triggers", $Triggers)
foreach ($p in $Protos.Split(",")) { $hargs += @("--proto", "$p=$($exe[$p])") }
if ($NoDrag) { $hargs += "--no-drag" }
$env:BPS_SHOW_MODE = $ShowMode
try {
    & "$bin\overlay-harness.exe" @hargs
    if ($LASTEXITCODE) { throw "测量失败" }
} finally {
    $env:BPS_SHOW_MODE = $null
    Get-Process overlay-native, overlay-egui -ErrorAction SilentlyContinue | Stop-Process -Force
}

# 产物体积：release（opt-level 3 + LTO + strip + panic=abort），以及 zip 压缩后的大小。
$sizes = "`n## release 二进制体积`n`n| 原型 | exe | zip 压缩后 |`n|---|---|---|`n"
foreach ($p in $Protos.Split(",")) {
    $f = Get-Item $exe[$p]
    $zip = Join-Path $env:TEMP "bps-size-$p.zip"
    Compress-Archive -Path $f.FullName -DestinationPath $zip -Force
    $z = (Get-Item $zip).Length
    Remove-Item $zip
    $sizes += "| $p | {0:N2} MB | {1:N2} MB |`n" -f ($f.Length / 1MB), ($z / 1MB)
}

# 本机环境
$cpu = (Get-CimInstance Win32_Processor).Name.Trim()
$os = Get-CimInstance Win32_OperatingSystem
$ram = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
$gpus = (Get-CimInstance Win32_VideoController | Where-Object { $_.CurrentHorizontalResolution } | ForEach-Object { "$($_.Name)（驱动 $($_.DriverVersion)，$($_.CurrentHorizontalResolution)x$($_.CurrentVerticalResolution)@$($_.CurrentRefreshRate)Hz）" }) -join "；"
$ready = Select-String -Path (Join-Path $out "harness.log") -Pattern "READY" | Select-Object -First 1
$outputs = if ($ready -and $ready.Line -match "outputs=(\S+)") { $Matches[1] } else { "?" }
$rust = (rustc -V)
$envmd = @"

## 测量环境

- 时间：$stamp；参数：每配置冷启动 $ColdRuns 次 × 每次 1 次首次触发 + $Triggers 次后续触发；遮罩显示 250 ms 后自动关闭；触发间隔随机 250–450 ms；显示顺序模式：$(if ($ShowMode) { $ShowMode } else { "默认（Present → 取消 cloak）" })
- CPU：$cpu；内存：$ram GB
- 系统：$($os.Caption) $($os.Version)（build $($os.BuildNumber)）
- 有画面输出的显卡：$gpus
- 显示器（DXGI）：$outputs
- 工具链：$rust
"@
Add-Content -Path (Join-Path $out "summary.md") -Value ($sizes + $envmd) -Encoding utf8
Write-Host ($sizes + $envmd)
Write-Host "结果目录：$out"
