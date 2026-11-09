# Installs the latest Brasa release on Windows (x86_64): the real installer when it runs cleanly
# (that is what lets the in-app updater self-update later,
# telaio_update::install::windows_is_installed, vendored at vendor/telaio), falling back to the
# portable zip when it does not.
#
# Neither brasa.exe nor the installer is code-signed (no certificate), so an endpoint protection
# product can flag the fresh, unsigned installer as unwanted software and refuse to run it —
# `Start-Process` throws rather than the installer itself failing. That is a false positive, not
# a real detection, but there is no overriding it from here: the fallback below gets Brasa running
# either way, just without the self-update ability the installer gives (a later
# `irm .../install.ps1 | iex` re-run can pick it up once that block clears, which can take a few
# days as Microsoft's own reputation systems see more people run the same file).
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

$version = $release.tag_name.TrimStart("v")
$installDir = Join-Path $env:LOCALAPPDATA "Brasa"

function Install-Portable {
    $asset = $release.assets | Where-Object { $_.name -like "Brasa-*-windows-x86_64-portable.zip" } | Select-Object -First 1
    if (-not $asset) {
        Write-Error "No Windows build found in the latest release ($($release.tag_name))."
        exit 1
    }
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    $zipPath = Join-Path $env:TEMP "Brasa-$version.zip"
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zipPath
    Expand-Archive -Force -Path $zipPath -DestinationPath $installDir
    Remove-Item $zipPath

    $startMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut((Join-Path $startMenu "Brasa.lnk"))
    $shortcut.TargetPath = Join-Path $installDir "brasa.exe"
    $shortcut.WorkingDirectory = $installDir
    $shortcut.Save()
}

$setupAsset = $release.assets | Where-Object { $_.name -like "Brasa-*-windows-x86_64-setup.exe" } | Select-Object -First 1
$installedViaSetup = $false

if ($setupAsset) {
    Write-Host "Installing Brasa $version..."
    $installerPath = Join-Path $env:TEMP "Brasa-$version-setup.exe"
    Invoke-WebRequest -Uri $setupAsset.browser_download_url -OutFile $installerPath

    # Clears the zone-identifier Invoke-WebRequest sets, which otherwise can turn /VERYSILENT
    # below into an interactive "Windows protected your PC" SmartScreen prompt. The macOS
    # equivalent is install.sh clearing the quarantine flag on the downloaded .app. This is a
    # different, narrower thing than the endpoint-protection block the doc comment above is
    # about: this only ever suppresses a *prompt*, never a hard AV block.
    Unblock-File -Path $installerPath

    try {
        # PrivilegesRequired=lowest (brasa.iss) means this never prompts for admin rights.
        $process = Start-Process -FilePath $installerPath `
            -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -Wait -PassThru
        if ($process.ExitCode -ne 0) {
            throw "the installer exited with code $($process.ExitCode)"
        }
        $installedViaSetup = $true
    } catch {
        Write-Warning "The installer could not run ($($_.Exception.Message)); installing the portable build instead."
        Write-Warning "Brasa will still work, but won't be able to update itself until a later install.ps1 run gets the installer through."
    } finally {
        Remove-Item -ErrorAction SilentlyContinue $installerPath
    }
}

if (-not $installedViaSetup) {
    Install-Portable
}

$exePath = Join-Path $installDir "brasa.exe"
Write-Host "Installed Brasa $version"
Write-Host "Launch it from the Start Menu, or: $exePath"
