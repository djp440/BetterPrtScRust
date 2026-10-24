<#
.SYNOPSIS
  发布脚本自测（体积检查、常驻内存判定、便携 zip、Scoop manifest、版本校验、发布说明）。

.DESCRIPTION
  不依赖 Pester：本机自带的是 Pester 3.4，CI 上是 5.x，两者断言语法互不兼容。
  任一用例失败时退出码为 1。
    pwsh -NoProfile -File scripts\release\test-release.ps1
#>
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Release.psm1') -Force

$script:failures = 0
$script:tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("bps-release-test-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $script:tempRoot | Out-Null

function Test-Case([string]$Name, [scriptblock]$Body) {
    try {
        & $Body
        "通过  $Name"
    }
    catch {
        $script:failures++
        "失败  $Name"
        "      $($_.Exception.Message)"
    }
}

function Assert-Equal($Expected, $Actual, [string]$Because = '') {
    if ($Expected -ne $Actual) { throw "期望 <$Expected>，实际 <$Actual>。$Because" }
}

function Assert-True([bool]$Condition, [string]$Because) {
    if (-not $Condition) { throw $Because }
}

function Assert-Throws([scriptblock]$Body, [string]$Pattern) {
    try { & $Body }
    catch {
        if ($_.Exception.Message -notmatch $Pattern) {
            throw "抛出的错误 [$($_.Exception.Message)] 不含 /$Pattern/"
        }
        return
    }
    throw "应当抛出含 /$Pattern/ 的错误，实际没有抛出"
}

# 在临时目录里建一个指定字节数的文件。
function New-SizedFile([string]$Name, [long]$Bytes) {
    $path = Join-Path $script:tempRoot $Name
    $stream = [IO.File]::Create($path)
    try { $stream.SetLength($Bytes) } finally { $stream.Dispose() }
    $path
}

$checkSize = Join-Path $PSScriptRoot 'check-size.ps1'

# ---- 安装包体积检查：安装包 < 15 MB（十进制 MB，取更严格的口径） ----

Test-Case '体积检查：小于 15 MB 的安装包通过' {
    $file = New-SizedFile 'small-setup.exe' 14999999
    & $checkSize -Path $file *> $null
    Assert-Equal 0 $LASTEXITCODE
}

Test-Case '体积检查：达到 15 MB 的安装包失败' {
    $file = New-SizedFile 'big-setup.exe' 15000000
    & $checkSize -Path $file *> $null
    Assert-Equal 1 $LASTEXITCODE
}

Test-Case '体积检查：通配符一个文件都没匹配到时失败（不能漏检）' {
    & $checkSize -Path (Join-Path $script:tempRoot 'nothing-*-setup.exe') *> $null
    Assert-Equal 1 $LASTEXITCODE
}

# ---- 常驻内存：等待稳定，按私有工作集判定（ADR 0005：任务管理器“内存”列） ----

$MiB = 1024 * 1024

Test-Case '内存稳定：最近一个窗口内波动不超过容差即稳定，取窗口内最大值' {
    $samples = @(4, 9, 12, 12.1, 12.05, 12.2, 12.1, 12.15, 12.1) | ForEach-Object { [long]($_ * $MiB) }
    $stable = Find-StableSample -Samples $samples -WindowSize 6 -ToleranceBytes (512KB)
    Assert-Equal ([long](12.2 * $MiB)) $stable
}

Test-Case '内存稳定：仍在增长时不算稳定' {
    $samples = @(10, 11, 12, 13, 14, 15, 16) | ForEach-Object { [long]($_ * $MiB) }
    Assert-Equal $null (Find-StableSample -Samples $samples -WindowSize 6 -ToleranceBytes (512KB))
}

Test-Case '内存稳定：样本数不足一个窗口时不算稳定' {
    $samples = @(12, 12, 12) | ForEach-Object { [long]($_ * $MiB) }
    Assert-Equal $null (Find-StableSample -Samples $samples -WindowSize 6 -ToleranceBytes (512KB))
}

Test-Case '内存判定：以私有工作集对比目标，工作集偏高不影响结论' {
    $report = New-MemoryReport -PrivateWorkingSet (20 * $MiB) -WorkingSet (40 * $MiB) -PrivateBytes (25 * $MiB) -TargetMB 30 -Stable $true
    Assert-True $report.withinTarget '私有工作集 20 MB 应在 30 MB 目标内'
}

Test-Case '内存判定：私有工作集达到 30 MB（任务管理器口径 MiB）即超标' {
    $report = New-MemoryReport -PrivateWorkingSet (30 * $MiB) -WorkingSet (31 * $MiB) -PrivateBytes (32 * $MiB) -TargetMB 30 -Stable $true
    Assert-True (-not $report.withinTarget) '私有工作集 30 MB 不满足 < 30 MB'
}

# ---- 版本：tag 必须与 Cargo.toml 的 workspace 版本一致 ----

$cargoToml = @'
[workspace]
members = ["crates/core"]

[workspace.dependencies]
png = { version = "0.18", default-features = false }

[workspace.package]
version = "0.3.1"
edition = "2024"
'@

Test-Case '版本：从 [workspace.package] 读取版本，不被依赖表里的 version 干扰' {
    Assert-Equal '0.3.1' (Get-WorkspaceVersion -CargoToml $cargoToml)
}

Test-Case '版本：tag v0.3.1 与 workspace 版本一致时得到 0.3.1' {
    Assert-Equal '0.3.1' (Resolve-ReleaseVersion -Tag 'v0.3.1' -WorkspaceVersion '0.3.1')
}

Test-Case '版本：没有 tag（手动试跑）时用 workspace 版本' {
    Assert-Equal '0.3.1' (Resolve-ReleaseVersion -Tag '' -WorkspaceVersion '0.3.1')
}

Test-Case '版本：tag 与 workspace 版本不一致时报错' {
    Assert-Throws { Resolve-ReleaseVersion -Tag 'v0.4.0' -WorkspaceVersion '0.3.1' } '0\.4\.0.*0\.3\.1'
}

Test-Case '版本：不是 v 开头的 tag 报错' {
    Assert-Throws { Resolve-ReleaseVersion -Tag '0.3.1' -WorkspaceVersion '0.3.1' } '形如 v'
}

# ---- 便携 zip：程序文件放在根目录，并附带 portable 标记文件（便携模式，见 spec #1） ----

Test-Case '便携 zip：根目录是两个程序文件加 portable 标记，内容原样保留' {
    $stage = Join-Path $script:tempRoot 'stage'
    New-Item -ItemType Directory -Force $stage | Out-Null
    [IO.File]::WriteAllBytes((Join-Path $stage 'BetterPrtSc.exe'), [byte[]](1, 2, 3))
    [IO.File]::WriteAllBytes((Join-Path $stage 'bps.exe'), [byte[]](4, 5))
    $zipPath = Join-Path $script:tempRoot 'portable.zip'

    New-PortableZip -Files (Join-Path $stage 'BetterPrtSc.exe'), (Join-Path $stage 'bps.exe') -Destination $zipPath

    $zip = [IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
        $names = ($zip.Entries | ForEach-Object FullName | Sort-Object) -join ','
        Assert-Equal 'BetterPrtSc.exe,bps.exe,portable' $names
        $reader = [IO.BinaryReader]::new($zip.GetEntry('bps.exe').Open())
        try { Assert-Equal '4,5' ($reader.ReadBytes(16) -join ',') } finally { $reader.Dispose() }
    }
    finally { $zip.Dispose() }
}

# ---- Scoop manifest ----

$hash = '0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF'
$manifestJson = New-ScoopManifest -Version '0.3.1' -Repository 'someone/BetterPrtScRust' -PortableZipHash $hash
$manifest = $manifestJson | ConvertFrom-Json

Test-Case 'Scoop：下载本版本 GitHub Release 上的便携 zip 并校验 SHA256' {
    Assert-Equal 'https://github.com/someone/BetterPrtScRust/releases/download/v0.3.1/BetterPrtSc-0.3.1-x64-portable.zip' $manifest.architecture.'64bit'.url
    Assert-Equal $hash.ToLowerInvariant() $manifest.architecture.'64bit'.hash
    Assert-Equal '0.3.1' $manifest.version
}

Test-Case 'Scoop：bps 进 PATH，开始菜单有 BetterPrtSc 快捷方式' {
    Assert-Equal 'bps.exe' $manifest.bin
    Assert-Equal 'BetterPrtSc.exe|BetterPrtSc' ($manifest.shortcuts[0] -join '|')
}

Test-Case 'Scoop：安装时去掉 portable 标记，配置放在 %APPDATA%，升级不丢' {
    $dir = Join-Path $script:tempRoot 'scoop-app'
    New-Item -ItemType Directory -Force $dir | Out-Null
    New-Item -ItemType File (Join-Path $dir 'portable') | Out-Null
    New-Item -ItemType File (Join-Path $dir 'bps.exe') | Out-Null
    # Scoop 以 Invoke-Expression 执行 pre_install，$dir 是本版本的安装目录。
    Invoke-Expression (@($manifest.pre_install) -join "`n")
    Assert-True (-not (Test-Path (Join-Path $dir 'portable'))) 'portable 标记应被删除'
    Assert-True (Test-Path (Join-Path $dir 'bps.exe')) '其他文件不能动'
}

Test-Case 'Scoop：checkver 跟随 GitHub Releases，autoupdate 按新版本号拼下载地址' {
    Assert-Equal 'https://github.com/someone/BetterPrtScRust' $manifest.checkver.github
    Assert-Equal 'https://github.com/someone/BetterPrtScRust/releases/download/v$version/BetterPrtSc-$version-x64-portable.zip' $manifest.autoupdate.architecture.'64bit'.url
}

# ---- 发布说明：每次发布记录常驻内存与产物体积 ----

Test-Case '发布说明：记录私有工作集（判定值）与参考值、各产物的体积和 SHA256' {
    $memory = New-MemoryReport -PrivateWorkingSet ([long](12.5 * $MiB)) -WorkingSet ([long](21.25 * $MiB)) -PrivateBytes ([long](18 * $MiB)) -TargetMB 30 -Stable $true
    $assets = @(
        [pscustomobject]@{ name = 'BetterPrtSc-0.3.1-x64-setup.exe'; bytes = 2345678; sha256 = 'aa11' }
        [pscustomobject]@{ name = 'BetterPrtSc-0.3.1-x64-portable.zip'; bytes = 1234567; sha256 = 'bb22' }
    )
    $notes = New-ReleaseNotes -Version '0.3.1' -Repository 'someone/BetterPrtScRust' -Assets $assets -Memory $memory

    Assert-True ($notes -match '私有工作集[^\n]*12\.5 MB') "应记录私有工作集 12.5 MB：`n$notes"
    Assert-True ($notes -match '< 30 MB') "应写明目标：`n$notes"
    Assert-True ($notes -match '工作集[^\n]*21\.25 MB') "应记录工作集参考值：`n$notes"
    Assert-True ($notes -match '私有字节[^\n]*18 MB') "应记录私有字节参考值：`n$notes"
    Assert-True ($notes -match 'BetterPrtSc-0\.3\.1-x64-setup\.exe[^\n]*2\.35 MB[^\n]*aa11') "应记录安装包体积与 SHA256：`n$notes"
    Assert-True ($notes -match 'scoop install https://github\.com/someone/BetterPrtScRust/releases/download/v0\.3\.1/betterprtsc\.json') "应给出 Scoop 安装命令：`n$notes"
}

Test-Case '发布说明：私有工作集超出目标时明确标出' {
    $memory = New-MemoryReport -PrivateWorkingSet (31 * $MiB) -WorkingSet (40 * $MiB) -PrivateBytes (35 * $MiB) -TargetMB 30 -Stable $true
    $notes = New-ReleaseNotes -Version '0.3.1' -Repository 'someone/BetterPrtScRust' -Assets @() -Memory $memory
    Assert-True ($notes -match '超出目标') "应标出超出目标：`n$notes"
}

Remove-Item -LiteralPath $script:tempRoot -Recurse -Force
if ($script:failures) {
    "共 $script:failures 个用例失败"
    exit 1
}
'全部通过'
