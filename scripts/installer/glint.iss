; Compiled by scripts/build-installer.ps1 with Inno Setup 6.
#ifndef AppVersion
  #error AppVersion must be supplied by build-installer.ps1
#endif

[Setup]
AppId={{693765B9-5155-492A-A97E-C78E555CC816}
AppName=Glint
AppVersion={#AppVersion}
AppPublisher=Glint
DefaultDirName={localappdata}\Programs\Glint
DefaultGroupName=Glint
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.18362
OutputDir={#ProjectRoot}\dist
OutputBaseFilename=Glint-v{#AppVersion}-windows-x64-setup
SetupIconFile={#ProjectRoot}\assets\glint.ico
UninstallDisplayIcon={app}\glint-settings.exe
LicenseFile={#ProjectRoot}\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
CloseApplicationsFilter=glint.exe,glint-settings.exe
RestartApplications=no
DisableProgramGroupPage=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#BinaryDirectory}\glint.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinaryDirectory}\glint-settings.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RuntimeDirectory}\*.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectRoot}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectRoot}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectRoot}\docs\*.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#ProjectRoot}\assets\*.png"; DestDir: "{app}\assets"; Flags: ignoreversion

[Icons]
Name: "{group}\Glint"; Filename: "{app}\glint-settings.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Glint"; Filename: "{app}\glint-settings.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\glint-settings.exe"; Description: "{cm:LaunchProgram,Glint}"; Flags: nowait postinstall skipifsilent unchecked

[Code]
// Remove startup only when it belongs to this installation. Preserve all user data.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Command, Prefix: String;
begin
  if CurUninstallStep = usUninstall then
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Glint', Command) then
    begin
      Prefix := '"' + ExpandConstant('{app}\glint.exe') + '"';
      if (CompareText(Command, Prefix) = 0) or
         (CompareText(Copy(Command, 1, Length(Prefix) + 1), Prefix + ' ') = 0) then
        RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Glint');
    end;
end;
