; NSIS installer hook for the Tauri bundle. Wired via tauri.conf.json:
;   bundle.windows.nsis.installerHooks = "../../installers/windows/install-service.nsh"
; Runs elevated during install: registers and starts the voidns service so the
; user never has to install it by hand (the bug this fixes: on Windows the
; service was never registered, so the GUI showed "NO SERVICE").
;
; The privileged service (voidns-service.exe) and the CLI (voidns.exe) are
; shipped next to voidns-gui.exe in $INSTDIR as Tauri sidecars (externalBin) —
; Tauri strips the target-triple suffix at install, so the paths below resolve.

!macro NSIS_HOOK_POSTINSTALL
  ; Stop/remove any prior instance so re-installs are idempotent.
  ExecWait 'sc stop voidns'
  ExecWait 'sc delete voidns'
  ExecWait 'sc create voidns binPath= "$INSTDIR\voidns-service.exe run" start= auto DisplayName= "voidns"'
  ExecWait 'sc description voidns "voidns local DoH proxy and DNS redirector"'
  ExecWait 'sc start voidns'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ExecWait 'sc stop voidns'
  ExecWait 'sc delete voidns'
!macroend
