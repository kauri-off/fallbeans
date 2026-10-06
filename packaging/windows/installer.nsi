; Per-user install without admin rights (cargo xtask dist nsis). `/S /UPDATE` is the game's silent self-update;
; with `/ARGS=` + a backtick-quoted command line (no backticks inside, before any `/D=`), e.g.
; /S /UPDATE /ARGS=`--profile a --name "Боб"`, the restarted game gets those arguments.
Unicode true
ManifestDPIAware true
!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"

!define APP "Fall Beans"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\FallBeans"

Name "${APP}"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Programs\FallBeans"
; An update (and a reinstall) goes where the game already is, not to the default folder.
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define MUI_ICON "${SRC}\fallbeans.ico"
!define MUI_UNICON "${SRC}\fallbeans.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\fb_client.exe"
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE DedicatedDir
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Russian"

; Shown in the properties of setup.exe (the numeric form has four parts: 0.1.0-alpha → 0.1.0.0).
VIProductVersion "${VI_VERSION}"
VIFileVersion "${VI_VERSION}"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "Установка ${APP}"
VIAddVersionKey "LegalCopyright" "AGPL-3.0-or-later"

Var Update
Var Args

; The game goes into a folder of its own: the installer replaces assets\ in it and the uninstaller removes it, so a
; chosen folder that is not the game's (D:\Games) gets FallBeans\ appended.
Function DedicatedDir
  ${IfNot} ${FileExists} "$INSTDIR\fb_client.exe"
    StrCpy $R0 $INSTDIR 1 -1
    ${If} $R0 == "\"
      StrCpy $INSTDIR $INSTDIR -1
    ${EndIf}
    ${GetFileName} $INSTDIR $R0
    ${If} $R0 != "FallBeans"
      StrCpy $INSTDIR "$INSTDIR\FallBeans"
    ${EndIf}
  ${EndIf}
FunctionEnd

Function .onInit
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/ARGS=" $Args
  ClearErrors
  ${GetOptions} $R0 "/UPDATE" $R1
  ${If} ${Errors}
    StrCpy $Update 0
    ; (A silent install skips the directory page: `/D=` gets the same rule.)
    ${If} ${Silent}
      Call DedicatedDir
    ${EndIf}
  ${Else}
    StrCpy $Update 1
    ; The game that started the update is closing: wait (up to 30 s) until its exe can be written. Still locked
    ; (the game hangs): give up before anything is touched, or the old game would be left with the new assets.
    ${If} ${FileExists} "$INSTDIR\fb_client.exe"
      StrCpy $R2 0
      ${Do}
        Sleep 500
        ClearErrors
        FileOpen $R3 "$INSTDIR\fb_client.exe" a
        ${IfNot} ${Errors}
          FileClose $R3
          ${ExitDo}
        ${EndIf}
        IntOp $R2 $R2 + 1
        ${If} $R2 >= 60
          SetErrorLevel 2
          Abort
        ${EndIf}
      ${Loop}
    ${Else}
      Sleep 2000
    ${EndIf}
  ${EndIf}
FunctionEnd

Section
  SetOutPath "$INSTDIR"
  RMDir /r "$INSTDIR\assets"
  File "${SRC}\fb_client.exe"
  File "${SRC}\fallbeans.ico"
  File "${SRC}\LICENSE"
  File "${SRC}\THIRD-PARTY-LICENSES.html"
  File /r "${SRC}\assets"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\fb_client.exe" "" "$INSTDIR\fallbeans.ico"
  ${If} $Update == 0
    CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\fb_client.exe" "" "$INSTDIR\fallbeans.ico"
  ${EndIf}

  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "kauri-off"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\fallbeans.ico"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" "$0"

  ${If} $Update == 1
    Exec '"$INSTDIR\fb_client.exe" $Args'
  ${EndIf}
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\fb_client.exe"
  Delete "$INSTDIR\fallbeans.ico"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD-PARTY-LICENSES.html"
  Delete "$INSTDIR\uninstall.exe"
  RMDir /r "$INSTDIR\assets"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
SectionEnd
