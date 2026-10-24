<#
.SYNOPSIS
  安装包体积检查：任一文件达到上限（默认 15 MB）即失败，退出码 1。

.DESCRIPTION
  轻量指标“安装包 < 15 MB”。MB 按十进制（1 MB = 1,000,000 字节）计算，比 MiB 口径更严格，
  无论用户在资源管理器还是 GitHub Releases 页面看到的数字都不会超标。
  -Path 支持通配符；某个路径一个文件都没匹配到时同样失败，避免漏检。
    pwsh -NoProfile -File scripts\release\check-size.ps1 -Path dist\*-setup.exe, dist\*-portable.zip
#>
param(
    [Parameter(Mandatory)]
    [string[]]$Path,
    [double]$LimitMB = 15
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Release.psm1') -Force

$limitBytes = [long]($LimitMB * 1000 * 1000)
$failed = $false
$rows = foreach ($p in $Path) {
    $files = @(Get-ChildItem -Path $p -File -ErrorAction SilentlyContinue)
    if (-not $files) {
        $failed = $true
        Write-CiMessage error "没有找到要检查的文件：$p"
        continue
    }
    foreach ($file in $files) {
        $ok = $file.Length -lt $limitBytes
        if (-not $ok) {
            $failed = $true
            Write-CiMessage error ("{0} 为 {1:N2} MB，超过上限 {2} MB" -f $file.Name, ($file.Length / 1e6), $LimitMB)
        }
        [pscustomobject]@{
            文件   = $file.Name
            'MB' = [math]::Round($file.Length / 1e6, 2)
            字节   = $file.Length
            结论   = if ($ok) { '通过' } else { '超标' }
        }
    }
}

$rows | Format-Table -AutoSize | Out-String -Width 200 | Write-Host
Add-StepSummary (@("### 安装包体积（上限 $LimitMB MB）", '', '| 文件 | MB | 结论 |', '|---|---:|---|') +
    @($rows | ForEach-Object { "| $($_.文件) | $($_.MB) | $($_.结论) |" }))

if ($failed) { exit 1 }
exit 0
