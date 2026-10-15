; rnetrbin installer (Inno Setup 6). Build with installer\build.ps1.
; Internal use only: the exe embeds the third-party netrbin.sys (see AGENTS.md).
; Setup installs everything: on a first install it asks for the recycle-bin folder and
; writes a config with zero protected folders (`init-config`), then registers and loads
; the driver and registers the service (`install`, see PrepareToInstall) and starts it.

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
  ERROR_SERVICE_ALREADY_RUNNING = 1056;
  ERROR_SERVICE_CANNOT_ACCEPT_CTRL = 1061;
  ERROR_SERVICE_NOT_ACTIVE = 1062;
  ERROR_SERVICE_DOES_NOT_EXIST = 1060;
  STOP_TIMEOUT_SECS = 150; { the service may take up to 120 s to hand SNetRBin back }
  { Fixed paths of the service (crates\svc\src\config.rs CONFIG_PATH). }
  CONFIG_PATH = 'C:\ProgramData\rnetrbin\config.toml';
  SETUP_LOG = 'C:\ProgramData\rnetrbin\setup.log';
  SENTINEL = '.rnetrbin-root';

var
  { Sticky across a Back + retry of PrepareToInstall: the first stop decides. }
  ServiceExisted: Boolean;
  ServiceWasRunning: Boolean;
  ConfigExisted: Boolean;
  { `install` succeeded in PrepareToInstall / the files were copied. }
  Installed: Boolean;
  FilesDone: Boolean;
  BinPage: TInputDirWizardPage;

procedure AppendSetupLog(const Text: String);
var
  Lines: TArrayOfString;
begin
  Log(Text);
  ForceDirectories(ExtractFileDir(SETUP_LOG));
  SetArrayLength(Lines, 1);
  Lines[0] := GetDateTimeString('yyyy/mm/dd hh:nn:ss', '-', ':') + ' ' + Text;
  SaveStringsToUTF8File(SETUP_LOG, Lines, True);
end;

{ Run a console program hidden and return its exit code (-1 if it could not
  run); Output gets stdout then stderr. Everything also goes to SETUP_LOG. }
function RunCapture(const Exe, Params: String; var Output: String): Integer;
var
  O: TExecOutput;
  Code, I: Integer;
begin
  Output := '';
  try
    if ExecAndCaptureOutput(Exe, Params, '', SW_SHOWNORMAL, ewWaitUntilTerminated, Code, O) then
    begin
      for I := 0 to GetArrayLength(O.StdOut) - 1 do
        Output := Output + O.StdOut[I] + #13#10;
      for I := 0 to GetArrayLength(O.StdErr) - 1 do
        Output := Output + O.StdErr[I] + #13#10;
    end else
    begin
      Output := 'Não foi possível executar ' + Exe + ': ' + SysErrorMessage(Code);
      Code := -1;
    end;
  except
    Output := 'Não foi possível executar ' + Exe + ': ' + GetExceptionMessage;
    Code := -1;
  end;
  Output := Trim(Output);
  AppendSetupLog(ExtractFileName(Exe) + ' ' + Params + ' -> código ' + IntToStr(Code) + #13#10 + Output);
  Result := Code;
end;

{ The new exe extracted to the temp dir: it runs before [Files] copies it to the app dir. }
function RunSvc(const Params: String; var Output: String): Integer;
begin
  Result := RunCapture(ExpandConstant('{tmp}\{#ExeName}'), Params, Output);
end;

function Sc(Params: String): Integer;
var
  Code: Integer;
begin
  if not Exec(ExpandConstant('{sys}\sc.exe'), Params, '', SW_HIDE, ewWaitUntilTerminated, Code) then
    Code := -1;
  Result := Code;
end;

{ ---- "Pasta da lixeira" page: only on a first install (no config yet). ---- }

procedure InitializeWizard();
begin
  ConfigExisted := FileExists(CONFIG_PATH);
  BinPage := CreateInputDirPage(wpSelectDir, 'Pasta da lixeira',
    'Onde o rnetrbin guarda as cópias dos arquivos apagados?',
    'Escolha uma pasta nova ou vazia num disco local (não a raiz do disco). ' +
    'As pastas protegidas são escolhidas depois, na janela do rnetrbin.',
    False, 'RecycleBinNet');
  BinPage.Add('');
  BinPage.Values[0] := ExpandConstant('{sd}\RecycleBinNet');
end;

function ShouldSkipPage(PageID: Integer): Boolean;
begin
  Result := (BinPage <> nil) and (PageID = BinPage.ID) and ConfigExisted;
end;

function BinDir(): String;
begin
  Result := RemoveBackslashUnlessRoot(Trim(BinPage.Values[0]));
end;

{ Same rule as config::ensure_dedicated_dir: new, empty, or already rnetrbin's. }
function DirIsEmptyOrOurs(const Dir: String): Boolean;
var
  F: TFindRec;
  More: Boolean;
begin
  Result := True;
  if (not DirExists(Dir)) or FileExists(AddBackslash(Dir) + SENTINEL) then
    Exit;
  if FindFirst(AddBackslash(Dir) + '*', F) then
  begin
    More := True;
    while More and Result do
    begin
      if (F.Name <> '.') and (F.Name <> '..') then
        Result := False;
      More := FindNext(F);
    end;
    FindClose(F);
  end;
end;

function NextButtonClick(CurPageID: Integer): Boolean;
var
  Dir: String;
begin
  Result := True;
  if (BinPage = nil) or (CurPageID <> BinPage.ID) then
    Exit;
  Dir := BinDir();
  if (Length(Dir) < 4) or (Copy(Dir, 2, 2) <> ':\') then
  begin
    SuppressibleMsgBox('Escolha uma pasta num disco local, como D:\RecycleBinNet ' +
      '(não a raiz do disco nem um caminho de rede).', mbError, MB_OK, IDOK);
    Result := False;
  end else if not DirIsEmptyOrOurs(Dir) then
  begin
    SuppressibleMsgBox('A pasta ' + Dir + ' já tem arquivos. Escolha uma pasta nova ou vazia para a lixeira.',
      mbError, MB_OK, IDOK);
    Result := False;
  end;
end;

{ ---- Install: driver + service before any file is copied. ---- }

{ Upgrade: stop the service before its exe is replaced. sc stop returns 0 when the
  stop was sent, 1061 while it is still stopping, 1062 once stopped, 1060 if absent. }
function StopService(): String;
var
  Code, Waited: Integer;
begin
  Result := '';
  Code := Sc('stop {#ServiceName}');
  if Code = ERROR_SERVICE_DOES_NOT_EXIST then
    Exit;
  ServiceExisted := True;
  if Code = ERROR_SERVICE_NOT_ACTIVE then
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

function Failure(const Head, Output: String): String;
begin
  AppendSetupLog(Head);
  Result := Head + #13#10#13#10 + Output + #13#10#13#10 + 'Registro completo: ' + SETUP_LOG;
  SuppressibleMsgBox(Result, mbCriticalError, MB_OK, IDOK);
end;

{ A failure after [Files] cannot be rolled back (Inno finalizes the uninstaller
  first, and an exception in AfterInstall/ssPostInstall is only logged), so the
  driver and the service are installed HERE, from the exe extracted to the temp dir,
  with the service pointed at the app dir. A non-empty result stops Setup on the
  Preparing page before anything is copied. }
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
  Output, Ignored: String;
  CreatedConfig: Boolean;
begin
  Installed := False;
  Result := StopService();
  if Result <> '' then
    Exit;
  try
    ExtractTemporaryFile('{#ExeName}');
  except
    Result := Failure('Não foi possível extrair o {#ExeName}; nada foi instalado.', GetExceptionMessage);
    Exit;
  end;
  CreatedConfig := False;
  if not FileExists(CONFIG_PATH) then
  begin
    Code := RunSvc('init-config --recycle-dir ' + AddQuotes(BinDir()), Output);
    if Code <> 0 then
    begin
      Result := Failure('Não foi possível criar a configuração inicial; nada foi instalado.', Output);
      Exit;
    end;
    CreatedConfig := True;
  end;
  Code := RunSvc('install --exe ' + AddQuotes(ExpandConstant('{app}\{#ExeName}')), Output);
  if Code <> 0 then
  begin
    if ServiceExisted then
      Result := Failure('A atualização do driver e do serviço falhou. Os arquivos do rnetrbin não ' +
        'foram trocados e o serviço anterior continua registrado.', Output)
    else
    begin
      { First install: undo whatever install did (SNetRBin's driver is kept). }
      RunSvc('uninstall', Ignored);
      if CreatedConfig then
        DeleteFile(CONFIG_PATH);
      Result := Failure('A instalação do driver e do serviço falhou; nada foi instalado.', Output);
    end;
    Exit;
  end;
  Installed := True;
end;

{ After the copy: start the service (first install, or it was running before). A
  failure here cannot abort Setup any more; the service stays installed and the
  window shows its state. }
procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
  Output: String;
begin
  if CurStep <> ssPostInstall then
    Exit;
  FilesDone := True;
  if ServiceExisted and not ServiceWasRunning then
    Exit;
  Code := RunCapture(ExpandConstant('{sys}\sc.exe'), 'start {#ServiceName}', Output);
  if (Code = 0) or (Code = ERROR_SERVICE_ALREADY_RUNNING) then
    ServiceWasRunning := False { started; DeinitializeSetup has nothing left to do }
  else
    SuppressibleMsgBox('O rnetrbin foi instalado, mas o serviço não iniciou (código ' + IntToStr(Code) + ').' + #13#10#13#10 +
      Output + #13#10#13#10 +
      'Veja C:\ProgramData\rnetrbin\rnetrbin-svc.log e use Iniciar em Configurações na janela do rnetrbin.',
      mbError, MB_OK, IDOK);
end;

{ Setup ends (cancelled, failed or done). If the first install registered the service
  but the files never arrived, remove it again (the temp dir still exists here). Otherwise
  restart a service PrepareToInstall stopped (errors ignored). }
procedure DeinitializeSetup();
var
  Output: String;
begin
  if Installed and (not FilesDone) and (not ServiceExisted) then
    RunSvc('uninstall', Output)
  else if ServiceWasRunning then
    Sc('start {#ServiceName}');
end;

{ ---- Uninstall ---- }

{ Remove the service, the driver (unless SNetRBin still uses it) and the audit settings
  before any file is deleted. [UninstallRun] would ignore a failure and delete the exe,
  leaving an automatic service pointing at nothing; here a failure aborts and keeps it all. }
function InitializeUninstall(): Boolean;
var
  Code: Integer;
  Output: String;
begin
  Result := True;
  { An exe that cannot even be launched (already gone) has nothing left to remove. }
  if not FileExists(ExpandConstant('{app}\{#ExeName}')) then
    Exit;
  Code := RunCapture(ExpandConstant('{app}\{#ExeName}'), 'uninstall', Output);
  if Code = -1 then
    Exit;
  if Code <> 0 then
  begin
    SuppressibleMsgBox('Não foi possível remover o serviço do rnetrbin (código ' + IntToStr(Code) + '). ' +
      'Nada foi apagado. Pare o serviço e tente de novo.' + #13#10#13#10 + Output, mbError, MB_OK, IDOK);
    Result := False;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
    MsgBox('O serviço foi removido. A configuração e o índice em C:\ProgramData\rnetrbin ' +
      'e a pasta da lixeira foram mantidos; apague-os à mão se não precisar mais.', mbInformation, MB_OK);
end;
