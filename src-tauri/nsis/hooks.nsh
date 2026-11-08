; Tauri NSIS hooks. The uninstaller removes what the app registered outside its folder:
; the File Explorer context menu entry ("Open in Multi Claude") and the "Start with Windows" value.
; Runs on upgrades too, because the old version's uninstaller is executed first; the new version
; registers the menu again at its first start when the setting is on.

!macro NSIS_HOOK_POSTUNINSTALL
  ; Removing the sparse package also ends the COM surrogate that keeps the menu DLL loaded.
  ; The installer is 32-bit: Sysnative reaches the 64-bit PowerShell that has the Appx module.
  StrCpy $0 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  IfFileExists "$0" +2 0
    StrCpy $0 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
  nsExec::Exec '"$0" -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "Get-AppxPackage -Name MultiClaude.ExplorerMenu | Remove-AppxPackage"'
  Pop $1

  ; Tauri's uninstaller runs during an upgrade as well; the classic verbs and the login item are
  ; harmless to recreate, so they are removed every time and written back by the app.
  DeleteRegKey HKCU "Software\Classes\Directory\shell\MultiClaude"
  DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\MultiClaude"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Multi Claude"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Multi Claude"
!macroend
