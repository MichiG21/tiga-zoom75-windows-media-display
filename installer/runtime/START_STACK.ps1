$taskName = "TIGA Windows Media Display"
if (-not (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue)) {
    throw "Scheduled task not installed. Reinstall TIGA Windows Media Display."
}

# Running this by hand is the "I have power-cycled the keyboard, try again" action
# (issue #4): it clears a halted uploader. Nothing else clears it.
. (Join-Path $PSScriptRoot "tiga_supervisor_common.ps1")
$haltFile = Join-Path $PSScriptRoot "logs\uploader_halted.json"
$stream = Join-Path $PSScriptRoot "app\_internal\extracted\session_01_bulk_stream.bin"
# A stop request left by an interrupted STOP_STACK.ps1 would keep the uploader from starting.
[void](Clear-UploaderStopRequest $stream)
Clear-UploaderHaltWithNotice -HaltFile $haltFile -StreamPath $stream

Start-ScheduledTask -TaskName $taskName
Write-Host "TIGA stack start requested."
