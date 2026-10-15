# Helpers shared by the development supervisor (tiga_startup_supervisor.ps1 at the
# repository root) and the packaged one (installer/runtime). Dot-source it after
# setting $SupervisorLog:
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
# The uploader (windows_ble_probe/src/bin/tiga_stream_watch_flow.rs) exits with a
# class code and keeps <stream>.session while a media session is open on the keyboard:
#
#   10 unreachable, 11 GATT          -> retry with backoff, then every 5 min, never halt
#   20 interrupted, 21 no completion -> halt: the keyboard needs a power cycle
#   30 bad arguments / stream        -> halt: configuration problem
#   anything else, no marker         -> retry with backoff, then give up
#
# 10 and 11 never open a media session, so the keyboard is not in a bad state; it is
# off, asleep or out of range (11: no candidate had the media service, issue #14).
# Halting on those meant a dead display the next morning until START_STACK.ps1.
#
# A surviving session marker means "halt, power-cycle" whatever the exit code: a
# killed or crashed uploader cannot report its own class.
#
# Halting writes logs\uploader_halted.json and leaves the renderer running.
# START_STACK.ps1 deletes the halt file (and the marker); a running supervisor notices
# and starts the uploader again. Nothing clears it on a timer.

$BleRetryDelays       = @(8, 16, 32, 60)  # seconds before retries 1..4
$BleSlowRetryDelay    = 300               # 10 and 11 after the backoff: every 5 min, indefinitely
$BleSlowRetryCodes    = @(10, 11)
$RendererRestartDelay = 8

$UploaderExitMeanings = @{
    0  = "success"
    10 = "keyboard not reachable (no adapter, not found, connect failed, or link lost before the media session opened)"
    11 = "connected, but the keyboard's GATT service was missing or unusable"
    20 = "transfer interrupted after CONTROL2; the keyboard may hold a partial media session"
    21 = "all blocks sent but the keyboard never confirmed completion"
    30 = "bad arguments, missing or invalid stream, or internal error"
}

$PowerCycleInstruction = "Power-cycle the keyboard (switch it off and on again; if the display " +
    "stays white or frozen, do the full battery power-cycle), then run START_STACK.ps1."

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
    param([int]$ExitCode, $Session, [int]$Attempt)

    $meaning = Get-UploaderExitMeaning $ExitCode

    if ($null -ne $Session) {
        return New-UploaderDecision halt "session-open" $true 0 `
            "uploader exited (code ${ExitCode}: $meaning) while a media session was open on the keyboard" `
            $PowerCycleInstruction
    }
    if ($ExitCode -eq 20 -or $ExitCode -eq 21) {
        return New-UploaderDecision halt "session-open" $true 0 "code ${ExitCode}: $meaning" $PowerCycleInstruction
    }
    if ($ExitCode -eq 30) {
        return New-UploaderDecision halt "fatal" $false 0 "code ${ExitCode}: $meaning" `
            "Check logs\ble_error.log for the cause and fix it, then run START_STACK.ps1. The keyboard was not touched."
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

function Write-UploaderHaltState {
    param([string]$Path, $Decision, [int]$ExitCode, $Session, $ExitRecord, [int]$Attempts)
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
        attempts          = $Attempts
        instruction       = $Decision.Instruction
    }
    [System.IO.File]::WriteAllText($Path, ($state | ConvertTo-Json), $Utf8NoBom)
}

function New-SupervisorState {
    [pscustomobject]@{
        Renderer = $null; Uploader = $null
        NextRendererStart = [datetime]::MinValue; NextUploaderStart = [datetime]::MinValue
        Attempts = 0; Halted = $false; RendererStarts = 0; UploaderStarts = 0
    }
}

function Resolve-UploaderExit {
    param($State, [string]$HaltFile, [string]$StreamPath)

    $proc = $State.Uploader
    $State.Uploader = $null
    $code = if ($null -eq $proc.ExitCode) { -1 } else { [int]$proc.ExitCode }

    $session = Read-KeyValueFile "$StreamPath.session"
    $exitRecord = Read-KeyValueFile "$StreamPath.exit"
    if ($exitRecord -and $exitRecord["run_id"] -ne $proc.TigaRunId) { $exitRecord = $null }  # an older run's

    $delivered = Get-Item -LiteralPath "$StreamPath.delivered" -ErrorAction SilentlyContinue
    if ($delivered -and $delivered.LastWriteTime -gt $proc.TigaStartedAt) {
        if ($State.Attempts -gt 0) { Write-SupervisorLog "Uploader delivered a frame during this run; retry counter reset." }
        $State.Attempts = 0
    }
    if ($code -ne 0) { $State.Attempts++ }

    $detail = if ($exitRecord) { " Detail: $($exitRecord["detail"])" } else { "" }
    Write-SupervisorLog "BLE uploader exited with code $code ($(Get-UploaderExitMeaning $code)).$detail"

    $d = Get-UploaderDecision -ExitCode $code -Session $session -Attempt $State.Attempts
    if ($d.Action -eq "halt") {
        Write-UploaderHaltState -Path $HaltFile -Decision $d -ExitCode $code -Session $session `
            -ExitRecord $exitRecord -Attempts $State.Attempts
        $State.Halted = $true
        Write-SupervisorLog "BLE uploader HALTED ($($d.Class)): $($d.Reason). $($d.Instruction) The renderer keeps running."
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

    if (Test-Path -LiteralPath $HaltFile) {
        $State.Halted = $true
        Write-SupervisorLog "BLE uploader is halted ($HaltFile); not starting it. Power-cycle the keyboard, then run START_STACK.ps1."
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
        if ($State.Halted -and -not (Test-Path -LiteralPath $HaltFile)) {
            Write-SupervisorLog "Halt cleared ($HaltFile removed); starting the BLE uploader again."
            $State.Halted = $false
            $State.Attempts = 0
            $State.NextUploaderStart = [datetime]::MinValue
        }
        if ((-not $State.Halted) -and ($null -eq $State.Uploader) -and ((Get-Date) -ge $State.NextUploaderStart)) {
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
