# TIGA Windows Media Display startup supervisor. What it shares with the packaged one is in
# tiga_supervisor_common.ps1: this file holds the paths and how to start each child.

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

# The active stream isn't tracked by Git, so a fresh checkout seeds it (issue #10).
Initialize-StackStream -Stream $StreamAbsolute `
    -Template (Join-Path $Root "extracted\session_01_bulk_stream.redgreen-backup.bin") `
    -Required @($Renderer, $Snapshot, $BleExe, $StreamAbsolute)

$Python = (Get-Command py.exe -ErrorAction Stop).Source

function Start-Renderer {
    Write-SupervisorLog "Starting renderer."
    return Start-LoggedProcess `
        -Name "renderer" `
        -FilePath $Python `
        -Arguments (@("-u", $Renderer) + @(Get-RendererFrameArgs)) `
        -WorkingDirectory $Root `
        -StdOut $RendererOut `
        -StdErr $RendererErr
}

function Start-BleUploader {
    Write-SupervisorLog "Starting BLE uploader."

    $bleArgs = @(
        '--stream',
        $BleStreamRelative,
        '--intra-block-ms',
        '0',
        '--confirm-flow-watch'
    ) + @(Get-UploaderFrameArgs)

    return Start-LoggedProcess `
        -Name "ble" `
        -FilePath $BleExe `
        -Arguments $bleArgs `
        -WorkingDirectory $BleWorkDir `
        -StdOut $BleOut `
        -StdErr $BleErr
}

Invoke-StackSupervisor -LaunchMessage "TIGA startup supervisor launched (pid $PID)." `
    -StartRenderer { Start-Renderer } `
    -StartUploader { Start-BleUploader } `
    -HaltFile $HaltFile `
    -StreamPath $StreamAbsolute
