. (Join-Path $PSScriptRoot "tiga_supervisor_common.ps1")

Show-StackStatus -TaskName "TIGA Windows Media Display" `
    -Stream (Join-Path $PSScriptRoot "extracted\session_01_bulk_stream.bin") `
    -Logs (Join-Path $PSScriptRoot "logs") `
    -ProcessPattern "tiga_startup_supervisor|tiga_windows_nowplaying_v2.py|tiga_stream_watch_flow"
