# Offline test for the uploader failure policy (issue #4).
#
# Drives the real supervisor loop (Invoke-SupervisorLoop from tiga_supervisor_common.ps1)
# with dummy children: the "uploader" is a Python script that exits with a chosen code
# and can leave the files the real uploader leaves (session marker, exit record,
# delivery record). The "renderer" just sleeps. Nothing touches Bluetooth.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\test_supervisor_retry.ps1

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot
$failures = 0

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if ($Condition) { Write-Host "  ok   $Message" }
    else { Write-Host "  FAIL $Message"; $script:failures++ }
}

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("tiga retry test " + [guid]::NewGuid().ToString("N").Substring(0, 6))
$logs = Join-Path $work "logs"
New-Item -ItemType Directory -Force -Path $logs | Out-Null
$SupervisorLog = Join-Path $logs "startup_supervisor.log"

. (Join-Path $RepoRoot "tiga_supervisor_common.ps1")

$python = (Get-Command py.exe -ErrorAction Stop).Source
$stream = Join-Path $work "session_01_bulk_stream.bin"
$haltFile = Join-Path $logs "uploader_halted.json"

# ---- Pure decision table -----------------------------------------------------------
Write-Host "== decision table"

function Decide { param($Code, $Attempt, $Session = $null) Get-UploaderDecision -ExitCode $Code -Session $Session -Attempt $Attempt }

$delays = 1..4 | ForEach-Object { (Decide 10 $_).Delay }
Assert-True (($delays -join ",") -eq "8,16,32,60") "code 10 backs off 8,16,32,60 ($($delays -join ','))"
foreach ($c in 10, 11) {
    $early = 1..4 | ForEach-Object { (Decide $c $_).Delay }
    Assert-True (($early -join ",") -eq "8,16,32,60") "code $c backs off 8,16,32,60"
    $late = 5, 6, 50 | ForEach-Object { Decide $c $_ }
    Assert-True (@($late | Where-Object { $_.Action -eq "retry" -and $_.Class -eq "slow-retry" -and $_.Delay -eq 300 }).Count -eq 3) "code $c never halts: every 300 s from the 5th failure on"
    Assert-True (-not ($late | Where-Object NeedsPowerCycle)) "code $c slow retry does not ask for a power cycle"
}
Assert-True ((Decide 1 4).Action -eq "retry" -and (Decide 1 5).Class -eq "gave-up") "an unexplained exit with no marker still gives up on the 5th failure"
Assert-True (-not (Decide 1 5).NeedsPowerCycle) "giving up does not ask for a power cycle"
foreach ($c in 20, 21) {
    $d = Decide $c 1
    Assert-True ($d.Action -eq "halt" -and $d.NeedsPowerCycle -and $d.Instruction -match "START_STACK") "code $c halts and asks for a power cycle"
}
$d = Decide 30 1
Assert-True ($d.Action -eq "halt" -and $d.Class -eq "fatal" -and -not $d.NeedsPowerCycle) "code 30 halts without a power cycle"
$d = Decide 1 1 @{ frame_id = "r-f0001" }
Assert-True ($d.Action -eq "halt" -and $d.Class -eq "session-open") "a killed uploader (code 1) with a session marker halts"
$d = Decide 10 1 @{ frame_id = "r-f0001" }
Assert-True ($d.Action -eq "halt") "a marker overrides even a retryable code"
Assert-True ((Decide 1 1).Action -eq "retry") "a crash with no marker is retried (no session was open)"
Assert-True ((Decide 0 0).Action -eq "retry") "a clean exit is restarted"

# ---- Loop with dummy children --------------------------------------------------------

$dummy = Join-Path $work "dummy uploader.py"
Set-Content -LiteralPath $dummy -Encoding ASCII -Value @'
import os, sys, time
stream, code, actions = sys.argv[1], int(sys.argv[2]), sys.argv[3:]
run_id = os.environ.get("TIGA_RUN_ID", "")
def put(ext, text):
    with open(stream + ext, "w") as f:
        f.write(text)
if "deliver" in actions:
    time.sleep(0.05)
    put(".delivered", "frame_id=r-f0009\ncrc32=00000000\nrun_id=%s\n" % run_id)
if "marker" in actions:
    put(".session", "frame_id=r-f0042\nlast_acked_block=17\ntotal_blocks=53\nrun_id=%s\n" % run_id)
if "stale" in actions:
    put(".exit", "run_id=ble-some-older-run\ncode=20\ndetail=from an older run\n")
if "record" in actions:
    put(".exit", "run_id=%s\ncode=%d\ndetail=PROGRESS timeout after block 18\nframe_id=r-f0042\nlast_acked_block=17\n" % (run_id, code))
print("dummy uploader exiting with", code)
sys.exit(code)
'@
$sleeper = Join-Path $work "renderer.py"
Set-Content -LiteralPath $sleeper -Encoding ASCII -Value 'import time; time.sleep(120)'

# Fast schedule so the test takes seconds, not minutes.
$BleRetryDelays = @(0.3, 0.6, 1.2, 2.4)
$BleSlowRetryDelay = 3
$RendererRestartDelay = 0.2

function Invoke-Scenario {
    # $Plan: one entry per uploader start: @(<exit code>, <actions...>); the last repeats.
    param([string]$Name, [object[]]$Plan, [int]$Iterations, [switch]$Halted)
    Write-Host "== $Name"
    foreach ($ext in ".session", ".exit", ".delivered") { Remove-Item -LiteralPath "$stream$ext" -ErrorAction SilentlyContinue }
    if (-not $Halted) { Remove-Item -LiteralPath $haltFile -ErrorAction SilentlyContinue }
    Set-Content -LiteralPath $SupervisorLog -Value "" -Encoding ASCII

    $starts = New-Object System.Collections.ArrayList
    $state = New-SupervisorState
    $startUploader = {
        $n = [Math]::Min($starts.Count, $Plan.Count - 1)
        $step = @($Plan[$n])
        [void]$starts.Add((Get-Date))
        Start-LoggedProcess -Name "ble" -FilePath $python `
            -Arguments (@($dummy, $stream) + ($step | ForEach-Object { "$_" })) `
            -WorkingDirectory $work -StdOut (Join-Path $logs "ble.log") -StdErr (Join-Path $logs "ble_error.log")
    }
    $startRenderer = {
        Start-LoggedProcess -Name "renderer" -FilePath $python -Arguments @($sleeper) `
            -WorkingDirectory $work -StdOut (Join-Path $logs "renderer.log") -StdErr (Join-Path $logs "renderer_error.log")
    }
    try {
        Invoke-SupervisorLoop -State $state -StartRenderer $startRenderer -StartUploader $startUploader `
            -HaltFile $haltFile -StreamPath $stream -PollSeconds 0.1 -MaxIterations $Iterations
        $state | Add-Member -NotePropertyName RendererAlive -NotePropertyValue (-not $state.Renderer.HasExited)
        $state | Add-Member -NotePropertyName Starts -NotePropertyValue @($starts)
    } finally {
        foreach ($p in @($state.Renderer, $state.Uploader)) { try { Stop-LoggedProcess $p } catch {} }
    }
    return $state
}

function Read-Halt { if (Test-Path $haltFile) { Get-Content -Raw $haltFile | ConvertFrom-Json } }

# 1. Unreachable / no qualifying device: back off, then keep retrying slowly, never halt.
foreach ($c in 10, 11) {
    $s = Invoke-Scenario "code $c every time" -Plan @(, @($c)) -Iterations 110
    $gaps = @(); for ($i = 1; $i -lt $s.Starts.Count; $i++) { $gaps += ($s.Starts[$i] - $s.Starts[$i - 1]).TotalSeconds }
    $shown = ($gaps | ForEach-Object { '{0:n2}' -f $_ }) -join ', '
    Assert-True ($s.UploaderStarts -ge 6) "code ${c}: uploader still restarted after the backoff (starts: $($s.UploaderStarts))"
    Assert-True ($gaps[1] -gt $gaps[0] -and $gaps[2] -gt $gaps[1] -and $gaps[3] -gt $gaps[2] -and $gaps[4] -gt $gaps[3]) "code ${c}: gaps grow, then the slow interval: ${shown}s"
    Assert-True (-not $s.Halted -and $null -eq (Read-Halt)) "code ${c}: never halted, no halt file"
    Assert-True ((Get-Content -Raw $SupervisorLog) -match "retrying every") "code ${c}: slow retry is logged"
    Assert-True ($s.RendererAlive -and $s.RendererStarts -eq 1) "code ${c}: renderer untouched"
}

# 1b. Any other exit without a marker still gives up.
$s = Invoke-Scenario "code 1 (no marker) every time" -Plan @(, @(1)) -Iterations 90
$h = Read-Halt
Assert-True ($s.UploaderStarts -eq 5) "uploader started 5 times (1 + 4 retries), got $($s.UploaderStarts)"
Assert-True ($s.Halted -and $h.class -eq "gave-up" -and $h.exit_code -eq 1 -and -not $h.needs_power_cycle) "halted as gave-up, no power cycle"

# 2. Interrupted transfer: no restart, renderer keeps running, halt file complete.
$s = Invoke-Scenario "code 20 with marker and exit record" -Plan @(, @(20, "marker", "record")) -Iterations 25
$h = Read-Halt
Assert-True ($s.UploaderStarts -eq 1) "uploader not restarted (starts: $($s.UploaderStarts))"
Assert-True ($s.RendererAlive -and $s.RendererStarts -eq 1) "renderer still running"
Assert-True ($null -ne $h) "halt file written"
Assert-True ($h.exit_code -eq 20 -and $h.class -eq "session-open" -and $h.needs_power_cycle) "halt: code 20, session-open, needs power cycle"
Assert-True ($h.frame_id -eq "r-f0042" -and $h.last_acked_block -eq 17 -and $h.total_blocks -eq 53) "halt: frame r-f0042, block 17/53"
Assert-True ($h.detail -match "PROGRESS timeout" -and $h.halted_at -and $h.instruction -match "Disconnect the keyboard's battery for at least 30 s" -and $h.instruction -notmatch "power switch \(") "halt: detail, time and plain-English instruction"
Assert-True ((Get-Content -Raw $SupervisorLog) -match "HALTED \(session-open\)") "supervisor log says HALTED"

# 3. Uploader killed mid-transfer: the exit code is meaningless (1) but the marker survives.
$s = Invoke-Scenario "killed (code 1) with marker" -Plan @(, @(1, "marker", "stale")) -Iterations 20
$h = Read-Halt
Assert-True ($s.UploaderStarts -eq 1 -and $h.class -eq "session-open" -and $h.last_acked_block -eq 17) "marker alone halts it, with the block from the marker"
Assert-True ($null -eq $h.detail) "an exit record from another run is not used"

# 4. Configuration error.
$s = Invoke-Scenario "code 30" -Plan @(, @(30)) -Iterations 20
$h = Read-Halt
Assert-True ($s.UploaderStarts -eq 1 -and $h.class -eq "fatal" -and -not $h.needs_power_cycle) "code 30 halts as fatal, no restart"

# 5. Code 21 without a marker still halts.
$s = Invoke-Scenario "code 21" -Plan @(, @(21)) -Iterations 20
Assert-True ($s.UploaderStarts -eq 1 -and (Read-Halt).needs_power_cycle) "code 21 halts and asks for a power cycle"

# 6. A successful upload resets the counter: fail, fail, (deliver then fail), then fail x4.
#    (Code 1, because 10 and 11 no longer give up at all.)
$s = Invoke-Scenario "delivery resets the counter" -Plan @(@(1), @(1), @(1, "deliver"), @(1)) -Iterations 140
Assert-True ($s.UploaderStarts -eq 7) "3 runs, reset, then 1 + 3 retries before giving up (starts: $($s.UploaderStarts))"
Assert-True ((Get-Content -Raw $SupervisorLog) -match "retry counter reset") "reset is logged"
Assert-True ((Read-Halt).attempts -eq 5) "gave up at 5 consecutive failures after the reset"

# 7. Halted at startup: the uploader is not started; clearing resumes it.
Write-Host "== halted at startup, then cleared"
[System.IO.File]::WriteAllText($haltFile, '{"class":"session-open"}')
Set-Content -LiteralPath "$stream.session" -Value "frame_id=r-f0001" -Encoding ASCII
$s = Invoke-Scenario "halted at startup" -Plan @(, @(10)) -Iterations 10 -Halted
Assert-True ($s.UploaderStarts -eq 0 -and $s.RendererStarts -eq 1) "halt file present: renderer starts, uploader does not"
[System.IO.File]::WriteAllText($haltFile, '{"class":"session-open"}')
Set-Content -LiteralPath "$stream.session" -Value "frame_id=r-f0001" -Encoding ASCII
$cleared = Clear-UploaderHalt -HaltFile $haltFile -StreamPath $stream
Assert-True (-not (Test-Path $haltFile) -and -not (Test-Path "$stream.session") -and $cleared.Count -eq 2) "Clear-UploaderHalt removes halt file and session marker"
Assert-True ((Clear-UploaderHalt -HaltFile $haltFile -StreamPath $stream).Count -eq 0) "clearing twice is harmless"

# Live un-halt: a running supervisor notices the halt file disappearing.
Write-Host "== live un-halt"
$state = New-SupervisorState
[System.IO.File]::WriteAllText($haltFile, '{}')
$clearer = Start-Job -ScriptBlock { param($f) Start-Sleep -Milliseconds 800; Remove-Item -LiteralPath $f } -ArgumentList $haltFile
try {
    Invoke-SupervisorLoop -State $state `
        -StartRenderer { Start-LoggedProcess -Name "renderer" -FilePath $python -Arguments @($sleeper) -WorkingDirectory $work -StdOut (Join-Path $logs "r.log") -StdErr (Join-Path $logs "re.log") } `
        -StartUploader { Start-LoggedProcess -Name "ble" -FilePath $python -Arguments @($sleeper) -WorkingDirectory $work -StdOut (Join-Path $logs "b.log") -StdErr (Join-Path $logs "be.log") } `
        -HaltFile $haltFile -StreamPath $stream -PollSeconds 0.1 -MaxIterations 25
    Assert-True ($state.UploaderStarts -eq 1 -and -not $state.Halted) "uploader started once the halt file was removed"
} finally {
    foreach ($p in @($state.Renderer, $state.Uploader)) { try { Stop-LoggedProcess $p } catch {} }
    Remove-Job $clearer -Force
}

# STATUS_STACK.ps1 (dev layout) shows the halt prominently. It only reads, so a copy
# can run against a fake install root.
Write-Host "== STATUS_STACK.ps1"
$root = Join-Path $work "status root"
New-Item -ItemType Directory -Force -Path (Join-Path $root "logs"), (Join-Path $root "extracted") | Out-Null
Copy-Item (Join-Path $RepoRoot "STATUS_STACK.ps1") $root
$fakeStream = Join-Path $root "extracted\session_01_bulk_stream.bin"
Set-Content -LiteralPath "$fakeStream.frame" -Encoding ASCII -Value "frame_id=r-f0042", "crc32=aaaa0042", "rendered_at=2026-10-15T20:00:00+02:00", "reason=media changed"
$decision = Get-UploaderDecision -ExitCode 20 -Session @{ frame_id = "r-f0042" } -Attempt 1
Write-UploaderHaltState -Path (Join-Path $root "logs\uploader_halted.json") -Decision $decision -ExitCode 20 `
    -Session @{ frame_id = "r-f0042"; last_acked_block = "17"; total_blocks = "53" } `
    -ExitRecord @{ detail = "timed out waiting for PROGRESS after 4K block 18/53" } -Attempts 1
$out = (& (Join-Path $root "STATUS_STACK.ps1") 6>&1 | Out-String -Width 400)
Assert-True ($out -match "Status\s+: HALTED") "status verdict is HALTED"
Assert-True ($out -match "exit code 20") "status shows the exit code"
Assert-True ($out -match "r-f0042; last ACKed block 17/53") "status shows frame and block"
Assert-True ($out -match "Action\s+: Disconnect the keyboard's battery.*START_STACK\.ps1") "status gives the power-cycle instruction"
Remove-Item (Join-Path $root "logs\uploader_halted.json")
Set-Content -LiteralPath "$fakeStream.session" -Encoding ASCII -Value "frame_id=r-f0042", "last_acked_block=5", "total_blocks=53"
$out = (& (Join-Path $root "STATUS_STACK.ps1") 6>&1 | Out-String -Width 400)
Assert-True ($out -notmatch "HALTED" -and $out -match "media session open: frame r-f0042, block 5/53") "an in-progress upload is shown, not as halted"
Remove-Item -LiteralPath "$fakeStream.session"

# "Last failure" must not read as current once an upload has succeeded since.
Set-Content -LiteralPath (Join-Path $root "logs\ble.log") -Encoding ASCII -Value "2026-10-15T19:39:53Z UPLOAD FAILED frame=r-f0041 crc32=1 after 4.21s: timed out"
Set-Content -LiteralPath "$fakeStream.delivered" -Encoding ASCII -Value "frame_id=r-f0042", "crc32=aaaa0042", "delivered_at=2026-10-15T20:05:00Z", "elapsed_s=8.4"
$out = (& (Join-Path $root "STATUS_STACK.ps1") 6>&1 | Out-String -Width 400)
Assert-True ($out -match "Past failure\s+: .*historical" -and $out -notmatch "Last failure") "an old failure is labelled historical after a later delivery"
Set-Content -LiteralPath "$fakeStream.delivered" -Encoding ASCII -Value "frame_id=r-f0040", "crc32=aaaa0040", "delivered_at=2026-10-15T19:00:00Z", "elapsed_s=8.4"
$out = (& (Join-Path $root "STATUS_STACK.ps1") 6>&1 | Out-String -Width 400)
Assert-True ($out -match "Last failure\s+: 2026-10-15T19:39:53Z UPLOAD FAILED") "a failure newer than the last delivery is shown as the last failure"

# START_STACK.ps1 (dev and packaged) must clear the halt before starting the task.
# (Running it here would start the real scheduled task, so its text is checked instead.)
Write-Host "== START_STACK.ps1"
foreach ($script in @("START_STACK.ps1", "installer\runtime\START_STACK.ps1")) {
    $text = Get-Content -Raw (Join-Path $RepoRoot $script)
    $clear = $text.IndexOf("Clear-UploaderHalt")
    $start = $text.IndexOf("Start-ScheduledTask")
    Assert-True ($clear -ge 0 -and $clear -lt $start) "$script clears the halt before Start-ScheduledTask"
}

Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
if ($failures) { Write-Host "$failures failure(s)"; exit 1 }
Write-Host "All supervisor retry checks passed."
