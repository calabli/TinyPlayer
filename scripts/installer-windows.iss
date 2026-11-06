; Windows installer, built by scripts\package-windows.ps1 with Inno Setup.
; Installs for the current user, so it needs no administrator rights.
#ifndef AppVersion
  #error Pass /DAppVersion=<version>
#endif
#ifndef SourceDir
  #error Pass /DSourceDir=<folder with the packaged files>
#endif

[Setup]
AppId={{7F2B9A34-5C1E-4E0B-9A6D-2D1C8E4F7A10}
AppName=TinyPlayer
AppVersion={#AppVersion}
AppPublisher=TinyPlayer
DefaultDirName={autopf}\TinyPlayer
DefaultGroupName=TinyPlayer
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=TinyPlayer-{#AppVersion}-windows-x64-setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\TinyPlayer.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "zh"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{group}\TinyPlayer"; Filename: "{app}\TinyPlayer.exe"
Name: "{group}\{cm:UninstallProgram,TinyPlayer}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\TinyPlayer"; Filename: "{app}\TinyPlayer.exe"; Tasks: desktopicon

[Registry]
; Lists TinyPlayer under "Open with" without taking over any file type.
Root: HKA; Subkey: "Software\Classes\Applications\TinyPlayer.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "TinyPlayer"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\TinyPlayer.exe\shell\open\command"; ValueType: string; ValueData: """{app}\TinyPlayer.exe"" ""%1"""
#define SupportedType(ext) 'Root: HKA; Subkey: "Software\Classes\Applications\TinyPlayer.exe\SupportedTypes"; ValueType: string; ValueName: ".' + ext + '"; ValueData: ""'
#emit SupportedType("mp4")
#emit SupportedType("mkv")
#emit SupportedType("webm")
#emit SupportedType("mov")
#emit SupportedType("avi")
#emit SupportedType("flv")
#emit SupportedType("wmv")
#emit SupportedType("m4v")
#emit SupportedType("ts")
#emit SupportedType("m2ts")
#emit SupportedType("mpg")
#emit SupportedType("mpeg")
#emit SupportedType("rmvb")
#emit SupportedType("mp3")
#emit SupportedType("flac")
#emit SupportedType("wav")
#emit SupportedType("m4a")

[Run]
Filename: "{app}\TinyPlayer.exe"; Description: "{cm:LaunchProgram,TinyPlayer}"; Flags: nowait postinstall skipifsilent
