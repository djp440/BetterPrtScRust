<#
.SYNOPSIS
  打包发布产物：release 构建 → 便携 zip → Inno Setup 安装包 → Scoop manifest → SHA256SUMS.txt。

.DESCRIPTION
  产物写到 OutDir（默认仓库根目录的 dist），文件名见 Release.psm1 的 Get-ReleaseAssetName：
    BetterPrtSc-<版本>-x64-portable.zip  BetterPrtSc.exe、bps.exe 与 portable 标记文件
    BetterPrtSc-<版本>-x64-setup.exe     Inno Setup 安装包（按用户安装，不弹 UAC）
    betterprtsc.json                     Scoop manifest（指向本版本 Release 上的便携 zip）
    SHA256SUMS.txt
  -Tag 给出时必须是 v<Cargo.toml 的 workspace 版本>；不给时直接用 workspace 版本（手动试跑）。
  找不到 Inno Setup 编译器（ISCC.exe，可用环境变量 ISCC 指定）时失败；本机没装可加 -SkipInstaller。
    pwsh -NoProfile -File scripts\release\package.ps1 [-Tag v0.1.0] [-SkipBuild] [-SkipInstaller]
  之后依次运行 check-size.ps1、measure-memory.ps1，发布用 publish.ps1。
#>
param(
    [AllowEmptyString()]
    [string]$Tag = '',
    [string]$OutDir = (Join-Path $PSScriptRoot '..\..\dist'),
    [string]$Repository = $(if ($env:GITHUB_REPOSITORY) { $env:GITHUB_REPOSITORY } else { 'rt-game-440/BetterPrtScRust' }),
    [switch]$SkipBuild,
    [switch]$SkipInstaller
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Release.psm1') -Force

function Find-Iscc {
    $candidates = @(
        $env:ISCC
        (Get-Command ISCC.exe -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source)
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe')
        (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
    )
    $candidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$version = Resolve-ReleaseVersion -Tag $Tag -WorkspaceVersion (Get-WorkspaceVersion -CargoToml (Get-Content -Raw (Join-Path $repoRoot 'Cargo.toml')))
Write-Host "版本 $version"

$iscc = $null
if (-not $SkipInstaller) {
    $iscc = Find-Iscc
    if (-not $iscc) { throw '找不到 Inno Setup 编译器 ISCC.exe（可用环境变量 ISCC 指定路径，或加 -SkipInstaller 跳过安装包）' }
}

if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
        cargo build --release --locked -p bps-app
        if ($LASTEXITCODE) { throw "cargo build 失败（退出码 $LASTEXITCODE）" }
    }
    finally { Pop-Location }
}
$binDir = Join-Path $repoRoot 'target\release'
$programFiles = foreach ($name in 'BetterPrtSc.exe', 'bps.exe') {
    $path = Join-Path $binDir $name
    if (-not (Test-Path -LiteralPath $path)) { throw "找不到 $path，先构建或去掉 -SkipBuild" }
    $path
}
$license = Get-ChildItem -LiteralPath $repoRoot -File | Where-Object Name -in 'LICENSE', 'LICENSE.txt', 'COPYING' | Select-Object -First 1
if (-not $license) { throw "仓库根目录没有 LICENSE 文件（GPL-3.0 要求随产物分发许可证文本）：$repoRoot" }

$OutDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutDir)
New-Item -ItemType Directory -Force $OutDir | Out-Null
# 清掉上一次打包的产物（任何版本），避免体积检查和上传时混进旧文件。
Get-ChildItem -LiteralPath $OutDir -File |
    Where-Object { $_.Name -like 'BetterPrtSc-*-x64-*' -or $_.Name -in 'betterprtsc.json', 'SHA256SUMS.txt', 'memory.json', 'release-notes.md' } |
    Remove-Item -Force

$assets = @()

$zipPath = Join-Path $OutDir (Get-ReleaseAssetName -Kind Portable -Version $version)
New-PortableZip -Files (@($programFiles) + @($license | ForEach-Object FullName)) -Destination $zipPath
$assets += $zipPath

if ($iscc) {
    $setupName = Get-ReleaseAssetName -Kind Setup -Version $version
    $isccArgs = @(
        '/Qp'
        "/DAppVersion=$version"
        "/DBinDir=$binDir"
        "/O$OutDir"
        "/F$([IO.Path]::GetFileNameWithoutExtension($setupName))"
    )
    if ($license) { $isccArgs += "/DLicenseFile=$($license.FullName)" }
    & $iscc @isccArgs (Join-Path $PSScriptRoot 'BetterPrtSc.iss')
    if ($LASTEXITCODE) { throw "Inno Setup 编译失败（退出码 $LASTEXITCODE）" }
    $assets += Join-Path $OutDir $setupName
}
else {
    Write-CiMessage warning '已跳过 Inno Setup 安装包（-SkipInstaller）'
}

$zipHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
$manifestPath = Join-Path $OutDir (Get-ReleaseAssetName -Kind Scoop -Version $version)
$utf8 = [Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText($manifestPath, (New-ScoopManifest -Version $version -Repository $Repository -PortableZipHash $zipHash), $utf8)
$assets += $manifestPath

$sums = foreach ($asset in $assets) {
    '{0}  {1}' -f (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant(), (Split-Path -Leaf $asset)
}
[IO.File]::WriteAllText((Join-Path $OutDir (Get-ReleaseAssetName -Kind Checksums -Version $version)), (($sums -join "`n") + "`n"), $utf8)

Write-Host "产物（$OutDir）："
Get-ChildItem -LiteralPath $OutDir -File | Format-Table Name, Length -AutoSize | Out-String | Write-Host
