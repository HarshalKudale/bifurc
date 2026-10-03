; The Bifurc installer: the staged desktop folder (scripts/package-desktop.mjs), installed for the current
; user with no elevation, the `bifurc` command on the user's PATH, and an entry in Apps & features.
;
; makensis /DVERSION=0.3.3 /DSTAGE=<staged folder> /DOUT=<setup.exe> /DICON=<icon.ico> bifurc.nsi

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"

!define APP "Bifurc"
!define PUBLISHER "Harshal Kudale"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\BifurcDesktop"
!define SETTINGS_KEY "Software\BifurcDesktop"

Name "${APP}"
OutFile "${OUT}"
; Not `Programs\Bifurc`: that is where the Electron build installs, and the two must not share a folder.
InstallDir "$LOCALAPPDATA\Programs\bifurc-desktop"
InstallDirRegKey HKCU "${SETTINGS_KEY}" "InstallDir"
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "FileDescription" "${APP} Setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "${PUBLISHER}"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\bifurc-ui.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Open ${APP}"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; A running engine holds bifurc-engine.exe and the tray helper open, and the interface holds itself. Stopping the
; engine closes the interfaces attached to it; `--no-start` keeps the command from starting one to stop.
!macro StopBifurc
  ${If} ${FileExists} "$INSTDIR\bin\bifurc.exe"
    nsExec::Exec '"$INSTDIR\bin\bifurc.exe" engine stop --no-start'
    Pop $0
    Sleep 500
  ${EndIf}
!macroend

Section "Install"
  !insertmacro StopBifurc

  SetOutPath "$INSTDIR"
  ; What an earlier version left in these folders is not part of this one (`runtime` held the Node engine's Node).
  RMDir /r "$INSTDIR\resources"
  RMDir /r "$INSTDIR\runtime"
  File /r "${STAGE}\*.*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\bifurc-ui.exe"
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\bifurc-ui.exe"

  ; `bin` holds the `bifurc` command and nothing else, so the engine and the CLI it runs stay off PATH.
  nsExec::Exec '"$INSTDIR\bin\bifurc.exe" --register-path'
  Pop $0

  WriteRegStr HKCU "${SETTINGS_KEY}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\bifurc-ui.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" "$0"
SectionEnd

; Only what the installer put there is removed: the install folder may be one the user chose, and the
; workspaces live elsewhere (%LOCALAPPDATA%\Bifurc) and stay.
Section "Uninstall"
  !insertmacro StopBifurc

  nsExec::Exec '"$INSTDIR\bin\bifurc.exe" --unregister-path'
  Pop $0

  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"
  Delete "$INSTDIR\bifurc-ui.exe"
  Delete "$INSTDIR\bifurc-tray.exe"
  Delete "$INSTDIR\bifurc-engine.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir /r "$INSTDIR\bin"
  RMDir /r "$INSTDIR\runtime"
  RMDir /r "$INSTDIR\resources"
  RMDir "$INSTDIR"

  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "${SETTINGS_KEY}"
SectionEnd
