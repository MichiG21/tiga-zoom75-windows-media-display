$taskName = "TIGA Windows Media Display"
Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
Get-CimInstance Win32_Process | Where-Object {
    $_.CommandLine -match "tiga_startup_supervisor|tiga_windows_nowplaying_v2.py|tiga_stream_watch_flow"
} | ForEach-Object {
    Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
}
Write-Host "TIGA stack stopped."
