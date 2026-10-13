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
; Windows keeps the icons it has shown by file name, and doesn't notice when a program's own
; icon changes: after the campfire icon came in, the taskbar and Start menu kept showing the
; old one. So shortcuts use a copy of the icon named after its contents (build.sh passes the
; name): a new icon gets a name Windows has never seen, and it reads the new one.
!ifndef ICONFILE
  !define ICONFILE "ingle.ico"
!endif
!define ICONPATH "$INSTDIR\icons\${ICONFILE}"

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

; Taskbar pins are shortcuts Windows made when Ingle was pinned. Point the ones that open this
; Ingle (or its server) at the icon file too (only their icon changes), so the taskbar shows the
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
          ${If} $4 == "$INSTDIR\ingle.exe"
          ${OrIf} $4 == "$INSTDIR\ingle-server.exe"
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

Section "Ingle" SecApp
  SectionIn RO
  SetOutPath "$INSTDIR"
  !insertmacro ReplaceFile "ingle.exe" app "${HOWTO_APP}"
  File "${BINDIR}/ingle.exe"
  SetOutPath "$INSTDIR\icons"
  File "/oname=${ICONFILE}" "../../crates/app/assets/icon/ingle.ico"
  SetOutPath "$INSTDIR"
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
  WriteRegStr HKCU "Software\Classes\ingle\DefaultIcon" "" "${ICONPATH}"
  WriteRegStr HKCU "Software\Classes\ingle\shell\open\command" "" '${LINKCMD}'

  CreateShortcut "$SMPROGRAMS\Ingle.lnk" "$INSTDIR\ingle.exe" "" "${ICONPATH}" 0 SW_SHOWNORMAL "" "Voice, webcams, screen sharing and chat"
  Call PinsUseIcon

  WriteUninstaller "$INSTDIR\Uninstall Ingle.exe"
  WriteRegStr HKCU "Software\Ingle" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "Ingle"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "${ICONPATH}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${COMPANY}"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\Uninstall Ingle.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\Uninstall Ingle.exe" /S'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\Ingle.lnk" "$INSTDIR\ingle.exe" "" "${ICONPATH}" 0 SW_SHOWNORMAL "" "Voice, webcams, screen sharing and chat"
SectionEnd

Section "Ingle Server (host a community on this PC)" SecServer
  SetOutPath "$INSTDIR"
  !insertmacro ReplaceFile "ingle-server.exe" server "${HOWTO_SERVER}"
  File "${BINDIR}/ingle-server.exe"
  ; The server keeps its database and TLS key with your other Ingle data, not in the
  ; install folder, so reinstalling or uninstalling never touches it.
  CreateDirectory "$APPDATA\Ingle\server"
  SetOutPath "$APPDATA\Ingle\server"
  CreateShortcut "$SMPROGRAMS\Ingle Server.lnk" "$INSTDIR\ingle-server.exe" '--data "$APPDATA\Ingle\server"' "${ICONPATH}" 0 SW_SHOWNORMAL "" "Host an Ingle community on this PC"
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
  ; Tell Windows ingle.exe changed (SHCNE_UPDATEITEM) and that icons changed
  ; (SHCNE_ASSOCCHANGED): the taskbar, Start menu and Explorer look at them again.
  StrCpy $0 "$INSTDIR\ingle.exe"
  System::Call "shell32::SHChangeNotify(i 0x2000, i 0x5, w r0, p 0)"
  System::Call "shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)"
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
  Delete "$INSTDIR\icons\ingle*.ico"
  RMDir "$INSTDIR\icons"
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
  ; No longer opens when Windows starts.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Ingle"
  ; Your identity, settings and server data in %APPDATA%\Ingle are kept on purpose:
  ; the identity key is your account.
SectionEnd
