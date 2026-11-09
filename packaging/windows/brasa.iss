; Builds Brasa's Windows installer. A real install (not the portable zip) is what the in-app
; updater needs: it only offers a silent self-update when it finds `unins000.exe` next to the
; exe (`telaio_update::install::windows_is_installed`, vendored at vendor/telaio) — a copy that
; came from the zip has none, and stays "portable" (announce only, see that module's doc comment).
;
; Per-user install (`PrivilegesRequired=lowest`), the same `%LOCALAPPDATA%\Brasa` the earlier
; portable `install.ps1` used: no UAC prompt, which matters most for a background self-update
; that must not pop one up.
;
; `AppVersion` comes from the command line: `iscc /DAppVersion=1.2.3 brasa.iss`. The GitHub-hosted
; windows-latest runner has Inno Setup (`iscc`) preinstalled, so the release workflow calls it
; directly, no extra setup step.
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{2E6B7C9D-6B2B-4C4E-9B3E-6B2C8B6B6A11}
AppName=Brasa
AppVersion={#AppVersion}
AppPublisher=Brasa
AppPublisherURL=https://github.com/guialveess/brasa
DefaultDirName={localappdata}\Brasa
DefaultGroupName=Brasa
DisableProgramGroupPage=yes
DisableDirPage=yes
DisableWelcomePage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
OutputDir=.
OutputBaseFilename=Brasa-{#AppVersion}-windows-x86_64-setup
UninstallDisplayIcon={app}\brasa.exe
UninstallDisplayName=Brasa

[Files]
Source: "brasa.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "NOTICE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Brasa"; Filename: "{app}\brasa.exe"
Name: "{autodesktop}\Brasa"; Filename: "{app}\brasa.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Run]
; A person running the installer by hand: offer to launch Brasa, like any installer. Skipped
; entirely in silent mode (`skipifsilent`), so this never fires for a background self-update.
Filename: "{app}\brasa.exe"; Description: "Launch Brasa"; Flags: nowait postinstall skipifsilent
; The self-update path: `windows_installer_args` runs this installer `/VERYSILENT
; /SUPPRESSMSGBOXES /NORESTART /relaunch=1`. Brasa already quit to let the installer replace its
; exe, so this is what actually relaunches it — the flag above does not run in silent mode.
Filename: "{app}\brasa.exe"; Flags: nowait; Check: ShouldRelaunch

[Code]
function ShouldRelaunch: Boolean;
begin
  Result := CompareText(ExpandConstant('{param:relaunch|0}'), '1') = 0;
end;
