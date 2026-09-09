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

[Files]
Source: "staging\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\TIGA Windows Media Display\Display Control"; Filename: "{app}\app\_internal\tiga_display_control.exe"
Name: "{autoprograms}\TIGA Windows Media Display\Start Display"; Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\START_STACK.ps1"""; WorkingDir: "{app}"
Name: "{autoprograms}\TIGA Windows Media Display\Stop Display"; Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\STOP_STACK.ps1"""; WorkingDir: "{app}"
Name: "{autoprograms}\TIGA Windows Media Display\Status"; Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\STATUS_STACK.ps1"""; WorkingDir: "{app}"

[Run]
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\STOP_STACK.ps1"""; WorkingDir: "{app}"; Flags: runhidden waituntilterminated; StatusMsg: "Stopping any previous TIGA display session..."
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\install_tiga_startup.ps1"""; WorkingDir: "{app}"; Flags: runhidden waituntilterminated; StatusMsg: "Installing automatic startup task..."
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command ""Start-ScheduledTask -TaskName 'TIGA Windows Media Display'"""; WorkingDir: "{app}"; Flags: runhidden waituntilterminated; StatusMsg: "Starting TIGA Windows Media Display..."
Filename: "{app}\app\_internal\tiga_display_control.exe"; Description: "Open TIGA Display Control"; Flags: nowait postinstall skipifsilent unchecked

[UninstallRun]
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\STOP_STACK.ps1"""; WorkingDir: "{app}"; Flags: runhidden waituntilterminated
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command ""Unregister-ScheduledTask -TaskName 'TIGA Windows Media Display' -Confirm:$false -ErrorAction SilentlyContinue"""; Flags: runhidden waituntilterminated
