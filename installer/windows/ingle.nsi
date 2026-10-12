; Ingle setup for Windows (NSIS 3).
;
;   makensis installer/windows/ingle.nsi
;   makensis -DBINDIR=path/to/exes -DOUTDIR=dist installer/windows/ingle.nsi
;
; BINDIR must hold ingle.exe, ingle-server.exe, ingle-bot.exe and vps\ingle-server-linux-*
; (the Linux server programs the app installs on VPSes). Installs per user into
; %LOCALAPPDATA%\Programs\Ingle (no admin rights needed, like Discord or VS Code).

; A 64-bit installer for a 64-bit app (also runs on ARM64 Windows through emulation).
Target amd64-unicode
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!define APPNAME "Ingle"
!define COMPANY "Firelit Games"
!ifndef VERSION
  !define VERSION "0.2.0"
!endif
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Ingle"
!define LINKCMD '"$INSTDIR\ingle.exe" "%1"'
!ifndef BINDIR
  !define BINDIR "../../dist/windows"
!endif
!ifndef OUTDIR
  !define OUTDIR "../../dist"
!endif

Name "${APPNAME}"
OutFile "${OUTDIR}/Ingle-Setup-${VERSION}.exe"
InstallDir "$LOCALAPPDATA\Programs\Ingle"
InstallDirRegKey HKCU "Software\Ingle" "InstallDir"
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

!define MUI_ICON "../../crates/app/assets/icon/ingle.ico"
!define MUI_UNICON "../../crates/app/assets/icon/ingle.ico"
!define MUI_WELCOMEFINISHPAGE_BITMAP "welcome.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "welcome.bmp"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_SMALLDESC

!define MUI_WELCOMEPAGE_TITLE "Welcome to Ingle"
!define MUI_WELCOMEPAGE_TEXT "Ingle is voice, webcams and screen sharing for your group, with text channels.$\r$\n$\r$\nThis installs Ingle for your Windows account only, so it doesn't need administrator rights.$\r$\n$\r$\nClick Next to continue."
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_TITLE "Ingle is installed"
!define MUI_FINISHPAGE_TEXT "Open Ingle and choose Create a server: Ingle can rent one for you (about $$6 a month from DigitalOcean) or set up a Linux server you already have.$\r$\n$\r$\nGot an invite link from a friend? Choose Join a server and paste it."
!define MUI_FINISHPAGE_RUN "$INSTDIR\ingle.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start Ingle now"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; Replacing a file that's in use fails. Ingle may be running hidden in the tray (its close
; button keeps it running), so first ask it to quit with `ingle.exe --quit`, which waits until
; it has; if the file is still busy, ask the person to close it.
Var QuitAsked
; Retries while an updating Ingle finishes quitting (only with /RELAUNCH).
Var UpdateWait

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
        ; Ingle's own updater started us and is quitting right now: give it a few seconds.
        IntOp $UpdateWait $UpdateWait + 1
        Sleep 500
        Goto retry_${LABEL}
      ${EndIf}
      ${If} "${NAME}" == "ingle.exe"
      ${AndIf} $QuitAsked == 0
      ${AndIf} ${FileExists} "$INSTDIR\ingle.exe"
        StrCpy $QuitAsked 1
        ExecWait '"$INSTDIR\ingle.exe" --quit'
        Sleep 500
        Goto retry_${LABEL}
      ${EndIf}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${HOWTO}" /SD IDCANCEL IDRETRY retry_${LABEL}
      Abort "${NAME} is still running."
    ${EndIf}
!macroend

!define HOWTO_APP "Ingle is still running.$\r$\n$\r$\nQuit it, then click Retry: right-click the Ingle icon by the clock (it may be under the ^ arrow) and choose Quit Ingle. If there's no icon there, close the Ingle window."
!define HOWTO_SERVER "Ingle Server is running.$\r$\n$\r$\nClose its window, then click Retry."
!define HOWTO_BOT "The Ingle test bot is running.$\r$\n$\r$\nClose its window, then click Retry."

Function .onInit
  ; Ingle's own updater runs us with /S /RELAUNCH: wait for it to quit, and reopen it after.
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
    Exec '"$INSTDIR\ingle.exe"'
  ${EndIf}
FunctionEnd

Section "Ingle" SecApp
  SectionIn RO
  SetOutPath "$INSTDIR"
  !insertmacro ReplaceFile "ingle.exe" app "${HOWTO_APP}"
  File "${BINDIR}/ingle.exe"
  File "/oname=README.txt" "../../README.md"
  File "/oname=Inter font license.txt" "../../crates/app/assets/fonts/OFL.txt"
  ; Linux server programs the app uploads when it sets up a VPS for you.
  SetOutPath "$INSTDIR\vps"
  File "${BINDIR}/vps/ingle-server-linux-x86_64"
  File "${BINDIR}/vps/ingle-server-linux-aarch64"
  SetOutPath "$INSTDIR"

  ; Clicking an ingle:// invite link opens Ingle (which hands it to a copy that's already open).
  WriteRegStr HKCU "Software\Classes\ingle" "" "URL:Ingle invite link"
  WriteRegStr HKCU "Software\Classes\ingle" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\ingle\DefaultIcon" "" "$INSTDIR\ingle.exe,0"
  WriteRegStr HKCU "Software\Classes\ingle\shell\open\command" "" '${LINKCMD}'

  CreateShortcut "$SMPROGRAMS\Ingle.lnk" "$INSTDIR\ingle.exe" "" "$INSTDIR\ingle.exe" 0 SW_SHOWNORMAL "" "Voice, webcams, screen sharing and chat"

  WriteUninstaller "$INSTDIR\Uninstall Ingle.exe"
  WriteRegStr HKCU "Software\Ingle" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "Ingle"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\ingle.exe,0"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${COMPANY}"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\Uninstall Ingle.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\Uninstall Ingle.exe" /S'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\Ingle.lnk" "$INSTDIR\ingle.exe" "" "$INSTDIR\ingle.exe" 0 SW_SHOWNORMAL "" "Voice, webcams, screen sharing and chat"
SectionEnd

Section "Ingle Server (host a community on this PC)" SecServer
  SetOutPath "$INSTDIR"
  !insertmacro ReplaceFile "ingle-server.exe" server "${HOWTO_SERVER}"
  File "${BINDIR}/ingle-server.exe"
  ; The server keeps its database and TLS key with your other Ingle data, not in the
  ; install folder, so reinstalling or uninstalling never touches it.
  CreateDirectory "$APPDATA\Ingle\server"
  SetOutPath "$APPDATA\Ingle\server"
  CreateShortcut "$SMPROGRAMS\Ingle Server.lnk" "$INSTDIR\ingle-server.exe" '--data "$APPDATA\Ingle\server"' "$INSTDIR\ingle-server.exe" 0 SW_SHOWNORMAL "" "Host an Ingle community on this PC"
  SetOutPath "$INSTDIR"
SectionEnd

Section /o "Test bot (try calls on your own)" SecBot
  SetOutPath "$INSTDIR"
  !insertmacro ReplaceFile "ingle-bot.exe" bot "${HOWTO_BOT}"
  File "${BINDIR}/ingle-bot.exe"
SectionEnd

Section "-Size"
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize" "$0"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecApp} "Text channels, voice, webcams and screen sharing."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "An Ingle shortcut on your desktop."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecServer} "Host your own community. Adds Ingle Server to the Start menu."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecBot} "A pretend caller (tone, camera, stream) for testing alone."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Function un.onInit
  StrCpy $UpdateWait 20
FunctionEnd

Section "Uninstall"
  !insertmacro ReplaceFile "ingle.exe" unapp "${HOWTO_APP}"
  !insertmacro ReplaceFile "ingle-server.exe" unserver "${HOWTO_SERVER}"
  Delete "$INSTDIR\ingle-bot.exe"
  Delete "$INSTDIR\README.txt"
  Delete "$INSTDIR\Inter font license.txt"
  Delete "$INSTDIR\vps\ingle-server-linux-x86_64"
  Delete "$INSTDIR\vps\ingle-server-linux-aarch64"
  RMDir "$INSTDIR\vps"
  Delete "$INSTDIR\Uninstall Ingle.exe"
  ; Only removes the folder if it's empty, so nothing you put there is lost.
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\Ingle.lnk"
  Delete "$SMPROGRAMS\Ingle Server.lnk"
  Delete "$DESKTOP\Ingle.lnk"

  ; Leave the link handler alone if another install of Ingle has taken it over since.
  ReadRegStr $0 HKCU "Software\Classes\ingle\shell\open\command" ""
  ${If} $0 == '${LINKCMD}'
    DeleteRegKey HKCU "Software\Classes\ingle"
  ${EndIf}
  DeleteRegKey HKCU "${UNINSTKEY}"
  DeleteRegKey HKCU "Software\Ingle"
  ; Your identity, settings and server data in %APPDATA%\Ingle are kept on purpose:
  ; the identity key is your account.
SectionEnd
