; NSIS installer hook for the Tauri bundle (tauri.conf.json:
;   bundle.windows.nsis.installerHooks = "../../installers/windows/install-service.nsh")
; Runs elevated during install: registers and starts the voidns service.

!macro NSIS_HOOK_POSTINSTALL
  ; The service binary is shipped alongside the GUI in $INSTDIR.
  ExecWait 'sc create voidns binPath= "$INSTDIR\voidns-service.exe run" start= auto DisplayName= "voidns"'
  ExecWait 'sc description voidns "voidns local DoH proxy and DNS redirector"'
  ExecWait 'sc start voidns'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ExecWait 'sc stop voidns'
  ExecWait 'sc delete voidns'
!macroend
