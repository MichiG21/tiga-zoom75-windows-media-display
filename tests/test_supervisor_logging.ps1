# Offline test for the supervisor logging helpers (issues #8, #9).
#
# Loads the helpers both supervisors share (tiga_supervisor_common.ps1) without running
# either supervisor's main loop, then launches a harmless child (python -c ...) from a directory whose path contains
# spaces. Nothing here touches Bluetooth or the keyboard.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\test_supervisor_logging.ps1

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot
$failures = 0

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if ($Condition) { Write-Host "  ok   $Message" }
    else { Write-Host "  FAIL $Message"; $script:failures++ }
}

# Both supervisors must take their helpers from the common file, not carry copies.
foreach ($supervisor in @("tiga_startup_supervisor.ps1", "installer\runtime\tiga_startup_supervisor.ps1")) {
    $tokens = $null; $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile((Join-Path $RepoRoot $supervisor), [ref]$tokens, [ref]$errors)
    $defined = @($ast.FindAll({ $args[0] -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true) | ForEach-Object { $_.Name })
    Assert-True ($errors.Count -eq 0) "$supervisor parses"
    Assert-True ($ast.Extent.Text -match 'tiga_supervisor_common\.ps1') "$supervisor dot-sources the common helpers"
    Assert-True (-not ($defined -contains "Start-LoggedProcess")) "$supervisor has no private copy of the helpers"
}

$python = (Get-Command py.exe -ErrorAction Stop).Source

foreach ($common in @("tiga_supervisor_common.ps1")) {
    Write-Host "== $common"
    $work = Join-Path ([System.IO.Path]::GetTempPath()) ("tiga log test " + [guid]::NewGuid().ToString("N").Substring(0, 6))
    $logs = Join-Path $work "log dir"
    New-Item -ItemType Directory -Force -Path $logs | Out-Null

    $SupervisorLog = Join-Path $logs "startup_supervisor.log"
    . (Join-Path $RepoRoot $common)
    $LogMaxBytes   = 2KB   # after dot-sourcing, so the test limit wins

    # Issue #8: the old dev supervisor passed the script path unquoted, so a checkout
    # under e.g. "C:\My Projects\" launched Python with a truncated path.
    $line = Get-LoggedCommandLine "C:\Program Files\Py\py.exe" @("-u", "C:\My Projects\tiga\renderer.py") "C:\l o g\out.log" "C:\l o g\err.log"
    Assert-True ($line -eq '"C:\Program Files\Py\py.exe" "-u" "C:\My Projects\tiga\renderer.py" >> "C:\l o g\out.log" 2>> "C:\l o g\err.log"') "command line quotes every path"

    $child = Join-Path $work "child script.py"
    Set-Content -LiteralPath $child -Encoding ASCII -Value @(
        'import os, sys',
        'print("stdout run=" + os.environ.get("TIGA_RUN_ID", "?") + " unbuffered=" + os.environ.get("PYTHONUNBUFFERED", "?"))',
        'print("stderr line", file=sys.stderr)',
        'sys.exit(int(sys.argv[1]))'
    )
    $out = Join-Path $logs "renderer.log"
    $err = Join-Path $logs "renderer_error.log"

    $codes = @()
    foreach ($exitCode in @(3, 0)) {
        $p = Start-LoggedProcess -Name "renderer" -FilePath $python -Arguments @($child, "$exitCode") `
            -WorkingDirectory $work -StdOut $out -StdErr $err
        $p.WaitForExit()
        $codes += $p.ExitCode
    }

    $outText = Get-Content -LiteralPath $out -Raw
    $errText = Get-Content -LiteralPath $err -Raw
    Assert-True (([regex]::Matches($outText, "run=renderer-\d{8}-\d{6}-\w{6} start")).Count -eq 2) "two run headers in stdout log"
    Assert-True (([regex]::Matches($outText, "stdout run=renderer-")).Count -eq 2) "both runs appended, child saw TIGA_RUN_ID"
    Assert-True ($outText -match "unbuffered=1") "PYTHONUNBUFFERED passed to child"
    Assert-True (([regex]::Matches($errText, "stderr line")).Count -eq 2) "stderr appended for both runs"
    Assert-True (($codes[0] -eq 3) -and ($codes[1] -eq 0)) "exit codes propagated through cmd.exe ($($codes -join ','))"
    Assert-True ((Get-Content -LiteralPath $SupervisorLog -Raw) -match "Started renderer run=") "supervisor log records the run ID"

    # Rotation: fill past the (test) 2 KB limit and start again.
    # Stop-LoggedProcess must take down the real child, not just the cmd.exe wrapper.
    $sleeper = Join-Path $work "sleeper.py"
    Set-Content -LiteralPath $sleeper -Encoding ASCII -Value 'import time; time.sleep(60)'
    $p = Start-LoggedProcess -Name "sleeper" -FilePath $python -Arguments @($sleeper) `
        -WorkingDirectory $work -StdOut $out -StdErr $err
    Start-Sleep -Seconds 2
    $kids = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($p.Id)" | Select-Object -ExpandProperty ProcessId)
    Stop-LoggedProcess $p
    Start-Sleep -Seconds 1
    $alive = @($kids | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
    Assert-True (($kids.Count -ge 1) -and ($alive.Count -eq 0) -and $p.HasExited) "stop kills wrapper and child ($($kids.Count) child, $($alive.Count) left)"

    $small = Join-Path $logs "small.log"
    [System.IO.File]::AppendAllText($small, "tiny")
    Invoke-LogRotation $small
    Assert-True (-not (Test-Path "$small.1")) "log under the limit is not rotated"

    for ($i = 0; $i -lt 3; $i++) {
        [System.IO.File]::AppendAllText($out, ("x" * 3000) + "`r`n")
        Invoke-LogRotation $out
    }
    Assert-True ((Test-Path "$out.1") -and (Test-Path "$out.2") -and (Test-Path "$out.3")) "rotated to .1 .2 .3"
    Assert-True ((Get-Content -LiteralPath "$out.3" -Raw) -match "run=renderer-") "oldest kept file is the original log"

    [System.IO.File]::AppendAllText($out, ("x" * 3000) + "`r`n")
    Invoke-LogRotation $out
    Assert-True (-not (Test-Path "$out.4")) "keeps at most $LogKeep rotated files"
    Assert-True (-not ((Get-Content -LiteralPath "$out.3" -Raw) -match "run=renderer-")) "oldest file dropped on overflow"

    Remove-Item -LiteralPath $work -Recurse -Force
}

if ($failures) { Write-Host "$failures failure(s)"; exit 1 }
Write-Host "All supervisor logging checks passed."
