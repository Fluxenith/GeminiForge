@echo off
chcp 65001 >nul
call "%~dp0server\lang.cmd"
title %MSG_SERVER_TITLE%
cd /d "%~dp0server"
if not exist ".venv\Scripts\python.exe" (
  echo %MSG_FIRST_RUN%
  python -m venv .venv
  if errorlevel 1 (
    echo %MSG_NO_PYTHON%
    pause
    exit /b 1
  )
  ".venv\Scripts\python.exe" -m pip install -r requirements.txt
  if errorlevel 1 (
    echo %MSG_PIP_FAILED%
    pause
    exit /b 1
  )
)
set PYTHONIOENCODING=utf-8
".venv\Scripts\python.exe" -u server.py
pause
