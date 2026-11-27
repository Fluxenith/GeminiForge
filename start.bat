@echo off
rem Starts the GeminiForge local server for the Chrome extension.
chcp 65001 >nul
call "%~dp0server\lang.cmd"
title %MSG_SERVER_TITLE%
cd /d "%~dp0server"
rem Fast waits: go on as soon as the window or screen has reacted instead of
rem always waiting the fixed time. Change the 1 to 0 for fixed waits.
if not defined GF_FAST_WAIT set "GF_FAST_WAIT=1"
call "%~dp0server\exe.cmd"
if errorlevel 1 goto :fail
"%EXE%"
pause
exit /b
:fail
pause
exit /b 1
