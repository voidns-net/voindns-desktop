; NSIS installer hook for the Tauri bundle (tauri.conf.json:
;   bundle.windows.nsis.installerHooks = "../../installers/windows/install-service.nsh")
; Runs elevated during install: registers and starts the voindns service.

!macro NSIS_HOOK_POSTINSTALL
  ; The service binary is shipped alongside the GUI in $INSTDIR.
  ExecWait 'sc create voindns binPath= "$INSTDIR\voindns-service.exe run" start= auto DisplayName= "voindns"'
  ExecWait 'sc description voindns "voindns local DoH proxy and DNS redirector"'
  ExecWait 'sc start voindns'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ExecWait 'sc stop voindns'
  ExecWait 'sc delete voindns'
!macroend
