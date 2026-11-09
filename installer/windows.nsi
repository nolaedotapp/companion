Unicode True
!include "MUI2.nsh"
!include "x64.nsh"
!include "WinVer.nsh"
!ifndef APP_VERSION
  !error "APP_VERSION is required"
!endif
Name "Nolae Companion"
OutFile "${OUTPUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\NolaeCompanion"
RequestExecutionLevel user
VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey "ProductName" "Nolae Companion"
VIAddVersionKey "FileDescription" "Nolae Companion installer"
VIAddVersionKey "FileVersion" "${APP_VERSION}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "LegalCopyright" "Nolae contributors"
SetCompressor /SOLID lzma
!define MUI_ICON "..\assets\nolae.ico"
!define MUI_UNICON "..\assets\nolae.ico"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\nolae-companion.exe"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!macro RemoveLegacyFiles prefix
Function ${prefix}RemoveLegacyFiles
  Delete "$INSTDIR\nolae-companion.exe.new"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\OFL-Manrope.txt"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.html"
  Delete "$INSTDIR\nolae.ico"
FunctionEnd
!macroend
!insertmacro RemoveLegacyFiles ""
!insertmacro RemoveLegacyFiles "un."

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "This release requires 64-bit Windows 10 or later."
    Abort
  ${EndIf}
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_ICONSTOP "This release requires Windows 10 or later."
    Abort
  ${EndIf}
  SetRegView 64
  SetShellVarContext current
FunctionEnd

Section "Nolae" SEC_MAIN
  IfFileExists "$INSTDIR\nolae-companion.exe" 0 install
  ExecWait '"$INSTDIR\nolae-companion.exe" --quit' $0
  ${If} $0 != 0
    MessageBox MB_ICONSTOP "Close Nolae Companion and try again."
    Abort
  ${EndIf}
  install:
  SetOutPath "$INSTDIR"
  File "${APP_EXE}"
  SetOutPath "$INSTDIR\ui\${APP_VERSION}"
  File "${GUI_EXE}"
  SetOutPath "$INSTDIR\licenses"
  File "..\LICENSE"
  File "..\assets\fonts\OFL-Manrope.txt"
  File "${NOTICES_FILE}"
  SetOutPath "$INSTDIR"
  Call RemoveLegacyFiles
  CreateShortcut "$SMPROGRAMS\Nolae Companion.lnk" "$INSTDIR\nolae-companion.exe" "" "$INSTDIR\nolae-companion.exe" 0
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NolaeCompanion" "DisplayName" "Nolae Companion"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NolaeCompanion" "DisplayIcon" "$INSTDIR\nolae-companion.exe,0"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NolaeCompanion" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NolaeCompanion" "Publisher" "Nolae"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NolaeCompanion" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NolaeCompanion" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NolaeCompanion" "NoRepair" 1
SectionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext current
FunctionEnd
Section "Uninstall"
  ExecWait '"$INSTDIR\nolae-companion.exe" --uninstall' $0
  ${If} $0 != 0
    MessageBox MB_ICONSTOP "Nolae could not finish disconnecting this PC. Close the app and try again."
    Abort
  ${EndIf}
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "NolaeCompanion"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NolaeCompanion"
  Delete "$SMPROGRAMS\Nolae Companion.lnk"
  Delete "$INSTDIR\nolae-companion.exe"
  FindFirst $0 $1 "$INSTDIR\ui\*"
  ui_cleanup:
    StrCmp $1 "" ui_done
    StrCmp $1 "." ui_next
    StrCmp $1 ".." ui_next
    Delete "$INSTDIR\ui\$1\nolae-companion-ui.exe"
    Delete "$INSTDIR\ui\$1\nolae-companion.previous.exe"
    RMDir "$INSTDIR\ui\$1"
    ui_next:
    FindNext $0 $1
    Goto ui_cleanup
  ui_done:
  FindClose $0
  RMDir "$INSTDIR\ui"
  Delete "$INSTDIR\licenses\LICENSE"
  Delete "$INSTDIR\licenses\OFL-Manrope.txt"
  Delete "$INSTDIR\licenses\THIRD_PARTY_NOTICES.html"
  RMDir "$INSTDIR\licenses"
  Call un.RemoveLegacyFiles
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
