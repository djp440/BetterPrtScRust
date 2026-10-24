; BetterPrtSc 安装包（需要 Inno Setup 6.3 及以后的版本）。由 scripts\release\package.ps1 调用：
;   ISCC /DAppVersion=<版本号> /DBinDir=<含 BetterPrtSc.exe 与 bps.exe 的目录> /O<输出目录> /F<文件名> BetterPrtSc.iss
; 可选 /DLicenseFile=<许可证文件>。安装包体积上限 15 MB，由 check-size.ps1 检查。

#ifndef AppVersion
  #error 需要 /DAppVersion=<版本号>
#endif
#ifndef BinDir
  #error 需要 /DBinDir=<含 BetterPrtSc.exe 与 bps.exe 的目录>
#endif

#define AppName "BetterPrtSc"
#define AppExeName "BetterPrtSc.exe"
#define AppUrl "https://github.com/djp440/BetterPrtScRust"

[Setup]
; AppId 发布后不能再改：升级与卸载都靠它识别同一个程序。
AppId={{9766FF1D-3D2A-4139-9B47-BDDCDD572D08}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=djp440
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
; 默认按用户安装到 %LOCALAPPDATA%\Programs，不弹 UAC；也可以在对话框里改为给所有用户安装。
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 1903
MinVersion=10.0.18362
OutputDir=..\..\dist
OutputBaseFilename={#AppName}-{#AppVersion}-x64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ShowLanguageDialog=auto
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#AppExeName}
#ifdef LicenseFile
LicenseFile={#LicenseFile}
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
#if FileExists(AddBackslash(CompilerPath) + "Languages\ChineseSimplified.isl")
Name: "chinesesimplified"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"
#endif

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; 安装版不放 portable 标记：配置和日志在 %APPDATA%\BetterPrtSc。
Source: "{#BinDir}\BetterPrtSc.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\bps.exe"; DestDir: "{app}"; Flags: ignoreversion
#ifdef LicenseFile
Source: "{#LicenseFile}"; DestDir: "{app}"; Flags: ignoreversion
#endif

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(AppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  ResidentWindowClass = 'BetterPrtSc.ResidentWindow';
  WM_CLOSE = $0010;

{ 让正在运行的常驻实例自行退出（注销热键、移除托盘图标），以便覆盖或删除程序文件；最多等 5 秒。
  同一会话里只会有一个常驻实例（单实例）。 }
procedure CloseResidentInstance();
var
  Wnd: HWND;
  I: Integer;
begin
  Wnd := FindWindowByClassName(ResidentWindowClass);
  if Wnd = 0 then
    Exit;
  PostMessage(Wnd, WM_CLOSE, 0, 0);
  for I := 1 to 50 do
  begin
    Sleep(100);
    if FindWindowByClassName(ResidentWindowClass) = 0 then
    begin
      { 窗口销毁后进程还要片刻才退出并释放 exe。 }
      Sleep(500);
      Exit;
    end;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  CloseResidentInstance();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  CloseResidentInstance();
  Result := True;
end;
