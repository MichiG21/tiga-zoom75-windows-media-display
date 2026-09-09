# TIGA Windows Media Display startup supervisor
# Supervisor v3: BLE launch mirrors the proven-good Start-Process test exactly.

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$Logs = Join-Path $Root "logs"
New-Item -ItemType Directory -Force -Path $Logs | Out-Null

$SupervisorLog = Join-Path $Logs "startup_supervisor.log"
$RendererOut   = Join-Path $Logs "renderer.log"
$RendererErr   = Join-Path $Logs "renderer_error.log"
$BleOut        = Join-Path $Logs "ble.log"
$BleErr        = Join-Path $Logs "ble_error.log"

function Write-SupervisorLog {
    param([string]$Message)
    Add-Content -Path $SupervisorLog `
        -Value ("{0:yyyy-MM-dd HH:mm:ss} {1}" -f (Get-Date), $Message) `
        -Encoding UTF8
}

$Renderer = Join-Path $Root "tiga_windows_nowplaying_v2.py"
$Snapshot = Join-Path $Root "tiga_windows_media_snapshot.exe"

$BleWorkDir = Join-Path $Root "windows_ble_probe"
$BleExe = Join-Path $BleWorkDir "target\release\tiga_stream_watch_flow.exe"
$BleStreamRelative = "..\extracted\session_01_bulk_stream.bin"
$StreamAbsolute = Join-Path $Root "extracted\session_01_bulk_stream.bin"

foreach ($required in @($Renderer, $Snapshot, $BleExe, $StreamAbsolute)) {
    if (-not (Test-Path $required)) {
        Write-SupervisorLog "Required file missing: $required"
        throw "Required file missing: $required"
    }
}

$Python = (Get-Command py.exe -ErrorAction Stop).Source

# Prevent duplicate supervisors.
$createdNew = $false
$mutex = New-Object System.Threading.Mutex(
    $true,
    "Global\TigaWindowsMediaDisplay",
    [ref]$createdNew
)

if (-not $createdNew) {
    Write-SupervisorLog "Another supervisor instance is already running; exiting."
    $mutex.Dispose()
    exit 0
}

$rendererProc = $null
$bleProc = $null

function Start-Renderer {
    Write-SupervisorLog "Starting renderer."
    return Start-Process `
        -FilePath $Python `
        -ArgumentList @($Renderer) `
        -WorkingDirectory $Root `
        -WindowStyle Hidden `
        -RedirectStandardOutput $RendererOut `
        -RedirectStandardError $RendererErr `
        -PassThru
}

function Start-BleUploader {
    Write-SupervisorLog "Starting BLE uploader (exact proven-good Start-Process argument layout)."

    # This intentionally mirrors the manual Start-Process test that stayed alive:
    # -ArgumentList '--stream','..\extracted\session_01_bulk_stream.bin',
    #               '--intra-block-ms','0','--confirm-flow-watch'
    $bleArgs = @(
        '--stream',
        $BleStreamRelative,
        '--intra-block-ms',
        '0',
        '--confirm-flow-watch'
    )

    return Start-Process `
        -FilePath $BleExe `
        -ArgumentList $bleArgs `
        -WorkingDirectory $BleWorkDir `
        -WindowStyle Hidden `
        -RedirectStandardOutput $BleOut `
        -RedirectStandardError $BleErr `
        -PassThru
}

try {
    Write-SupervisorLog "TIGA startup supervisor launched."

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
            try {
                Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
            } catch {}
        }
    }

    try { $mutex.ReleaseMutex() } catch {}
    $mutex.Dispose()
}
