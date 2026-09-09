$taskName = "TIGA Windows Media Display"
Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue | Select-Object TaskName,State
Write-Host ""
Get-CimInstance Win32_Process | Where-Object {
    $_.CommandLine -match "tiga_startup_supervisor|tiga_windows_nowplaying.exe|tiga_stream_watch_flow"
} | Select-Object ProcessId,Name,CommandLine
