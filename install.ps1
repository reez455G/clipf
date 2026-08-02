# param() must be the first executable statement in a PowerShell script, so
# preferences are set after it.
param(
    [string]$Version = 'latest',
    [string]$BinDir = "$env:LOCALAPPDATA\Programs\clipf"
)

$ErrorActionPreference = 'Stop'

$Repo = "reez455G/clipf"
$Asset = "clipf-x86_64-pc-windows-msvc.zip"

Write-Host "install.ps1: preparing to install $Asset ($Version)..."

if ($Version -eq 'latest') {
    $BaseUrl = "https://github.com/$Repo/releases/latest/download"
} else {
    $BaseUrl = "https://github.com/$Repo/releases/download/$Version"
}

$tmp = Join-Path $env:TEMP ("clipf-install-" + $PID)

try {
    if (Test-Path $tmp) { Remove-Item -Recurse -Force -Path $tmp }
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null

    $zipPath = Join-Path $tmp $Asset
    $shaPath = Join-Path $tmp "SHA256SUMS"

    Write-Host "install.ps1: downloading..."
    Invoke-WebRequest -UseBasicParsing -Uri "$BaseUrl/$Asset" -OutFile $zipPath
    Invoke-WebRequest -UseBasicParsing -Uri "$BaseUrl/SHA256SUMS" -OutFile $shaPath

    Write-Host "install.ps1: verifying checksum..."
    $fileHash = (Get-FileHash -Algorithm SHA256 -Path $zipPath).Hash
    $sums = Get-Content $shaPath
    $found = $false
    foreach ($line in $sums) {
        $parts = $line.Trim() -split '\s+'
        if ($parts.Count -ge 2 -and $parts[1] -eq $Asset) {
            if ($parts[0] -eq $fileHash) {
                $found = $true
                break
            } else {
                throw "clipf: checksum mismatch for $Asset (expected $($parts[0]), got $fileHash)"
            }
        }
    }
    if (-not $found) {
        throw "clipf: checksum not found for $Asset in SHA256SUMS"
    }

    Write-Host "install.ps1: extracting..."
    $extractPath = Join-Path $tmp "extract"
    Expand-Archive -Path $zipPath -DestinationPath $extractPath -Force

    $exe = Get-ChildItem -Recurse -Filter "clipf.exe" -Path $extractPath | Select-Object -First 1
    if (-not $exe) { throw "clipf: could not find clipf.exe in archive" }

    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    Copy-Item -Path $exe.FullName -Destination (Join-Path $BinDir "clipf.exe") -Force

    Write-Host "install.ps1: updating PATH..."
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $paths = if ($userPath) { $userPath -split ';' } else { @() }
    $exists = $false
    $binDirNormalized = $BinDir.TrimEnd('\')
    foreach ($p in $paths) {
        if ($p.TrimEnd('\') -eq $binDirNormalized) {
            $exists = $true
            break
        }
    }

    if (-not $exists) {
        $newUserPath = if ($userPath) { "$userPath;$BinDir" } else { $BinDir }
        [Environment]::SetEnvironmentVariable('Path', $newUserPath, 'User')
        $env:Path = "$env:Path;$BinDir"
        Write-Host "install.ps1: added $BinDir to PATH in the User environment - restart your terminal to apply"
    }

    Write-Host "install.ps1: verification:"
    & (Join-Path $BinDir "clipf.exe") --version
    Write-Host "install.ps1: installed. Run 'clipf --check' to verify this environment."

} finally {
    Remove-Item -Recurse -Force -Path $tmp -ErrorAction SilentlyContinue
}
