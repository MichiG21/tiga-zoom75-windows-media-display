$taskName = "TIGA Windows Media Display"
if (-not (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue)) {
    throw "Scheduled task not installed. Run .\install_tiga_startup.ps1 first."
}

# Running this by hand is the "I have power-cycled the keyboard, try again" action
# (issue #4): it clears a halted uploader. Nothing else clears it.
. (Join-Path $PSScriptRoot "tiga_supervisor_common.ps1")
$haltFile = Join-Path $PSScriptRoot "logs\uploader_halted.json"
$stream = Join-Path $PSScriptRoot "extracted\session_01_bulk_stream.bin"
# A stop request left by an interrupted STOP_STACK.ps1 would keep the uploader from starting.
[void](Clear-UploaderStopRequest $stream)
Clear-UploaderHaltWithNotice -HaltFile $haltFile -StreamPath $stream

Start-ScheduledTask -TaskName $taskName
Write-Host "TIGA stack start requested. The logon stack normally settles within ~15 seconds."
