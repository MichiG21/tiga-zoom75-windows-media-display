$taskName = "TIGA Windows Media Display"
if (-not (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue)) {
    throw "Scheduled task not installed. Run .\install_tiga_startup.ps1 first."
}
Start-ScheduledTask -TaskName $taskName
Write-Host "TIGA stack start requested. The logon stack normally settles within ~15 seconds."
