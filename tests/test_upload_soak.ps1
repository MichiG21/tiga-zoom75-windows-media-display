# Offline test for tools\upload_soak.ps1. It runs against a temporary fake install, with a
# small fake renderer and uploader that answer through files, and stops that child by its
# own PID. Nothing here touches Bluetooth, the keyboard, the scheduled task or the real stack.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\test_upload_soak.ps1

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot
$Soak = Join-Path $RepoRoot "tools\upload_soak.ps1"
$failures = 0

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if ($Condition) { Write-Host "  ok   $Message" }
    else { Write-Host "  FAIL $Message"; $script:failures++ }
}

function New-FakeInstall {
    $root = Join-Path ([System.IO.Path]::GetTempPath()) ("tiga soak test " + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path (Join-Path $root "logs"), (Join-Path $root "extracted") | Out-Null
    return $root
}

# Answers each new command: writes the ack and, for a screen, the delivery record and log
# lines. After -HaltAfter screens it writes a halt file instead of delivering.
$FakeStack = @'
param([string]$Root, [int]$HaltAfter = 0)
$command = Join-Path $Root "tiga_display_command.txt"
$ack = Join-Path $Root "tiga_display_command.ack"
$delivered = Join-Path $Root "extracted\session_01_bulk_stream.bin.delivered"
$ble = Join-Path $Root "logs\ble.log"
$last = ""
$screens = 0
# Replacing a file fails while the soak has it open for reading (a sharing violation). The
# default error mode would carry on with the answer lost, and the soak would wait out its
# timeout (exit 3; seen in CI on PR #46). Retry until it lands, as the soak does for its own
# command file.
function Move-Replace([string]$From, [string]$To) {
    for ($try = 1; ; $try++) {
        try { Move-Item -LiteralPath $From -Destination $To -Force -ErrorAction Stop; return }
        catch { if ($try -ge 100) { throw }; Start-Sleep -Milliseconds 20 }
    }
}
while ($true) {
    $text = ""
    try { if (Test-Path -LiteralPath $command) { $text = [IO.File]::ReadAllText($command).Trim() } } catch {}
    if ($text -and $text -ne $last) {
        $last = $text
        $crc = [Guid]::NewGuid().ToString("N").Substring(0, 8)
        $name = $text.Split(" ")[0]
        Set-Content -LiteralPath "$ack.tmp" -Value "command_text=$text`ncommand=$name`nframe_id=f$screens`ncrc32=$crc" -NoNewline
        Move-Replace "$ack.tmp" $ack
        if ($name -ne "auto") {
            $screens++
            $now = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
            if ($HaltAfter -and $screens -gt $HaltAfter) {
                Add-Content -LiteralPath $ble -Value "$now UPLOAD FAILED frame=f$screens crc32=$crc after 4.00s: timed out"
                Add-Content -LiteralPath $ble -Value "$now SIGNATURE slowing frame=f$screens"
                Set-Content -LiteralPath (Join-Path $Root "logs\uploader_halted.json") -Value "{}"
            } else {
                Add-Content -LiteralPath $ble -Value "$now CANDIDATE name=`"Zoom75 Tiga`" address=07:01:88:B3:66:1B rssi=-56 (test)"
                Add-Content -LiteralPath $ble -Value "$now UPLOAD COMPLETE frame=f$screens crc32=$crc in 9.50s"
                Set-Content -LiteralPath "$delivered.tmp" -Value "delivered_at=$now-$screens`nframe_id=f$screens`ncrc32=$crc" -NoNewline
                Move-Replace "$delivered.tmp" $delivered
            }
        }
    }
    Start-Sleep -Milliseconds 50
}
'@

function Start-FakeStack {
    param([string]$Root, [int]$HaltAfter = 0, [switch]$NeverDeliver)
    $script_ = Join-Path $Root "fake_stack.ps1"
    $code = if ($NeverDeliver) { $FakeStack -replace 'if \(\$name -ne "auto"\) \{', 'if ($false) {' } else { $FakeStack }
    Set-Content -LiteralPath $script_ -Value $code
    return Start-Process powershell -PassThru -WindowStyle Hidden -ArgumentList @(
        "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$script_`"", "-Root", "`"$Root`"", "-HaltAfter", "$HaltAfter"
    )
}

function Invoke-Soak {
    param([string]$Root, [string[]]$Extra)
    $output = & powershell -NoProfile -ExecutionPolicy Bypass -File $Soak -Root $Root -PauseSeconds 0 -PollMilliseconds 50 @Extra 2>&1 | Out-String
    return [pscustomobject]@{ Code = $LASTEXITCODE; Output = $output }
}

function Read-Command {
    param([string]$Root)
    $path = Join-Path $Root "tiga_display_command.txt"
    if (Test-Path -LiteralPath $path) { return [IO.File]::ReadAllText($path) }
    return $null
}

Write-Host "Dry run sends nothing"
$root = New-FakeInstall
try {
    $r = Invoke-Soak $root @("-Count", "4")
    Assert-True ($r.Code -eq 0) "exit code 0"
    Assert-True ($r.Output -match "nothing was sent") "says so"
    Assert-True ($null -eq (Read-Command $root)) "no command written"
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $root "logs\upload_soak.log"))) "no report written"
} finally { Remove-Item -LiteralPath $root -Recurse -Force }

Write-Host "A halt before the start sends nothing"
$root = New-FakeInstall
try {
    Set-Content -LiteralPath (Join-Path $root "logs\uploader_halted.json") -Value "{}"
    $r = Invoke-Soak $root @("-Count", "4", "-ConfirmUploads")
    Assert-True ($r.Code -eq 1) "exit code 1"
    Assert-True ($null -eq (Read-Command $root)) "no command written"
} finally { Remove-Item -LiteralPath $root -Recurse -Force }

Write-Host "A full run delivers every screen and restores automatic mode"
$root = New-FakeInstall
$fake = Start-FakeStack $root
try {
    $r = Invoke-Soak $root @("-Count", "4", "-ConfirmUploads", "-AckTimeoutSeconds", "20")
    Assert-True ($r.Code -eq 0) "exit code 0 (got $($r.Code))"
    Assert-True ($r.Output -match "Screens\s+: 4 of 4 delivered") "4 of 4 delivered"
    Assert-True ($r.Output -match "Upload attempts: 4, 4 complete, 0 failed \(0%\)") "counts the attempts from ble.log"
    Assert-True ($r.Output -match "Median upload : 9\.50 s") "median upload time"
    Assert-True ($r.Output -match "Signal readings: -56") "signal readings"
    $sequence = @([regex]::Matches($r.Output, "\d/4 (standby|black)") | ForEach-Object { $_.Groups[1].Value }) -join ","
    Assert-True ($sequence -eq "standby,black,standby,black") "alternates the screens ($sequence)"
    Assert-True ((Read-Command $root) -match "^auto soak-") "automatic mode restored"
    Assert-True (Test-Path -LiteralPath (Join-Path $root "logs\upload_soak.log")) "report written"
} finally {
    Stop-Process -Id $fake.Id -Force -ErrorAction SilentlyContinue
    $fake.WaitForExit(5000) | Out-Null
    Remove-Item -LiteralPath $root -Recurse -Force
}

Write-Host "A halt during the run stops it and restores automatic mode"
$root = New-FakeInstall
$fake = Start-FakeStack $root -HaltAfter 2
try {
    $r = Invoke-Soak $root @("-Count", "6", "-ConfirmUploads", "-AckTimeoutSeconds", "20")
    Assert-True ($r.Code -eq 2) "exit code 2 (got $($r.Code))"
    Assert-True ($r.Output -match "HALTED at upload 3") "names the upload it halted at"
    Assert-True ($r.Output -match "Screens\s+: 2 of 6 delivered") "2 of 6 delivered"
    Assert-True ($r.Output -match "1 failed") "counts the failure"
    Assert-True ($r.Output -match "0 sudden, 1 slowing") "counts its type"
    Assert-True ((Read-Command $root) -match "^auto soak-") "automatic mode restored"
} finally {
    Stop-Process -Id $fake.Id -Force -ErrorAction SilentlyContinue
    $fake.WaitForExit(5000) | Out-Null
    Remove-Item -LiteralPath $root -Recurse -Force
}

Write-Host "No renderer: stops instead of waiting forever"
$root = New-FakeInstall
try {
    $r = Invoke-Soak $root @("-Count", "4", "-ConfirmUploads", "-AckTimeoutSeconds", "1")
    Assert-True ($r.Code -eq 3) "exit code 3 (got $($r.Code))"
    Assert-True ($r.Output -match "did not answer") "says the renderer did not answer"
} finally { Remove-Item -LiteralPath $root -Recurse -Force }

Write-Host "Delivery never comes: stops at the timeout"
$root = New-FakeInstall
$fake = Start-FakeStack $root -NeverDeliver
try {
    $r = Invoke-Soak $root @("-Count", "4", "-ConfirmUploads", "-AckTimeoutSeconds", "20", "-DeliveryTimeoutSeconds", "1")
    Assert-True ($r.Code -eq 4) "exit code 4 (got $($r.Code))"
    Assert-True ($r.Output -match "not delivered within 1 s") "says so"
    Assert-True ((Read-Command $root) -match "^auto soak-") "automatic mode restored"
} finally {
    Stop-Process -Id $fake.Id -Force -ErrorAction SilentlyContinue
    $fake.WaitForExit(5000) | Out-Null
    Remove-Item -LiteralPath $root -Recurse -Force
}

Write-Host ""
if ($failures) { Write-Host "$failures FAILED"; exit 1 }
Write-Host "All passed"
