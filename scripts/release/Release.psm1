# 发布流水线共用的函数：打包、体积检查、常驻内存测量、发布脚本都从这里取逻辑。
# 自测见 test-release.ps1。

Set-StrictMode -Version 3
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem

# 便携模式的标记文件名，与 bps-core 的 PORTABLE_MARKER 一致：程序目录下有这个文件时，
# 配置和日志都放在程序目录里。
$script:PortableMarker = 'portable'
$script:PortableMarkerText = @'
BetterPrtSc 便携模式标记：程序目录下有这个文件时，配置（config.toml）和日志（logs）都保存在本目录，不写入用户目录。
删除这个文件即改用 %APPDATA%\BetterPrtSc。
BetterPrtSc portable-mode marker: while this file exists, config and logs are kept in this folder.
'@

# 在 GitHub Actions 里输出为注解（::error::），本地输出为带级别前缀的普通文本。
function Write-CiMessage {
    param(
        [ValidateSet('error', 'warning', 'notice')]
        [string]$Level,
        [string]$Message
    )
    if ($env:GITHUB_ACTIONS -eq 'true') {
        Write-Host "::${Level}::$Message"
    }
    else {
        $label = @{ error = '错误'; warning = '警告'; notice = '提示' }[$Level]
        Write-Host "${label}：$Message"
    }
}

# 追加到 GitHub Actions 的作业摘要；本地运行时什么也不做。
function Add-StepSummary {
    param([string[]]$Lines)
    if ($env:GITHUB_STEP_SUMMARY) {
        Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Value ($Lines + '') -Encoding utf8
    }
}

# 常驻内存采样是否已稳定：最近 WindowSize 个样本的最大值与最小值之差不超过 ToleranceBytes。
# 稳定时返回窗口内的最大值（偏保守），否则返回 $null。
function Find-StableSample {
    param(
        [long[]]$Samples,
        [int]$WindowSize = 6,
        [long]$ToleranceBytes = 512KB
    )
    if ($Samples.Count -lt $WindowSize) { return $null }
    $window = $Samples[($Samples.Count - $WindowSize)..($Samples.Count - 1)]
    $stats = $window | Measure-Object -Minimum -Maximum
    if ($stats.Maximum - $stats.Minimum -gt $ToleranceBytes) { return $null }
    [long]$stats.Maximum
}

# 常驻内存的测量结果。判定指标是私有工作集（ADR 0005：任务管理器“内存”列），
# 工作集与私有字节只作参考。MB 按任务管理器的口径换算（1 MB = 1024 × 1024 字节）。
function New-MemoryReport {
    param(
        [long]$PrivateWorkingSet,
        [long]$WorkingSet,
        [long]$PrivateBytes,
        [double]$TargetMB = 30,
        [bool]$Stable
    )
    $mib = 1024 * 1024
    [pscustomobject][ordered]@{
        metric                = 'privateWorkingSet'
        privateWorkingSetMB   = [math]::Round($PrivateWorkingSet / $mib, 2)
        workingSetMB          = [math]::Round($WorkingSet / $mib, 2)
        privateBytesMB        = [math]::Round($PrivateBytes / $mib, 2)
        targetMB              = $TargetMB
        withinTarget          = $PrivateWorkingSet -lt [long]($TargetMB * $mib)
        stable                = $Stable
        privateWorkingSetBytes = $PrivateWorkingSet
        workingSetBytes       = $WorkingSet
        privateBytes          = $PrivateBytes
    }
}

# 根 Cargo.toml 中 [workspace.package] 的 version。
function Get-WorkspaceVersion {
    param([string]$CargoToml)
    $section = ''
    foreach ($line in $CargoToml -split "`r?`n") {
        if ($line -match '^\s*\[([^\]]+)\]\s*$') { $section = $Matches[1].Trim(); continue }
        if ($section -eq 'workspace.package' -and $line -match '^\s*version\s*=\s*"([^"]+)"') {
            return $Matches[1]
        }
    }
    throw 'Cargo.toml 的 [workspace.package] 里没有 version'
}

# 发布版本号：打 tag 发布时 tag 必须是 v<workspace 版本>；没有 tag（手动试跑）时直接用 workspace 版本。
function Resolve-ReleaseVersion {
    param(
        [AllowEmptyString()][string]$Tag,
        [string]$WorkspaceVersion
    )
    if (-not $Tag) { return $WorkspaceVersion }
    if ($Tag -notmatch '^v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$') {
        throw "tag $Tag 必须形如 v<版本号>，例如 v$WorkspaceVersion"
    }
    if ($Matches[1] -ne $WorkspaceVersion) {
        throw "tag $Tag 的版本 $($Matches[1]) 与 Cargo.toml 的版本 $WorkspaceVersion 不一致，先改 [workspace.package] 的 version"
    }
    $WorkspaceVersion
}

# 便携 zip：Files 平铺在 zip 根目录，另加 portable 标记文件。已存在的 Destination 会被覆盖。
function New-PortableZip {
    param(
        [string[]]$Files,
        [string]$Destination
    )
    if (Test-Path -LiteralPath $Destination) { Remove-Item -LiteralPath $Destination -Force }
    $zip = [IO.Compression.ZipFile]::Open($Destination, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($file in $Files) {
            [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                $zip, $file, (Split-Path -Leaf $file), [IO.Compression.CompressionLevel]::Optimal) | Out-Null
        }
        $writer = [IO.StreamWriter]::new($zip.CreateEntry($script:PortableMarker).Open(), [Text.UTF8Encoding]::new($false))
        try { $writer.Write($script:PortableMarkerText) } finally { $writer.Dispose() }
    }
    finally { $zip.Dispose() }
}

# 发布产物的文件名（GitHub Release 的附件名），打包、Scoop manifest、上传都用它。
function Get-ReleaseAssetName {
    param(
        [ValidateSet('Portable', 'Setup', 'Scoop', 'Checksums')]
        [string]$Kind,
        [string]$Version
    )
    switch ($Kind) {
        'Portable' { "BetterPrtSc-$Version-x64-portable.zip" }
        'Setup' { "BetterPrtSc-$Version-x64-setup.exe" }
        'Scoop' { 'betterprtsc.json' }
        'Checksums' { 'SHA256SUMS.txt' }
    }
}

# Scoop manifest（JSON 文本）。用户可直接
#   scoop install https://github.com/<repo>/releases/latest/download/betterprtsc.json
# 也可以把它放进自己的 bucket，checkver/autoupdate 会跟随 GitHub Releases。
# 下载的是便携 zip，但安装时删掉 portable 标记：Scoop 每个版本装在单独的目录里，
# 便携模式下配置会留在旧版本目录，升级即丢失；非便携模式的配置在 %APPDATA%\BetterPrtSc。
function New-ScoopManifest {
    param(
        [string]$Version,
        [string]$Repository,
        [string]$PortableZipHash
    )
    $homepage = "https://github.com/$Repository"
    $zipUrl = { param($v) "$homepage/releases/download/v$v/" + (Get-ReleaseAssetName -Kind Portable -Version $v) }
    $manifest = [ordered]@{
        version      = $Version
        description  = 'Lightweight tray-resident screenshot and screen recording tool driven by hotkeys.'
        homepage     = $homepage
        license      = 'GPL-3.0'
        notes        = 'Config is stored in %APPDATA%\BetterPrtSc. The bps command forwards captures to the resident instance (BetterPrtSc.exe).'
        architecture = [ordered]@{
            '64bit' = [ordered]@{
                url  = & $zipUrl $Version
                hash = $PortableZipHash.ToLowerInvariant()
            }
        }
        pre_install  = "Remove-Item -LiteralPath (Join-Path `$dir '$script:PortableMarker') -Force -ErrorAction SilentlyContinue"
        bin          = 'bps.exe'
        shortcuts    = @(, @('BetterPrtSc.exe', 'BetterPrtSc'))
        checkver     = [ordered]@{ github = $homepage }
        autoupdate   = [ordered]@{
            architecture = [ordered]@{
                '64bit' = [ordered]@{ url = & $zipUrl '$version' }
            }
        }
    }
    $manifest | ConvertTo-Json -Depth 8
}

function Format-Number([double]$Value) {
    $Value.ToString('0.##', [Globalization.CultureInfo]::InvariantCulture)
}

# GitHub Release 的说明（Markdown）：下载表（体积、SHA256）、安装方式、常驻内存记录。
# Assets 的元素需有 name、bytes、sha256；Memory 是 New-MemoryReport 的结果。
function New-ReleaseNotes {
    param(
        [string]$Version,
        [string]$Repository,
        [object[]]$Assets,
        [object]$Memory
    )
    $scoopUrl = "https://github.com/$Repository/releases/download/v$Version/" + (Get-ReleaseAssetName -Kind Scoop -Version $Version)
    $verdict = if ($Memory.withinTarget) { '达标' } else { '**超出目标**' }
    $stability = if ($Memory.stable) { '' } else { '；采样期间未完全稳定，取最后一个采样窗口的最大值' }
    $lines = @(
        '## 下载', '',
        '| 文件 | 大小 | SHA256 |', '|---|---:|---|'
    )
    foreach ($asset in $Assets) {
        $lines += "| $($asset.name) | $(Format-Number ($asset.bytes / 1e6)) MB | ``$($asset.sha256)`` |"
    }
    $lines += @(
        '',
        '- 安装包：按用户安装到 `%LOCALAPPDATA%\Programs\BetterPrtSc`，不需要管理员权限。',
        '- 便携 zip：解压到任意目录即可运行；目录里的 `portable` 标记文件让配置和日志都留在该目录。',
        "- Scoop：``scoop install $scoopUrl``（配置保存在 ``%APPDATA%\BetterPrtSc``）",
        '',
        '## 轻量指标',
        '',
        "- 常驻内存（私有工作集，即任务管理器的「内存」列）：$(Format-Number $Memory.privateWorkingSetMB) MB（目标 < $(Format-Number $Memory.targetMB) MB，$verdict）",
        "- 参考值：工作集 $(Format-Number $Memory.workingSetMB) MB，私有字节 $(Format-Number $Memory.privateBytesMB) MB",
        "- 测量方式：在发布构建机上启动常驻实例，空闲待内存稳定后采样$stability。"
    )
    $lines -join "`n"
}

Export-ModuleMember -Function *
