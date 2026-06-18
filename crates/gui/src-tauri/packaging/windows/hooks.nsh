; NSIS installer hooks for the VoidNS Windows service.
; Wired in via tauri.conf.json -> bundle.windows.nsis.installerHooks.
; Mirrors AmneziaVPN's componentscript.js (`sc create ... start= auto` + start +
; failure-restart), adapted to Tauri's NSIS template. $INSTDIR is the install dir
; where Tauri places the externalBin sidecar `voidns-service.exe`.

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Registering VoidNS service..."
  ; Remove any stale service from a previous install (ignore failures).
  nsExec::ExecToLog 'sc stop voidns'
  nsExec::ExecToLog 'sc delete voidns'
  ; Create the own-process service, auto-start at boot. Note the required space
  ; after each `=` is sc.exe syntax, not a typo.
  nsExec::ExecToLog 'sc create voidns binPath= "$INSTDIR\voidns-service.exe" start= auto type= own DisplayName= "VoidNS Service"'
  nsExec::ExecToLog 'sc description voidns "VoidNS privileged DNS service"'
  ; Restart on crash: 3 attempts, 2s apart, reset count after 100s.
  nsExec::ExecToLog 'sc failure voidns reset= 100 actions= restart/2000/restart/2000/restart/2000'
  nsExec::ExecToLog 'sc start voidns'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing VoidNS service..."
  nsExec::ExecToLog 'sc stop voidns'
  nsExec::ExecToLog 'sc delete voidns'
!macroend
