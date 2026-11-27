@echo off
chcp 65001 >nul
call "%~dp0server\lang.cmd"
powershell -NoProfile -ExecutionPolicy Bypass -Command "Remove-Item -LiteralPath ([Environment]::GetFolderPath('Startup')+'\GeminiForge.lnk') -ErrorAction SilentlyContinue"
echo %MSG_AUTO_OFF%
pause
