; Cuter 的安装包定义（Inno Setup 6）。
;
; 编译（不要直接双击本文件，先跑脚本，它会准备负载与路径）：
;   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\package\build_installer.ps1
;
; 本文件只负责"怎么装"，不负责"装什么"：负载目录由 /DPayloadDir= 传进来，
; 版本号直接**从 exe 自己的版本资源**里读（这样安装包的版本号不可能和程序对不上）。
;
; 几个刻意的决定：
;
; * **按用户安装**（`{localappdata}\Programs\Cuter`，`PrivilegesRequired=lowest`）：
;   截图工具没有理由要管理员权限；需要装到 Program Files 的用户可以在向导里选。
; * **不带 `cuter.portable` 标记**：安装版就该用 `%APPDATA%\Cuter`，便携标记只属于 zip 版。
; * **自启不是直接写注册表**，而是调用程序自己的 `--enable-autostart`：
;   自启是"配置 + 注册表"两处状态，必须由同一个作用点写，否则第一次启动就会被对账改回去。
; * **卸载保留用户配置**（配置文件根本不在安装目录里，Inno 也不会去动 `%APPDATA%`），
;   并在确认页把这件事说清楚。

#ifndef PayloadDir
  #error 请通过 /DPayloadDir=<目录> 指定负载目录（用 tools\package\build_installer.ps1）
#endif

#define AppName "Cuter"
#define AppPublisher "Cuter contributors"
#define AppExeName "cuter.exe"
#define AppExe PayloadDir + "\" + AppExeName
; 版本号来自 exe 的版本资源（build.rs 写进去的），单一来源。
#define AppVersion GetFileVersionString(AppExe)
; 图标也从仓库里那份来（它由 src/icon.rs 生成，见 crates/cuter-app/build.rs 的说明）。
#ifdef IconFile
  #define SetupIcon IconFile
#else
  #define SetupIcon "..\..\crates\cuter-app\assets\cuter.ico"
#endif

[Setup]
; AppId 必须**永远不变**：升级安装靠它认出旧版本。换掉它 = 用户会装出两份程序。
AppId={{2F6C1E4A-9B3D-4A7E-8C51-6D0B7F3A5E92}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppCopyright=GPL-3.0-or-later
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableWelcomePage=no
LicenseFile={#SourcePath}\..\..\LICENSE
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir={#OutputDir}
OutputBaseFilename=cuter-{#AppVersion}-x64-setup
SetupIconFile={#SetupIcon}
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName} {#AppVersion}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
; 安装包自己也要有版本资源：Explorer 的详细信息里看得到，企业部署脚本也靠它比对。
VersionInfoVersion={#AppVersion}
VersionInfoCompany={#AppPublisher}
VersionInfoDescription={#AppName} {#AppVersion} Setup
VersionInfoProductName={#AppName}
VersionInfoProductVersion={#AppVersion}
VersionInfoCopyright=GPL-3.0-or-later
; 便携版 zip 已经能满足"绿色使用"，安装包不该再塞给别人一个可选捆绑的东西。
AllowNoIcons=yes

[Languages]
Name: "chinese"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[CustomMessages]
chinese.CreateDesktopIcon=创建桌面快捷方式
chinese.AdditionalIcons=附加图标：
chinese.AutoStartTask=开机时自动启动（只驻留托盘，不弹窗口）
chinese.AutoStartTaskHint=不勾选也可以：装完之后在「设置 → 通用」里随时可以打开。
chinese.LaunchApp=立即运行 {#AppName}
chinese.KeepSettingsHint=你的设置、日志与截图保存在 %APPDATA%\Cuter，卸载时**不会被删除**。
chinese.InstalledAs={#AppName} {#AppVersion} 已安装到：

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "autostart"; Description: "{cm:AutoStartTask}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\cuter.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\README.txt"; DestDir: "{app}"; Flags: ignoreversion isreadme

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Run]
; 自启：让程序自己去写（配置 + 注册表两处状态），并且**必须等它跑完**，
; 否则安装器可能先退出，用户第一次启动时看到的还是"没开自启"。
Filename: "{app}\{#AppExeName}"; Parameters: "--enable-autostart"; Flags: runhidden waituntilterminated; Tasks: autostart
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchApp}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; 卸载时把自己那条自启项删掉（同样走程序自己的作用点，只删指向自己的那条）。
; skipifdoesntexist：文件已经没了就别让卸载失败。
; RunOnceId：Inno 要求有它，否则卸载在"多用户安装"等场景下可能重复执行。
Filename: "{app}\{#AppExeName}"; Parameters: "--disable-autostart"; Flags: runhidden waituntilterminated skipifdoesntexist; RunOnceId: "DisableAutostart"

[UninstallDelete]
; 只删安装目录里我们自己生成的东西；用户配置在 %APPDATA% 里，不碰。
Type: filesandordirs; Name: "{app}\logs"

[Messages]
chinese.ConfirmUninstall=确定要卸载 %1 吗？%n%n程序文件会被删除；你的设置、日志与截图会保留在 %APPDATA%\Cuter。
chinese.FinishedLabel=%n%n{cm:InstalledAs}%n%n%1%n%n{cm:KeepSettingsHint}
chinese.FinishedLabelNoIcons=%n%n{cm:InstalledAs}%n%n%1%n%n{cm:KeepSettingsHint}
