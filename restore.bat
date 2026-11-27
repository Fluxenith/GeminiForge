@echo off
rem Lists the file backups and restores one.
chcp 65001 >nul
call "%~dp0server\lang.cmd"
cd /d "%~dp0server"
call "%~dp0server\exe.cmd"
if errorlevel 1 goto :fail
echo %MSG_RESTORE_TITLE%
echo.
"%EXE%" backups list 30
echo.
set "ID="
set /p ID=%MSG_RESTORE_ASK%
if not defined ID goto :eof
"%EXE%" backups restore "%ID%"
echo.
pause
exit /b
:fail
pause
exit /b 1
