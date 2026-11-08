# Installs the latest Brasa release on Windows (x86_64), portable (no installer yet).
#
# Run this in a normal Windows PowerShell, not inside WSL: WSL runs a Linux desktop through
# WSLg, which adds GPU virtualization and remote-display overhead a GPU-rendered app like Brasa
# visibly feels (stutter, slow redraws). This script only installs the native Windows build.
#
# Usage (PowerShell):
#   irm https://raw.githubusercontent.com/guialveess/brasa/main/install.ps1 | iex

$ErrorActionPreference = "Stop"

$owner = "guialveess"
$repo = "brasa"

Write-Host "Fetching the latest release..."
$release = Invoke-RestMethod -Uri "https://api.github.com/repos/$owner/$repo/releases/latest" `
    -Headers @{ "User-Agent" = "brasa-install-script" }

$asset = $release.assets | Where-Object { $_.name -like "Brasa-*-windows-x86_64-portable.zip" } | Select-Object -First 1
if (-not $asset) {
    Write-Error "No Windows build found in the latest release ($($release.tag_name))."
    exit 1
}

$version = $release.tag_name.TrimStart("v")
Write-Host "Installing Brasa $version..."

$installDir = Join-Path $env:LOCALAPPDATA "Brasa"
New-Item -ItemType Directory -Force -Path $installDir | Out-Null

$zipPath = Join-Path $env:TEMP "Brasa-$version.zip"
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zipPath

Expand-Archive -Force -Path $zipPath -DestinationPath $installDir
Remove-Item $zipPath

$exePath = Join-Path $installDir "brasa.exe"

# Start Menu shortcut, so Brasa shows up without the user having to find $env:LOCALAPPDATA.
$startMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut((Join-Path $startMenu "Brasa.lnk"))
$shortcut.TargetPath = $exePath
$shortcut.WorkingDirectory = $installDir
$shortcut.Save()

Write-Host "Installed Brasa $version to $installDir"
Write-Host "Launch it from the Start Menu, or: $exePath"
