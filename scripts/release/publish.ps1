<#
.SYNOPSIS
  把 package.ps1 打出的产物发布到 GitHub Releases，说明里记录产物体积与常驻内存。

.DESCRIPTION
  需要先运行 package.ps1（产物）与 measure-memory.ps1 -OutFile <DistDir>\memory.json（常驻内存）。
  用 gh release create 创建 tag 对应的 Release 并上传便携 zip、安装包、Scoop manifest、SHA256SUMS.txt；
  版本号带预发布后缀（如 0.2.0-beta.1）时标为预发布。tag 必须已推送到 GitHub（--verify-tag）。
  -DryRun 只生成 release-notes.md 并打印将要执行的命令，不联网。
    pwsh -NoProfile -File scripts\release\publish.ps1 -Tag v0.1.0 [-DryRun]
#>
param(
    [Parameter(Mandatory)]
    [string]$Tag,
    [string]$DistDir = (Join-Path $PSScriptRoot '..\..\dist'),
    [string]$Repository = $(if ($env:GITHUB_REPOSITORY) { $env:GITHUB_REPOSITORY } else { 'djp440/BetterPrtScRust' }),
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Release.psm1') -Force

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$version = Resolve-ReleaseVersion -Tag $Tag -WorkspaceVersion (Get-WorkspaceVersion -CargoToml (Get-Content -Raw (Join-Path $repoRoot 'Cargo.toml')))
$DistDir = (Resolve-Path -LiteralPath $DistDir).Path

$files = foreach ($kind in 'Setup', 'Portable', 'Scoop', 'Checksums') {
    $path = Join-Path $DistDir (Get-ReleaseAssetName -Kind $kind -Version $version)
    if (-not (Test-Path -LiteralPath $path)) { throw "缺少发布产物 $path，先运行 package.ps1" }
    $path
}
$memoryPath = Join-Path $DistDir 'memory.json'
if (-not (Test-Path -LiteralPath $memoryPath)) { throw "缺少常驻内存记录 $memoryPath，先运行 measure-memory.ps1 -OutFile $memoryPath" }
$memory = Get-Content -LiteralPath $memoryPath -Raw -Encoding UTF8 | ConvertFrom-Json

$packages = foreach ($path in $files[0..1]) {
    $item = Get-Item -LiteralPath $path
    [pscustomobject]@{
        name   = $item.Name
        bytes  = $item.Length
        sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
$notesPath = Join-Path $DistDir 'release-notes.md'
$notes = New-ReleaseNotes -Version $version -Repository $Repository -Assets $packages -Memory $memory
[IO.File]::WriteAllText($notesPath, $notes + "`n", [Text.UTF8Encoding]::new($false))

$ghArgs = @('release', 'create', $Tag) + $files + @(
    '--repo', $Repository
    '--title', "BetterPrtSc $version"
    '--notes-file', $notesPath
    '--verify-tag'
)
if ($version -match '-') { $ghArgs += '--prerelease' }

if ($DryRun) {
    Write-Host "--- $notesPath ---"
    Write-Host $notes
    Write-Host '--- 将要执行 ---'
    Write-Host ('gh ' + ($ghArgs -join ' '))
    exit 0
}
gh @ghArgs
if ($LASTEXITCODE) { throw "gh release create 失败（退出码 $LASTEXITCODE）" }
