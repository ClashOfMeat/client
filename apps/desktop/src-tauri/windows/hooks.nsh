; Clash of Meat installer hooks (Tauri NSIS). The installer is per-user and needs no admin rights.

!macro NSIS_HOOK_PREINSTALL
  ; An update replaces clashofmeat.exe, which the running daemon holds open. The app starts it
  ; again when it relaunches.
  nsExec::Exec 'taskkill /F /IM clashofmeat.exe'
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Records the web app this build belongs to and wires Cursor; pairing happens in the app.
  nsExec::Exec '"$INSTDIR\clashofmeat.exe" install --no-connect'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'taskkill /F /IM clashofmeat.exe'
  nsExec::Exec '"$INSTDIR\clashofmeat.exe" uninstall'
  ; The app registers itself to start at sign-in (tauri-plugin-autostart).
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Clash of Meat"
!macroend
