$taskName = "TIGA Windows Media Display"
if (-not (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue)) {
    throw "Scheduled task not installed. Reinstall TIGA Windows Media Display."
}

# Running this by hand is the "I have power-cycled the keyboard, try again" action
# (issue #4): it clears a halted uploader. Nothing else clears it.
. (Join-Path $PSScriptRoot "tiga_supervisor_common.ps1")
$haltFile = Join-Path $PSScriptRoot "logs\uploader_halted.json"
if (Test-Path -LiteralPath $haltFile) {
    try {
        $halt = Get-Content -Raw -LiteralPath $haltFile | ConvertFrom-Json
        Write-Host "The BLE uploader was halted at $($halt.halted_at): $($halt.reason)"
    } catch {}
}
$stream = Join-Path $PSScriptRoot "app\_internal\extracted\session_01_bulk_stream.bin"
# A stop request left by an interrupted STOP_STACK.ps1 would keep the uploader from starting.
[void](Clear-UploaderStopRequest $stream)
$cleared = Clear-UploaderHalt -HaltFile $haltFile -StreamPath $stream
if ($cleared) {
    Write-Host "Cleared the halt, assuming the keyboard's battery has been disconnected and reconnected:"
    $cleared | ForEach-Object { Write-Host "  removed $_" }
    Write-Host "If it was NOT, run STOP_STACK.ps1 now. Then:"
    Write-Host "Disconnect the keyboard's battery for at least 30 s (there is no power switch; unplugging USB doesn't count), then run START_STACK.ps1."
}

Start-ScheduledTask -TaskName $taskName
Write-Host "TIGA stack start requested."
