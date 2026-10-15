# TIGA Windows Media Display packaged startup supervisor
# Child output is appended (never truncated), every child start is stamped with a run ID,
# and logs rotate by size. See issue #9.
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

# Size-based rotation: <log> -> <log>.1 -> ... -> <log>.$LogKeep, oldest dropped.
$LogMaxBytes = 5MB
$LogKeep     = 3
$Utf8NoBom   = New-Object System.Text.UTF8Encoding $false

function Add-LogLine {
    param([string]$Path, [string]$Line)
    [System.IO.File]::AppendAllText($Path, $Line + "`r`n", $Utf8NoBom)
}

function Write-SupervisorLog {
    param([string]$Message)
    Add-LogLine $SupervisorLog ("{0:yyyy-MM-dd HH:mm:ss} {1}" -f (Get-Date), $Message)
}

function Invoke-LogRotation {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return }
    if ((Get-Item -LiteralPath $Path).Length -lt $LogMaxBytes) { return }
    for ($i = $LogKeep - 1; $i -ge 1; $i--) {
        if (Test-Path -LiteralPath "$Path.$i") {
            Move-Item -LiteralPath "$Path.$i" -Destination "$Path.$($i + 1)" -Force
        }
    }
    Move-Item -LiteralPath $Path -Destination "$Path.1" -Force
}

function New-RunId {
    param([string]$Name)
    "{0}-{1:yyyyMMdd-HHmmss}-{2}" -f $Name, (Get-Date), [guid]::NewGuid().ToString("N").Substring(0, 6)
}

function Get-LoggedCommandLine {
    # Every element is quoted, so paths containing spaces survive (issue #8).
    param([string]$FilePath, [string[]]$Arguments, [string]$StdOut, [string]$StdErr)
    $parts = @($FilePath) + @($Arguments) | ForEach-Object { '"' + $_ + '"' }
    return ($parts -join ' ') + ' >> "' + $StdOut + '" 2>> "' + $StdErr + '"'
}

function Start-LoggedProcess {
    # Start-Process -RedirectStandardOutput truncates its target, which destroyed the
    # output of every failed run on restart. cmd.exe '>>' appends instead.
    param(
        [string]$Name,
        [string]$FilePath,
        [string[]]$Arguments,
        [string]$WorkingDirectory,
        [string]$StdOut,
        [string]$StdErr
    )

    $runId = New-RunId $Name
    $stamp = "===== {0:yyyy-MM-dd HH:mm:ss zzz} {1} run={2} start =====" -f (Get-Date), $Name, $runId
    foreach ($log in @($StdOut, $StdErr)) {
        Invoke-LogRotation $log
        Add-LogLine $log $stamp
    }

    $cmdLine = Get-LoggedCommandLine $FilePath $Arguments $StdOut $StdErr

    # Inherited by the child: the renderer and uploader both include it in their logs.
    $env:TIGA_RUN_ID = $runId
    $env:PYTHONUNBUFFERED = "1"

    $proc = Start-Process `
        -FilePath $env:ComSpec `
        -ArgumentList "/d /s /c `"$cmdLine`"" `
        -WorkingDirectory $WorkingDirectory `
        -WindowStyle Hidden `
        -PassThru

    # Touch the handle now; otherwise ExitCode is not available after the process exits.
    $null = $proc.Handle

    Write-SupervisorLog "Started $Name run=$runId pid=$($proc.Id)"
    return $proc
}

function Stop-LoggedProcess {
    param($Proc)
    if (($null -ne $Proc) -and (-not $Proc.HasExited)) {
        # /T: the tracked process is the cmd.exe wrapper; take its child with it.
        & taskkill.exe /PID $Proc.Id /T /F 2>&1 | Out-Null
    }
}

Invoke-LogRotation $SupervisorLog

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
    Start-LoggedProcess -Name "renderer" -FilePath $Renderer -Arguments @() -WorkingDirectory $RuntimeRoot -StdOut $RendererOut -StdErr $RendererErr
}

function Start-BleUploader {
    Write-SupervisorLog "Starting packaged BLE uploader."
    Start-LoggedProcess -Name "ble" -FilePath $BleExe -Arguments @('--stream','extracted\session_01_bulk_stream.bin','--intra-block-ms','0','--confirm-flow-watch') -WorkingDirectory $RuntimeRoot -StdOut $BleOut -StdErr $BleErr
}

try {
    Write-SupervisorLog "TIGA packaged startup supervisor launched (pid $PID)."
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
        try { Stop-LoggedProcess $proc } catch {}
    }
    try { $mutex.ReleaseMutex() } catch {}
    $mutex.Dispose()
}
