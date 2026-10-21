; SetupSweep PortableApps.com Format installer
; Copyright (C) 2026 Sean Shaffer. MIT License: free software, no warranty.
;
; Installs a PortableApps.com Format package the way PortableApps.com
; installers do: into <destination>\<AppID>, keeping Data on upgrades and
; replacing App and Other. Understands the PortableApps.com Platform's
; /DESTINATION= and /AUTOCLOSE= options and NSIS's /S (silent).
;
; Defines passed by SetupSweep: PACKAGE, APPID, NAME, PACKAGEVERSION,
; DISPLAYVERSION, OUTFILE.

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"

Name "${NAME}"
OutFile "${OUTFILE}"
InstallDir "$EXEDIR\${APPID}"
BrandingText "PortableApps.com Format"
ShowInstDetails nevershow

!if /FileExists "${PACKAGE}\App\AppInfo\appicon.ico"
	!define MUI_ICON "${PACKAGE}\App\AppInfo\appicon.ico"
!endif
!define MUI_ABORTWARNING
!define MUI_DIRECTORYPAGE_TEXT_TOP "${NAME} will be installed in this folder. To update an existing copy, choose its folder: your settings (the Data folder) are kept."
!define MUI_FINISHPAGE_NOAUTOCLOSE

!if /FileExists "${PACKAGE}\App\AppInfo\EULA.txt"
	!insertmacro MUI_PAGE_LICENSE "${PACKAGE}\App\AppInfo\EULA.txt"
!endif
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_LANGUAGE "English"

VIProductVersion "${PACKAGEVERSION}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "ProductName" "${NAME}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "FileDescription" "${NAME} (PortableApps.com Format)"
VIAddVersionKey /LANG=${LANG_ENGLISH} "FileVersion" "${PACKAGEVERSION}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "ProductVersion" "${DISPLAYVERSION}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "LegalCopyright" "See App\AppInfo\appinfo.ini"
VIAddVersionKey /LANG=${LANG_ENGLISH} "PortableApps.comAppID" "${APPID}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "PortableApps.comInstallerVersion" "SetupSweep"

Function .onInit
	${GetParameters} $R0

	; The PortableApps.com Platform passes its PortableApps folder.
	ClearErrors
	${GetOptions} $R0 "/DESTINATION=" $0
	${IfNot} ${Errors}
	${AndIf} $0 != ""
		StrCpy $1 $0 1 -1
		${If} $1 != "\"
			StrCpy $0 "$0\"
		${EndIf}
		StrCpy $INSTDIR "$0${APPID}"
	${Else}
		; Otherwise default to the PortableApps folder of this drive, if any.
		${GetRoot} $EXEDIR $1
		${If} ${FileExists} "$1\PortableApps\*.*"
			StrCpy $INSTDIR "$1\PortableApps\${APPID}"
		${EndIf}
	${EndIf}

	ClearErrors
	${GetOptions} $R0 "/AUTOCLOSE=" $0
	${IfNot} ${Errors}
	${AndIf} $0 == "true"
		SetAutoClose true
	${EndIf}
FunctionEnd

Section "${NAME}"
	; The program must not be running while it is replaced.
	${If} ${FileExists} "$INSTDIR\${APPID}.exe"
		ClearErrors
		FileOpen $0 "$INSTDIR\${APPID}.exe" a
		${If} ${Errors}
			MessageBox MB_OK|MB_ICONSTOP "Please close ${NAME} before updating it." /SD IDOK
			Abort
		${EndIf}
		FileClose $0
	${EndIf}

	; Upgrades replace the program and keep your settings in Data.
	RMDir /r "$INSTDIR\App"
	RMDir /r "$INSTDIR\Other"

	SetOutPath "$INSTDIR"
	File "${PACKAGE}\${APPID}.exe"
	!if /FileExists "${PACKAGE}\help.html"
		File "${PACKAGE}\help.html"
	!endif
	SetOutPath "$INSTDIR\App"
	File /r "${PACKAGE}\App\*.*"
	!if /FileExists "${PACKAGE}\Other\*.*"
		SetOutPath "$INSTDIR\Other"
		File /r "${PACKAGE}\Other\*.*"
	!endif
	CreateDirectory "$INSTDIR\Data"
	SetOutPath "$INSTDIR"
SectionEnd
