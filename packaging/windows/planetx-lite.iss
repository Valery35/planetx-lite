; PlanetX Lite - глобус PlanetX без QGIS.
; Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
; SPDX-License-Identifier: Apache-2.0
;
; Установщик Windows, Inno Setup 6. Ставит программу пользователю без
; прав администратора в %LOCALAPPDATA%\Programs\PlanetX Lite, ярлык
; в меню «Пуск», связь с файлами .planetx. Связь с KML и KMZ - по выбору,
; по умолчанию выключена: эти файлы может открывать Google Earth.
;
; ISCC /DVersion=0.1.0 /DSource=папка-с-файлами /Oпапка-выхода planetx-lite.iss

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef Source
  #define Source "..\..\target\release"
#endif

[Setup]
AppId={{6F0B8E2A-4D1C-4B7E-9A55-3C2F8E1D7A40}
AppName=PlanetX Lite
AppVersion={#Version}
AppVerName=PlanetX Lite {#Version}
AppPublisher=Inform++ LLC
AppPublisherURL=https://github.com/Valery35/planetx-lite
AppSupportURL=https://github.com/Valery35/planetx-lite/issues
DefaultDirName={autopf}\PlanetX Lite
DefaultGroupName=PlanetX Lite
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile={#Source}\LICENSE.txt
SetupIconFile=..\planetx.ico
UninstallDisplayIcon={app}\planetx-lite.exe
UninstallDisplayName=PlanetX Lite
OutputBaseFilename=PlanetX-Lite-{#Version}-windows-x86_64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes
VersionInfoVersion={#Version}
VersionInfoProductName=PlanetX Lite
VersionInfoCompany=Inform++ LLC

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
ru.KmlAssoc=Открывать файлы KML и KMZ в PlanetX Lite
en.KmlAssoc=Open KML and KMZ files in PlanetX Lite
ru.SceneType=Сцена PlanetX
en.SceneType=PlanetX scene

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "kml"; Description: "{cm:KmlAssoc}"; Flags: unchecked

[Files]
Source: "{#Source}\planetx-lite.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\THIRD-PARTY-LICENSES.html"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\PlanetX Lite"; Filename: "{app}\planetx-lite.exe"
Name: "{autodesktop}\PlanetX Lite"; Filename: "{app}\planetx-lite.exe"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\.planetx"; ValueType: string; ValueName: ""; ValueData: "PlanetXLite.Scene"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\PlanetXLite.Scene"; ValueType: string; ValueName: ""; ValueData: "{cm:SceneType}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\PlanetXLite.Scene\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\planetx-lite.exe,0"
Root: HKA; Subkey: "Software\Classes\PlanetXLite.Scene\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\planetx-lite.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.kml\OpenWithProgids"; ValueType: string; ValueName: "PlanetXLite.Scene"; ValueData: ""; Flags: uninsdeletevalue; Tasks: kml
Root: HKA; Subkey: "Software\Classes\.kmz\OpenWithProgids"; ValueType: string; ValueName: "PlanetXLite.Scene"; ValueData: ""; Flags: uninsdeletevalue; Tasks: kml

[Run]
Filename: "{app}\planetx-lite.exe"; Description: "{cm:LaunchProgram,PlanetX Lite}"; Flags: nowait postinstall skipifsilent
