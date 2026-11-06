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
; Kept next to this script: Inno Setup doesn't install a Chinese translation.
Name: "zh"; MessagesFile: "ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{group}\TinyPlayer"; Filename: "{app}\TinyPlayer.exe"
Name: "{group}\{cm:UninstallProgram,TinyPlayer}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\TinyPlayer"; Filename: "{app}\TinyPlayer.exe"; Tasks: desktopicon

[CustomMessages]
en.PlayWith=Play with TinyPlayer
zh.PlayWith=用 TinyPlayer 播放
en.MediaFile=Media file
zh.MediaFile=媒体文件

; Keep in step with MEDIA_EXTS in src/playlist.rs.
#define public i 0
#dim Exts[23] {"mp4", "mkv", "webm", "mov", "avi", "flv", "wmv", "m4v", "ts", "m2ts", "mpg", "mpeg", "3gp", "ogv", "rmvb", "mp3", "flac", "wav", "m4a", "aac", "ogg", "opus", "wma"}

[Registry]
; Windows doesn't let a program make itself the default player, so nothing here
; touches a file type's current handler. This only offers TinyPlayer: in "Open
; with", in Settings → Default apps, and as a right-click item. Each file type's
; own thumbnail handler is left alone as well.
Root: HKA; Subkey: "Software\Classes\TinyPlayer.Media"; ValueType: string; ValueData: "{cm:MediaFile}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\TinyPlayer.Media\DefaultIcon"; ValueType: string; ValueData: "{app}\TinyPlayer.exe,0"
Root: HKA; Subkey: "Software\Classes\TinyPlayer.Media\shell\open\command"; ValueType: string; ValueData: """{app}\TinyPlayer.exe"" ""%1"""

Root: HKA; Subkey: "Software\Classes\Applications\TinyPlayer.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "TinyPlayer"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\TinyPlayer.exe\shell\open\command"; ValueType: string; ValueData: """{app}\TinyPlayer.exe"" ""%1"""

Root: HKA; Subkey: "Software\TinyPlayer"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\TinyPlayer\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "TinyPlayer"
Root: HKA; Subkey: "Software\TinyPlayer\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "TinyPlayer"
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "TinyPlayer"; ValueData: "Software\TinyPlayer\Capabilities"; Flags: uninsdeletevalue

#sub AddExtension
Root: HKA; Subkey: "Software\Classes\Applications\TinyPlayer.exe\SupportedTypes"; ValueType: string; ValueName: ".{#Exts[i]}"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.{#Exts[i]}\OpenWithProgids"; ValueType: string; ValueName: "TinyPlayer.Media"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\TinyPlayer\Capabilities\FileAssociations"; ValueType: string; ValueName: ".{#Exts[i]}"; ValueData: "TinyPlayer.Media"
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.{#Exts[i]}\shell\TinyPlayer"; ValueType: string; ValueData: "{cm:PlayWith}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.{#Exts[i]}\shell\TinyPlayer"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\TinyPlayer.exe,0"
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\.{#Exts[i]}\shell\TinyPlayer\command"; ValueType: string; ValueData: """{app}\TinyPlayer.exe"" ""%1"""
#endsub
#for {i = 0; i < DimOf(Exts); i++} AddExtension

[Run]
Filename: "{app}\TinyPlayer.exe"; Description: "{cm:LaunchProgram,TinyPlayer}"; Flags: nowait postinstall skipifsilent
