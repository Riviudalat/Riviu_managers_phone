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

  ; The stock running-app check comes after this hook and silently kills the
  ; app during /S updates. Require its graceful shutdown before changing files.
  nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
  Pop $R0
  ${If} $R0 != 1
    MessageBox MB_OK|MB_ICONSTOP "Close Riviu Manager and wait for its device work to finish before installing." /SD IDOK
    SetErrorLevel 1618
    Abort
  ${EndIf}

  ; File overlays retain obsolete dependency metadata and invalidate the frozen
  ; runtime manifest. Replace only the installed, owned runtime payload.
  ${If} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
  ${AndIf} ${FileExists} "$INSTDIR\sidecars\pymobiledevice3\runtime\runtime-manifest.json"
    ClearErrors
    RMDir /r "$INSTDIR\sidecars\pymobiledevice3\runtime"
    ${If} ${Errors}
      MessageBox MB_OK|MB_ICONSTOP "The installed Riviu runtime is still in use; close its processes before installing." /SD IDOK
      SetErrorLevel 1618
      Abort
    ${EndIf}
  ${EndIf}
!macroend
