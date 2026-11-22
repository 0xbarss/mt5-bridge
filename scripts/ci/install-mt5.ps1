# Install MetaTrader 5 on a Windows runner for MQL5 EA compile verification.
#
# Installs the terminal to "C:\Program Files\MetaTrader 5" and leaves
# metaeditor64.exe available for `metaeditor64.exe /compile`.
#
# Idempotent: if metaeditor64.exe is already present, download and install are
# skipped so repeated runs on the same machine do not reinstall MT5.

$ErrorActionPreference = "Stop"
$InstallDir = "C:\Program Files\MetaTrader 5"
$Installer = "$env:TEMP\mt5setup.exe"
$Mt5Url = "https://download.mql5.com/cdn/web/metaquotes.software.corp/mt5/mt5setup.exe"

# Already installed? Skip everything.
if (Test-Path "$InstallDir\metaeditor64.exe") {
    Write-Host "MetaTrader 5 already installed at $InstallDir — skipping download and install."
    exit 0
}

Write-Host "Installing MetaTrader 5 to $InstallDir ..."

try {
    Invoke-WebRequest -Uri $Mt5Url -OutFile $Installer -TimeoutSec 300
    Write-Host "Downloaded MT5 installer ($Installer)."
} catch {
    Write-Error "Failed to download MT5 installer: $_"
    exit 1
}

# /auto installs silently to the default location.
& "$Installer" /auto

if (-not (Test-Path "$InstallDir\metaeditor64.exe")) {
    Write-Error "MT5 installation did not produce metaeditor64.exe at $InstallDir"
    exit 1
}

Write-Host "MetaTrader 5 installed. metaeditor64.exe found at $InstallDir\metaeditor64.exe"
