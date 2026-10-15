# TIGA Windows Media Display startup supervisor
# Supervisor v4: child output is appended (never truncated), every child start is
# stamped with a run ID, and logs rotate by size. See issue #9.

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$Logs = Join-Path $Root "logs"
New-Item -ItemType Directory -Force -Path $Logs | Out-Null

$SupervisorLog = Join-Path $Logs "startup_supervisor.log"
$RendererOut   = Join-Path $Logs "renderer.log"
$RendererErr   = Join-Path $Logs "renderer_error.log"
$BleOut        = Join-Path $Logs "ble.log"
$BleErr        = Join-Path $Logs "ble_error.log"
# Present while the uploader is halted (issue #4); START_STACK.ps1 removes it.
$HaltFile      = Join-Path $Logs "uploader_halted.json"

. (Join-Path $PSScriptRoot "tiga_supervisor_common.ps1")

Invoke-LogRotation $SupervisorLog

$Renderer = Join-Path $Root "tiga_windows_nowplaying_v2.py"
$Snapshot = Join-Path $Root "tiga_windows_media_snapshot.exe"

$BleWorkDir = Join-Path $Root "windows_ble_probe"
$BleExe = Join-Path $BleWorkDir "target\release\tiga_stream_watch_flow.exe"
$BleStreamRelative = "..\extracted\session_01_bulk_stream.bin"
$StreamAbsolute = Join-Path $Root "extracted\session_01_bulk_stream.bin"

# The active stream is runtime output and not tracked by Git; seed it from the
# committed template so a fresh checkout starts (issue #10).
$StreamTemplate = Join-Path $Root "extracted\session_01_bulk_stream.redgreen-backup.bin"
if ((-not (Test-Path $StreamAbsolute)) -and (Test-Path $StreamTemplate)) {
    Copy-Item -LiteralPath $StreamTemplate -Destination $StreamAbsolute
    Write-SupervisorLog "Seeded $StreamAbsolute from $StreamTemplate."
}

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

$state = New-SupervisorState

function Start-Renderer {
    Write-SupervisorLog "Starting renderer."
    return Start-LoggedProcess `
        -Name "renderer" `
        -FilePath $Python `
        -Arguments @("-u", $Renderer) `
        -WorkingDirectory $Root `
        -StdOut $RendererOut `
        -StdErr $RendererErr
}

function Start-BleUploader {
    Write-SupervisorLog "Starting BLE uploader."

    # Same arguments as the manual Start-Process test that stayed alive:
    # --stream ..\extracted\session_01_bulk_stream.bin --intra-block-ms 0 --confirm-flow-watch
    $bleArgs = @(
        '--stream',
        $BleStreamRelative,
        '--intra-block-ms',
        '0',
        '--confirm-flow-watch'
    )

    return Start-LoggedProcess `
        -Name "ble" `
        -FilePath $BleExe `
        -Arguments $bleArgs `
        -WorkingDirectory $BleWorkDir `
        -StdOut $BleOut `
        -StdErr $BleErr
}

try {
    Write-SupervisorLog "TIGA startup supervisor launched (pid $PID)."
    Invoke-SupervisorLoop -State $state `
        -StartRenderer { Start-Renderer } `
        -StartUploader { Start-BleUploader } `
        -HaltFile $HaltFile `
        -StreamPath $StreamAbsolute
}
finally {
    Write-SupervisorLog "Supervisor stopping; cleaning up children."

    foreach ($proc in @($state.Renderer, $state.Uploader)) {
        try { Stop-LoggedProcess $proc } catch {}
    }

    try { $mutex.ReleaseMutex() } catch {}
    $mutex.Dispose()
}
