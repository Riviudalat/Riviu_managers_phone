; The bundled Java launcher uses process-local UTF-8, available on Windows 10 1903+.
; Keep this floor aligned with deployment_check::windows_version_status.
!include "LogicLib.nsh"
!include "WinVer.nsh"

!macro NSIS_HOOK_PREINSTALL
  ${IfNot} ${AtLeastWin10}
  ${OrIfNot} ${AtLeastBuild} 18362
    MessageBox MB_OK|MB_ICONSTOP "Riviu Manager requires Windows 10 version 1903 (build 18362) or newer." /SD IDOK
    SetErrorLevel 1633
    Abort
  ${EndIf}
!macroend
