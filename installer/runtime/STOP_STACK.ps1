$taskName = "TIGA Windows Media Display"

# Ask the BLE uploader to finish any upload, disconnect and exit, then fall back to the
# old force-kill (docs/design/CLEAN_STOP.md).
$Logs = Join-Path $PSScriptRoot "logs"
New-Item -ItemType Directory -Force -Path $Logs | Out-Null
$SupervisorLog = Join-Path $Logs "startup_supervisor.log"
. (Join-Path $PSScriptRoot "tiga_supervisor_common.ps1")

[void](Invoke-StackStop `
    -StreamPath (Join-Path $PSScriptRoot "app\_internal\extracted\session_01_bulk_stream.bin") `
    -GetUploaderProcessIds {
        Get-CimInstance Win32_Process -Filter "Name = 'tiga_stream_watch_flow.exe'" | ForEach-Object { $_.ProcessId }
    } `
    -StopTask { Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue } `
    -ForceKillAll {
        Get-CimInstance Win32_Process | Where-Object {
            $_.CommandLine -match "tiga_startup_supervisor|tiga_windows_nowplaying_v2.py|tiga_windows_nowplaying.exe|tiga_stream_watch_flow"
        } | ForEach-Object {
            Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
        }
    })
Write-Host "TIGA stack stopped."
