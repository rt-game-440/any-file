; AnyFile Windows 安装包脚本（Inno Setup 6.3+）
; 构建：scripts/build_installer.ps1（先装配 dist/payload，再调用 ISCC）
; 验证：scripts/verify_installer.ps1（编译 + 注册表项/卸载清理对称性检查，不会运行安装包、不写注册表）
;
; 设计要点：
;   - 默认当前用户安装（PrivilegesRequired=lowest，免 UAC），启动时对话框可切换为全机安装。
;   - 所有注册表条目一律使用 HKA 根：当前用户安装 -> HKCU\Software\...，全机安装 -> HKLM\Software\...。
;   - 按 图像/视频/音频/文本代码 4 类任务（Tasks）分别注册 ProgID、Capabilities\FileAssociations、
;     以及各扩展名的 OpenWithProgids；Capabilities 根键与 RegisteredApplications 在任一类被勾选时注册。
;   - 所有注册表条目均带 uninsdeletekey / uninsdeletevalue，卸载时对称清理。
;   - 扩展名清单需与 src/probing.rs::classify_by_extension 保持一致（tests/installer_script.rs 会校验）。

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef PayloadDir
  #define PayloadDir "..\dist\payload"
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

#define AppName "AnyFile"
#define AppExe "anyfile.exe"
#define AppPublisher "AnyFile Contributors"
#define AppUrl "https://github.com/rt-game-440/any-file"

; ---- 各大类扩展名（分号分隔，不含点；与 src/probing.rs 保持一致）----
#define ExtImage "png;jpg;jpeg;webp;gif;bmp;ico;cur;tif;tiff;svg;avif;apng"
#define ExtVideo "mp4;mkv;webm;mov;avi;wmv;flv;m4v;3gp"
#define ExtAudio "mp3;wav;flac;ogg;m4a;aac;opus;wma"
#define ExtText "txt;md;markdown;json;yaml;yml;toml;xml;ini;cfg;conf;log;csv;tsv;rs;js;mjs;cjs;jsx;ts;tsx;py;c;cpp;cc;cxx;h;hpp;cs;java;go;rb;php;swift;kt;html;htm;css;scss;less;sql;sh;bash;zsh;ps1;bat;cmd;env;gitignore;slint"

[Setup]
AppId={{6F2C8E1A-5B7D-4C39-9A64-3E1D2B8F7A10}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
LicenseFile={#PayloadDir}\LICENSE
OutputDir={#OutputDir}
OutputBaseFilename=AnyFile-{#AppVersion}-setup-x64
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ChangesAssociations=yes
WizardStyle=modern
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
english.AssocGroup=File associations (register AnyFile as an available handler):
english.AssocImage=Images / 图像 (PNG, JPEG, WebP, GIF, BMP, ICO, TIFF, SVG, AVIF ...)
english.AssocVideo=Videos / 视频 (MP4, MKV, WebM, MOV, AVI ...)
english.AssocAudio=Audio / 音频 (MP3, WAV, FLAC, OGG, M4A, AAC, OPUS ...)
english.AssocText=Text and code / 文本与代码 (TXT, MD, JSON, source code ...)
english.OpenDefaultApps=Open Windows Default apps settings to finish choosing AnyFile / 打开系统“默认应用”完成默认设置
english.RemoveUserData=Also delete AnyFile user settings and recent list (%APPDATA%\AnyFile)?

[Tasks]
Name: "assoc_image"; Description: "{cm:AssocImage}"; GroupDescription: "{cm:AssocGroup}"
Name: "assoc_video"; Description: "{cm:AssocVideo}"; GroupDescription: "{cm:AssocGroup}"
Name: "assoc_audio"; Description: "{cm:AssocAudio}"; GroupDescription: "{cm:AssocGroup}"
Name: "assoc_text"; Description: "{cm:AssocText}"; GroupDescription: "{cm:AssocGroup}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\mpv-2.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#PayloadDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\THIRD_PARTY_LICENSES.md"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#PayloadDir}\GPL-COMPLIANCE.md"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
; 完成页：一键跳转 Windows“默认应用”设置（用户自行勾选后触发）
Filename: "ms-settings:defaultapps"; Description: "{cm:OpenDefaultApps}"; Flags: postinstall shellexec nowait skipifsilent unchecked
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: postinstall nowait skipifsilent unchecked

; ---------------------------------------------------------------------------
; 注册表：全部使用 HKA；每一条都带 uninsdeletekey 或 uninsdeletevalue
; ---------------------------------------------------------------------------
[Registry]
; 通用：Applications\anyfile.exe（“打开方式”列表）
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExe}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Flags: uninsdeletekey

; Capabilities 根（任一类被勾选时才注册）与 RegisteredApplications
Root: HKA; Subkey: "Software\AnyFile"; ValueType: none; Flags: uninsdeletekeyifempty; Check: AnyAssocSelected
Root: HKA; Subkey: "Software\AnyFile\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "{#AppName}"; Flags: uninsdeletekey; Check: AnyAssocSelected
Root: HKA; Subkey: "Software\AnyFile\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "Lightweight, cold-start-fast viewer for images, video, audio and text"; Flags: uninsdeletekey; Check: AnyAssocSelected
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "{#AppName}"; ValueData: "Software\AnyFile\Capabilities"; Flags: uninsdeletevalue; Check: AnyAssocSelected

; ProgIDs（按大类）
Root: HKA; Subkey: "Software\Classes\AnyFile.Image"; ValueType: string; ValueName: ""; ValueData: "AnyFile Image"; Flags: uninsdeletekey; Tasks: assoc_image
Root: HKA; Subkey: "Software\Classes\AnyFile.Image\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"; Flags: uninsdeletekey; Tasks: assoc_image
Root: HKA; Subkey: "Software\Classes\AnyFile.Image\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Flags: uninsdeletekey; Tasks: assoc_image
Root: HKA; Subkey: "Software\Classes\AnyFile.Video"; ValueType: string; ValueName: ""; ValueData: "AnyFile Video"; Flags: uninsdeletekey; Tasks: assoc_video
Root: HKA; Subkey: "Software\Classes\AnyFile.Video\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"; Flags: uninsdeletekey; Tasks: assoc_video
Root: HKA; Subkey: "Software\Classes\AnyFile.Video\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Flags: uninsdeletekey; Tasks: assoc_video
Root: HKA; Subkey: "Software\Classes\AnyFile.Audio"; ValueType: string; ValueName: ""; ValueData: "AnyFile Audio"; Flags: uninsdeletekey; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\AnyFile.Audio\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"; Flags: uninsdeletekey; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\AnyFile.Audio\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Flags: uninsdeletekey; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\AnyFile.Text"; ValueType: string; ValueName: ""; ValueData: "AnyFile Text"; Flags: uninsdeletekey; Tasks: assoc_text
Root: HKA; Subkey: "Software\Classes\AnyFile.Text\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"; Flags: uninsdeletekey; Tasks: assoc_text
Root: HKA; Subkey: "Software\Classes\AnyFile.Text\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Flags: uninsdeletekey; Tasks: assoc_text

; 各扩展名：OpenWithProgids（值级删除，不碰扩展名键本身）+ Capabilities\FileAssociations
#define i 0
#define Cur ""
#define Rest ""
#define CurProg ""
#define CurTask ""
#sub EmitNext
  #expr Cur = Copy(Rest, 1, Pos(";", Rest) - 1)
  #expr Rest = Copy(Rest, Pos(";", Rest) + 1)
Root: HKA; Subkey: "Software\Classes\.{#Cur}\OpenWithProgids"; ValueType: string; ValueName: "{#CurProg}"; ValueData: ""; Flags: uninsdeletevalue; Tasks: {#CurTask}
Root: HKA; Subkey: "Software\AnyFile\Capabilities\FileAssociations"; ValueType: string; ValueName: ".{#Cur}"; ValueData: "{#CurProg}"; Flags: uninsdeletevalue; Tasks: {#CurTask}
#endsub
#expr CurProg = "AnyFile.Image", CurTask = "assoc_image", Rest = ExtImage + ";"
#for {i = 0; Rest != ""; i++} EmitNext
#expr CurProg = "AnyFile.Video", CurTask = "assoc_video", Rest = ExtVideo + ";"
#for {i = 0; Rest != ""; i++} EmitNext
#expr CurProg = "AnyFile.Audio", CurTask = "assoc_audio", Rest = ExtAudio + ";"
#for {i = 0; Rest != ""; i++} EmitNext
#expr CurProg = "AnyFile.Text", CurTask = "assoc_text", Rest = ExtText + ";"
#for {i = 0; Rest != ""; i++} EmitNext

[Code]
function AnyAssocSelected: Boolean;
begin
  Result := WizardIsTaskSelected('assoc_image') or WizardIsTaskSelected('assoc_video')
         or WizardIsTaskSelected('assoc_audio') or WizardIsTaskSelected('assoc_text');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    { 用户设置位于 %APPDATA%\AnyFile；静默卸载默认保留，交互卸载时询问 }
    DataDir := ExpandConstant('{userappdata}\AnyFile');
    if DirExists(DataDir) and (not UninstallSilent) then
      if MsgBox(CustomMessage('RemoveUserData'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(DataDir, True, True, True);
  end;
end;

#ifdef DumpPreprocessed
  #expr SaveToFile(DumpPreprocessed)
#endif
