$taskName = "TIGA Windows Media Display"
if (-not (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue)) {
    throw "Scheduled task not installed. Run .\install_tiga_startup.ps1 first."
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
$cleared = Clear-UploaderHalt -HaltFile $haltFile -StreamPath (Join-Path $PSScriptRoot "extracted\session_01_bulk_stream.bin")
if ($cleared) {
    Write-Host "Cleared the halt, assuming the keyboard has been power-cycled:"
    $cleared | ForEach-Object { Write-Host "  removed $_" }
    Write-Host "If it was NOT power-cycled, run STOP_STACK.ps1 now, power-cycle it, and start again."
    Write-Host "Use the keyboard's power switch. Unplugging USB does NOT power-cycle it (it has a battery);"
    Write-Host "if the display stays stuck, disconnect the battery."
}

Start-ScheduledTask -TaskName $taskName
Write-Host "TIGA stack start requested. The logon stack normally settles within ~15 seconds."
