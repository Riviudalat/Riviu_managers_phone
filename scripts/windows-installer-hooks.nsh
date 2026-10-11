; The bundled Java launcher uses process-local UTF-8, available on Windows 10 1903+.
; Keep this floor aligned with deployment_check::windows_version_status.
!include "LogicLib.nsh"
!include "WinVer.nsh"

; The name the operator sees. productName stays "Riviu Manager Full" for the
; shipped variant because Tauri's NSIS template keys the uninstall entry, the
; install folder and the previous-install lookup on it; renaming it would
; install a second copy beside 0.2.81 and collide with the base variant.
; These hooks only relabel what productName also names: the Apps entry and
; the shortcuts. They are no-ops when productName already is this name.
!define RIVIU_DISPLAY_NAME "Riviu Manager"
; The literal Full productName, for .onGUIEnd, which is compiled before the
; template defines PRODUCTNAME and MAINBINARYNAME.
!define RIVIU_FULL_PRODUCT_NAME "Riviu Manager Full"
!define RIVIU_MAIN_BINARY "riviu-managers-phone.exe"

; Rename one of our productName shortcuts to the display name. A display-name
; shortcut that already points at this install wins and the old one goes; one
; that points elsewhere (another variant) is left alone and ours keeps its name.
!macro RIVIU_ADOPT_SHORTCUT DIR OLD_NAME TARGET
  !insertmacro IsShortcutTarget "${DIR}\${OLD_NAME}.lnk" "${TARGET}"
  Pop $0
  ${If} $0 = 1
    !insertmacro IsShortcutTarget "${DIR}\${RIVIU_DISPLAY_NAME}.lnk" "${TARGET}"
    Pop $0
    ${If} $0 = 1
      !insertmacro UnpinShortcut "${DIR}\${OLD_NAME}.lnk"
      Delete "${DIR}\${OLD_NAME}.lnk"
    ${ElseIfNot} ${FileExists} "${DIR}\${RIVIU_DISPLAY_NAME}.lnk"
      Rename "${DIR}\${OLD_NAME}.lnk" "${DIR}\${RIVIU_DISPLAY_NAME}.lnk"
    ${EndIf}
  ${EndIf}
!macroend

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

!macro RIVIU_REMOVE_SHORTCUT LINK TARGET
  !insertmacro IsShortcutTarget "${LINK}" "${TARGET}"
  Pop $0
  ${If} $0 = 1
    !insertmacro UnpinShortcut "${LINK}"
    Delete "${LINK}"
  ${EndIf}
!macroend

; Runs for silent, passive and updater installs, and for the Start-menu
; shortcut of interactive ones. Only $SMPROGRAMS and $DESKTOP are handled:
; bundle.windows.nsis.startMenuFolder is unset, so no subfolder exists.
!macro NSIS_HOOK_POSTINSTALL
  !if "${PRODUCTNAME}" != "${RIVIU_DISPLAY_NAME}"
    WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${RIVIU_DISPLAY_NAME}"
    !insertmacro RIVIU_ADOPT_SHORTCUT "$SMPROGRAMS" "${PRODUCTNAME}" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro RIVIU_ADOPT_SHORTCUT "$DESKTOP" "${PRODUCTNAME}" "$INSTDIR\${MAINBINARYNAME}.exe"
  !endif
!macroend

; The template only deletes "${PRODUCTNAME}.lnk"; remove the renamed ones too.
; After the files are gone, so a cancelled running-app prompt keeps them, and
; never during an update, which must keep the operator's shortcuts and pins.
!macro NSIS_HOOK_POSTUNINSTALL
  !if "${PRODUCTNAME}" != "${RIVIU_DISPLAY_NAME}"
    ${If} $UpdateMode <> 1
      !insertmacro RIVIU_REMOVE_SHORTCUT "$SMPROGRAMS\${RIVIU_DISPLAY_NAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
      !insertmacro RIVIU_REMOVE_SHORTCUT "$DESKTOP\${RIVIU_DISPLAY_NAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    ${EndIf}
  !endif
!macroend

; An interactive install creates its desktop shortcut on the finish page,
; after NSIS_HOOK_POSTINSTALL. Rename it once the wizard closes.
Function .onGUIEnd
  !insertmacro RIVIU_ADOPT_SHORTCUT "$DESKTOP" "${RIVIU_FULL_PRODUCT_NAME}" "$INSTDIR\${RIVIU_MAIN_BINARY}"
FunctionEnd
