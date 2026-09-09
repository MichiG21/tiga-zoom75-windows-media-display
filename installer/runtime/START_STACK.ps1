$taskName = "TIGA Windows Media Display"
if (-not (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue)) {
    throw "Scheduled task not installed. Reinstall TIGA Windows Media Display."
}
Start-ScheduledTask -TaskName $taskName
Write-Host "TIGA stack start requested."
