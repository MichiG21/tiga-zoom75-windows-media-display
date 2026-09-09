$ErrorActionPreference = "Stop"
$Root = $PSScriptRoot
Set-Location $Root

if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    throw "MSVC cl.exe not found. Open 'Developer PowerShell for VS 2022' and run this script there."
}
if (-not (Get-Command cargo.exe -ErrorAction SilentlyContinue)) {
    throw "Rust cargo.exe not found. Install the Rust toolchain first."
}
if (-not (Get-Command py.exe -ErrorAction SilentlyContinue)) {
    throw "Python launcher py.exe not found."
}

Write-Host "[1/5] Checking Pillow..."
& py -c "import PIL" 2>$null
if ($LASTEXITCODE -ne 0) {
    & py -m pip install -r "$Root\requirements.txt"
    if ($LASTEXITCODE -ne 0) { throw "Pillow installation failed." }
}

Write-Host "[2/5] Building Windows media snapshot helper..."
& cl /nologo /std:c++20 /EHsc "$Root\tiga_windows_media_snapshot.cpp" /Fe:"$Root\tiga_windows_media_snapshot.exe" /link windowsapp.lib
if ($LASTEXITCODE -ne 0) { throw "Snapshot helper build failed." }

Write-Host "[3/5] Building display control utility..."
& cl /nologo /std:c++20 /EHsc /DUNICODE /D_UNICODE "$Root\tiga_display_control.cpp" /Fe:"$Root\tiga_display_control.exe" /link /SUBSYSTEM:WINDOWS user32.lib shell32.lib ole32.lib
if ($LASTEXITCODE -ne 0) { throw "Display control build failed." }

Write-Host "[4/5] Building flow-controlled BLE uploader..."
Push-Location "$Root\windows_ble_probe"
try {
    & cargo build --release --bin tiga_stream_watch_flow
    if ($LASTEXITCODE -ne 0) { throw "BLE uploader build failed." }
} finally { Pop-Location }

Write-Host "[5/5] Installing startup task..."
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$Root\install_tiga_startup.ps1"
if ($LASTEXITCODE -ne 0) { throw "Scheduled-task install failed." }

Write-Host ""
Write-Host "Build/install complete."
Write-Host "Start it with:  .\START_STACK.ps1"
Write-Host "Stop it with:   .\STOP_STACK.ps1"
Write-Host "Status:         .\STATUS_STACK.ps1"
