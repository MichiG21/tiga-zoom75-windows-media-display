$taskName = "TIGA Windows Media Display"
$Stream = Join-Path $PSScriptRoot "app\_internal\extracted\session_01_bulk_stream.bin"
$Logs = Join-Path $PSScriptRoot "logs"

Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue | Select-Object TaskName,State
Write-Host ""
Get-CimInstance Win32_Process | Where-Object {
    $_.CommandLine -match "tiga_startup_supervisor|tiga_windows_nowplaying.exe|tiga_stream_watch_flow"
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

# Halted uploader (issue #4): this outranks every other state.
$haltFile = Join-Path $Logs "uploader_halted.json"
$halt = $null
if (Test-Path -LiteralPath $haltFile) {
    try { $halt = Get-Content -Raw -LiteralPath $haltFile | ConvertFrom-Json } catch { $halt = $null }
    if ($null -eq $halt) { $halt = [pscustomobject]@{ reason = "halt file unreadable: $haltFile" } }
    $verdict = "HALTED - BLE uploader stopped and will NOT retry by itself: $($halt.reason)"
}

# Automatic retry after a sudden failure (docs/design/RETRY_RECOVERY.md). The supervisor
# keeps logs\uploader_retry.json up to date; a waiting or running retry outranks the
# open session marker that goes with it.
$retryFile = Join-Path $Logs "uploader_retry.json"
$retry = $null
if (Test-Path -LiteralPath $retryFile) {
    try { $retry = Get-Content -Raw -LiteralPath $retryFile | ConvertFrom-Json } catch { $retry = $null }
}
# A waiting or running state older than 10 minutes is left over from a stopped stack.
$retrying = (-not $halt) -and $retry -and ($retry.status -eq "waiting" -or $retry.status -eq "running") -and
    ((ConvertTo-LocalTime $retry.updated_at) -gt (Get-Date).AddMinutes(-10))
if ($retrying) {
    if ($retry.status -eq "waiting") {
        $at = ConvertTo-LocalTime $retry.next_attempt_at
        $verdict = "RETRYING - sudden upload failure; automatic retry $($retry.retry) of $($retry.max_retries) at {0:HH:mm:ss} (no power-off needed yet)" -f $at
    } else {
        $verdict = "RETRYING - automatic retry $($retry.retry) of $($retry.max_retries) after a sudden upload failure is in progress"
    }
} elseif ((-not $halt) -and (Test-Path -LiteralPath "$Stream.session")) {
    $session = Read-KeyValueFile "$Stream.session"
    $verdict += " (media session open: frame $($session.frame_id), block $($session.last_acked_block)/$($session.total_blocks))"
}

if ($halt) {
    Write-Host ""
    Write-Host "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!" -ForegroundColor Red
}
Write-Host "Status        : $verdict" -ForegroundColor $(if ($halt) { "Red" } else { "Gray" })
if ($halt) {
    Write-Host ("Halted at     : {0}  (exit code {1}: {2})" -f $halt.halted_at, $halt.exit_code, $halt.meaning)
    if ($halt.frame_id) {
        $blocks = if ($null -ne $halt.last_acked_block) { "; last ACKed block $($halt.last_acked_block)$(if ($halt.total_blocks) { "/$($halt.total_blocks)" })" } else { "" }
        Write-Host "Frame         : $($halt.frame_id)$blocks"
    }
    if ($halt.detail) { Write-Host "Detail        : $($halt.detail)" }
    if ($halt.signature) { Write-Host "Signature     : $($halt.signature)" }
    if ($halt.auto_retries) { Write-Host "Auto-retries  : $($halt.auto_retries) automatic retr$(if ($halt.auto_retries -eq 1) { 'y' } else { 'ies' }) used before halting" }
    Write-Host "Action        : $($halt.instruction)" -ForegroundColor Yellow
    Write-Host "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!" -ForegroundColor Red
}

if ($retry) {
    $color = if ($retrying) { "Yellow" } else { "DarkGray" }
    Write-Host ("Auto-retry    : {0}, retry {1} of {2}; last failure: {3}" -f $retry.status, $retry.retry, $retry.max_retries, $retry.last_failure) -ForegroundColor $color
    foreach ($e in @($retry.events) | Select-Object -Last 4) { Write-Host "                $e" -ForegroundColor $color }
}

$lastFailure = Get-ChildItem -Path $Logs -Filter "ble.log*" -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime |
    Select-String -Pattern "UPLOAD FAILED" -ErrorAction SilentlyContinue |
    Select-Object -Last 1
if ($lastFailure) {
    # The log keeps history, so this can be days old. Say so when a delivery since
    # then shows it has been recovered from.
    $failedAt = ConvertTo-LocalTime (($lastFailure.Line -split " ", 2)[0])
    $deliveredAt = if ($delivered) { ConvertTo-LocalTime $delivered.delivered_at } else { $null }
    if ($failedAt -and $deliveredAt -and ($deliveredAt -gt $failedAt)) {
        Write-Host ("Past failure  : {0:yyyy-MM-dd HH:mm:ss} (historical; uploads have succeeded since)" -f $failedAt) -ForegroundColor DarkGray
        Write-Host "                $($lastFailure.Line)" -ForegroundColor DarkGray
    } else {
        Write-Host "Last failure  : $($lastFailure.Line)"
    }
}
