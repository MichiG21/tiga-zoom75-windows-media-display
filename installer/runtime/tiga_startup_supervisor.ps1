# TIGA Windows Media Display packaged startup supervisor. What it shares with the development
# one is in tiga_supervisor_common.ps1: this file holds the paths and how to start each child.
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
# Present while the uploader is halted (issue #4); START_STACK.ps1 removes it.
$HaltFile      = Join-Path $Logs "uploader_halted.json"

. (Join-Path $PSScriptRoot "tiga_supervisor_common.ps1")

Invoke-LogRotation $SupervisorLog

$Renderer = Join-Path $AppRoot "tiga_windows_nowplaying.exe"
$Snapshot = Join-Path $RuntimeRoot "tiga_windows_media_snapshot.exe"
$BleExe   = Join-Path $RuntimeRoot "tiga_stream_watch_flow.exe"
$Stream   = Join-Path $RuntimeRoot "extracted\session_01_bulk_stream.bin"

# The installer stages the stream, but it is recreated from the template if it was deleted.
Initialize-StackStream -Stream $Stream `
    -Template (Join-Path $RuntimeRoot "extracted\session_01_bulk_stream.redgreen-backup.bin") `
    -Required @($Renderer, $Snapshot, $BleExe, $Stream)

function Start-Renderer {
    Write-SupervisorLog "Starting packaged renderer."
    Start-LoggedProcess -Name "renderer" -FilePath $Renderer -Arguments @() -WorkingDirectory $RuntimeRoot -StdOut $RendererOut -StdErr $RendererErr
}

function Start-BleUploader {
    Write-SupervisorLog "Starting packaged BLE uploader."
    Start-LoggedProcess -Name "ble" -FilePath $BleExe -Arguments @('--stream','extracted\session_01_bulk_stream.bin','--intra-block-ms','0','--confirm-flow-watch') -WorkingDirectory $RuntimeRoot -StdOut $BleOut -StdErr $BleErr
}

Invoke-StackSupervisor -LaunchMessage "TIGA packaged startup supervisor launched (pid $PID)." `
    -StartRenderer { Start-Renderer } `
    -StartUploader { Start-BleUploader } `
    -HaltFile $HaltFile `
    -StreamPath $Stream
