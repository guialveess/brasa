# Installs the latest Brasa release on Windows (x86_64), portable (just the exe, next to the
# license files, in %LOCALAPPDATA%\Brasa plus a Start Menu shortcut).
#
# A real installer also exists (packaging/windows/brasa.iss, built with Inno Setup) and is what
# lets the in-app updater self-update later (telaio_update::install::windows_is_installed,
# vendored at vendor/telaio); it is not what this script uses, though, because downloading and
# running it got flagged outright as a trojan and quarantined by an endpoint protection product
# on a locked-down machine (#241) — a false positive (neither brasa.exe nor the installer is
# code-signed, and self-extracting installers from a repository with no history are exactly the
# shape that heuristic gets triggered by), but one with no override from inside this script.
# The portable exe alone has not drawn the same reaction. Once the installer has either a code
# signature or enough of a track record that Microsoft stops flagging it, this script can go back
# to using it; until then, re-running this one-liner is how you get a newer version here.
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
