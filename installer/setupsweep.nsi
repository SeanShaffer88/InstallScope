; SetupSweep installer (NSIS 3, Unicode)
; Copyright (C) 2026 Sean Shaffer — MIT
;
; Build:  makensis -DVERSION=0.1.0 -DEXE=path\to\SetupSweep.exe installer\setupsweep.nsi
; Output: SetupSweep-Setup-<VERSION>.exe next to this script (or -DOUTDIR=...)

Unicode true
ManifestDPIAware true
RequestExecutionLevel admin
SetCompressor /SOLID lzma

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef EXE
  !define EXE "..\target\release\setupsweep.exe"
!endif
!ifndef OUTDIR
  !define OUTDIR "."
!endif

!define APP "SetupSweep"
!define PUBLISHER "Sean Shaffer"
!define URL "https://github.com/SeanShaffer88/SetupSweep"
; Breaking rename: the Expunge-era uninstall key is not reused.
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\SetupSweep"

Name "${APP} ${VERSION}"
OutFile "${OUTDIR}\SetupSweep-Setup-${VERSION}.exe"
InstallDir "$PROGRAMFILES64\${APP}"
InstallDirRegKey HKLM "${UNINST_KEY}" "InstallLocation"
BrandingText "${APP} ${VERSION} · © ${PUBLISHER}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "${APP}"
VIAddVersionKey /LANG=1033 "CompanyName" "${PUBLISHER}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "© 2026 ${PUBLISHER} (MIT)"
VIAddVersionKey /LANG=1033 "FileDescription" "${APP} Setup"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${VERSION}"

!include "MUI2.nsh"
!include "x64.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "WinVer.nsh"

!macro AddShellVerb EXT
  WriteRegStr HKLM "Software\Classes\SystemFileAssociations\${EXT}\shell\SetupSweepInstall" "MUIVerb" "Install with SetupSweep"
  WriteRegStr HKLM "Software\Classes\SystemFileAssociations\${EXT}\shell\SetupSweepInstall" "Icon" '"$INSTDIR\SetupSweep.exe",0'
  WriteRegStr HKLM "Software\Classes\SystemFileAssociations\${EXT}\shell\SetupSweepInstall\command" "" '"$INSTDIR\SetupSweep.exe" --monitor-install "%1"'
!macroend

!define MUI_ICON "..\assets\setupsweep.ico"
!define MUI_UNICON "..\assets\setupsweep.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\SetupSweep.exe"
!define MUI_FINISHPAGE_LINK "${URL}"
!define MUI_FINISHPAGE_LINK_LOCATION "${URL}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; Same languages as the app itself.
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Spanish"
!insertmacro MUI_LANGUAGE "German"
!insertmacro MUI_LANGUAGE "French"
!insertmacro MUI_LANGUAGE "PortugueseBR"
!insertmacro MUI_LANGUAGE "Italian"
!insertmacro MUI_LANGUAGE "Russian"
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "Japanese"
!insertmacro MUI_LANGUAGE "Korean"
!insertmacro MUI_LANGUAGE "Polish"
!insertmacro MUI_RESERVEFILE_LANGDLL

Function .onInit
  ; SetupSweep is built for Windows 11 only (build 22000 or later, 64-bit).
  ${IfNot} ${RunningX64}
  ${OrIfNot} ${AtLeastBuild} 22000
    MessageBox MB_ICONSTOP "SetupSweep requires 64-bit Windows 11."
    Abort
  ${EndIf}
  SetRegView 64
  !insertmacro MUI_LANGDLL_DISPLAY
FunctionEnd

Function un.onInit
  SetRegView 64
FunctionEnd

Section "SetupSweep" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "/oname=SetupSweep.exe" "${EXE}"
  !ifdef TOOLS
    ; The PortableApps.com Launcher (official source and its NSIS compiler)
    ; used by PortableApps Forge.
    RMDir /r "$INSTDIR\tools\PortableApps.comLauncher"
    SetOutPath "$INSTDIR\tools\PortableApps.comLauncher"
    File /r "${TOOLS}\*.*"
    SetOutPath "$INSTDIR"
  !endif
  File "/oname=LICENSE.txt" "..\LICENSE"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; Start menu
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\SetupSweep.exe"

  ; Apps & features entry
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\SetupSweep.exe,0"
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "URLInfoAbout" "${URL}"
  WriteRegStr HKLM "${UNINST_KEY}" "HelpLink" "${URL}/issues"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKLM "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKLM "${UNINST_KEY}" "EstimatedSize" $0
SectionEnd

Section "Right-click: Install with SetupSweep" SecShell
  ; Adds "Install with SetupSweep" to the context menu of .exe and .msi files.
  !insertmacro AddShellVerb ".exe"
  !insertmacro AddShellVerb ".msi"
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\SetupSweep.exe"
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\SetupSweep.exe"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir /r "$INSTDIR\tools\PortableApps.comLauncher"
  RMDir "$INSTDIR\tools"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"
  DeleteRegKey HKLM "${UNINST_KEY}"
  DeleteRegKey HKLM "Software\Classes\SystemFileAssociations\.exe\shell\SetupSweepInstall"
  DeleteRegKey HKLM "Software\Classes\SystemFileAssociations\.msi\shell\SetupSweepInstall"
  ; Install logs and backups in %LOCALAPPDATA%\SetupSweep are kept on uninstall
  ; and may be needed to undo earlier removals. Expunge-era data is left untouched.
SectionEnd
