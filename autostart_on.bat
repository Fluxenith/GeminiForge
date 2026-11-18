@echo off
chcp 65001 >nul
call "%~dp0server\lang.cmd"
powershell -NoProfile -ExecutionPolicy Bypass -Command "$s=(New-Object -ComObject WScript.Shell).CreateShortcut([Environment]::GetFolderPath('Startup')+'\GeminiForge.lnk'); $s.TargetPath='%~dp0start.bat'; $s.WorkingDirectory='%~dp0'; $s.WindowStyle=7; $s.Save()"
if errorlevel 1 (
  echo %MSG_AUTO_FAIL%
) else (
  echo %MSG_AUTO_ON%
  echo %MSG_AUTO_ON_HINT%
)
pause
