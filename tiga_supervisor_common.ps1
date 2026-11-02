# Helpers shared by the development scripts (repository root) and the packaged ones
# (installer/runtime). The scripts themselves hold what differs: paths, process patterns
# and how to start each child. Dot-source it after setting $SupervisorLog, if logging:
#
#   . (Join-Path $PSScriptRoot "tiga_supervisor_common.ps1")
#
# Covered offline by tests/test_supervisor_logging.ps1 and tests/test_supervisor_retry.ps1.

# Size-based rotation: <log> -> <log>.1 -> ... -> <log>.$LogKeep, oldest dropped.
$LogMaxBytes = 5MB
$LogKeep     = 3
$Utf8NoBom   = New-Object System.Text.UTF8Encoding $false

function Add-LogLine {
    param([string]$Path, [string]$Line)
    [System.IO.File]::AppendAllText($Path, $Line + "`r`n", $Utf8NoBom)
}

function Write-SupervisorLog {
    param([string]$Message)
    Add-LogLine $SupervisorLog ("{0:yyyy-MM-dd HH:mm:ss} {1}" -f (Get-Date), $Message)
}

function Invoke-LogRotation {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return }
    if ((Get-Item -LiteralPath $Path).Length -lt $LogMaxBytes) { return }
    for ($i = $LogKeep - 1; $i -ge 1; $i--) {
        if (Test-Path -LiteralPath "$Path.$i") {
            Move-Item -LiteralPath "$Path.$i" -Destination "$Path.$($i + 1)" -Force
        }
    }
    Move-Item -LiteralPath $Path -Destination "$Path.1" -Force
}

function New-RunId {
    param([string]$Name)
    "{0}-{1:yyyyMMdd-HHmmss}-{2}" -f $Name, (Get-Date), [guid]::NewGuid().ToString("N").Substring(0, 6)
}

function Get-LoggedCommandLine {
    # Every element is quoted, so paths containing spaces survive (issue #8).
    param([string]$FilePath, [string[]]$Arguments, [string]$StdOut, [string]$StdErr)
    $parts = @($FilePath) + @($Arguments) | ForEach-Object { '"' + $_ + '"' }
    return ($parts -join ' ') + ' >> "' + $StdOut + '" 2>> "' + $StdErr + '"'
}

function Start-LoggedProcess {
    # Start-Process -RedirectStandardOutput truncates its target, which destroyed the
    # output of every failed run on restart. cmd.exe '>>' appends instead.
    param(
        [string]$Name,
        [string]$FilePath,
        [string[]]$Arguments,
        [string]$WorkingDirectory,
        [string]$StdOut,
        [string]$StdErr
    )

    $runId = New-RunId $Name
    $stamp = "===== {0:yyyy-MM-dd HH:mm:ss zzz} {1} run={2} start =====" -f (Get-Date), $Name, $runId
    foreach ($log in @($StdOut, $StdErr)) {
        Invoke-LogRotation $log
        Add-LogLine $log $stamp
    }

    $cmdLine = Get-LoggedCommandLine $FilePath $Arguments $StdOut $StdErr

    # Inherited by the child: the renderer and uploader both include it in their logs.
    $env:TIGA_RUN_ID = $runId
    $env:PYTHONUNBUFFERED = "1"

    $proc = Start-Process `
        -FilePath $env:ComSpec `
        -ArgumentList "/d /s /c `"$cmdLine`"" `
        -WorkingDirectory $WorkingDirectory `
        -WindowStyle Hidden `
        -PassThru

    # Touch the handle now; otherwise ExitCode is not available after the process exits.
    $null = $proc.Handle
    # Lets the caller match the child's own records (the uploader's <stream>.exit) to this run.
    $proc | Add-Member -NotePropertyName TigaRunId -NotePropertyValue $runId
    $proc | Add-Member -NotePropertyName TigaStartedAt -NotePropertyValue (Get-Date)

    Write-SupervisorLog "Started $Name run=$runId pid=$($proc.Id)"
    return $proc
}

function Stop-LoggedProcess {
    param($Proc)
    if (($null -ne $Proc) -and (-not $Proc.HasExited)) {
        # /T: the tracked process is the cmd.exe wrapper; take its child with it.
        & taskkill.exe /PID $Proc.Id /T /F 2>&1 | Out-Null
    }
}

# ---- Uploader failure policy (issue #4) -------------------------------------------
#
# The uploader (windows_ble_probe/src/bin/tiga_stream_watch_flow/) exits with a
# class code and keeps <stream>.session while a media session is open on the keyboard:
#
#   10 unreachable, 11 GATT          -> retry with backoff, then every 5 min, never halt
#   20 with the "sudden" signature   -> automatic retry after 120 s, up to 3 per incident,
#                                       then halt as below
#   20 otherwise, 21 no completion   -> halt: the keyboard needs a power cycle
#   30 bad arguments / stream        -> halt: configuration problem
#   anything else, no marker         -> retry with backoff, then give up
#
# 10 and 11 never open a media session, so the keyboard is not in a bad state; it is
# off, asleep or out of range (11: no candidate had the media service, issue #14).
# Halting on those meant a dead display the next morning until START_STACK.ps1.
#
# The sudden-failure retry (approved by Michi on 2026-10-18; evidence in
# docs/investigations/UPLOAD_FAILURES_2026-10-18.md, PR #24): a code 20 whose exit
# record says signature=sudden (a normal pace, then no PROGRESS) is not a stuck
# keyboard. Uploads after such a failure failed 5 of 12, the same rate as any first
# upload on a connection (4 of 12), and plain retries without a power-off worked 2 of 4.
# So the supervisor removes the session marker and starts the uploader again after
# 120 s, at most 3 times per incident; a delivered frame ends the incident. Slowing or
# unclassified failures, 21, and a marker without this run's exit record still halt
# at once: the slowing pair of 2026-10-15 is the one hint of a state that persists.
#
# A surviving session marker otherwise means "halt, power-cycle" whatever the exit
# code: a killed or crashed uploader cannot report its own class.
#
# Halting writes logs\uploader_halted.json and leaves the renderer running.
# START_STACK.ps1 deletes the halt file (and the marker); a running supervisor notices
# and starts the uploader again. Nothing clears a halt on a timer. Retry progress is
# written to logs\uploader_retry.json for STATUS_STACK.ps1.

$BleRetryDelays       = @(8, 16, 32, 60)  # seconds before retries 1..4
$BleSlowRetryDelay    = 300               # 10 and 11 after the backoff: every 5 min, indefinitely
$BleSlowRetryCodes    = @(10, 11)
$BleSuddenRetryDelay  = 120               # seconds before each automatic retry after a sudden code 20
$BleSuddenRetryMax    = 3                 # automatic retries per incident before halting
$RendererRestartDelay = 8

$UploaderExitMeanings = @{
    0  = "success"
    10 = "keyboard not reachable (no adapter, not found, connect failed, or link lost before the media session opened)"
    11 = "connected, but the keyboard's GATT service was missing or unusable"
    20 = "transfer interrupted after CONTROL2; the keyboard may hold a partial media session"
    21 = "all blocks sent but the keyboard never confirmed completion"
    30 = "bad arguments, missing or invalid stream, or internal error"
}

# The TIGA has no power switch, and unplugging USB does not reset it: it keeps running on
# its battery. On 2026-10-15 a 10-15 s battery disconnect did not clear a stuck keyboard;
# 30 s did (one observation each; PROJECT_HISTORY section 7.1).
$PowerCycleInstruction = "Disconnect the keyboard's battery for at least 30 s (there is no power switch; " +
    "unplugging USB doesn't count), then run START_STACK.ps1."

function Get-UploaderExitMeaning {
    param([int]$Code)
    if ($UploaderExitMeanings.ContainsKey($Code)) { return $UploaderExitMeanings[$Code] }
    return "unexpected exit (crashed or killed)"
}

function Read-KeyValueFile {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    $map = @{}
    foreach ($line in [System.IO.File]::ReadAllLines($Path)) {
        $i = $line.IndexOf("=")
        if ($i -gt 0) { $map[$line.Substring(0, $i).Trim()] = $line.Substring($i + 1).Trim() }
    }
    return $map
}

function New-UploaderDecision {
    param([string]$Action, [string]$Class, [bool]$NeedsPowerCycle, [double]$Delay, [string]$Reason, [string]$Instruction)
    [pscustomobject]@{
        Action = $Action; Class = $Class; NeedsPowerCycle = $NeedsPowerCycle
        Delay = $Delay; Reason = $Reason; Instruction = $Instruction
    }
}

function Get-UploaderDecision {
    # What to do after the uploader exited. No side effects.
    # $Attempt counts consecutive failed runs including this one; a successful upload resets it.
    # $Signature is this run's exit record's signature (sudden, slowing, unclassified or "");
    # $SuddenRetries is how many automatic retries this incident has already used.
    # $StopRequested: STOP_STACK.ps1 asked it to stop (<stream>.stop). A failure still wins:
    # a sudden code 20 is still scheduled for its automatic retry (which then waits while
    # the request stands, and is cancelled by STOP_STACK), and anything else that needs a
    # power cycle still halts. Only an exit that isn't a failure becomes "stopped-on-request".
    param([int]$ExitCode, $Session, [int]$Attempt, [string]$Signature = "", [int]$SuddenRetries = 0, [switch]$StopRequested)

    $meaning = Get-UploaderExitMeaning $ExitCode

    if ($ExitCode -eq 20 -and $Signature -eq "sudden") {
        if ($SuddenRetries -lt $BleSuddenRetryMax) {
            $n = $SuddenRetries + 1
            return New-UploaderDecision retry "sudden-retry" $false $BleSuddenRetryDelay `
                "code ${ExitCode}: $meaning; signature sudden: automatic retry $n of $BleSuddenRetryMax in $BleSuddenRetryDelay s" ""
        }
        return New-UploaderDecision halt "session-open" $true 0 `
            ("uploader exited (code ${ExitCode}: $meaning) while a media session was open on the keyboard; " +
             "signature sudden, and all $BleSuddenRetryMax automatic retries have failed") `
            $PowerCycleInstruction
    }

    $why = if ($ExitCode -eq 20 -and $Signature) { "; signature $Signature, so no automatic retry" } else { "" }
    if ($null -ne $Session) {
        return New-UploaderDecision halt "session-open" $true 0 `
            "uploader exited (code ${ExitCode}: $meaning) while a media session was open on the keyboard$why" `
            $PowerCycleInstruction
    }
    if ($ExitCode -eq 20 -or $ExitCode -eq 21) {
        return New-UploaderDecision halt "session-open" $true 0 "code ${ExitCode}: $meaning$why" $PowerCycleInstruction
    }
    if ($ExitCode -eq 30) {
        return New-UploaderDecision halt "fatal" $false 0 "code ${ExitCode}: $meaning" `
            "Check logs\ble_error.log for the cause and fix it, then run START_STACK.ps1. The keyboard was not touched."
    }
    if ($StopRequested) {
        return New-UploaderDecision stop "stopped-on-request" $false 0 "code ${ExitCode}: $meaning; a stop was requested" ""
    }

    if ($ExitCode -ne 0 -and $Attempt -gt $BleRetryDelays.Count) {
        if ($BleSlowRetryCodes -contains $ExitCode) {
            return New-UploaderDecision retry "slow-retry" $false $BleSlowRetryDelay `
                "code ${ExitCode}: $meaning; still not reachable after $Attempt attempts, retrying every $BleSlowRetryDelay s" ""
        }
        return New-UploaderDecision halt "gave-up" $false 0 `
            "gave up after $Attempt failed attempts; last was code ${ExitCode}: $meaning" `
            "Check that Bluetooth is on and the keyboard is awake and in range, then run START_STACK.ps1. No media session was opened."
    }

    $index = [Math]::Min([Math]::Max($Attempt, 1), $BleRetryDelays.Count) - 1
    return New-UploaderDecision retry "retryable" $false $BleRetryDelays[$index] "code ${ExitCode}: $meaning" ""
}

function Get-RetryStatePath {
    param([string]$HaltFile)
    return Join-Path (Split-Path -Parent $HaltFile) "uploader_retry.json"
}

function Write-RetryState {
    # logs\uploader_retry.json: where the current sudden-failure incident stands, plus the
    # last 20 retry events, for STATUS_STACK.ps1. Status: waiting, running, succeeded,
    # failed (one retry failed; the next decision follows), exhausted (halted), abandoned.
    param([string]$HaltFile, [string]$Status, [int]$Retry, [string]$Event, [string]$LastFailure = "", $NextAttemptAt = $null)
    $path = Get-RetryStatePath $HaltFile
    $events = @()
    $previousFailure = ""
    if (Test-Path -LiteralPath $path) {
        try {
            $old = [System.IO.File]::ReadAllText($path) | ConvertFrom-Json
            if ($old.events) { $events = @($old.events) }
            if ($old.last_failure) { $previousFailure = [string]$old.last_failure }
        } catch {}
    }
    $events += ("{0:yyyy-MM-dd HH:mm:ss} {1}" -f (Get-Date), $Event)
    if ($events.Count -gt 20) { $events = $events[($events.Count - 20)..($events.Count - 1)] }
    $state = [ordered]@{
        updated_at      = (Get-Date).ToString("yyyy-MM-ddTHH:mm:sszzz")
        status          = $Status
        retry           = $Retry
        max_retries     = $BleSuddenRetryMax
        next_attempt_at = if ($NextAttemptAt) { ([datetime]$NextAttemptAt).ToString("yyyy-MM-ddTHH:mm:sszzz") } else { $null }
        last_failure    = if ($LastFailure) { $LastFailure } else { $previousFailure }
        events          = $events
    }
    [System.IO.File]::WriteAllText($path, ($state | ConvertTo-Json), $Utf8NoBom)
}

function Complete-SuddenRetry {
    # A frame was delivered by the uploader started as automatic retry N: the incident is over.
    param($State, [string]$HaltFile)
    $n = $State.RetryInFlight
    Write-SupervisorLog "Automatic retry $n of $BleSuddenRetryMax SUCCEEDED: a frame was delivered. Automatic-retry count reset."
    Write-RetryState -HaltFile $HaltFile -Status "succeeded" -Retry $n -Event "retry $n of $BleSuddenRetryMax succeeded (frame delivered)"
    $State.RetryInFlight = 0
    $State.SuddenRetries = 0
}

function Write-UploaderHaltState {
    param([string]$Path, $Decision, [int]$ExitCode, $Session, $ExitRecord, [int]$Attempts, [int]$AutoRetries = 0)
    $source = if ($null -ne $Session) { $Session } elseif ($null -ne $ExitRecord) { $ExitRecord } else { @{} }
    $block = $source["last_acked_block"]
    $state = [ordered]@{
        halted_at         = (Get-Date).ToString("yyyy-MM-ddTHH:mm:sszzz")
        exit_code         = $ExitCode
        meaning           = Get-UploaderExitMeaning $ExitCode
        class             = $Decision.Class
        needs_power_cycle = $Decision.NeedsPowerCycle
        reason            = $Decision.Reason
        frame_id          = $source["frame_id"]
        last_acked_block  = if ($block) { [int]$block } else { $null }
        total_blocks      = if ($Session -and $Session["total_blocks"]) { [int]$Session["total_blocks"] } else { $null }
        detail            = if ($ExitRecord) { $ExitRecord["detail"] } else { $null }
        signature         = if ($ExitRecord -and $ExitRecord["signature"]) { $ExitRecord["signature"] } else { $null }
        attempts          = $Attempts
        auto_retries      = $AutoRetries
        instruction       = $Decision.Instruction
    }
    [System.IO.File]::WriteAllText($Path, ($state | ConvertTo-Json), $Utf8NoBom)
}

function New-SupervisorState {
    [pscustomobject]@{
        Renderer = $null; Uploader = $null
        NextRendererStart = [datetime]::MinValue; NextUploaderStart = [datetime]::MinValue
        Attempts = 0; Halted = $false; RendererStarts = 0; UploaderStarts = 0; StopHeld = $false
        # Sudden-failure retries: used in this incident, one waiting to start, one running.
        SuddenRetries = 0; RetryPending = $false; RetryInFlight = 0
    }
}

function Resolve-UploaderExit {
    param($State, [string]$HaltFile, [string]$StreamPath)

    $proc = $State.Uploader
    $State.Uploader = $null
    $code = if ($null -eq $proc.ExitCode) { -1 } else { [int]$proc.ExitCode }

    $session = Read-KeyValueFile "$StreamPath.session"
    $stopRequested = Test-Path -LiteralPath (Get-StopRequestPath $StreamPath)
    $exitRecord = Read-KeyValueFile "$StreamPath.exit"
    if ($exitRecord -and $exitRecord["run_id"] -ne $proc.TigaRunId) { $exitRecord = $null }  # an older run's

    $delivered = Get-Item -LiteralPath "$StreamPath.delivered" -ErrorAction SilentlyContinue
    if ($delivered -and $delivered.LastWriteTime -gt $proc.TigaStartedAt) {
        if ($State.Attempts -gt 0) { Write-SupervisorLog "Uploader delivered a frame during this run; retry counter reset." }
        $State.Attempts = 0
        if ($State.RetryInFlight -gt 0) {
            Complete-SuddenRetry -State $State -HaltFile $HaltFile
        } elseif ($State.SuddenRetries -gt 0) {
            Write-SupervisorLog "Uploader delivered a frame during this run; automatic-retry count reset."
            $State.SuddenRetries = 0
        }
    }
    if ($code -ne 0) { $State.Attempts++ }

    $signature = if ($exitRecord) { [string]$exitRecord["signature"] } else { "" }
    $detail = if ($exitRecord) { " Detail: $($exitRecord["detail"])" } else { "" }
    $sigText = if ($signature) { " Signature: $signature." } else { "" }
    Write-SupervisorLog "BLE uploader exited with code $code ($(Get-UploaderExitMeaning $code)).$detail$sigText"

    if ($State.RetryInFlight -gt 0) {
        $n = $State.RetryInFlight
        if ($stopRequested -and $code -eq 0) {
            # Exited on request before delivering a frame: neither a success nor a failure.
            Write-SupervisorLog "Automatic retry $n of $BleSuddenRetryMax ended by a stop request before it delivered a frame; not counted as a failure."
            Write-RetryState -HaltFile $HaltFile -Status "cancelled" -Retry $n -Event "retry $n of $BleSuddenRetryMax ended by a stop request (not a failure)"
            $State.RetryInFlight = 0
        } elseif ($BleSlowRetryCodes -contains $code) {
            # No session was opened; the retry is still pending its outcome.
            Write-SupervisorLog "Automatic retry $n of $BleSuddenRetryMax has not reached the keyboard yet (code $code); the normal reconnect backoff applies and it still counts as retry $n."
        } else {
            $what = if ($signature) { "code $code, signature $signature" } else { "code $code" }
            Write-SupervisorLog "Automatic retry $n of $BleSuddenRetryMax FAILED ($what)."
            Write-RetryState -HaltFile $HaltFile -Status "failed" -Retry $n -Event "retry $n of $BleSuddenRetryMax failed ($what)"
            $State.RetryInFlight = 0
        }
    }

    $d = Get-UploaderDecision -ExitCode $code -Session $session -Attempt $State.Attempts `
        -Signature $signature -SuddenRetries $State.SuddenRetries -StopRequested:$stopRequested
    if ($d.Action -eq "stop") {
        Write-SupervisorLog "BLE uploader stopped on request ($($d.Reason)); not restarting it while the request stands."
    } elseif ($d.Action -eq "halt") {
        $auto = $State.SuddenRetries
        Write-UploaderHaltState -Path $HaltFile -Decision $d -ExitCode $code -Session $session `
            -ExitRecord $exitRecord -Attempts $State.Attempts -AutoRetries $auto
        $State.Halted = $true
        if ($auto -gt 0) {
            $why = if ($signature -eq "sudden") { "all $BleSuddenRetryMax automatic retries failed" } else { "after automatic retry $auto, code $code$(if ($signature) { ", signature $signature" })" }
            Write-RetryState -HaltFile $HaltFile -Status "exhausted" -Retry $auto -Event "halted: $why"
        }
        Write-SupervisorLog "BLE uploader HALTED ($($d.Class)): $($d.Reason). $($d.Instruction) The renderer keeps running."
    } elseif ($d.Class -eq "sudden-retry") {
        $State.SuddenRetries++
        $State.RetryPending = $true
        $at = (Get-Date).AddSeconds($d.Delay)
        $State.NextUploaderStart = $at
        $n = $State.SuddenRetries
        $block = if ($exitRecord -and $exitRecord["last_acked_block"]) { ", last ACKed block $($exitRecord["last_acked_block"])" } else { "" }
        $failure = "code 20, signature sudden$block"
        Write-SupervisorLog (("Sudden failure ($failure): automatic retry $n of $BleSuddenRetryMax at {0:HH:mm:ss}, in $($d.Delay)s. " -f $at) +
            "No power-off for this retry (policy since 2026-10-18); the uploader halts as before if all $BleSuddenRetryMax retries fail.")
        Write-RetryState -HaltFile $HaltFile -Status "waiting" -Retry $n -NextAttemptAt $at -LastFailure $failure `
            -Event ("sudden failure ($failure); retry $n of $BleSuddenRetryMax scheduled for {0:HH:mm:ss}" -f $at)
    } else {
        $State.NextUploaderStart = (Get-Date).AddSeconds($d.Delay)
        $slow = if ($d.Class -eq "slow-retry") { " Keyboard still not reachable; retrying every $($d.Delay)s until it is (no halt)." } else { "" }
        Write-SupervisorLog "Restarting BLE uploader in $($d.Delay)s (consecutive failures: $($State.Attempts)).$slow"
    }
}

function Invoke-SupervisorLoop {
    # Runs until stopped (or for $MaxIterations polls, for tests). Never blocks on a
    # restart delay, so one child's backoff does not stall supervision of the other.
    param(
        $State,
        [scriptblock]$StartRenderer,
        [scriptblock]$StartUploader,
        [string]$HaltFile,
        [string]$StreamPath,
        [double]$PollSeconds = 2,
        [int]$MaxIterations = 0
    )

    # A stop request at supervisor start is left over from an earlier STOP_STACK.ps1.
    if (Clear-UploaderStopRequest $StreamPath) {
        Write-SupervisorLog "Removed a stale stop request ($(Get-StopRequestPath $StreamPath))."
    }

    if (Test-Path -LiteralPath $HaltFile) {
        $State.Halted = $true
        Write-SupervisorLog "BLE uploader is halted ($HaltFile); not starting it. Disconnect the keyboard's battery for at least 30 s (there is no power switch; unplugging USB doesn't count), then run START_STACK.ps1."
    }
    # A retry that was waiting or running when the previous supervisor stopped is not
    # resumed: if its session marker is still there, the uploader refuses to start and
    # the supervisor halts, as before the retry existed.
    $retryPath = Get-RetryStatePath $HaltFile
    if (Test-Path -LiteralPath $retryPath) {
        try { $old = [System.IO.File]::ReadAllText($retryPath) | ConvertFrom-Json } catch { $old = $null }
        if ($old -and ($old.status -eq "waiting" -or $old.status -eq "running")) {
            Write-RetryState -HaltFile $HaltFile -Status "abandoned" -Retry ([int]$old.retry) `
                -Event "retry $($old.retry) of $BleSuddenRetryMax abandoned: the supervisor restarted"
            Write-SupervisorLog "An automatic retry ($($old.retry) of $BleSuddenRetryMax) was $($old.status) when the previous supervisor stopped; it is not resumed."
        }
    }

    $i = 0
    while (($MaxIterations -eq 0) -or ($i -lt $MaxIterations)) {
        $i++

        # Renderer: always restarted. It never talks to the keyboard.
        if (($null -ne $State.Renderer) -and $State.Renderer.HasExited) {
            Write-SupervisorLog "Renderer exited with code $($State.Renderer.ExitCode); restarting in ${RendererRestartDelay}s."
            $State.Renderer = $null
            $State.NextRendererStart = (Get-Date).AddSeconds($RendererRestartDelay)
        }
        if (($null -eq $State.Renderer) -and ((Get-Date) -ge $State.NextRendererStart)) {
            $State.Renderer = & $StartRenderer
            $State.RendererStarts++
        }

        # Uploader: restarted only when the exit class allows it.
        if (($null -ne $State.Uploader) -and $State.Uploader.HasExited) {
            Resolve-UploaderExit -State $State -HaltFile $HaltFile -StreamPath $StreamPath
        }
        # An automatic retry succeeds while its uploader keeps running (watch mode).
        if (($State.RetryInFlight -gt 0) -and ($null -ne $State.Uploader) -and -not $State.Uploader.HasExited) {
            $delivered = Get-Item -LiteralPath "$StreamPath.delivered" -ErrorAction SilentlyContinue
            if ($delivered -and $delivered.LastWriteTime -gt $State.Uploader.TigaStartedAt) {
                Complete-SuddenRetry -State $State -HaltFile $HaltFile
            }
        }
        if ($State.Halted -and -not (Test-Path -LiteralPath $HaltFile)) {
            Write-SupervisorLog "Halt cleared ($HaltFile removed); starting the BLE uploader again."
            $State.Halted = $false
            $State.Attempts = 0
            $State.SuddenRetries = 0
            $State.RetryPending = $false
            $State.RetryInFlight = 0
            $State.NextUploaderStart = [datetime]::MinValue
        }
        $stopRequested = Test-Path -LiteralPath (Get-StopRequestPath $StreamPath)
        if ($stopRequested -and -not $State.StopHeld -and $State.RetryPending) {
            Write-SupervisorLog "Stop requested while automatic retry $($State.SuddenRetries) of $BleSuddenRetryMax is waiting; it will not start while the request stands."
        }
        if ($State.StopHeld -and -not $stopRequested) {
            $what = if ($State.RetryPending) { "the waiting automatic retry may start again" } else { "starting the BLE uploader again" }
            Write-SupervisorLog "Stop request withdrawn; $what."
            if (-not $State.RetryPending) { $State.NextUploaderStart = [datetime]::MinValue }
        }
        $State.StopHeld = $stopRequested
        if ((-not $State.Halted) -and (-not $stopRequested) -and ($null -eq $State.Uploader) -and ((Get-Date) -ge $State.NextUploaderStart)) {
            if ($State.RetryPending) {
                # The uploader refuses to start while the marker exists, so the retry removes
                # it, as START_STACK.ps1 does by hand.
                $marker = "$StreamPath.session"
                $removed = if (Test-Path -LiteralPath $marker) { Remove-Item -LiteralPath $marker -Force; "session marker removed; " } else { "" }
                $State.RetryPending = $false
                $State.RetryInFlight = $State.SuddenRetries
                $n = $State.RetryInFlight
                Write-SupervisorLog "Automatic retry $n of ${BleSuddenRetryMax}: ${removed}starting the BLE uploader (no power-off)."
                Write-RetryState -HaltFile $HaltFile -Status "running" -Retry $n -Event "retry $n of $BleSuddenRetryMax started"
            }
            $State.Uploader = & $StartUploader
            $State.UploaderStarts++
        }

        Start-Sleep -Milliseconds ([int]($PollSeconds * 1000))
    }
}

function Clear-UploaderHalt {
    # The manual "I have power-cycled the keyboard" action, called by START_STACK.ps1.
    # Removes the session marker first, so a supervisor that notices the missing halt
    # file never restarts the uploader while the marker is still there.
    param([string]$HaltFile, [string]$StreamPath)
    $cleared = @()
    foreach ($path in @("$StreamPath.session", $HaltFile)) {
        if (Test-Path -LiteralPath $path) {
            Remove-Item -LiteralPath $path -Force
            $cleared += $path
        }
    }
    return $cleared
}

# ---- Clean stop (docs/design/CLEAN_STOP.md) -----------------------------------------
#
# STOP_STACK.ps1 used to force-kill the uploader, which can cut an upload in half and
# always drops the BLE link without a disconnect. Now it first writes <stream>.stop
# and waits. The uploader checks for that file only between uploads, so an upload in
# progress is finished (or fails the normal way) first; then it disconnects and exits
# 0. Only if it has not exited in time does STOP_STACK.ps1 fall back to the force-kill.
# While the file exists the supervisor does not start the uploader. Windows shutdown
# and Stop-ScheduledTask on their own still end it with a kill.

# One healthy upload takes ~8.4-10.1 s, and a failing one ends within ~15 s at the
# 2500 ms block timeout (PROJECT_HISTORY 7.1). The uploader may also be in a scan (up
# to 15 s) or a connect, which it does not interrupt. 30 s covers an upload that has
# only just started, with margin, without making a stop feel hung.
$UploaderStopTimeoutSeconds = 30

function Get-StopRequestPath {
    param([string]$StreamPath)
    return "$StreamPath.stop"
}

function Clear-UploaderStopRequest {
    # Returns $true if a request was removed.
    param([string]$StreamPath)
    $path = Get-StopRequestPath $StreamPath
    if (-not (Test-Path -LiteralPath $path)) { return $false }
    Remove-Item -LiteralPath $path -Force
    return $true
}

function Request-UploaderStop {
    # Asks the running uploader to stop and waits up to $TimeoutSeconds for it to exit.
    # Kills nothing: the caller decides what to do on a timeout. The request file stays,
    # so a supervisor that is still running does not start the uploader again; the
    # caller removes it once the stack is down.
    #   $GetUploaderProcessIds returns the process IDs to wait for.
    # Returns Outcome "not-running", "clean", "failed" or "timeout", the seconds waited,
    # the process IDs still running (on a timeout), and for "failed" the exit code and
    # detail. "failed": the uploader did exit, but by failing the upload it was finishing
    # (it wrote <stream>.exit after the request), not on request.
    param(
        [string]$StreamPath,
        [scriptblock]$GetUploaderProcessIds,
        [double]$TimeoutSeconds = $UploaderStopTimeoutSeconds,
        [double]$PollSeconds = 0.25
    )
    $ids = @(& $GetUploaderProcessIds | Where-Object { $_ })
    if ($ids.Count -eq 0) {
        return [pscustomobject]@{ Outcome = "not-running"; Seconds = 0.0; ProcessIds = @() }
    }
    $requestedAt = Get-Date
    $text = "requested_at={0}`r`nrequested_by=STOP_STACK.ps1 pid {1}`r`nwaiting_for={2}`r`n" -f `
        $requestedAt.ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ"), $PID, ($ids -join ",")
    [System.IO.File]::WriteAllText((Get-StopRequestPath $StreamPath), $text, $Utf8NoBom)

    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    while ($true) {
        $alive = @($ids | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
        if ($alive.Count -eq 0) {
            # The uploader writes <stream>.exit only when it fails; an exit on request doesn't.
            $exitFile = Get-Item -LiteralPath "$StreamPath.exit" -ErrorAction SilentlyContinue
            if ($exitFile -and $exitFile.LastWriteTime -ge $requestedAt) {
                $exit = Read-KeyValueFile $exitFile.FullName
                return [pscustomobject]@{
                    Outcome = "failed"; Seconds = $watch.Elapsed.TotalSeconds; ProcessIds = @()
                    ExitCode = $exit["code"]; Detail = $exit["detail"]; Signature = $exit["signature"]
                }
            }
            return [pscustomobject]@{ Outcome = "clean"; Seconds = $watch.Elapsed.TotalSeconds; ProcessIds = @() }
        }
        if ($watch.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
            return [pscustomobject]@{ Outcome = "timeout"; Seconds = $watch.Elapsed.TotalSeconds; ProcessIds = $alive }
        }
        Start-Sleep -Milliseconds ([int]($PollSeconds * 1000))
    }
}

function Get-StopOutcomeMessage {
    param($Result, [double]$TimeoutSeconds = $UploaderStopTimeoutSeconds)
    switch ($Result.Outcome) {
        "not-running" { return "STOP_STACK: no BLE uploader was running." }
        "clean"       { return ("STOP_STACK: BLE uploader exited on request after {0:n1} s." -f $Result.Seconds) }
        "failed"      {
            $sig = if ($Result.Signature) { ", signature $($Result.Signature)" } else { "" }
            return ("STOP_STACK: BLE uploader exited after {0:n1} s, but not on request: the upload it was finishing failed (code {1}{2}: {3}). " -f $Result.Seconds, $Result.ExitCode, $sig, $Result.Detail) +
                "See startup_supervisor.log and STATUS_STACK.ps1."
        }
        default       { return ("STOP_STACK: BLE uploader did not exit within {0:n0} s; force-killing it (pid {1})." -f $TimeoutSeconds, ($Result.ProcessIds -join ",")) }
    }
}

function Invoke-StackStop {
    # The body of STOP_STACK.ps1 (dev and packaged), with every side effect passed in so
    # tests can run it against dummy processes:
    #   $GetUploaderProcessIds  uploader processes to ask and wait for
    #   $StopTask               stops the scheduled task (and with it the supervisor)
    #   $ForceKillAll           the old force-kill of every stack process
    #   $HaltFile               logs\uploader_halted.json; its folder holds uploader_retry.json
    # Order: ask, wait, stop the task, force-kill whatever is left, cancel a waiting
    # automatic retry, withdraw the request. Returns the Request-UploaderStop result.
    param(
        [string]$StreamPath,
        [scriptblock]$GetUploaderProcessIds,
        [scriptblock]$StopTask,
        [scriptblock]$ForceKillAll,
        [string]$HaltFile = "",
        [double]$TimeoutSeconds = $UploaderStopTimeoutSeconds,
        [double]$PollSeconds = 0.25
    )
    $result = Request-UploaderStop -StreamPath $StreamPath -GetUploaderProcessIds $GetUploaderProcessIds `
        -TimeoutSeconds $TimeoutSeconds -PollSeconds $PollSeconds
    $message = Get-StopOutcomeMessage $result $TimeoutSeconds
    Write-SupervisorLog $message
    Write-Host $message
    try {
        & $StopTask
        & $ForceKillAll
        if ($HaltFile) {
            $cancelled = Stop-PendingSuddenRetry -HaltFile $HaltFile
            if ($cancelled) { Write-SupervisorLog $cancelled; Write-Host $cancelled }
        }
    } finally {
        [void](Clear-UploaderStopRequest $StreamPath)
    }
    return $result
}

function Stop-PendingSuddenRetry {
    # Called by STOP_STACK.ps1 once the supervisor is gone. An automatic retry that was
    # waiting (no uploader running, so no request was needed) died with the supervisor;
    # this records that, so STATUS_STACK.ps1 doesn't show it as still coming. The session
    # marker from the failed upload stays: START_STACK.ps1 clears it, as after any halt.
    # Returns the message, or $null if no retry was pending.
    param([string]$HaltFile)
    $path = Get-RetryStatePath $HaltFile
    if (-not (Test-Path -LiteralPath $path)) { return $null }
    try { $state = [System.IO.File]::ReadAllText($path) | ConvertFrom-Json } catch { return $null }
    if (-not ($state.status -eq "waiting" -or $state.status -eq "running")) { return $null }
    $n = [int]$state.retry
    Write-RetryState -HaltFile $HaltFile -Status "cancelled" -Retry $n -Event "retry $n of $BleSuddenRetryMax cancelled by STOP_STACK.ps1 (was $($state.status))"
    return "STOP_STACK: automatic retry $n of $BleSuddenRetryMax was $($state.status); cancelled. START_STACK.ps1 starts the uploader afresh (it clears the session marker)."
}

function Clear-UploaderHaltWithNotice {
    # START_STACK.ps1 (dev and packaged): says what the halt was, clears it and asks that the
    # battery really has been disconnected.
    param([string]$HaltFile, [string]$StreamPath)
    if (Test-Path -LiteralPath $HaltFile) {
        try {
            $halt = Get-Content -Raw -LiteralPath $HaltFile | ConvertFrom-Json
            Write-Host "The BLE uploader was halted at $($halt.halted_at): $($halt.reason)"
        } catch {}
    }
    $cleared = Clear-UploaderHalt -HaltFile $HaltFile -StreamPath $StreamPath
    if ($cleared) {
        Write-Host "Cleared the halt, assuming the keyboard's battery has been disconnected and reconnected:"
        $cleared | ForEach-Object { Write-Host "  removed $_" }
        Write-Host "If it was NOT, run STOP_STACK.ps1 now. Then:"
        Write-Host $PowerCycleInstruction
    }
}

# ---- STATUS_STACK.ps1 ----------------------------------------------------------------

function Get-StackProcesses {
    # Every process whose command line matches. The pattern differs by layout.
    param([string]$Pattern)
    Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -match $Pattern }
}

function ConvertTo-LocalTime {
    param([string]$Text)
    try { return ([DateTimeOffset]::Parse($Text)).LocalDateTime } catch { return $null }
}

function Show-StackStatus {
    # The body of STATUS_STACK.ps1 (dev and packaged). Only reads.
    param([string]$TaskName, [string]$Stream, [string]$Logs, [string]$ProcessPattern)

    Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue | Select-Object TaskName,State
    Write-Host ""
    Get-StackProcesses $ProcessPattern | Select-Object ProcessId,Name,CommandLine

    # ---- Delivery (issue #9): a live process is not proof the keyboard is being updated ----

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
}

function Get-UploaderProcessIds {
    Get-CimInstance Win32_Process -Filter "Name = 'tiga_stream_watch_flow.exe'" | ForEach-Object { $_.ProcessId }
}

function Stop-ProcessesMatching {
    # STOP_STACK.ps1's last resort. The tests never call it: it kills by command line.
    param([string]$Pattern)
    Get-StackProcesses $Pattern | ForEach-Object {
        Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
    }
}

# ---- The body of both supervisors ------------------------------------------------------

function Initialize-StackStream {
    # The active stream is runtime output: recreate it from the template if it is missing.
    # Then every file the stack needs must exist.
    param([string]$Stream, [string]$Template, [string[]]$Required)
    if ((-not (Test-Path $Stream)) -and (Test-Path $Template)) {
        Copy-Item -LiteralPath $Template -Destination $Stream
        Write-SupervisorLog "Seeded $Stream from $Template."
    }
    foreach ($file in $Required) {
        if (-not (Test-Path $file)) {
            Write-SupervisorLog "Required file missing: $file"
            throw "Required file missing: $file"
        }
    }
}

function Invoke-StackSupervisor {
    # One supervisor at a time; runs the loop, and on the way out stops its children. Runs
    # until stopped. The tests pass their own $MutexName, $PollSeconds and $MaxIterations.
    param(
        [string]$LaunchMessage,
        [scriptblock]$StartRenderer,
        [scriptblock]$StartUploader,
        [string]$HaltFile,
        [string]$StreamPath,
        [string]$MutexName = "Global\TigaWindowsMediaDisplay",
        [double]$PollSeconds = 2,
        [int]$MaxIterations = 0
    )
    $createdNew = $false
    $mutex = New-Object System.Threading.Mutex($true, $MutexName, [ref]$createdNew)
    if (-not $createdNew) {
        Write-SupervisorLog "Another supervisor instance is already running; exiting."
        $mutex.Dispose()
        return
    }

    $state = New-SupervisorState
    try {
        Write-SupervisorLog $LaunchMessage
        Invoke-SupervisorLoop -State $state -StartRenderer $StartRenderer -StartUploader $StartUploader `
            -HaltFile $HaltFile -StreamPath $StreamPath -PollSeconds $PollSeconds -MaxIterations $MaxIterations
    }
    finally {
        Write-SupervisorLog "Supervisor stopping; cleaning up children."
        foreach ($proc in @($state.Renderer, $state.Uploader)) {
            try { Stop-LoggedProcess $proc } catch {}
        }
        try { $mutex.ReleaseMutex() } catch {}
        $mutex.Dispose()
    }
}
