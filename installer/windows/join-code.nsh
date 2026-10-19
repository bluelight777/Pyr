; Setup downloaded from an invite's page is named Pyr-Setup-join-<code>.exe (see pyr.nsi).
; (Its own file so the check can be tested: installer/windows/test-join-code.sh.)

; The code in setup's own file name, after "-join-" (up to the ".exe", or a " (1)" a browser
; added): only a-z, 0-9 and -, 3 to 32 of them, starting and ending with a letter or digit.
; Anything else: no code.
Function FindJoinCode
  StrCpy $JoinCode ""
  StrCpy $R0 $EXEFILE
  StrLen $R1 $R0
  StrCpy $R2 0
  ; Find "-join-" (case doesn't matter: StrCmp ignores it).
  ${Do}
    IntCmp $R2 $R1 done 0 done
    StrCpy $R3 $R0 6 $R2
    StrCmp $R3 "-join-" found
    IntOp $R2 $R2 + 1
  ${Loop}
  found:
  IntOp $R2 $R2 + 6
  StrCpy $R4 ""
  ${Do}
    StrCpy $R3 $R0 1 $R2
    StrCmp $R3 "" stop
    ; One character: kept if a-z (lowered), 0-9 or -.
    StrCpy $R5 "abcdefghijklmnopqrstuvwxyz0123456789-"
    StrCpy $R6 0
    StrCpy $R7 0
    ${Do}
      StrCpy $R8 $R5 1 $R6
      StrCmp $R8 "" nomatch
      StrCmp $R8 $R3 0 +3
        StrCpy $R7 1
        Goto matched
      IntOp $R6 $R6 + 1
    ${Loop}
    nomatch:
    matched:
    IntCmp $R7 1 0 stop stop
    StrCpy $R4 "$R4$R8"
    StrLen $R9 $R4
    IntCmp $R9 32 0 0 toolong
    IntOp $R2 $R2 + 1
  ${Loop}
  toolong:
    Goto done
  stop:
  ; Ends at ".exe" or " (": anything else after the code means it isn't one.
  StrCmp $R3 "." +3 0
  StrCmp $R3 " " +2 0
  Goto done
  StrLen $R9 $R4
  IntCmp $R9 3 0 done 0
  StrCpy $R8 $R4 1
  StrCmp $R8 "-" done
  StrCpy $R8 $R4 1 -1
  StrCmp $R8 "-" done
  StrCpy $JoinCode $R4
  done:
FunctionEnd
