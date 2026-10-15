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
