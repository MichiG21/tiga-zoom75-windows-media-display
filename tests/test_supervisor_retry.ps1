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

function Decide {
    param($Code, $Attempt, $Session = $null, $Signature = "", $SuddenRetries = 0)
    Get-UploaderDecision -ExitCode $Code -Session $Session -Attempt $Attempt -Signature $Signature -SuddenRetries $SuddenRetries
}

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

# Sudden-failure retry (approved 2026-10-18; RETRY_RECOVERY.md).
$marker = @{ frame_id = "r-f0042" }
$sudden = 0..2 | ForEach-Object { Decide 20 ($_ + 1) $marker "sudden" $_ }
Assert-True (@($sudden | Where-Object { $_.Action -eq "retry" -and $_.Class -eq "sudden-retry" -and $_.Delay -eq 120 -and -not $_.NeedsPowerCycle }).Count -eq 3) "sudden code 20: retries 1-3 after 120 s, no power cycle"
Assert-True ($sudden[0].Reason -match "automatic retry 1 of 3" -and $sudden[2].Reason -match "automatic retry 3 of 3") "the reason names the retry number"
$d = Decide 20 4 $marker "sudden" 3
Assert-True ($d.Action -eq "halt" -and $d.Class -eq "session-open" -and $d.NeedsPowerCycle -and $d.Instruction -eq $PowerCycleInstruction) "after 3 retries: halt exactly as before (same class and instruction)"
Assert-True ($d.Reason -match "all 3 automatic retries have failed") "the halt reason says the retries failed"
foreach ($sig in "slowing", "unclassified", "") {
    $d = Decide 20 1 $marker $sig 0
    Assert-True ($d.Action -eq "halt" -and $d.NeedsPowerCycle) "code 20, signature '$sig': halts at once"
}
Assert-True ((Decide 20 1 $marker "slowing" 0).Reason -match "signature slowing, so no automatic retry") "a slowing halt says why there was no retry"
Assert-True ((Decide 21 1 $marker "sudden" 0).Action -eq "halt") "code 21 halts at once, whatever the signature"
Assert-True ((Decide 1 1 $marker "sudden" 0).Action -eq "halt") "a killed uploader (code 1) with a marker halts, even with a stray signature"
Assert-True ((Decide 20 1 $null "sudden" 0).Action -eq "retry") "sudden code 20 retries even if the marker is already gone"
Assert-True ((Decide 0 0).Action -eq "retry") "a clean exit is restarted"
$d = Get-UploaderDecision -ExitCode 0 -Session $null -Attempt 0 -StopRequested
Assert-True ($d.Action -eq "stop" -and $d.Class -eq "stopped-on-request" -and -not $d.NeedsPowerCycle) "a clean exit after a stop request is not restarted"
Assert-True ((Get-UploaderDecision -ExitCode 10 -Session $null -Attempt 1 -StopRequested).Action -eq "stop") "code 10 after a stop request (disconnect failed) is not retried"
Assert-True ((Get-UploaderDecision -ExitCode 20 -Session $null -Attempt 1 -StopRequested).Action -eq "halt") "an upload that failed on its way out still halts"
Assert-True ((Get-UploaderDecision -ExitCode 1 -Session @{ frame_id = "r-f0001" } -Attempt 1 -StopRequested).Class -eq "session-open") "a session marker still halts when a stop was requested"

# ---- Loop with dummy children --------------------------------------------------------

$dummy = Join-Path $work "dummy uploader.py"
Set-Content -LiteralPath $dummy -Encoding ASCII -Value @'
import os, sys, time
stream, code, actions = sys.argv[1], int(sys.argv[2]), sys.argv[3:]
run_id = os.environ.get("TIGA_RUN_ID", "")
if "check" in actions and os.path.exists(stream + ".session"):
    print("dummy uploader: session marker still present at start")
    sys.exit(77)
def put(ext, text):
    with open(stream + ext, "w") as f:
        f.write(text)
if "wait-stop" in actions:  # like the real uploader: run until <stream>.stop appears
    for _ in range(600):
        if os.path.exists(stream + ".stop"):
            break
        time.sleep(0.05)
if "ignore-stop" in actions:  # an uploader that never honours it (an old build, or stuck)
    time.sleep(60)
if "deliver" in actions:
    time.sleep(0.05)
    put(".delivered", "frame_id=r-f0009\ncrc32=00000000\nrun_id=%s\n" % run_id)
if "marker" in actions:
    put(".session", "frame_id=r-f0042\nlast_acked_block=17\ntotal_blocks=53\nrun_id=%s\n" % run_id)
if "stale" in actions:
    put(".exit", "run_id=ble-some-older-run\ncode=20\ndetail=from an older run\nsignature=sudden\n")
if "record" in actions:
    sig = next((a for a in actions if a in ("sudden", "slowing", "unclassified")), "")
    put(".exit", "run_id=%s\ncode=%d\ndetail=PROGRESS timeout after block 18\nframe_id=r-f0042\nlast_acked_block=17\nsignature=%s\n" % (run_id, code, sig))
if "stay" in actions:
    time.sleep(60)  # a running uploader in watch mode
print("dummy uploader exiting with", code)
sys.exit(code)
'@
$sleeper = Join-Path $work "renderer.py"
Set-Content -LiteralPath $sleeper -Encoding ASCII -Value 'import time; time.sleep(120)'

# Fast schedule so the test takes seconds, not minutes.
$BleRetryDelays = @(0.3, 0.6, 1.2, 2.4)
$BleSlowRetryDelay = 3
$BleSuddenRetryDelay = 0.4
$RendererRestartDelay = 0.2

function Invoke-Scenario {
    # $Plan: one entry per uploader start: @(<exit code>, <actions...>); the last repeats.
    param([string]$Name, [object[]]$Plan, [int]$Iterations, [switch]$Halted)
    Write-Host "== $Name"
    foreach ($ext in ".session", ".exit", ".delivered", ".stop") { Remove-Item -LiteralPath "$stream$ext" -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath (Get-RetryStatePath $haltFile) -ErrorAction SilentlyContinue
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

# 2b. Sudden failures: three automatic retries, each with the marker removed, then halt.
$s = Invoke-Scenario "sudden code 20 every time" -Plan @(, @(20, "check", "marker", "record", "sudden")) -Iterations 80
$h = Read-Halt
$log = Get-Content -Raw $SupervisorLog
$r = Get-Content -Raw (Get-RetryStatePath $haltFile) | ConvertFrom-Json
Assert-True ($s.UploaderStarts -eq 4) "1 run + 3 automatic retries (starts: $($s.UploaderStarts))"
Assert-True ($log -notmatch "code 77") "the marker was removed before every retry"
foreach ($n in 1..3) {
    Assert-True ($log -match "automatic retry $n of 3 at \d\d:\d\d:\d\d" -and $log -match "Automatic retry $n of 3: session marker removed; starting" -and $log -match "Automatic retry $n of 3 FAILED \(code 20, signature sudden\)") "retry ${n}: scheduled, started and its failure logged"
}
Assert-True ($s.Halted -and $h.class -eq "session-open" -and $h.needs_power_cycle -and $h.instruction -eq $PowerCycleInstruction) "then halted exactly as before"
Assert-True ($h.auto_retries -eq 3 -and $h.signature -eq "sudden" -and $h.reason -match "all 3 automatic retries have failed") "halt file: 3 automatic retries, signature sudden"
Assert-True ($r.status -eq "exhausted" -and $r.retry -eq 3 -and @($r.events).Count -ge 7) "retry state: exhausted, with the events ($(@($r.events).Count))"
Assert-True ($s.RendererAlive -and $s.RendererStarts -eq 1) "renderer untouched"

# 2c. A retry that delivers (and keeps running, as in watch mode) ends the incident.
$s = Invoke-Scenario "sudden code 20, then a retry that delivers" -Plan @(@(20, "marker", "record", "sudden"), @(0, "check", "deliver", "stay")) -Iterations 40
$log = Get-Content -Raw $SupervisorLog
$r = Get-Content -Raw (Get-RetryStatePath $haltFile) | ConvertFrom-Json
Assert-True ($s.UploaderStarts -eq 2 -and -not $s.Halted -and $null -eq (Read-Halt)) "one retry, not halted"
Assert-True ($log -match "Automatic retry 1 of 3 SUCCEEDED" -and $s.SuddenRetries -eq 0 -and $s.RetryInFlight -eq 0) "success logged while the uploader runs; count reset"
Assert-True ($r.status -eq "succeeded" -and $r.retry -eq 1) "retry state: succeeded"

# 2d. Slowing and unclassified failures still halt at once.
foreach ($sig in "slowing", "unclassified") {
    $s = Invoke-Scenario "code 20, signature $sig" -Plan @(, @(20, "marker", "record", $sig)) -Iterations 20
    $h = Read-Halt
    Assert-True ($s.UploaderStarts -eq 1 -and $h.class -eq "session-open" -and $h.auto_retries -eq 0 -and $h.reason -match "signature $sig, so no automatic retry") "signature ${sig}: halted at once, no retry"
}
$s = Invoke-Scenario "code 21 with a sudden-looking record" -Plan @(, @(21, "marker", "record", "sudden")) -Iterations 20
Assert-True ($s.UploaderStarts -eq 1 -and (Read-Halt).needs_power_cycle) "code 21 halts at once"

# 2e. A delivery between failures starts a new incident with 3 fresh retries.
$s = Invoke-Scenario "sudden, then deliver-and-fail, then sudden x4" `
    -Plan @(@(20, "marker", "record", "sudden"), @(20, "deliver", "marker", "record", "sudden"), @(20, "marker", "record", "sudden")) -Iterations 110
$log = Get-Content -Raw $SupervisorLog
Assert-True ($s.UploaderStarts -eq 5) "1 + 1 retry (delivered, then failed) + 3 fresh retries (starts: $($s.UploaderStarts))"
Assert-True ($log -match "Automatic retry 1 of 3 SUCCEEDED" -and (Read-Halt).auto_retries -eq 3) "the delivery reset the count; halted after 3 more"

# 2f. A retry that can't reach the keyboard (code 10) keeps its number and the normal backoff.
$s = Invoke-Scenario "sudden, then code 10, then deliver" -Plan @(@(20, "marker", "record", "sudden"), @(10, "check"), @(0, "deliver", "stay")) -Iterations 60
$log = Get-Content -Raw $SupervisorLog
Assert-True ($log -match "Automatic retry 1 of 3 has not reached the keyboard yet \(code 10\)" -and $log -match "Automatic retry 1 of 3 SUCCEEDED") "code 10 during a retry: backoff, then retry 1 succeeds"
Assert-True ($s.UploaderStarts -eq 3 -and -not $s.Halted) "not halted (starts: $($s.UploaderStarts))"

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

# A retry that was waiting when the previous supervisor stopped is not resumed.
Write-Host "== leftover waiting retry"
Remove-Item -LiteralPath $haltFile, "$stream.session" -ErrorAction SilentlyContinue
Set-Content -LiteralPath $SupervisorLog -Value "" -Encoding ASCII
Write-RetryState -HaltFile $haltFile -Status "waiting" -Retry 2 -Event "retry 2 of 3 scheduled"
$state = New-SupervisorState
try {
    Invoke-SupervisorLoop -State $state `
        -StartRenderer { Start-LoggedProcess -Name "renderer" -FilePath $python -Arguments @($sleeper) -WorkingDirectory $work -StdOut (Join-Path $logs "r.log") -StdErr (Join-Path $logs "re.log") } `
        -StartUploader { Start-LoggedProcess -Name "ble" -FilePath $python -Arguments @($sleeper) -WorkingDirectory $work -StdOut (Join-Path $logs "b.log") -StdErr (Join-Path $logs "be.log") } `
        -HaltFile $haltFile -StreamPath $stream -PollSeconds 0.1 -MaxIterations 2
} finally {
    foreach ($p in @($state.Renderer, $state.Uploader)) { try { Stop-LoggedProcess $p } catch {} }
}
$r = Get-Content -Raw (Get-RetryStatePath $haltFile) | ConvertFrom-Json
Assert-True ($r.status -eq "abandoned" -and (Get-Content -Raw $SupervisorLog) -match "it is not resumed" -and $state.RetryInFlight -eq 0) "marked abandoned and logged; the new supervisor starts fresh"
Remove-Item -LiteralPath (Get-RetryStatePath $haltFile)

# ---- Clean stop (docs/design/CLEAN_STOP.md) ------------------------------------------

function Invoke-StopScenario {
    # Runs the real loop while a background job plays STOP_STACK.ps1's part on <stream>.stop.
    param([string]$Name, [object[]]$Plan, [int]$Iterations, [scriptblock]$Job, [switch]$StaleRequest)
    Write-Host "== $Name"
    foreach ($ext in ".session", ".exit", ".delivered", ".stop") { Remove-Item -LiteralPath "$stream$ext" -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $haltFile, (Get-RetryStatePath $haltFile) -ErrorAction SilentlyContinue
    Set-Content -LiteralPath $SupervisorLog -Value "" -Encoding ASCII
    if ($StaleRequest) { Set-Content -LiteralPath "$stream.stop" -Value "requested_at=yesterday" -Encoding ASCII }

    $state = New-SupervisorState
    $starts = New-Object System.Collections.ArrayList
    $startUploader = {
        $step = @($Plan[[Math]::Min($starts.Count, $Plan.Count - 1)])
        [void]$starts.Add((Get-Date))
        Start-LoggedProcess -Name "ble" -FilePath $python -Arguments (@($dummy, $stream) + ($step | ForEach-Object { "$_" })) `
            -WorkingDirectory $work -StdOut (Join-Path $logs "ble.log") -StdErr (Join-Path $logs "ble_error.log")
    }
    $startRenderer = {
        Start-LoggedProcess -Name "renderer" -FilePath $python -Arguments @($sleeper) `
            -WorkingDirectory $work -StdOut (Join-Path $logs "renderer.log") -StdErr (Join-Path $logs "renderer_error.log")
    }
    $bg = if ($Job) { Start-Job -ScriptBlock $Job -ArgumentList "$stream.stop" }
    try {
        Invoke-SupervisorLoop -State $state -StartRenderer $startRenderer -StartUploader $startUploader `
            -HaltFile $haltFile -StreamPath $stream -PollSeconds 0.1 -MaxIterations $Iterations
        $state | Add-Member -NotePropertyName RendererAlive -NotePropertyValue (-not $state.Renderer.HasExited)
    } finally {
        foreach ($p in @($state.Renderer, $state.Uploader)) { try { Stop-LoggedProcess $p } catch {} }
        if ($bg) { Remove-Job $bg -Force }
        Remove-Item -LiteralPath "$stream.stop" -ErrorAction SilentlyContinue
    }
    return $state
}

# Wait until the uploader is running, then request the stop.
$requestStop = { param($f) Start-Sleep -Milliseconds 500; Set-Content -LiteralPath $f -Value "requested_at=now" -Encoding ASCII }

$s = Invoke-StopScenario "stop requested while idle" -Plan @(, @(0, "wait-stop")) -Iterations 60 -Job $requestStop -StaleRequest
$log = Get-Content -Raw $SupervisorLog
Assert-True ($log -match "Removed a stale stop request") "a stop request left at supervisor start is removed and logged"
Assert-True ($s.UploaderStarts -eq 1) "uploader started once and not restarted after its requested exit (starts: $($s.UploaderStarts))"
Assert-True ($log -match "exited with code 0" -and $log -match "stopped on request") "supervisor logs the requested stop"
Assert-True ($log -notmatch "Restarting BLE uploader") "no restart is scheduled"
Assert-True (-not $s.Halted -and $null -eq (Read-Halt)) "a requested stop is not a halt"
Assert-True ($s.RendererAlive -and $s.RendererStarts -eq 1) "renderer untouched"

$s = Invoke-StopScenario "stop requested; the upload in progress fails on its way out" `
    -Plan @(, @(20, "wait-stop", "marker", "record")) -Iterations 60 -Job $requestStop
$h = Read-Halt
Assert-True ($s.UploaderStarts -eq 1 -and $s.Halted -and $h.class -eq "session-open" -and $h.needs_power_cycle) "it still halts and asks for a power cycle"

$withdraw = { param($f) Start-Sleep -Milliseconds 500; Set-Content -LiteralPath $f -Value "x" -Encoding ASCII; Start-Sleep -Milliseconds 2500; Remove-Item -LiteralPath $f }
$s = Invoke-StopScenario "stop request withdrawn while the supervisor still runs" -Plan @(, @(0, "wait-stop")) -Iterations 110 -Job $withdraw
Assert-True ($s.UploaderStarts -eq 2) "uploader started again once the request was gone (starts: $($s.UploaderStarts))"
Assert-True ((Get-Content -Raw $SupervisorLog) -match "Stop request withdrawn") "withdrawal is logged"

# Clean stop meets the automatic retry (#28).
$stopLater = { param($f) Start-Sleep -Milliseconds 1500; Set-Content -LiteralPath $f -Value "requested_at=now" -Encoding ASCII }
$s = Invoke-StopScenario "stop while an automatic retry is running" `
    -Plan @(@(20, "marker", "record", "sudden"), @(0, "check", "wait-stop")) -Iterations 50 -Job $stopLater
$log = Get-Content -Raw $SupervisorLog
$r = Get-Content -Raw (Get-RetryStatePath $haltFile) | ConvertFrom-Json
Assert-True ($s.UploaderStarts -eq 2 -and -not $s.Halted -and $null -eq (Read-Halt)) "retry 1 started, then stopped on request; no halt (starts: $($s.UploaderStarts))"
Assert-True ($log -match "Automatic retry 1 of 3 ended by a stop request before it delivered a frame; not counted as a failure" -and $log -notmatch "Automatic retry 1 of 3 FAILED") "the exit on request is not a failed retry"
Assert-True ($s.SuddenRetries -eq 1 -and $s.RetryInFlight -eq 0 -and -not $s.RetryPending -and $r.status -eq "cancelled") "count unchanged, nothing pending, retry state cancelled"
Assert-True ($log -notmatch "Restarting BLE uploader" -and $s.Attempts -eq 1) "no restart, and the stop added no failed attempt"

$BleSuddenRetryDelay = 3
$stopSoon = { param($f) Start-Sleep -Milliseconds 800; Set-Content -LiteralPath $f -Value "requested_at=now" -Encoding ASCII }
$s = Invoke-StopScenario "stop during the wait for an automatic retry" -Plan @(, @(20, "marker", "record", "sudden")) -Iterations 50 -Job $stopSoon
$log = Get-Content -Raw $SupervisorLog
Assert-True ($s.UploaderStarts -eq 1 -and $s.RetryPending) "the waiting retry did not start while the request stood (starts: $($s.UploaderStarts))"
Assert-True ($log -match "Stop requested while automatic retry 1 of 3 is waiting; it will not start while the request stands") "the hold is logged"
$msg = Stop-PendingSuddenRetry -HaltFile $haltFile
$r = Get-Content -Raw (Get-RetryStatePath $haltFile) | ConvertFrom-Json
Assert-True ($msg -match "automatic retry 1 of 3 was waiting; cancelled" -and $r.status -eq "cancelled" -and @($r.events)[-1] -match "cancelled by STOP_STACK") "STOP_STACK's cleanup records the cancelled wait"
Assert-True ($null -eq (Stop-PendingSuddenRetry -HaltFile $haltFile)) "cancelling twice does nothing"
$BleSuddenRetryDelay = 0.4

$s = Invoke-StopScenario "stop requested; the upload in progress fails suddenly on its way out" `
    -Plan @(, @(20, "wait-stop", "marker", "record", "sudden")) -Iterations 40 -Job $requestStop
$log = Get-Content -Raw $SupervisorLog
Assert-True ($s.UploaderStarts -eq 1 -and -not $s.Halted -and $s.RetryPending -and $s.SuddenRetries -eq 1) "a real sudden failure is still scheduled for its retry, held by the request"
Assert-True ($log -match "automatic retry 1 of 3 at" -and $log -notmatch "stopped on request") "logged as the sudden failure it is"

# Invoke-StackStop is STOP_STACK.ps1's logic. The task and the force-kill are stand-ins:
# nothing here touches the real scheduled task or kills anything but its own dummies.
Write-Host "== Invoke-StackStop"
function Start-Dummy {
    param([string[]]$Actions)
    $argLine = (@($dummy, $stream, "0") + $Actions | ForEach-Object { '"' + $_ + '"' }) -join ' '
    $p = Start-Process -FilePath $python -ArgumentList $argLine -WindowStyle Hidden -PassThru
    $null = $p.Handle
    Start-Sleep -Milliseconds 300
    return $p
}
Remove-Item -LiteralPath "$stream.stop" -ErrorAction SilentlyContinue
Set-Content -LiteralPath $SupervisorLog -Value "" -Encoding ASCII

$p = Start-Dummy @("wait-stop")
$calls = New-Object System.Collections.ArrayList
$r = Invoke-StackStop -StreamPath $stream -GetUploaderProcessIds { $p.Id } `
    -StopTask { [void]$calls.Add("task(uploader exited: $($p.HasExited))") } -ForceKillAll { [void]$calls.Add("kill") } `
    -TimeoutSeconds 15 -PollSeconds 0.1 6>$null
Assert-True ($r.Outcome -eq "clean" -and $p.HasExited -and $p.ExitCode -eq 0) "a cooperating uploader exits 0 on request ($($r.Outcome), $('{0:n1}' -f $r.Seconds) s)"
Assert-True (($calls -join ",") -eq "task(uploader exited: True),kill") "task stopped and force-kill run only after the uploader exited: $($calls -join ',')"
Assert-True (-not (Test-Path "$stream.stop")) "the request is withdrawn afterwards"
Assert-True ((Get-Content -Raw $SupervisorLog) -match "STOP_STACK: BLE uploader exited on request after") "clean exit logged"

$p = Start-Dummy @("ignore-stop")
$r = Invoke-StackStop -StreamPath $stream -GetUploaderProcessIds { $p.Id } -StopTask {} `
    -ForceKillAll { Stop-Process -Id $p.Id -Force } -TimeoutSeconds 1 -PollSeconds 0.1 6>$null
[void]$p.WaitForExit(5000)
Assert-True ($r.Outcome -eq "timeout" -and $r.Seconds -ge 1 -and @($r.ProcessIds) -contains $p.Id) "an uploader that ignores the request times out"
Assert-True ($p.HasExited) "and is force-killed afterwards"
Assert-True ((Get-Content -Raw $SupervisorLog) -match "did not exit within 1 s; force-killing it \(pid $($p.Id)\)") "fallback logged with the pid"
Assert-True (-not (Test-Path "$stream.stop")) "the request is withdrawn after a timeout too"

$sawFile = $null
$r = Invoke-StackStop -StreamPath $stream -GetUploaderProcessIds { @() } `
    -StopTask { $script:sawFile = Test-Path "$stream.stop" } -ForceKillAll {} -TimeoutSeconds 5 6>$null
Assert-True ($r.Outcome -eq "not-running" -and $sawFile -eq $false) "no uploader: no request written, no wait"
Assert-True ((Get-Content -Raw $SupervisorLog) -match "no BLE uploader was running") "that is logged too"

# The uploader exits while asked, but by failing the upload it was finishing.
Remove-Item -LiteralPath "$stream.exit" -ErrorAction SilentlyContinue
$p = Start-Dummy @("wait-stop", "record", "sudden")
$r = Invoke-StackStop -StreamPath $stream -GetUploaderProcessIds { $p.Id } -StopTask {} -ForceKillAll {} -TimeoutSeconds 15 -PollSeconds 0.1 6>$null
Assert-True ($r.Outcome -eq "failed" -and $r.Signature -eq "sudden") "an exit that wrote an exit record after the request is reported as failed ($($r.Outcome))"
Assert-True ((Get-Content -Raw $SupervisorLog) -match "exited after .* s, but not on request: the upload it was finishing failed \(code 0, signature sudden: PROGRESS timeout") "and logged as such, not as 'exited on request'"
$p = Start-Dummy @("wait-stop")
$r = Invoke-StackStop -StreamPath $stream -GetUploaderProcessIds { $p.Id } -StopTask {} -ForceKillAll {} -TimeoutSeconds 15 -PollSeconds 0.1 6>$null
Assert-True ($r.Outcome -eq "clean") "an older exit record does not make a later clean exit look failed"
Remove-Item -LiteralPath "$stream.exit" -ErrorAction SilentlyContinue

# A waiting automatic retry is cancelled by the stop (the supervisor that held it is gone).
Write-RetryState -HaltFile $haltFile -Status "waiting" -Retry 2 -Event "retry 2 of 3 scheduled"
Set-Content -LiteralPath $SupervisorLog -Value "" -Encoding ASCII
$r = Invoke-StackStop -StreamPath $stream -GetUploaderProcessIds { @() } -StopTask {} -ForceKillAll {} -HaltFile $haltFile -TimeoutSeconds 5 6>$null
$rs = Get-Content -Raw (Get-RetryStatePath $haltFile) | ConvertFrom-Json
Assert-True ($r.Outcome -eq "not-running" -and $rs.status -eq "cancelled") "a stop during a retry wait cancels it"
Assert-True ((Get-Content -Raw $SupervisorLog) -match "STOP_STACK: automatic retry 2 of 3 was waiting; cancelled") "and says so"
Remove-Item -LiteralPath (Get-RetryStatePath $haltFile)

# STOP_STACK.ps1 itself would stop the real scheduled task, so its text is checked instead.
Write-Host "== STOP_STACK.ps1"
foreach ($script in @("STOP_STACK.ps1", "installer\runtime\STOP_STACK.ps1")) {
    $text = Get-Content -Raw (Join-Path $RepoRoot $script)
    $invoke = $text.IndexOf("Invoke-StackStop")
    Assert-True ($invoke -ge 0) "$script uses Invoke-StackStop"
    $first = $text.IndexOf("Stop-ScheduledTask")
    $kill = $text.IndexOf("Stop-Process")
    Assert-True ($first -gt $invoke -and $kill -gt $invoke) "$script stops the task and kills only inside Invoke-StackStop's stand-ins"
    Assert-True ($text -match "session_01_bulk_stream\.bin") "$script names the stream the uploader watches"
    Assert-True ($text -match '-HaltFile \(Join-Path \$Logs "uploader_halted\.json"\)') "$script passes the halt file, so a waiting retry is cancelled"
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
# Halt after automatic retries: STATUS says so.
$decision = Get-UploaderDecision -ExitCode 20 -Session @{ frame_id = "r-f0042" } -Attempt 4 -Signature "sudden" -SuddenRetries 3
Write-UploaderHaltState -Path (Join-Path $root "logs\uploader_halted.json") -Decision $decision -ExitCode 20 `
    -Session @{ frame_id = "r-f0042"; last_acked_block = "51"; total_blocks = "53" } `
    -ExitRecord @{ detail = "timed out waiting for PROGRESS after 4K block 52/53"; signature = "sudden" } -Attempts 4 -AutoRetries 3
$out = (& (Join-Path $root "STATUS_STACK.ps1") 6>&1 | Out-String -Width 400)
Assert-True ($out -match "Status\s+: HALTED" -and $out -match "Signature\s+: sudden" -and $out -match "Auto-retries\s+: 3 automatic retries used before halting") "a halt after retries shows the signature and the retries"
Remove-Item (Join-Path $root "logs\uploader_halted.json")

# A waiting retry outranks the open session marker that goes with it.
$retryState = Join-Path $root "logs\uploader_retry.json"
Set-Content -LiteralPath "$fakeStream.session" -Encoding ASCII -Value "frame_id=r-f0042", "last_acked_block=26", "total_blocks=53"
Write-RetryState -HaltFile (Join-Path $root "logs\uploader_halted.json") -Status "waiting" -Retry 1 -LastFailure "code 20, signature sudden, last ACKed block 26" `
    -NextAttemptAt ((Get-Date).AddSeconds(120)) -Event "sudden failure; retry 1 of 3 scheduled"
$out = (& (Join-Path $root "STATUS_STACK.ps1") 6>&1 | Out-String -Width 400)
Assert-True ($out -match "Status\s+: RETRYING - sudden upload failure; automatic retry 1 of 3 at \d\d:\d\d:\d\d" -and $out -notmatch "HALTED") "a waiting retry is shown as RETRYING, with its time"
Assert-True ($out -match "Auto-retry\s+: waiting, retry 1 of 3; last failure: code 20, signature sudden, last ACKed block 26" -and $out -match "retry 1 of 3 scheduled") "the retry line and its events are shown"
Write-RetryState -HaltFile (Join-Path $root "logs\uploader_halted.json") -Status "succeeded" -Retry 1 -Event "retry 1 of 3 succeeded (frame delivered)"
Remove-Item -LiteralPath "$fakeStream.session"
$out = (& (Join-Path $root "STATUS_STACK.ps1") 6>&1 | Out-String -Width 400)
Assert-True ($out -notmatch "RETRYING" -and $out -match "Auto-retry\s+: succeeded, retry 1 of 3") "a finished retry is history, not the status"
Remove-Item -LiteralPath $retryState

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
    $clearStop = $text.IndexOf("Clear-UploaderStopRequest")
    Assert-True ($clearStop -ge 0 -and $clearStop -lt $start) "$script withdraws a leftover stop request before Start-ScheduledTask"
}

Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
if ($failures) { Write-Host "$failures failure(s)"; exit 1 }
Write-Host "All supervisor retry checks passed."
