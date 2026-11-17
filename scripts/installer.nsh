; Keep unsaved work safe and avoid terminating unrelated processes with the
; same executable name. Silent installs fail without opening a dialog.
!macro customCheckAppRunning
  ${nsProcess::FindProcess} "${APP_EXECUTABLE_FILENAME}" $R0
  ${If} $R0 == 0
    IfSilent +2 0
    MessageBox MB_OK|MB_ICONEXCLAMATION "Close Refract before installing or uninstalling."
    SetErrorLevel 2
    Quit
  ${EndIf}
!macroend

; electron-builder's default uninstall failure handler displays a message box
; even in silent mode. Report failure instead of waiting on an invisible dialog.
!macro refractCheckUninstallResult
  ${If} ${Errors}
    DetailPrint "Could not launch the previous Refract uninstaller."
    IfSilent +2 0
    MessageBox MB_OK|MB_ICONEXCLAMATION "Could not launch the previous Refract uninstaller."
    SetErrorLevel 2
    Quit
  ${ElseIf} $R0 != 0
    DetailPrint "Could not remove the previous Refract installation (error $R0)."
    IfSilent +2 0
    MessageBox MB_OK|MB_ICONEXCLAMATION "Could not remove the previous Refract installation (error $R0)."
    SetErrorLevel 2
    Quit
  ${EndIf}
!macroend

!macro customUnInstallCheck
  !insertmacro refractCheckUninstallResult
!macroend

!macro customUnInstallCheckCurrentUser
  !insertmacro refractCheckUninstallResult
!macroend
