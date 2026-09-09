$ErrorActionPreference = "Stop"

$tools = $PSScriptRoot
$launcher = Join-Path $tools "tiga_startup_supervisor.ps1"
$taskName = "TIGA Windows Media Display"

if (-not (Test-Path $launcher)) {
    throw "Startup supervisor not found: $launcher"
}

Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue

$action = New-ScheduledTaskAction `
    -Execute "powershell.exe" `
    -Argument "-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$launcher`""

$trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
$trigger.Delay = "PT15S"

$principal = New-ScheduledTaskPrincipal `
    -UserId "$env:USERDOMAIN\$env:USERNAME" `
    -LogonType Interactive `
    -RunLevel Limited

$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -ExecutionTimeLimit (New-TimeSpan -Days 3650) `
    -RestartCount 3 `
    -RestartInterval (New-TimeSpan -Minutes 1)

Register-ScheduledTask `
    -TaskName $taskName `
    -Action $action `
    -Trigger $trigger `
    -Principal $principal `
    -Settings $settings `
    -Description "Starts the TIGA Windows Now Playing renderer and flow-controlled BLE uploader at user logon." `
    -Force | Out-Null

Write-Host ""
Write-Host "Installed scheduled task: $taskName"
Write-Host "Project root: $tools"
Write-Host "It will start about 15 seconds after you sign in."
Write-Host ""
Write-Host "Start now:"
Write-Host "  Start-ScheduledTask -TaskName `"$taskName`""
Write-Host ""
Write-Host "Logs:"
Write-Host "  $tools\logs"
