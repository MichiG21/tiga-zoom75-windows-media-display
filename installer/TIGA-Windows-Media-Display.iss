#ifndef MyAppVersion
  #define MyAppVersion "dev"
#endif

#define MyAppName "TIGA Windows Media Display"
#define MyAppPublisher "MichiG21"
#define MyAppURL "https://github.com/MichiG21/tiga-zoom75-windows-media-display"

[Setup]
AppId={{B966B2FC-7032-4E68-82C8-66E3D37B944A}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
DefaultDirName={localappdata}\Programs\TIGA Windows Media Display
DefaultGroupName=TIGA Windows Media Display
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=output
OutputBaseFilename=TIGA-Windows-Media-Display-Setup-v{#MyAppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
UninstallDisplayIcon={app}\app\_internal\tiga_display_control.exe
SetupLogging=yes
CloseApplications=no

[InstallDelete]
; [Files] puts the template back as the active stream, so a .frame left by an earlier
; install would name a frame that is no longer there, and the uploader would send the
; template as unknown(sidecar-is-...). Without it the uploader holds the template until
; the renderer's first frame, as on a first install.
Type: files; Name: "{app}\app\_internal\extracted\session_01_bulk_stream.bin.frame"

[Files]
Source: "staging\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\TIGA Windows Media Display\Display Control"; Filename: "{app}\app\_internal\tiga_display_control.exe"
; -NoExit keeps the window open so the script's output can be read.
Name: "{autoprograms}\TIGA Windows Media Display\Start Display"; Filename: "powershell.exe"; Parameters: "-NoExit -NoProfile -ExecutionPolicy Bypass -File ""{app}\START_STACK.ps1"""; WorkingDir: "{app}"
Name: "{autoprograms}\TIGA Windows Media Display\Stop Display"; Filename: "powershell.exe"; Parameters: "-NoExit -NoProfile -ExecutionPolicy Bypass -File ""{app}\STOP_STACK.ps1"""; WorkingDir: "{app}"
Name: "{autoprograms}\TIGA Windows Media Display\Status"; Filename: "powershell.exe"; Parameters: "-NoExit -NoProfile -ExecutionPolicy Bypass -File ""{app}\STATUS_STACK.ps1"""; WorkingDir: "{app}"

[Run]
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\STOP_STACK.ps1"""; WorkingDir: "{app}"; Flags: runhidden waituntilterminated; StatusMsg: "Stopping any previous TIGA display session..."
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\install_tiga_startup.ps1"""; WorkingDir: "{app}"; Flags: runhidden waituntilterminated; StatusMsg: "Installing automatic startup task..."
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command ""Start-ScheduledTask -TaskName 'TIGA Windows Media Display'"""; WorkingDir: "{app}"; Flags: runhidden waituntilterminated; StatusMsg: "Starting TIGA Windows Media Display..."
Filename: "{app}\app\_internal\tiga_display_control.exe"; Description: "Open TIGA Display Control"; Flags: nowait postinstall skipifsilent unchecked

[UninstallRun]
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\STOP_STACK.ps1"""; WorkingDir: "{app}"; Flags: runhidden waituntilterminated
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command ""Unregister-ScheduledTask -TaskName 'TIGA Windows Media Display' -Confirm:$false -ErrorAction SilentlyContinue"""; Flags: runhidden waituntilterminated

[Code]
// Upgrades: stop the installed version before [Files] replaces anything. The [Run]
// STOP_STACK step comes after the copy, and on 2026-10-25 (Phase 8) installing over a
// running v0.2.0-beta.1 failed there with "DeleteFile failed; code 5. Access is denied"
// on app\_internal\tiga_stream_watch_flow.exe. The installed STOP_STACK.ps1 is the one
// that knows how to stop that version: v0.2's force-kills, later ones ask the uploader
// to finish its upload first (docs/design/CLEAN_STOP.md). A first install has none, and
// the [Run] step still covers it.

const
  StackExitTimeoutMs = 15000;
  StackExitPollMs = 250;

// The stack's executables still running from AppDir, as " name (pid N)" items; empty if
// none. Display Control is not part of the stack and STOP_STACK leaves it running.
function StackProcessesRunningFrom(const AppDir: String): String;
var
  Locator, Service, Processes, Process, ExePath: Variant;
  Prefix, Path, Name: String;
  I, Pid: Integer;
begin
  Result := '';
  Prefix := Lowercase(AddBackslash(AppDir));
  Locator := CreateOleObject('WbemScripting.SWbemLocator');
  Service := Locator.ConnectServer('.', 'root\CIMV2');
  Processes := Service.ExecQuery('SELECT ProcessId, Name, ExecutablePath FROM Win32_Process' +
    ' WHERE Name = ''tiga_stream_watch_flow.exe'' OR Name = ''tiga_windows_nowplaying.exe''' +
    ' OR Name = ''tiga_windows_media_snapshot.exe''');
  for I := 0 to Processes.Count - 1 do
  begin
    Process := Processes.ItemIndex(I);
    ExePath := Process.ExecutablePath;
    if not VarIsNull(ExePath) then
    begin
      Path := ExePath;
      if Pos(Prefix, Lowercase(Path)) = 1 then
      begin
        Name := Process.Name;
        Pid := Process.ProcessId;
        Result := Result + ' ' + Name + ' (pid ' + IntToStr(Pid) + ')';
      end;
    end;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  AppDir, StopScript, Running: String;
  ResultCode, Waited: Integer;
  Page: TOutputProgressWizardPage;
begin
  Result := '';
  AppDir := ExpandConstant('{app}');
  StopScript := AppDir + '\STOP_STACK.ps1';
  if not FileExists(StopScript) then
  begin
    Log('No ' + StopScript + ': no installed version to stop before copying files.');
    Exit;
  end;

  Page := CreateOutputProgressPage('Stopping TIGA Windows Media Display',
    'Setup stops the installed version so that its files can be replaced.');
  Page.SetText('Stopping any previous TIGA display session (this can take up to 30 seconds)...', '');
  Page.Show;
  try
    Log('Stopping the installed version before copying files: ' + StopScript);
    if Exec('powershell.exe', '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + StopScript + '"',
      AppDir, SW_HIDE, ewWaitUntilTerminated, ResultCode) then
      Log('STOP_STACK.ps1 exit code: ' + IntToStr(ResultCode))
    else
      Log('Could not run STOP_STACK.ps1: ' + SysErrorMessage(ResultCode));

    // Stop-Process returns before a killed process is gone, and v0.2's STOP_STACK.ps1
    // doesn't wait, so wait here until none of the stack's executables is in use.
    Waited := 0;
    try
      Running := StackProcessesRunningFrom(AppDir);
      while (Running <> '') and (Waited < StackExitTimeoutMs) do
      begin
        Sleep(StackExitPollMs);
        Waited := Waited + StackExitPollMs;
        Running := StackProcessesRunningFrom(AppDir);
      end;
    except
      Log('Could not list running processes, continuing: ' + GetExceptionMessage);
      Running := '';
    end;

    if Running = '' then
      Log('No TIGA stack process runs from ' + AppDir + ' (waited ' + IntToStr(Waited) + ' ms).')
    else
    begin
      Log('Still running from ' + AppDir + ' after ' + IntToStr(Waited) + ' ms:' + Running);
      Result := 'The installed TIGA Windows Media Display is still running (' + Trim(Running) + '), ' +
        'so its files cannot be replaced. Nothing has been changed.' + #13#10#13#10 +
        'Stop it with Start menu > TIGA Windows Media Display > Stop Display, then run Setup again.';
    end;
  finally
    Page.Hide;
  end;
end;
