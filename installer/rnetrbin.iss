; rnetrbin installer (Inno Setup 6). Build with installer\build.ps1.
; Internal use only: the exe embeds the third-party netrbin.sys (see AGENTS.md).
; The service is NOT registered here: the window's Configuracoes -> Instalar does it
; (it needs protected folders first) and points it at {app}\rnetrbin-svc.exe.

#ifndef AppVersion
  ; Not passed with /DAppVersion=x.y.z: read `version = "..."` from the crate manifest.
  #define AppVersion ""
  #define CargoLine ""
  #define CargoFile FileOpen(AddBackslash(SourcePath) + "..\crates\svc\Cargo.toml")
  ; `public`: a #define inside #sub is otherwise local to it.
  #sub ReadCargoLine
    #define public CargoLine FileRead(CargoFile)
    #define public AppVersion Pos('version = "', CargoLine) == 1 ? Copy(CargoLine, 12, Len(CargoLine) - 12) : ""
  #endsub
  #for {0; AppVersion == "" && !FileEof(CargoFile); 0} ReadCargoLine
  #expr FileClose(CargoFile)
  #if AppVersion == ""
    #error Could not read the version from crates\svc\Cargo.toml; pass /DAppVersion=x.y.z
  #endif
#endif

#define ServiceName "rnetrbin-svc"
#define ExeName "rnetrbin-svc.exe"

[Setup]
AppId={{CF494231-F3A9-44C0-8FB2-4B18A5A12B7D}
AppName=rnetrbin
AppVersion={#AppVersion}
AppVerName=rnetrbin {#AppVersion}
AppPublisher=rnetrbin
DefaultDirName={autopf}\rnetrbin
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\installer
OutputBaseFilename=rnetrbin-setup-{#AppVersion}
UninstallDisplayIcon={app}\{#ExeName}
UninstallDisplayName=rnetrbin - Lixeira de rede
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#ExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\config.example.toml"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\crates\svc\assets\LICENSE-MaterialIcons.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\rnetrbin - Lixeira de rede"; Filename: "{app}\{#ExeName}"
Name: "{autodesktop}\rnetrbin - Lixeira de rede"; Filename: "{app}\{#ExeName}"; Tasks: desktopicon

[Run]
; shellexec: the exe asks for UAC itself when needed.
Filename: "{app}\{#ExeName}"; Description: "Abrir o rnetrbin"; Flags: postinstall shellexec nowait skipifsilent

[Code]
const
  ERROR_SERVICE_CANNOT_ACCEPT_CTRL = 1061;
  ERROR_SERVICE_NOT_ACTIVE = 1062;
  ERROR_SERVICE_DOES_NOT_EXIST = 1060;
  STOP_TIMEOUT_SECS = 150; { the service may take up to 120 s to hand SNetRBin back }

var
  ServiceWasRunning: Boolean;

function Sc(Params: String): Integer;
var
  Code: Integer;
begin
  if not Exec(ExpandConstant('{sys}\sc.exe'), Params, '', SW_HIDE, ewWaitUntilTerminated, Code) then
    Code := -1;
  Result := Code;
end;

{ Upgrade: stop the service before its exe is replaced. sc stop returns 0 when the
  stop was sent, 1061 while it is still stopping, 1062 once stopped, 1060 if absent. }
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code, Waited: Integer;
begin
  Result := '';
  ServiceWasRunning := False;
  Code := Sc('stop {#ServiceName}');
  if (Code = ERROR_SERVICE_DOES_NOT_EXIST) or (Code = ERROR_SERVICE_NOT_ACTIVE) then
    Exit;
  if (Code <> 0) and (Code <> ERROR_SERVICE_CANNOT_ACCEPT_CTRL) then
  begin
    Result := 'Não foi possível parar o serviço {#ServiceName} (código ' + IntToStr(Code) + ').';
    Exit;
  end;
  ServiceWasRunning := True;
  Waited := 0;
  repeat
    Sleep(1000);
    Waited := Waited + 1;
    Code := Sc('stop {#ServiceName}');
  until (Code = ERROR_SERVICE_NOT_ACTIVE) or (Waited >= STOP_TIMEOUT_SECS);
  if Code <> ERROR_SERVICE_NOT_ACTIVE then
    Result := 'O serviço {#ServiceName} demorou demais para parar; tente de novo em instantes.';
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and ServiceWasRunning then
    Sc('start {#ServiceName}');
end;

{ Cancelled or failed after PrepareToInstall stopped the service: start it again.
  After a normal install it is already running and this is a no-op (errors ignored). }
procedure DeinitializeSetup();
begin
  if ServiceWasRunning then
    Sc('start {#ServiceName}');
end;

{ Remove the service, the driver (unless SNetRBin still uses it) and the audit settings
  before any file is deleted. [UninstallRun] would ignore a failure and delete the exe,
  leaving an automatic service pointing at nothing; here a failure aborts and keeps it all. }
function InitializeUninstall(): Boolean;
var
  Code: Integer;
begin
  Result := True;
  { An exe that cannot even be launched (already gone) has nothing left to remove. }
  if not Exec(ExpandConstant('{app}\{#ExeName}'), 'uninstall', '', SW_HIDE, ewWaitUntilTerminated, Code) then
    Exit;
  if Code <> 0 then
  begin
    SuppressibleMsgBox('Não foi possível remover o serviço do rnetrbin (código ' + IntToStr(Code) + '). ' +
      'Nada foi apagado. Pare o serviço e tente de novo.', mbError, MB_OK, IDOK);
    Result := False;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
    MsgBox('O serviço foi removido. A configuração e o índice em C:\ProgramData\rnetrbin ' +
      'e a pasta da lixeira foram mantidos; apague-os à mão se não precisar mais.', mbInformation, MB_OK);
end;
