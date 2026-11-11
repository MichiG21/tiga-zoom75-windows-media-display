# Forces real uploads through the running stack, to measure the failure rate when little
# media is changing. It alternates the standby and black screens the way Display Control
# does: the renderer draws each one, the uploader sends it, and the automatic retry and halt
# rules apply as usual. It never opens Bluetooth and never starts or stops the stack.
#
# Without -ConfirmUploads it only reports the current state and sends nothing.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\upload_soak.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\upload_soak.ps1 -ConfirmUploads
#
# It stops at a halt and leaves it for you to clear. At the end, or on Ctrl+C, it asks for
# automatic mode again, which draws one more screen. Results go to logs\upload_soak.log.
#
# Exit codes: 0 finished, 1 halted before starting, 2 halted during the run,
# 3 the renderer did not answer, 4 a frame was not delivered in time.

param(
    [string]$Root = (Split-Path -Parent $PSScriptRoot),
    [ValidateRange(1, 1000)][int]$Count = 30,
    [int]$PauseSeconds = 3,
    [int]$AckTimeoutSeconds = 15,
    # Long enough for all 3 automatic retries: each waits up to 300 s, then scans and uploads.
    [int]$DeliveryTimeoutSeconds = 1320,
    [int]$PollMilliseconds = 500,
    [switch]$ConfirmUploads
)

$ErrorActionPreference = "Stop"

$CommandFile   = Join-Path $Root "tiga_display_command.txt"
$AckFile       = Join-Path $Root "tiga_display_command.ack"
$DeliveredFile = Join-Path $Root "extracted\session_01_bulk_stream.bin.delivered"
$Logs          = Join-Path $Root "logs"
$HaltFile      = Join-Path $Logs "uploader_halted.json"
$BleLog        = Join-Path $Logs "ble.log"
$ReportFile    = Join-Path $Logs "upload_soak.log"

. (Join-Path $PSScriptRoot "..\tiga_supervisor_common.ps1")

$script:sequence = 0

function Write-Report {
    param([string]$Text)
    Write-Host $Text
    Add-Content -LiteralPath $ReportFile -Value $Text -Encoding UTF8
}

function Read-KeyValues {
    # The renderer and uploader replace these files while we poll; try again next poll.
    param([string]$Path)
    try { return Read-KeyValueFile $Path } catch { return $null }
}

function Send-DisplayCommand {
    # "<command> <unique id>", as Display Control writes it. Returns the text sent.
    param([string]$Command)
    $script:sequence++
    $text = "$Command soak-$([DateTime]::UtcNow.Ticks)-$PID-$($script:sequence)"
    $tmp = "$CommandFile.soak.tmp"
    [System.IO.File]::WriteAllText($tmp, $text, [System.Text.Encoding]::ASCII)
    for ($try = 1; ; $try++) {
        # The renderer reads this file every second, which can briefly block replacing it.
        try {
            Move-Item -LiteralPath $tmp -Destination $CommandFile -Force
            return $text
        } catch {
            if ($try -ge 10) { throw }
            Start-Sleep -Milliseconds 100
        }
    }
}

function Wait-CommandAck {
    # The renderer's answer: which frame (by CRC) it drew for this command, or $null.
    param([string]$Text, [int]$TimeoutSeconds)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $ack = Read-KeyValues $AckFile
        if ($ack -and $ack.command_text -eq $Text) { return $ack }
        Start-Sleep -Milliseconds $PollMilliseconds
    }
    return $null
}

function Get-MedianValue {
    param([double[]]$Values)
    $sorted = @($Values | Sort-Object)
    return $sorted[[int][Math]::Floor($sorted.Count / 2)]
}

function Write-SoakSummary {
    # Everything the uploader logged since the start, including the retries.
    param([string]$StartStamp, [int]$Delivered, [int]$Skipped, [int]$Requested, [string]$Ending)
    $complete = New-Object System.Collections.ArrayList
    $failed = 0; $sudden = 0; $slowing = 0
    $rssi = New-Object System.Collections.ArrayList
    if (Test-Path -LiteralPath $BleLog) {
        foreach ($line in [System.IO.File]::ReadAllLines($BleLog)) {
            if ($line -notmatch '^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ) ') { continue }
            if ([string]::CompareOrdinal($Matches[1], $StartStamp) -lt 0) { continue }
            if ($line -match ' UPLOAD COMPLETE .* in ([\d.]+)s') {
                [void]$complete.Add([double]::Parse($Matches[1], [Globalization.CultureInfo]::InvariantCulture))
            }
            elseif ($line -match ' UPLOAD FAILED ') { $failed++ }
            elseif ($line -match ' SIGNATURE sudden ') { $sudden++ }
            elseif ($line -match ' SIGNATURE slowing ') { $slowing++ }
            elseif ($line -match ' CANDIDATE .* rssi=(-?\d+)') { [void]$rssi.Add($Matches[1]) }
        }
    }
    $total = $complete.Count + $failed
    $percent = if ($total) { [Math]::Round(100 * $failed / $total, 1) } else { 0 }
    Write-Report ""
    Write-Report "Result        : $Ending"
    Write-Report "Screens       : $Delivered of $Requested delivered$(if ($Skipped) { ", $Skipped already showing" })"
    Write-Report "Upload attempts: $total, $($complete.Count) complete, $failed failed ($percent%)"
    Write-Report "Failure types : $sudden sudden, $slowing slowing"
    if ($complete.Count) {
        $median = (Get-MedianValue ([double[]]$complete.ToArray())).ToString("0.00", [Globalization.CultureInfo]::InvariantCulture)
        Write-Report "Median upload : $median s"
    }
    $rssiText = if ($rssi.Count) { $rssi -join ", " } else { "none (no new connection during the run)" }
    Write-Report "Signal readings: $rssiText"
}

if (-not (Test-Path -LiteralPath $Logs)) { throw "No logs folder at $Logs. Is -Root right?" }

if (Test-Path -LiteralPath $HaltFile) {
    Write-Host "The uploader is halted ($HaltFile). Clear that first; nothing was sent."
    exit 1
}

if (-not $ConfirmUploads) {
    $last = Read-KeyValues $DeliveredFile
    Write-Host "Dry run: nothing was sent. Add -ConfirmUploads to force $Count uploads."
    Write-Host "Install       : $Root"
    Write-Host "Last delivered: $(if ($last) { "$($last.frame_id) at $($last.delivered_at)" } else { 'none recorded' })"
    exit 0
}

$startStamp = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
Write-Report ""
Write-Report "===== upload soak started $startStamp, $Count uploads ====="

$delivered = 0
$skipped = 0
$sent = $false
$exitCode = 0
$ending = "stopped early (Ctrl+C)"

try {
    for ($i = 1; $i -le $Count; $i++) {
        $previous = Read-KeyValues $DeliveredFile
        $previousAt = if ($previous) { $previous.delivered_at } else { "" }

        $command = if ($i % 2 -eq 1) { "standby" } else { "black" }
        $text = Send-DisplayCommand $command
        $sent = $true
        $began = Get-Date

        $ack = Wait-CommandAck $text $AckTimeoutSeconds
        if (-not $ack) {
            $exitCode = 3
            $ending = "stopped: the renderer did not answer within $AckTimeoutSeconds s. Is the stack running?"
            break
        }

        $outcome = $null
        $deadline = $began.AddSeconds($DeliveryTimeoutSeconds)
        while ((Get-Date) -lt $deadline) {
            if (Test-Path -LiteralPath $HaltFile) { $outcome = "halted"; break }
            $now = Read-KeyValues $DeliveredFile
            if ($now -and $now.crc32 -eq $ack.crc32) {
                $outcome = if ($now.delivered_at -ne $previousAt) { "delivered" } else { "already showing" }
                break
            }
            Start-Sleep -Milliseconds $PollMilliseconds
        }

        $took = ((Get-Date) - $began).TotalSeconds.ToString("0.0", [Globalization.CultureInfo]::InvariantCulture)
        if ($outcome -eq "halted") {
            Write-Report ("{0,3}/{1} {2,-7} HALTED after {3} s" -f $i, $Count, $command, $took)
            $exitCode = 2
            $ending = "HALTED at upload $i. See STATUS_STACK.ps1; clear it with START_STACK.ps1 (battery reset first if it keeps halting)."
            break
        }
        if (-not $outcome) {
            Write-Report ("{0,3}/{1} {2,-7} NOT DELIVERED within {3} s" -f $i, $Count, $command, $DeliveryTimeoutSeconds)
            $exitCode = 4
            $ending = "stopped: upload $i was not delivered within $DeliveryTimeoutSeconds s."
            break
        }
        if ($outcome -eq "delivered") { $delivered++ } else { $skipped++ }
        Write-Report ("{0,3}/{1} {2,-7} {3} in {4} s" -f $i, $Count, $command, $outcome, $took)

        if ($i -lt $Count) { Start-Sleep -Seconds $PauseSeconds }
    }
    if ($exitCode -eq 0) { $ending = "finished" }
}
finally {
    if ($sent) {
        try {
            if (-not (Wait-CommandAck (Send-DisplayCommand "auto") $AckTimeoutSeconds)) {
                Write-Report "Automatic mode was requested but the renderer did not answer. Press Resume Automatic in Display Control."
            }
        } catch {
            Write-Report "Could not request automatic mode ($_). Press Resume Automatic in Display Control."
        }
    }
    Write-SoakSummary -StartStamp $startStamp -Delivered $delivered -Skipped $skipped -Requested $Count -Ending $ending
}

exit $exitCode
