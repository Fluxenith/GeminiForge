@echo off
chcp 65001 >nul
call "%~dp0server\lang.cmd"
title %MSG_MCP_TITLE%
cd /d "%~dp0server"
if not exist ".venv\Scripts\python.exe" (
  echo %MSG_FIRST_RUN%
  python -m venv .venv
  if errorlevel 1 (
    echo %MSG_NO_PYTHON%
    pause
    exit /b 1
  )
)
".venv\Scripts\python.exe" -c "import mcp, uiautomation" 2>nul || ".venv\Scripts\python.exe" -m pip install -r requirements.txt
set PYTHONIOENCODING=utf-8
".venv\Scripts\python.exe" -u mcp_server.py
pause
