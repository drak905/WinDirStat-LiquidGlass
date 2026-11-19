; Inno Setup script for WinDirStat Liquid Glass (native Tauri v2 + WebView2)
#ifndef AppVersion
  #define AppVersion "1.0.0"
#endif

[Setup]
AppId={{B4A8E9D2-7C31-4F62-9A5E-31D8C2B91A47}
AppName=WinDirStat Liquid Glass
AppVersion={#AppVersion}
AppPublisher=WinDirStat Liquid Glass
DefaultDirName={localappdata}\Programs\WinDirStat Liquid Glass
DefaultGroupName=WinDirStat Liquid Glass
PrivilegesRequired=lowest
OutputDir={#OutputDir}
OutputBaseFilename=WinDirStat-LiquidGlass-{#AppVersion}-setup
SetupIconFile={#IconFile}
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\WinDirStat-LiquidGlass.exe
WizardStyle=modern
CloseApplications=yes
CloseApplicationsFilter=*.exe
AppMutex=Local\WinDirStatLiquidGlass_PrimaryWebView2

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{group}\WinDirStat Liquid Glass"; Filename: "{app}\WinDirStat-LiquidGlass.exe"
Name: "{group}\Uninstall WinDirStat Liquid Glass"; Filename: "{uninstallexe}"
Name: "{userdesktop}\WinDirStat Liquid Glass"; Filename: "{app}\WinDirStat-LiquidGlass.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\WinDirStat-LiquidGlass.exe"; Description: "Launch WinDirStat Liquid Glass"; Flags: nowait postinstall skipifsilent
