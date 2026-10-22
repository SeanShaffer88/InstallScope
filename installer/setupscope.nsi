; SetupScope per-user installer. Copyright 2026 Sean Shaffer; MIT.
Unicode true
ManifestDPIAware true
RequestExecutionLevel user
SetCompressor /SOLID zlib
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "WinVer.nsh"
!include "FileFunc.nsh"
!ifndef PAYLOAD
  !error "Build with scripts/package-installer.ps1; a verified payload is required."
!endif
!define VERSION "1.0.0.0"
!define KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\SetupScope"
Name "SetupScope ${VERSION}"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\SetupScope"
InstallDirRegKey HKCU "${KEY}" "InstallLocation"
BrandingText "SetupScope - Windows Software Development Toolkit"
VIProductVersion "${VERSION}"
VIAddVersionKey /LANG=1033 "ProductName" "SetupScope"
VIAddVersionKey /LANG=1033 "CompanyName" "Sean Shaffer"
VIAddVersionKey /LANG=1033 "FileDescription" "SetupScope Installer"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Copyright 2026 Sean Shaffer; MIT"
!define MUI_ICON "..\assets\setupscope.ico"
!define MUI_UNICON "..\assets\setupscope.ico"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
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

!macro CheckRunning
  System::Call 'kernel32::OpenMutexW(i 0x100000, i 0, w "Local\SetupScope.Desktop.v1") p.r0'
  ${If} $0 != 0
    System::Call 'kernel32::CloseHandle(p r0)'
    MessageBox MB_ICONSTOP|MB_OK "Close SetupScope and its tool windows, then try again." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
!macroend

Function .onInit
  ${IfNot} ${IsNativeAMD64}
  ${OrIfNot} ${AtLeastBuild} 22000
    MessageBox MB_ICONSTOP|MB_OK "This package requires Windows 11 x64." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  SetRegView 64
  SetShellVarContext current
  !insertmacro CheckRunning
FunctionEnd

; Existing unrelated directories and portable folders must never become an
; installer-owned tree. The generated uninstaller deletes explicit files only.
Function ValidateDirectory
  IfFileExists "$INSTDIR\portable" unsafe
  IfFileExists "$INSTDIR\portable.txt" unsafe
  IfFileExists "$INSTDIR\SetupScope.install-owner" owned
  IfFileExists "$INSTDIR\*.*" unsafe done
  owned:
  FileOpen $0 "$INSTDIR\SetupScope.install-owner" r
  IfErrors unsafe
  FileRead $0 $1
  FileClose $0
  StrCmp $1 "SetupScope.Install.v1" done unsafe
  unsafe:
    MessageBox MB_ICONSTOP|MB_OK "Choose an empty folder or a folder installed by this SetupScope installer. Portable folders and unrelated files are preserved." /SD IDOK
    SetErrorLevel 2
    Abort
  done:
FunctionEnd

Section "SetupScope (required)" App
  SectionIn RO
  !insertmacro CheckRunning
  Call ValidateDirectory
  SetOutPath "$INSTDIR"
  SetOverwrite on
  FileOpen $0 "$INSTDIR\SetupScope.install-owner" w
  IfErrors install_failed
  FileWrite $0 "SetupScope.Install.v1"
  FileClose $0
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  IfErrors install_failed
  ClearErrors
  !include "${PAYLOAD}\install-files.nsh"
  IfErrors install_failed
  WriteRegStr HKCU "${KEY}" "DisplayName" "SetupScope"
  WriteRegStr HKCU "${KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${KEY}" "Publisher" "Sean Shaffer"
  WriteRegStr HKCU "${KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${KEY}" "DisplayIcon" "$INSTDIR\SetupScope.exe,0"
  WriteRegStr HKCU "${KEY}" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKCU "${KEY}" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
  WriteRegStr HKCU "${KEY}" "URLInfoAbout" "https://github.com/SeanShaffer88/SetupScope"
  WriteRegDWORD HKCU "${KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${KEY}" "EstimatedSize" $0
  Goto install_done
  install_failed:
    SetErrorLevel 1
    Abort "Installation could not finish. No user data was removed; retry setup."
  install_done:
SectionEnd

Section "Start Menu shortcuts" StartMenu
  CreateDirectory "$SMPROGRAMS\SetupScope"
  CreateShortcut "$SMPROGRAMS\SetupScope\SetupScope.lnk" "$INSTDIR\SetupScope.exe"
  CreateShortcut "$SMPROGRAMS\SetupScope\Uninstall SetupScope.lnk" "$INSTDIR\Uninstall.exe"
SectionEnd
Section /o "Desktop shortcut" Desktop
  CreateShortcut "$DESKTOP\SetupScope.lnk" "$INSTDIR\SetupScope.exe"
SectionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext current
  !insertmacro CheckRunning
  FileOpen $0 "$INSTDIR\SetupScope.install-owner" r
  IfErrors refuse
  FileRead $0 $1
  FileClose $0
  StrCmp $1 "SetupScope.Install.v1" ready refuse
  refuse:
    MessageBox MB_ICONSTOP|MB_OK "Installation ownership could not be verified. No files were removed." /SD IDOK
    SetErrorLevel 2
    Abort
  ready:
FunctionEnd

Section "Uninstall"
  !insertmacro CheckRunning
  !include "${PAYLOAD}\uninstall-files.nsh"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$INSTDIR\SetupScope.install-owner"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\SetupScope\SetupScope.lnk"
  Delete "$SMPROGRAMS\SetupScope\Uninstall SetupScope.lnk"
  RMDir "$SMPROGRAMS\SetupScope"
  Delete "$DESKTOP\SetupScope.lnk"
  DeleteRegKey HKCU "${KEY}"
  ; Deliberately retain %LOCALAPPDATA%\SetupScope and all unknown files.
  Goto uninstall_done
  uninstall_failed:
    SetErrorLevel 1
    Abort "Some application files could not be removed. Close programs using this folder and retry. User data was preserved."
  uninstall_done:
SectionEnd
