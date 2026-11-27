; Keep unsaved work safe and avoid terminating unrelated processes with the
; same executable name. Silent installs fail without opening a dialog.
!macro customCheckAppRunning
  ${nsProcess::FindProcess} "${APP_EXECUTABLE_FILENAME}" $R0
  ; The updater launches from Electron's quit event, just before the process exits.
  ; Wait for that normal exit; never kill a running editor or compiler.
  ${If} ${isUpdated}
    StrCpy $R1 0
    ${DoWhile} $R0 == 0
      ${If} $R1 >= 60
        ${ExitDo}
      ${EndIf}
      Sleep 500
      IntOp $R1 $R1 + 1
      ${nsProcess::FindProcess} "${APP_EXECUTABLE_FILENAME}" $R0
    ${Loop}
  ${EndIf}
  ${If} $R0 == 0
    IfSilent +2 0
    MessageBox MB_OK|MB_ICONEXCLAMATION "Close Refract before installing or uninstalling."
    SetErrorLevel 2
    Quit
  ${EndIf}
  !ifmacrodef refractBeforeInstall
    !insertmacro refractBeforeInstall
  !endif
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

; Use Electron's bundled Node runtime; nsExec hides the dependency process.
!macro customInstall
  DetailPrint "Setting up TinyTeX and SyncTeX (internet access required)..."
  System::Call 'Kernel32::SetEnvironmentVariable(t "ELECTRON_RUN_AS_NODE", t "1")i.r0'
  nsExec::ExecToLog '"$INSTDIR\${APP_EXECUTABLE_FILENAME}" "$INSTDIR\resources\app\build\scripts\install-dependencies.cjs"'
  Pop $R0
  ; Remove the variable entirely before StartApp inherits our environment.
  ; Electron treats even an empty ELECTRON_RUN_AS_NODE as Node mode.
  System::Call 'Kernel32::SetEnvironmentVariable(t "ELECTRON_RUN_AS_NODE", p 0)i.r1'
  ${If} $R0 != 0
    DetailPrint "TeX setup failed. Check %LOCALAPPDATA%\Refract\dependencies\setup-error.log and rerun the installer with internet access."
    IfSilent +2 0
    MessageBox MB_OK|MB_ICONEXCLAMATION "TeX setup failed. Check %LOCALAPPDATA%\Refract\dependencies\setup-error.log and rerun the installer with internet access."
    SetErrorLevel 3
    Quit
  ${EndIf}
!macroend
