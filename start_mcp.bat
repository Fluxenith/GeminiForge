@echo off
rem Starts the GeminiForge MCP server for Gemini's "@ custom app".
chcp 65001 >nul
call "%~dp0server\lang.cmd"
title %MSG_MCP_TITLE%
cd /d "%~dp0server"
rem Fast waits: change the 1 to 0 for fixed waits.
if not defined GF_FAST_WAIT set "GF_FAST_WAIT=1"
call "%~dp0server\exe.cmd"
if errorlevel 1 goto :fail
"%EXE%" mcp
pause
exit /b
:fail
pause
exit /b 1
