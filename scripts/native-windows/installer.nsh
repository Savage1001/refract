; Share the installed identity and existing safety checks with the Electron NSIS
; template. GPUI bundles its own TeX setup command.
!include "${__FILEDIR__}\..\installer.nsh"
!macroundef customInstall

!macro nativeInstallError message code
  DetailPrint "${message}"
  IfSilent +2 0
  MessageBox MB_OK|MB_ICONEXCLAMATION "${message}"
  SetErrorLevel ${code}
  Quit
!macroend

!ifndef BUILD_UNINSTALLER
!macro refractBeforeInstall
  ; Accept an empty destination or the current registered Refract installation.
  GetFullPathName $R2 "$INSTDIR"
  ${If} $R2 == ""
    !insertmacro nativeInstallError "Could not verify the Refract installation path." 2
  ${EndIf}
  ReadRegStr $R3 HKCU "${INSTALL_REGISTRY_KEY}" InstallLocation
  ${If} $R3 != ""
    GetFullPathName $R3 "$R3"
  ${EndIf}
  ${If} $R2 != $R3
    FindFirst $R4 $R5 "$R2\*"
    ${While} $R5 != ""
      ${If} $R5 != "."
      ${AndIf} $R5 != ".."
        FindClose $R4
        !insertmacro nativeInstallError "The destination already contains files. Choose an empty folder." 2
      ${EndIf}
      FindNext $R4 $R5
    ${EndWhile}
    FindClose $R4
    ClearErrors
  ${EndIf}

  ; Run before the template invokes the previous uninstaller, so a failed
  ; dependency download leaves the existing version intact.
  DetailPrint "Setting up TinyTeX and SyncTeX (internet access required)..."
  File /oname=$PLUGINSDIR\refract-install-dependencies.exe "$%REFRACT_NSIS_DEPENDENCY_HELPER%"
  nsExec::ExecToLog '"$PLUGINSDIR\refract-install-dependencies.exe"'
  Pop $R0
  ${If} $R0 != 0
    !insertmacro nativeInstallError "TeX setup failed. Check %LOCALAPPDATA%\Refract\dependencies\setup-error.log and rerun the installer with internet access." 3
  ${EndIf}
!macroend
!endif

!macro customUnInstall
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "--delete-app-data" $R1
  ${IfNot} ${Errors}
    !insertmacro nativeInstallError "Refract Setup preserves projects, preferences, and TeX dependencies." 2
  ${EndIf}
!macroend

!macro customRemoveFiles
  ; This uninstaller owns only the files staged by package.cts and the NSIS
  ; template. Preserve everything else under the installation directory.
  ${IfNot} ${FileExists} "$INSTDIR\refract-native-installed"
    !insertmacro nativeInstallError "Could not verify the installed Refract application." 2
  ${EndIf}
  SetOutPath "$TEMP"
  ClearErrors
  Delete "$INSTDIR\${APP_EXECUTABLE_FILENAME}"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\Inter-OFL.txt"
  Delete "$INSTDIR\uninstallerIcon.ico"
  Delete "$INSTDIR\${UNINSTALL_FILENAME}"
  ${If} ${Errors}
    !insertmacro nativeInstallError "Could not remove Refract. Close it and run the uninstaller again." 2
  ${EndIf}
  Delete "$INSTDIR\refract-native-installed"
  ${If} ${Errors}
    !insertmacro nativeInstallError "Could not remove the Refract installation marker." 2
  ${EndIf}
  RMDir "$INSTDIR"
!macroend
