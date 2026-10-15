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
Assert-True ((Decide 10 5).Action -eq "halt" -and (Decide 10 5).Class -eq "gave-up") "code 10 gives up on the 5th failure"
Assert-True (-not (Decide 10 5).NeedsPowerCycle) "giving up does not ask for a power cycle"
Assert-True ((Decide 11 2).Action -eq "retry" -and (Decide 11 3).Class -eq "gave-up") "code 11 has a lower cap ($BleGattRetryCap retries)"
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

# 1. Retryable failures back off, then give up.
$s = Invoke-Scenario "code 10 every time" -Plan @(, @(10)) -Iterations 90
$gaps = @(); for ($i = 1; $i -lt $s.Starts.Count; $i++) { $gaps += ($s.Starts[$i] - $s.Starts[$i - 1]).TotalSeconds }
Assert-True ($s.UploaderStarts -eq 5) "uploader started 5 times (1 + 4 retries), got $($s.UploaderStarts)"
Assert-True ($gaps.Count -eq 4 -and $gaps[1] -gt $gaps[0] -and $gaps[2] -gt $gaps[1] -and $gaps[3] -gt $gaps[2]) "gaps grow: $(($gaps | ForEach-Object { '{0:n2}' -f $_ }) -join ', ')s"
$h = Read-Halt
Assert-True ($s.Halted -and $h.class -eq "gave-up" -and $h.exit_code -eq 10 -and -not $h.needs_power_cycle) "halted as gave-up, no power cycle"
Assert-True ($s.RendererAlive -and $s.RendererStarts -eq 1) "renderer untouched"

# 2. Interrupted transfer: no restart, renderer keeps running, halt file complete.
$s = Invoke-Scenario "code 20 with marker and exit record" -Plan @(, @(20, "marker", "record")) -Iterations 25
$h = Read-Halt
Assert-True ($s.UploaderStarts -eq 1) "uploader not restarted (starts: $($s.UploaderStarts))"
Assert-True ($s.RendererAlive -and $s.RendererStarts -eq 1) "renderer still running"
Assert-True ($null -ne $h) "halt file written"
Assert-True ($h.exit_code -eq 20 -and $h.class -eq "session-open" -and $h.needs_power_cycle) "halt: code 20, session-open, needs power cycle"
Assert-True ($h.frame_id -eq "r-f0042" -and $h.last_acked_block -eq 17 -and $h.total_blocks -eq 53) "halt: frame r-f0042, block 17/53"
Assert-True ($h.detail -match "PROGRESS timeout" -and $h.halted_at -and $h.instruction -match "Power-cycle the keyboard") "halt: detail, time and plain-English instruction"
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
$s = Invoke-Scenario "delivery resets the counter" -Plan @(@(10), @(10), @(10, "deliver"), @(10)) -Iterations 140
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
