; Pyr setup for Windows (NSIS 3).
;
;   makensis installer/windows/pyr.nsi
;   makensis -DBINDIR=path/to/exes -DOUTDIR=dist installer/windows/pyr.nsi
;
; BINDIR must hold pyr.exe, pyr-mcp.exe, pyr-server.exe, pyr-bot.exe and vps\pyr-server-linux-*
; (the Linux server programs the app installs on VPSes). Installs per user into
; %LOCALAPPDATA%\Programs\Pyr (no admin rights needed, like Discord or VS Code).

; A 64-bit installer for a 64-bit app (also runs on ARM64 Windows through emulation).
Target amd64-unicode
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!define APPNAME "Pyr"
!define COMPANY "Firelit Games"
!ifndef VERSION
  !define VERSION "0.2.0"
!endif
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Pyr"
!define LINKCMD '"$INSTDIR\pyr.exe" "%1"'
!ifndef BINDIR
  !define BINDIR "../../dist/windows"
!endif
!ifndef OUTDIR
  !define OUTDIR "../../dist"
!endif
; Windows keeps the icons it has shown by file name, and doesn't notice when a program's own
; icon changes: after the campfire icon came in, the taskbar and Start menu kept showing the
; old one. So shortcuts use a copy of the icon named after its contents (build.sh passes the
; name): a new icon gets a name Windows has never seen, and it reads the new one.
!ifndef ICONFILE
  !define ICONFILE "pyr.ico"
!endif
!define ICONPATH "$INSTDIR\icons\${ICONFILE}"

Name "${APPNAME}"
OutFile "${OUTDIR}/Pyr-Setup-${VERSION}.exe"
InstallDir "$LOCALAPPDATA\Programs\Pyr"
InstallDirRegKey HKCU "Software\Pyr" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ManifestDPIAware true
BrandingText "${APPNAME} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIFileVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "CompanyName" "${COMPANY}"
VIAddVersionKey "FileDescription" "${APPNAME} Setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "${COMPANY}"

!define MUI_ICON "../../crates/app/assets/icon/pyr.ico"
!define MUI_UNICON "../../crates/app/assets/icon/pyr.ico"
!define MUI_WELCOMEFINISHPAGE_BITMAP "welcome.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "welcome.bmp"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_SMALLDESC

!define MUI_WELCOMEPAGE_TITLE "Welcome to Pyr"
!define MUI_WELCOMEPAGE_TEXT "Pyr is voice, webcams and screen sharing for your group, with text channels.$\r$\n$\r$\nThis installs Pyr for your Windows account only, so it doesn't need administrator rights.$\r$\n$\r$\nClick Next to continue."
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_TITLE "Pyr is installed"
!define MUI_FINISHPAGE_TEXT "Open Pyr and choose Create a server: Pyr can rent one for you (about $$6 a month from DigitalOcean) or set up a Linux server you already have.$\r$\n$\r$\nGot an invite link from a friend? Choose Join a server and paste it."
!define MUI_FINISHPAGE_RUN "$INSTDIR\pyr.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start Pyr now"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; Replacing a file that's in use fails. Pyr may be running hidden in the tray (its close
; button keeps it running), so first ask it to quit with `pyr.exe --quit`, which waits until
; it has; if the file is still busy, ask the person to close it.
Var QuitAsked
; Retries while an updating Pyr finishes quitting (only with /RELAUNCH).
Var UpdateWait
; Where Ingle was installed (Pyr's name before 0.8.0), and where Pyr Server keeps its data.
Var OldDir
Var ServerData

!macro ReplaceFile NAME LABEL HOWTO
  StrCpy $QuitAsked 0
  retry_${LABEL}:
    StrCpy $R9 0
    ClearErrors
    Delete "$INSTDIR\${NAME}"
    IfErrors 0 +2
      StrCpy $R9 1
    ${If} $R9 == 1
      ${If} $UpdateWait < 20
        ; Pyr's own updater started us and is quitting right now: give it a few seconds.
        IntOp $UpdateWait $UpdateWait + 1
        Sleep 500
        Goto retry_${LABEL}
      ${EndIf}
      ${If} "${NAME}" == "pyr.exe"
      ${AndIf} $QuitAsked == 0
      ${AndIf} ${FileExists} "$INSTDIR\pyr.exe"
        StrCpy $QuitAsked 1
        ExecWait '"$INSTDIR\pyr.exe" --quit'
        Sleep 500
        Goto retry_${LABEL}
      ${EndIf}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${HOWTO}" /SD IDCANCEL IDRETRY retry_${LABEL}
      Abort "${NAME} is still running."
    ${EndIf}
!macroend

; The Claude connector may be running: the Claude app starts it and keeps it while it's open. A
; running program's file can't be replaced, but it can be renamed, so it's set aside (and taken
; away by the next install or the uninstaller) and the new one goes in; Claude keeps using the
; old one until it next starts it.
!macro SetAsideFile NAME
  Delete "$INSTDIR\${NAME}.old"
  ClearErrors
  Delete "$INSTDIR\${NAME}"
  ${If} ${Errors}
    Rename "$INSTDIR\${NAME}" "$INSTDIR\${NAME}.old"
  ${EndIf}
!macroend

!define HOWTO_APP "Pyr is still running.$\r$\n$\r$\nQuit it, then click Retry: right-click the Pyr icon by the clock (it may be under the ^ arrow) and choose Quit Pyr. If there's no icon there, close the Pyr window."
!define HOWTO_SERVER "Pyr Server is running.$\r$\n$\r$\nClose its window, then click Retry."
!define HOWTO_BOT "The Pyr test bot is running.$\r$\n$\r$\nClose its window, then click Retry."

Function .onInit
  ; Pyr's own updater runs us with /S /RELAUNCH: wait for it to quit, and reopen it after.
  StrCpy $UpdateWait 20
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/RELAUNCH" $R1
  ${IfNot} ${Errors}
    StrCpy $UpdateWait 0
  ${EndIf}
FunctionEnd

Function .onInstSuccess
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/RELAUNCH" $R1
  ${IfNot} ${Errors}
    Exec '"$INSTDIR\pyr.exe"'
  ${EndIf}
FunctionEnd

; Taskbar pins are shortcuts Windows made when Pyr was pinned. Point the ones that open this
; Pyr (or its server) at the icon file too (only their icon changes), so the taskbar shows the
; new icon.
Function PinsUseIcon
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $7
  Push $8
  Push $9
  StrCpy $6 "$APPDATA\Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar"
  StrCpy $7 "${ICONPATH}"
  System::Call "ole32::CoInitialize(p 0) i .r8"
  ClearErrors
  FindFirst $0 $1 "$6\*.lnk"
  next_pin:
    StrCmp $1 "" pins_done
    StrCpy $5 "$6\$1"
    ; A shell link (IShellLinkW), read from the file through IPersistFile.
    System::Call "ole32::CoCreateInstance(g '{00021401-0000-0000-C000-000000000046}', p 0, i 1, g '{000214F9-0000-0000-C000-000000000046}', *p .r2) i .r4"
    ${If} $4 = 0
      System::Call "$2->0(g '{0000010b-0000-0000-C000-000000000046}', *p .r3) i .r4"
      ${If} $4 = 0
        ; IPersistFile::Load (read and write)
        System::Call "$3->5(w r5, i 2) i .r4"
        ${If} $4 = 0
          ; IShellLinkW::GetPath: where the pin points
          System::Call "$2->3(w .r4, i ${NSIS_MAX_STRLEN}, p 0, i 0) i .r9"
          ${If} $9 <> 0
            StrCpy $4 ""
          ${EndIf}
          ${If} $4 == "$OldDir\ingle.exe"
          ${AndIf} $OldDir != ""
            ; A pin made for Ingle: it opens Pyr now (IShellLinkW::SetPath), from Pyr's folder
            ; (SetWorkingDirectory: Ingle's is gone).
            StrCpy $4 "$INSTDIR\pyr.exe"
            System::Call "$2->20(w r4) i .r9"
            System::Call "$2->9(w '$INSTDIR') i .r9"
          ${EndIf}
          ${If} $4 == "$INSTDIR\pyr.exe"
          ${OrIf} $4 == "$INSTDIR\pyr-server.exe"
            ; IShellLinkW::SetIconLocation, then IPersistFile::Save
            System::Call "$2->17(w r7, i 0) i .r9"
            ${If} $9 = 0
              System::Call "$3->6(w r5, i 1) i .r9"
              ; SHCNE_UPDATEITEM, SHCNF_PATHW
              System::Call "shell32::SHChangeNotify(i 0x2000, i 0x5, w r5, p 0)"
            ${EndIf}
          ${EndIf}
        ${EndIf}
        System::Call "$3->2()"
      ${EndIf}
      System::Call "$2->2()"
    ${EndIf}
    FindNext $0 $1
    Goto next_pin
  pins_done:
  FindClose $0
  ${If} $8 >= 0
    System::Call "ole32::CoUninitialize()"
  ${EndIf}
  Pop $9
  Pop $8
  Pop $7
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; ---- Pyr was called Ingle before 0.8.0 ---------------------------------------------------
; Ingle's install gives way to Pyr's: a running Ingle is asked to quit first; once Pyr is in,
; Ingle's program files, shortcuts, uninstall entry and start-with-Windows entry go, taskbar
; pins that opened Ingle open Pyr, and ingle:// links open Pyr. Its server's community comes
; to Pyr Server's folder. Your identity and settings (%APPDATA%\Ingle) are copied over by Pyr
; itself when it first starts; Ingle's own are kept.
Function FindIngle
  ReadRegStr $OldDir HKCU "Software\Ingle" "InstallDir"
  ${If} $OldDir == ""
    StrCpy $OldDir "$LOCALAPPDATA\Programs\Ingle"
  ${EndIf}
FunctionEnd

Function QuitIngle
  Call FindIngle
  ${If} ${FileExists} "$OldDir\ingle.exe"
    ; (It may be hidden in the tray; when Ingle's own updater started us, it's quitting already.)
    ExecWait '"$OldDir\ingle.exe" --quit'
    Sleep 500
  ${EndIf}
FunctionEnd

Function RemoveIngle
  Call FindIngle
  ${If} $OldDir != $INSTDIR
    Delete "$OldDir\ingle.exe"
    Delete "$OldDir\ingle-bot.exe"
    Delete "$OldDir\ingle-mcp.exe.old"
    ; (Claude may be running Ingle's connector: then it's set aside, and goes next time.)
    ClearErrors
    Delete "$OldDir\ingle-mcp.exe"
    ${If} ${Errors}
      Rename "$OldDir\ingle-mcp.exe" "$OldDir\ingle-mcp.exe.old"
    ${EndIf}
    ; (Ingle Server, if it's still running, stays: see the server section.)
    Delete "$OldDir\ingle-server.exe"
    Delete "$OldDir\README.txt"
    Delete "$OldDir\Inter font license.txt"
    Delete "$OldDir\vps\ingle-server-linux-x86_64"
    Delete "$OldDir\vps\ingle-server-linux-aarch64"
    RMDir "$OldDir\vps"
    Delete "$OldDir\icons\ingle*.ico"
    RMDir "$OldDir\icons"
    Delete "$OldDir\Uninstall Ingle.exe"
    RMDir "$OldDir"
  ${EndIf}
  ; Its shortcuts: Pyr's take their place (on the desktop too, if Ingle had one there).
  Delete "$SMPROGRAMS\Ingle.lnk"
  Delete "$SMPROGRAMS\Ingle Server.lnk"
  ${If} ${FileExists} "$DESKTOP\Ingle.lnk"
    Delete "$DESKTOP\Ingle.lnk"
    CreateShortcut "$DESKTOP\Pyr.lnk" "$INSTDIR\pyr.exe" "" "${ICONPATH}" 0 SW_SHOWNORMAL "" "Voice, webcams, screen sharing and chat"
  ${EndIf}
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Ingle"
  DeleteRegKey HKCU "Software\Ingle"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Ingle"
  ; ingle:// invite links (made before the rename) open Pyr.
  WriteRegStr HKCU "Software\Classes\ingle" "" "URL:Pyr invite link"
  WriteRegStr HKCU "Software\Classes\ingle" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\ingle\DefaultIcon" "" "${ICONPATH}"
  WriteRegStr HKCU "Software\Classes\ingle\shell\open\command" "" '${LINKCMD}'
FunctionEnd

Section "Pyr" SecApp
  SectionIn RO
  Call QuitIngle
  SetOutPath "$INSTDIR"
  !insertmacro ReplaceFile "pyr.exe" app "${HOWTO_APP}"
  File "${BINDIR}/pyr.exe"
  ; The Claude connector (Settings, Claude tab, sets it up in the Claude app).
  !insertmacro SetAsideFile "pyr-mcp.exe"
  File "${BINDIR}/pyr-mcp.exe"
  SetOutPath "$INSTDIR\icons"
  File "/oname=${ICONFILE}" "../../crates/app/assets/icon/pyr.ico"
  SetOutPath "$INSTDIR"
  File "/oname=README.txt" "../../README.md"
  File "/oname=Inter font license.txt" "../../crates/app/assets/fonts/OFL.txt"
  ; Linux server programs the app uploads when it sets up a VPS for you.
  SetOutPath "$INSTDIR\vps"
  File "${BINDIR}/vps/pyr-server-linux-x86_64"
  File "${BINDIR}/vps/pyr-server-linux-aarch64"
  SetOutPath "$INSTDIR"

  ; Clicking a pyr:// invite link opens Pyr (which hands it to a copy that's already open).
  WriteRegStr HKCU "Software\Classes\pyr" "" "URL:Pyr invite link"
  WriteRegStr HKCU "Software\Classes\pyr" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\pyr\DefaultIcon" "" "${ICONPATH}"
  WriteRegStr HKCU "Software\Classes\pyr\shell\open\command" "" '${LINKCMD}'

  CreateShortcut "$SMPROGRAMS\Pyr.lnk" "$INSTDIR\pyr.exe" "" "${ICONPATH}" 0 SW_SHOWNORMAL "" "Voice, webcams, screen sharing and chat"
  Call FindIngle
  Call PinsUseIcon

  WriteUninstaller "$INSTDIR\Uninstall Pyr.exe"
  WriteRegStr HKCU "Software\Pyr" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "Pyr"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "${ICONPATH}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${COMPANY}"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\Uninstall Pyr.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\Uninstall Pyr.exe" /S'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\Pyr.lnk" "$INSTDIR\pyr.exe" "" "${ICONPATH}" 0 SW_SHOWNORMAL "" "Voice, webcams, screen sharing and chat"
SectionEnd

Section "Pyr Server (host a community on this PC)" SecServer
  SetOutPath "$INSTDIR"
  !insertmacro ReplaceFile "pyr-server.exe" server "${HOWTO_SERVER}"
  File "${BINDIR}/pyr-server.exe"
  ; The server keeps its database and TLS key with your other Pyr data, not in the
  ; install folder, so reinstalling or uninstalling never touches it.
  StrCpy $ServerData "$APPDATA\Pyr\server"
  ; A community hosted with Ingle Server comes along (copied; Ingle's folder is kept). While
  ; Ingle Server is still running its database can't be copied safely: Pyr Server then uses
  ; Ingle's folder as it is.
  Call FindIngle
  ${If} ${FileExists} "$APPDATA\Ingle\server\ingle.db"
  ${AndIfNot} ${FileExists} "$APPDATA\Pyr\server\ingle.db"
    ClearErrors
    ${If} ${FileExists} "$OldDir\ingle-server.exe"
      Rename "$OldDir\ingle-server.exe" "$OldDir\ingle-server.exe.moving"
    ${EndIf}
    ${If} ${Errors}
      StrCpy $ServerData "$APPDATA\Ingle\server"
    ${Else}
      Rename "$OldDir\ingle-server.exe.moving" "$OldDir\ingle-server.exe"
      CreateDirectory "$APPDATA\Pyr\server"
      CopyFiles /SILENT "$APPDATA\Ingle\server\*.*" "$APPDATA\Pyr\server"
    ${EndIf}
  ${EndIf}
  CreateDirectory "$ServerData"
  SetOutPath "$ServerData"
  CreateShortcut "$SMPROGRAMS\Pyr Server.lnk" "$INSTDIR\pyr-server.exe" '--data "$ServerData"' "${ICONPATH}" 0 SW_SHOWNORMAL "" "Host a Pyr community on this PC"
  SetOutPath "$INSTDIR"
SectionEnd

Section /o "Test bot (try calls on your own)" SecBot
  SetOutPath "$INSTDIR"
  !insertmacro ReplaceFile "pyr-bot.exe" bot "${HOWTO_BOT}"
  File "${BINDIR}/pyr-bot.exe"
SectionEnd

Section "-Ingle"
  Call RemoveIngle
SectionEnd

Section "-Size"
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize" "$0"
  ; Tell Windows pyr.exe changed (SHCNE_UPDATEITEM) and that icons changed
  ; (SHCNE_ASSOCCHANGED): the taskbar, Start menu and Explorer look at them again.
  StrCpy $0 "$INSTDIR\pyr.exe"
  System::Call "shell32::SHChangeNotify(i 0x2000, i 0x5, w r0, p 0)"
  System::Call "shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecApp} "Text channels, voice, webcams and screen sharing."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "A Pyr shortcut on your desktop."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecServer} "Host your own community. Adds Pyr Server to the Start menu."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecBot} "A pretend caller (tone, camera, stream) for testing alone."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Function un.onInit
  StrCpy $UpdateWait 20
FunctionEnd

Section "Uninstall"
  !insertmacro ReplaceFile "pyr.exe" unapp "${HOWTO_APP}"
  !insertmacro ReplaceFile "pyr-server.exe" unserver "${HOWTO_SERVER}"
  Delete "$INSTDIR\pyr-bot.exe"
  Delete "$INSTDIR\pyr-mcp.exe"
  Delete "$INSTDIR\pyr-mcp.exe.old"
  Delete "$INSTDIR\README.txt"
  Delete "$INSTDIR\Inter font license.txt"
  Delete "$INSTDIR\vps\pyr-server-linux-x86_64"
  Delete "$INSTDIR\vps\pyr-server-linux-aarch64"
  RMDir "$INSTDIR\vps"
  Delete "$INSTDIR\icons\pyr*.ico"
  RMDir "$INSTDIR\icons"
  Delete "$INSTDIR\Uninstall Pyr.exe"
  ; Only removes the folder if it's empty, so nothing you put there is lost.
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\Pyr.lnk"
  Delete "$SMPROGRAMS\Pyr Server.lnk"
  Delete "$DESKTOP\Pyr.lnk"

  ; Leave the link handler alone if another install of Pyr has taken it over since.
  ReadRegStr $0 HKCU "Software\Classes\pyr\shell\open\command" ""
  ${If} $0 == '${LINKCMD}'
    DeleteRegKey HKCU "Software\Classes\pyr"
  ${EndIf}
  ReadRegStr $0 HKCU "Software\Classes\ingle\shell\open\command" ""
  ${If} $0 == '${LINKCMD}'
    DeleteRegKey HKCU "Software\Classes\ingle"
  ${EndIf}
  DeleteRegKey HKCU "${UNINSTKEY}"
  DeleteRegKey HKCU "Software\Pyr"
  ; No longer opens when Windows starts.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Pyr"
  ; Your identity, settings and server data in %APPDATA%\Pyr are kept on purpose:
  ; the identity key is your account.
SectionEnd
