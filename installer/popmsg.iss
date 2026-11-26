; Installatore di POPMSG per Windows (Inno Setup 6).
; Compilato dalla CI: ISCC /DAppVer=x.y.z [/DHasCert] installer\popmsg.iss
; Installazione silenziosa (es. criterio di gruppo o script):
;   POPMSG-setup.exe /VERYSILENT /SUPPRESSMSGBOXES

#ifndef AppVer
  #define AppVer "0.0.0"
#endif

[Setup]
AppId={{8E2B6C3A-5F1D-4C7E-9A2B-3D4F5E6A7B8C}
AppName=POPMSG
AppVersion={#AppVer}
AppVerName=POPMSG {#AppVer}
AppPublisher=POPMSG
VersionInfoVersion={#AppVer}
DefaultDirName={autopf}\POPMSG
DefaultGroupName=POPMSG
DisableProgramGroupPage=yes
PrivilegesRequired=admin
OutputDir=..\out-setup
OutputBaseFilename=POPMSG-setup
SetupIconFile=..\assets\popmsg.ico
UninstallDisplayIcon={app}\POPMSG.exe
UninstallDisplayName=POPMSG
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=force
RestartApplications=no

[Languages]
Name: "it"; MessagesFile: "compiler:Languages\Italian.isl"

[Tasks]
Name: "autostart"; Description: "Avvia POPMSG all'accesso a Windows (tutti gli utenti)"
Name: "firewall"; Description: "Consenti POPMSG nel firewall di Windows (porta 2425)"
Name: "desktopicon"; Description: "Collegamento sul desktop"; Flags: unchecked

[Files]
Source: "..\target\release\popmsg.exe"; DestDir: "{app}"; DestName: "POPMSG.exe"; Flags: ignoreversion
#ifdef HasCert
Source: "..\POPMSG-firma.cer"; DestDir: "{app}"; Flags: ignoreversion
#endif

[Icons]
Name: "{autoprograms}\POPMSG"; Filename: "{app}\POPMSG.exe"
Name: "{autodesktop}\POPMSG"; Filename: "{app}\POPMSG.exe"; Tasks: desktopicon

[Registry]
Root: HKLM; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "POPMSG"; ValueData: """{app}\POPMSG.exe"""; Flags: uninsdeletevalue; Tasks: autostart

[Run]
#ifdef HasCert
; certificato della firma di POPMSG: attendibile su questo PC
Filename: "{sys}\certutil.exe"; Parameters: "-addstore -f Root ""{app}\POPMSG-firma.cer"""; Flags: runhidden; StatusMsg: "Installazione del certificato di POPMSG..."
Filename: "{sys}\certutil.exe"; Parameters: "-addstore -f TrustedPublisher ""{app}\POPMSG-firma.cer"""; Flags: runhidden
#endif
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=POPMSG"; Flags: runhidden; Tasks: firewall
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=POPMSG dir=in action=allow program=""{app}\POPMSG.exe"" enable=yes profile=any"; Flags: runhidden; StatusMsg: "Regola del firewall..."; Tasks: firewall
Filename: "{app}\POPMSG.exe"; Description: "Avvia POPMSG"; Flags: nowait postinstall skipifsilent runasoriginaluser
; aggiornamento lanciato da POPMSG (installazione silenziosa): lo riavvia
Filename: "{app}\POPMSG.exe"; Parameters: "--show"; Flags: nowait runasoriginaluser; Check: WizardSilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM POPMSG.exe"; Flags: runhidden; RunOnceId: "stop"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=POPMSG"; Flags: runhidden; RunOnceId: "fw"
#ifdef HasCert
Filename: "{sys}\certutil.exe"; Parameters: "-delstore TrustedPublisher POPMSG"; Flags: runhidden; RunOnceId: "cert1"
Filename: "{sys}\certutil.exe"; Parameters: "-delstore Root POPMSG"; Flags: runhidden; RunOnceId: "cert2"
#endif
