@echo off
chcp 65001 >nul
call "%~dp0server\lang.cmd"
cd /d "%~dp0server"
set PYTHONIOENCODING=utf-8
set PY=.venv\Scripts\python.exe
if not exist %PY% set PY=python
echo %MSG_RESTORE_TITLE%
echo.
%PY% backups.py list 30
echo.
set /p ID=%MSG_RESTORE_ASK%
if "%ID%"=="" goto :eof
%PY% backups.py restore %ID%
echo.
pause
