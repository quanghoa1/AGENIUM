; Agenium's installer hooks (ADR-154). The installer asks where Agenium works: the folder that holds the user's skills,
; MCP servers, agents, projects and memory (the root). It is remembered in %APPDATA%\agenium\bootstrap.toml, which the
; app and the `agenium` command read; the default is %USERPROFILE%\.agenium. An update shows the folder in use; the
; installer never moves, changes or removes what is in it, and uninstalling leaves it.
;
; Silent install: `/S /DATA=D:\Agenium` sets the folder; without /DATA a folder chosen before is kept.

!include nsDialogs.nsh
!include LogicLib.nsh

Var AgeniumData
Var AgeniumDataField
Var AgeniumDataVi

; The folder remembered by an earlier install (bootstrap.toml's `root`), else the default.
Function AgeniumDataDefault
  StrCpy $AgeniumData "$PROFILE\.agenium"
  ClearErrors
  FileOpen $0 "$APPDATA\agenium\bootstrap.toml" r
  IfErrors done
  loop:
    ClearErrors
    FileRead $0 $1
    IfErrors close
    StrCpy $2 $1 7
    StrCmp $2 "root = " 0 loop
    StrCpy $1 $1 "" 7
    ; Strip the line end and the quotes; a basic string doubles its backslashes.
    ${WordReplace} $1 "$\r" "" "+" $1
    ${WordReplace} $1 "$\n" "" "+" $1
    StrCpy $2 $1 1
    StrCpy $1 $1 "" 1
    StrCpy $1 $1 -1
    ${If} $2 == '"'
      ${WordReplace} $1 "\\" "\" "+" $1
    ${EndIf}
    ${If} $1 != ""
      StrCpy $AgeniumData $1
    ${EndIf}
  close:
    FileClose $0
  done:
FunctionEnd

; A folder picked that is not an Agenium folder gets one inside it: "D:\" becomes "D:\Agenium".
Function AgeniumDataUnder
  Exch $0
  StrCpy $1 $0 1 -1
  ${If} $1 == "\"
    StrCpy $0 $0 -1
  ${EndIf}
  StrCpy $1 $0 7 -7
  ${IfNot} ${FileExists} "$0\config.toml"
  ${AndIf} $1 != "agenium"
    StrCpy $0 "$0\Agenium"
  ${EndIf}
  Exch $0
FunctionEnd

Function AgeniumDataPage
  Call SkipIfPassive
  ${If} $AgeniumData == ""
    Call AgeniumDataDefault
  ${EndIf}
  ; 1066 is Vietnamese: the installer speaks the language picked at its start.
  ${If} $LANGUAGE == 1066
    StrCpy $AgeniumDataVi 1
    !insertmacro MUI_HEADER_TEXT "Thư mục làm việc của Agenium" "Nơi Agenium lưu skill, MCP server, agent, dự án và memory của bạn."
  ${Else}
    StrCpy $AgeniumDataVi 0
    !insertmacro MUI_HEADER_TEXT "Agenium's work folder" "Where Agenium keeps your skills, MCP servers, agents, projects and memory."
  ${EndIf}
  nsDialogs::Create 1018
  Pop $0
  ${If} $AgeniumDataVi == 1
    ${NSD_CreateLabel} 0 0 100% 20u "Chọn thư mục, ví dụ trên ổ D:. Mặc định là thư mục .agenium trong thư mục người dùng."
  ${Else}
    ${NSD_CreateLabel} 0 0 100% 20u "Choose a folder, for example on drive D:. The default is .agenium in your user folder."
  ${EndIf}
  Pop $0
  ${NSD_CreateDirRequest} 0 24u 78% 13u "$AgeniumData"
  Pop $AgeniumDataField
  ${If} $AgeniumDataVi == 1
    ${NSD_CreateBrowseButton} 80% 23u 20% 15u "Chọn..."
  ${Else}
    ${NSD_CreateBrowseButton} 80% 23u 20% 15u "Browse..."
  ${EndIf}
  Pop $0
  ${NSD_OnClick} $0 AgeniumDataBrowse
  ${If} $AgeniumDataVi == 1
    ${NSD_CreateLabel} 0 46u 100% 40u "Cập nhật hay gỡ cài đặt Agenium không bao giờ đụng tới thư mục này. Đổi sang thư mục khác không di chuyển dữ liệu đang có ở thư mục cũ: một thư mục đã có dữ liệu Agenium sẽ được mở lại như cũ."
  ${Else}
    ${NSD_CreateLabel} 0 46u 100% 40u "Updating or uninstalling Agenium never touches this folder. Choosing another folder does not move the data in the old one: a folder that already holds Agenium's data opens as it is."
  ${EndIf}
  Pop $0
  nsDialogs::Show
FunctionEnd

Function AgeniumDataBrowse
  ${NSD_GetText} $AgeniumDataField $0
  nsDialogs::SelectFolderDialog "" "$0"
  Pop $0
  ${If} $0 != "error"
  ${AndIf} $0 != ""
    Push $0
    Call AgeniumDataUnder
    Pop $0
    ${NSD_SetText} $AgeniumDataField "$0"
  ${EndIf}
FunctionEnd

Function AgeniumDataLeave
  ${NSD_GetText} $AgeniumDataField $0
  ; A full path: a drive ("D:\...") or a network share ("\\server\...").
  StrCpy $1 $0 2 1
  StrCpy $2 $0 2
  ${If} $1 != ":\"
  ${AndIf} $2 != "\\"
    ${If} $AgeniumDataVi == 1
      MessageBox MB_ICONEXCLAMATION "Hãy chọn một đường dẫn đầy đủ, ví dụ D:\Agenium."
    ${Else}
      MessageBox MB_ICONEXCLAMATION "Choose a full path, for example D:\Agenium."
    ${EndIf}
    Abort
  ${EndIf}
  StrCpy $1 $0 1 -1
  ${If} $1 == "\"
    StrCpy $0 $0 -1
  ${EndIf}
  StrCpy $AgeniumData $0
FunctionEnd

!macro NSIS_HOOK_POSTINSTALL
  ; A silent install takes /DATA=; otherwise the folder chosen before stays as it is.
  ${If} $AgeniumData == ""
    ${GetParameters} $R0
    ClearErrors
    ${GetOptions} $R0 "/DATA=" $R1
    ${IfNot} ${Errors}
    ${AndIf} $R1 != ""
      StrCpy $AgeniumData $R1
    ${EndIf}
  ${EndIf}
  ${If} $AgeniumData != ""
    CreateDirectory "$AgeniumData"
    CreateDirectory "$APPDATA\agenium"
    ${WordReplace} $AgeniumData "\" "\\" "+" $R2
    ClearErrors
    FileOpen $R3 "$APPDATA\agenium\bootstrap.toml" w
    ${IfNot} ${Errors}
      FileWrite $R3 'root = "$R2"$\n'
      FileClose $R3
    ${EndIf}
  ${EndIf}
!macroend
