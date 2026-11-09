# Installs the latest Brasa release on Windows (x86_64), silently, with the real installer (not
# just the portable zip): that is what lets the in-app updater self-update later
# (telaio_update::install::windows_is_installed, vendored at vendor/telaio, only offers a silent
# update to a copy that came from this installer).
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

$asset = $release.assets | Where-Object { $_.name -like "Brasa-*-windows-x86_64-setup.exe" } | Select-Object -First 1
if (-not $asset) {
    Write-Error "No Windows installer found in the latest release ($($release.tag_name))."
    exit 1
}

$version = $release.tag_name.TrimStart("v")
Write-Host "Installing Brasa $version..."

$installerPath = Join-Path $env:TEMP "Brasa-$version-setup.exe"
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $installerPath

# The installer is not code-signed (no certificate), and Invoke-WebRequest marks the download
# with the zone-identifier Windows reads for SmartScreen; without clearing it, /VERYSILENT below
# can still block on an interactive "Windows protected your PC" prompt. The macOS equivalent is
# install.sh clearing the quarantine flag on the downloaded .app.
Unblock-File -Path $installerPath

# /VERYSILENT: no installer UI at all, matching install.sh's own unattended feel on macOS.
# PrivilegesRequired=lowest (brasa.iss) means this never prompts for admin rights.
$process = Start-Process -FilePath $installerPath -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -Wait -PassThru
Remove-Item $installerPath

if ($process.ExitCode -ne 0) {
    Write-Error "The installer exited with code $($process.ExitCode)."
    exit 1
}

$exePath = Join-Path $env:LOCALAPPDATA "Brasa\brasa.exe"
Write-Host "Installed Brasa $version"
Write-Host "Launch it from the Start Menu, or: $exePath"
