; Mahjuro NSIS installer script.
;
; Expects the following /D defines passed on the makensis command line:
;   /DVERSION=x.y.z[-pre]  Display version (no leading "v"; may include semver pre-release)
;   /DVI_PRODUCT_VERSION=a.b.c.d
;                          Windows binary version resource (four dot-separated integers only).
;                          For prereleases, pass the core triad + 0 (e.g. VERSION=0.5.0-0 →
;                          VI_PRODUCT_VERSION=0.5.0.0).
;   /DSOURCE_DIR=path      Directory containing mahjuro.exe and icon.ico
;   /DOUTFILE=path         Output installer .exe path

!include "MUI2.nsh"

Name "Mahjuro"
OutFile "${OUTFILE}"
Unicode True

; Per-user install — no admin rights required, no UAC prompt.
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\Mahjuro"
InstallDirRegKey HKCU "Software\Mahjuro" "InstallDir"

VIProductVersion "${VI_PRODUCT_VERSION}"
VIAddVersionKey "ProductName" "Mahjuro"
VIAddVersionKey "FileDescription" "Mahjuro"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "Mahjuro"
VIAddVersionKey "LegalCopyright" "Mahjuro"

!define MUI_ICON "${SOURCE_DIR}\icon.ico"
!define MUI_UNICON "${SOURCE_DIR}\icon.ico"
!define MUI_ABORTWARNING

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\mahjuro.exe"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Mahjuro" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"

  File "${SOURCE_DIR}\mahjuro.exe"
  File "${SOURCE_DIR}\steam_api64.dll"
  File "${SOURCE_DIR}\dxcompiler.dll"
  File "${SOURCE_DIR}\dxil.dll"
  File "${SOURCE_DIR}\icon.ico"
  File "${SOURCE_DIR}\pack_manifest.json"
  File "${SOURCE_DIR}\mahjuro-pack-shared.zip"
  File "${SOURCE_DIR}\mahjuro-pack-rooms.zip"
  File "${SOURCE_DIR}\mahjuro-pack-gameplay-bulk.zip"
  File "${SOURCE_DIR}\mahjuro-pack-music.zip"

  WriteRegStr HKCU "Software\Mahjuro" "InstallDir" "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateDirectory "$SMPROGRAMS\Mahjuro"
  CreateShortCut "$SMPROGRAMS\Mahjuro\Mahjuro.lnk" "$INSTDIR\mahjuro.exe" "" "$INSTDIR\icon.ico"
  CreateShortCut "$SMPROGRAMS\Mahjuro\Uninstall Mahjuro.lnk" "$INSTDIR\Uninstall.exe"

  ; Add/Remove Programs entry (per-user, HKCU).
  !define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Mahjuro"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "Mahjuro"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Mahjuro"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" "$\"$INSTDIR\Uninstall.exe$\""
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" "$\"$INSTDIR\Uninstall.exe$\" /S"
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\mahjuro.exe"
  Delete "$INSTDIR\steam_api64.dll"
  Delete "$INSTDIR\dxcompiler.dll"
  Delete "$INSTDIR\dxil.dll"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\pack_manifest.json"
  Delete "$INSTDIR\mahjuro-pack-shared.zip"
  Delete "$INSTDIR\mahjuro-pack-rooms.zip"
  Delete "$INSTDIR\mahjuro-pack-gameplay-bulk.zip"
  Delete "$INSTDIR\mahjuro-pack-music.zip"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\Mahjuro\Mahjuro.lnk"
  Delete "$SMPROGRAMS\Mahjuro\Uninstall Mahjuro.lnk"
  RMDir "$SMPROGRAMS\Mahjuro"

  DeleteRegKey HKCU "Software\Mahjuro"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Mahjuro"
SectionEnd
