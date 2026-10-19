; InstallScope silent setup
; Copyright (C) 2026 Sean Shaffer. GPLv3: free software, no warranty.
;
; One file that redeploys a repackaged installation without any questions:
; it unpacks the package (files, install.reg, install.ps1, uninstall.ps1),
; runs install.ps1 and keeps uninstall.ps1 for later.
;
; Defines passed by InstallScope: PACKAGE, NAME, SAFENAME, VERSION, OUTFILE.

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel admin
SilentInstall silent

!include "LogicLib.nsh"

Name "${NAME}"
OutFile "${OUTFILE}"

VIProductVersion "${VERSION}"
VIAddVersionKey "ProductName" "${NAME}"
VIAddVersionKey "FileDescription" "${NAME} (silent setup made by InstallScope)"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "See the program's own license"

Section
	SetShellVarContext all
	InitPluginsDir
	SetOutPath "$PLUGINSDIR\package"
	File /r "${PACKAGE}\*.*"
	nsExec::ExecToLog 'powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\package\install.ps1"'
	Pop $0
	; Keep the removal script where an administrator can find it.
	CreateDirectory "$APPDATA\InstallScope\Packages\${SAFENAME}"
	CopyFiles /SILENT "$PLUGINSDIR\package\uninstall.ps1" "$APPDATA\InstallScope\Packages\${SAFENAME}"
	SetOutPath "$TEMP"
	${If} $0 != 0
		SetErrorLevel 1
	${EndIf}
SectionEnd
