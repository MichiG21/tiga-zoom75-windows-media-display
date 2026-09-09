# TIGA Windows Media Display packaged startup supervisor
$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$AppRoot = Join-Path $Root "app"
$RuntimeRoot = Join-Path $AppRoot "_internal"
$Logs = Join-Path $Root "logs"
New-Item -ItemType Directory -Force -Path $Logs | Out-Null

$SupervisorLog = Join-Path $Logs "startup_supervisor.log"
$RendererOut   = Join-Path $Logs "renderer.log"
$RendererErr   = Join-Path $Logs "renderer_error.log"
$BleOut        = Join-Path $Logs "ble.log"
$BleErr        = Join-Path $Logs "ble_error.log"

function Write-SupervisorLog {
    param([string]$Message)
    Add-Content -Path $SupervisorLog -Value ("{0:yyyy-MM-dd HH:mm:ss} {1}" -f (Get-Date), $Message) -Encoding UTF8
}

$Renderer = Join-Path $AppRoot "tiga_windows_nowplaying.exe"
$Snapshot = Join-Path $RuntimeRoot "tiga_windows_media_snapshot.exe"
$BleExe   = Join-Path $RuntimeRoot "tiga_stream_watch_flow.exe"
$Stream   = Join-Path $RuntimeRoot "extracted\session_01_bulk_stream.bin"

foreach ($required in @($Renderer, $Snapshot, $BleExe, $Stream)) {
    if (-not (Test-Path $required)) {
        Write-SupervisorLog "Required file missing: $required"
        throw "Required file missing: $required"
    }
}

$createdNew = $false
$mutex = New-Object System.Threading.Mutex($true, "Global\TigaWindowsMediaDisplay", [ref]$createdNew)
if (-not $createdNew) {
    Write-SupervisorLog "Another supervisor instance is already running; exiting."
    $mutex.Dispose()
    exit 0
}

$rendererProc = $null
$bleProc = $null

function Start-Renderer {
    Write-SupervisorLog "Starting packaged renderer."
    Start-Process -FilePath $Renderer -WorkingDirectory $RuntimeRoot -WindowStyle Hidden -RedirectStandardOutput $RendererOut -RedirectStandardError $RendererErr -PassThru
}

function Start-BleUploader {
    Write-SupervisorLog "Starting packaged BLE uploader."
    Start-Process -FilePath $BleExe -ArgumentList @('--stream','extracted\session_01_bulk_stream.bin','--intra-block-ms','0','--confirm-flow-watch') -WorkingDirectory $RuntimeRoot -WindowStyle Hidden -RedirectStandardOutput $BleOut -RedirectStandardError $BleErr -PassThru
}

try {
    Write-SupervisorLog "TIGA packaged startup supervisor launched."
    while ($true) {
        if (($null -eq $rendererProc) -or $rendererProc.HasExited) {
            if ($null -ne $rendererProc) {
                Write-SupervisorLog "Renderer exited with code $($rendererProc.ExitCode); restarting in 8s."
                Start-Sleep -Seconds 8
            }
            $rendererProc = Start-Renderer
        }
        if (($null -eq $bleProc) -or $bleProc.HasExited) {
            if ($null -ne $bleProc) {
                Write-SupervisorLog "BLE uploader exited with code $($bleProc.ExitCode); restarting in 8s."
                Start-Sleep -Seconds 8
            }
            $bleProc = Start-BleUploader
        }
        Start-Sleep -Seconds 2
    }
}
finally {
    Write-SupervisorLog "Supervisor stopping; cleaning up children."
    foreach ($proc in @($rendererProc, $bleProc)) {
        if (($null -ne $proc) -and (-not $proc.HasExited)) {
            try { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue } catch {}
        }
    }
    try { $mutex.ReleaseMutex() } catch {}
    $mutex.Dispose()
}
