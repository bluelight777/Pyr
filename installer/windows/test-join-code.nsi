; A test of FindJoinCode: writes the code found in this program's file name to $EXEDIR\code.txt.
Target amd64-unicode
!include "LogicLib.nsh"
Name "join code test"
OutFile "${OUT}"
RequestExecutionLevel user
SilentInstall silent
Var JoinCode
!include "join-code.nsh"
Section
  Call FindJoinCode
  FileOpen $0 "$EXEDIR\code.txt" w
  FileWrite $0 "[$JoinCode]"
  FileClose $0
SectionEnd
