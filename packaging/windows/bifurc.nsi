; The Bifurc installer: the staged desktop folder (`cargo xtask package`), installed for the current
; user with no elevation, the `bifurc` command on the user's PATH, and an entry in Apps & features.
;
; Running it asks nothing: one small borderless window (logo, title, status line, progress bar) installs
; into the per-user folder, starts Bifurc and closes. `/S` still installs silently and `/D=<dir>` still picks
; the folder. The window is the installer's own progress page, restyled in InstallShow.
;
; makensis /DVERSION=0.4.0 /DSTAGE=<staged folder> /DOUT=<setup.exe> /DICON=<icon.ico> bifurc.nsi

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user
ManifestDPIAware true
ShowInstDetails nevershow

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"

!define APP "Bifurc"
!define PUBLISHER "Harshal Kudale"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\BifurcDesktop"
!define SETTINGS_KEY "Software\BifurcDesktop"

; The window, in pixels at 96 dpi, and its colours (RRGGBB).
!define WIN_W 480
!define WIN_H 300
!define COL_BG 0B0F0E
!define COL_TEXT E8F5F0
!define COL_MUTED 7F918B
; The same two for the progress bar, which wants BBGGRR.
!define BAR_FILL 0xAEE62E
!define BAR_TRACK 0x22261B

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

Var Dpi
Var Inner
Var Status

!define MUI_PAGE_CUSTOMFUNCTION_SHOW InstallShow
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; ${Scale} <out> <96 dpi pixels>
!macro Scale OUT PX
  IntOp ${OUT} ${PX} * $Dpi
  IntOp ${OUT} ${OUT} / 96
!macroend
!define Scale "!insertmacro Scale"

; ${Hide} <parent window> <control id>
!macro Hide PARENT ID
  GetDlgItem $0 ${PARENT} ${ID}
  ShowWindow $0 ${SW_HIDE}
!macroend
!define Hide "!insertmacro Hide"

; A centred, coloured label on the inner dialog; its handle goes into <var>.
; ${Label} <var> <text> <x> <y> <w> <h> <font px> <weight> <RRGGBB>
!macro Label VAR TEXT X Y W H FONT WEIGHT COLOR
  ${Scale} $0 ${X}
  ${Scale} $1 ${Y}
  ${Scale} $2 ${W}
  ${Scale} $3 ${H}
  System::Call 'user32::CreateWindowEx(i 0, t "STATIC", t "${TEXT}", i 0x50000081, i $0, i $1, i $2, i $3, i $Inner, i 0, i 0, i 0) i.s'
  Pop ${VAR}
  ${Scale} $0 ${FONT}
  IntOp $0 0 - $0
  System::Call 'gdi32::CreateFont(i $0, i 0, i 0, i 0, i ${WEIGHT}, i 0, i 0, i 0, i 1, i 0, i 0, i 5, i 0, t "Segoe UI") i.s'
  Pop $0
  SendMessage ${VAR} ${WM_SETFONT} $0 1
  SetCtlColors ${VAR} ${COLOR} ${COL_BG}
!macroend
!define Label "!insertmacro Label"

; ${SetStatus} <text>
!macro SetStatus TEXT
  SendMessage $Status ${WM_SETTEXT} 0 "STR:${TEXT}"
!macroend
!define SetStatus "!insertmacro SetStatus"

; Turns the progress page into the installer window: no frame, no buttons, no file list; a logo, a title, a
; status line and a thin progress bar on a dark panel. Control ids are NSIS's: 1/2/3 the buttons, 1004 the
; progress bar, 1006 the status text, 1016 the file list, 1027 the details button, the rest the page header,
; the branding strip and the bottom rule.
Function InstallShow
  SetAutoClose true
  StrCpy $Dpi 96
  System::Call 'user32::GetDpiForWindow(i $HWNDPARENT) i.s'
  Pop $0
  ${If} $0 > 0
    StrCpy $Dpi $0
  ${EndIf}

  ${Hide} $HWNDPARENT 1
  ${Hide} $HWNDPARENT 2
  ${Hide} $HWNDPARENT 3
  ${Hide} $HWNDPARENT 1028
  ${Hide} $HWNDPARENT 1034
  ${Hide} $HWNDPARENT 1035
  ${Hide} $HWNDPARENT 1036
  ${Hide} $HWNDPARENT 1037
  ${Hide} $HWNDPARENT 1038
  ${Hide} $HWNDPARENT 1039
  ${Hide} $HWNDPARENT 1045
  ${Hide} $HWNDPARENT 1256

  ; A popup with a hairline border, centred on the primary screen.
  System::Call 'user32::SetWindowLong(i $HWNDPARENT, i -16, i 0x92800000)'
  ${Scale} $1 ${WIN_W}
  ${Scale} $2 ${WIN_H}
  System::Call 'user32::GetSystemMetrics(i 0) i.s'
  System::Call 'user32::GetSystemMetrics(i 1) i.s'
  Pop $4
  Pop $3
  IntOp $3 $3 - $1
  IntOp $3 $3 / 2
  IntOp $4 $4 - $2
  IntOp $4 $4 / 2
  System::Call 'user32::SetWindowPos(i $HWNDPARENT, i 0, i $3, i $4, i $1, i $2, i 0x24)'

  FindWindow $Inner "#32770" "" $HWNDPARENT
  System::Call 'user32::SetWindowPos(i $Inner, i 0, i 0, i 0, i $1, i $2, i 0x14)'
  SetCtlColors $Inner ${COL_TEXT} ${COL_BG}
  ${Hide} $Inner 1006
  ${Hide} $Inner 1016
  ${Hide} $Inner 1027

  ; The logo: the installer's own icon, loaded at the size it is shown.
  InitPluginsDir
  File "/oname=$PLUGINSDIR\icon.ico" "${ICON}"
  ${Scale} $0 72
  System::Call 'user32::LoadImage(i 0, t "$PLUGINSDIR\icon.ico", i 1, i $0, i $0, i 0x10) i.s'
  Pop $5
  ${Scale} $1 ${WIN_W}
  IntOp $1 $1 - $0
  IntOp $1 $1 / 2
  ${Scale} $2 40
  System::Call 'user32::CreateWindowEx(i 0, t "STATIC", t "", i 0x50000003, i $1, i $2, i $0, i $0, i $Inner, i 0, i 0, i 0) i.s'
  Pop $6
  SendMessage $6 0x172 1 $5
  SetCtlColors $6 ${COL_TEXT} ${COL_BG}

  ${Label} $7 "Installing ${APP}" 0 126 ${WIN_W} 36 26 600 ${COL_TEXT}
  ${Label} $Status "Getting things ready..." 0 168 ${WIN_W} 24 14 400 ${COL_MUTED}
  ${Label} $7 "Version ${VERSION}" 0 262 ${WIN_W} 20 12 400 ${COL_MUTED}

  ; The progress bar NSIS already drives, flattened: no visual style, so the two colours apply.
  GetDlgItem $8 $Inner 1004
  ${Scale} $0 70
  ${Scale} $1 214
  ${Scale} $2 340
  ${Scale} $3 6
  System::Call 'user32::SetWindowPos(i $8, i 0, i $0, i $1, i $2, i $3, i 0x14)'
  System::Call 'uxtheme::SetWindowTheme(i $8, w "", w "")'
  SendMessage $8 0x409 0 ${BAR_FILL}
  SendMessage $8 0x2001 0 ${BAR_TRACK}
FunctionEnd

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
  ${SetStatus} "Closing the running copy..."
  !insertmacro StopBifurc

  SetOutPath "$INSTDIR"
  ; What an earlier version left in these folders is not part of this one (`runtime` held the Node engine's Node).
  RMDir /r "$INSTDIR\resources"
  RMDir /r "$INSTDIR\runtime"
  ${SetStatus} "Installing files..."
  File /r "${STAGE}\*.*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ${SetStatus} "Adding shortcuts..."
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\bifurc-ui.exe"
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\bifurc-ui.exe"

  ; `bin` holds the `bifurc` command and nothing else, so the engine and the CLI it runs stay off PATH.
  nsExec::Exec '"$INSTDIR\bin\bifurc.exe" --register-path'
  Pop $0

  ${SetStatus} "Finishing up..."
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

; No finish page: the app opens as soon as the bar is full. A silent install installs and nothing else.
Function .onInstSuccess
  ${IfNot} ${Silent}
    Exec '"$INSTDIR\bifurc-ui.exe"'
  ${EndIf}
FunctionEnd

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
