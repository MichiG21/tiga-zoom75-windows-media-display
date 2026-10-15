$taskName = "TIGA Windows Media Display"
$Stream = Join-Path $PSScriptRoot "extracted\session_01_bulk_stream.bin"
$Logs = Join-Path $PSScriptRoot "logs"

Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue | Select-Object TaskName,State
Write-Host ""
Get-CimInstance Win32_Process | Where-Object {
    $_.CommandLine -match "tiga_startup_supervisor|tiga_windows_nowplaying_v2.py|tiga_stream_watch_flow"
} | Select-Object ProcessId,Name,CommandLine

# ---- Delivery (issue #9): a live process is not proof the keyboard is being updated ----

function Read-KeyValueFile {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    $map = @{}
    foreach ($line in Get-Content -LiteralPath $Path -Encoding UTF8) {
        $i = $line.IndexOf("=")
        if ($i -gt 0) { $map[$line.Substring(0, $i).Trim()] = $line.Substring($i + 1).Trim() }
    }
    return $map
}

function ConvertTo-LocalTime {
    param([string]$Text)
    try { return ([DateTimeOffset]::Parse($Text)).LocalDateTime } catch { return $null }
}

$rendered  = Read-KeyValueFile "$Stream.frame"
$delivered = Read-KeyValueFile "$Stream.delivered"

Write-Host "Delivery"
Write-Host "--------"
if ($rendered) {
    Write-Host ("Last rendered : {0}  {1:yyyy-MM-dd HH:mm:ss}  ({2})" -f $rendered.frame_id, (ConvertTo-LocalTime $rendered.rendered_at), $rendered.reason)
} else {
    Write-Host "Last rendered : none recorded"
}
if ($delivered) {
    Write-Host ("Last delivered: {0}  {1:yyyy-MM-dd HH:mm:ss}  in {2}s" -f $delivered.frame_id, (ConvertTo-LocalTime $delivered.delivered_at), $delivered.elapsed_s)
} else {
    Write-Host "Last delivered: none recorded"
}

$verdict = "UNKNOWN - nothing rendered yet"
if ($rendered) {
    $renderedAt = ConvertTo-LocalTime $rendered.rendered_at
    $age = if ($renderedAt) { [int]((Get-Date) - $renderedAt).TotalSeconds } else { 0 }
    if ($delivered -and ($delivered.crc32 -eq $rendered.crc32)) {
        $verdict = "OK - the keyboard shows the newest rendered frame"
    } elseif ($age -gt 60) {
        $verdict = "STALLED - newest frame rendered ${age}s ago and not delivered"
    } else {
        $verdict = "PENDING - newest frame rendered ${age}s ago; upload should be in progress"
    }
}
Write-Host "Status        : $verdict"

$lastFailure = Get-ChildItem -Path $Logs -Filter "ble.log*" -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime |
    Select-String -Pattern "UPLOAD FAILED" -ErrorAction SilentlyContinue |
    Select-Object -Last 1
if ($lastFailure) {
    Write-Host "Last failure  : $($lastFailure.Line)"
}
