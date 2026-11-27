@echo off
rem Sets EXE to the GeminiForge server program, building it the first time.
rem Callers run lang.cmd first. Ends with errorlevel 1 when there is no program.
set "EXE=%~dp0target\release\geminiforge-server.exe"
if exist "%EXE%" exit /b 0
set "EXE=%~dp0geminiforge-server.exe"
if exist "%EXE%" exit /b 0
set "EXE=%~dp0target\release\geminiforge-server.exe"
if exist "%USERPROFILE%\.cargo\bin\cargo.exe" set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
where cargo >nul 2>nul
if errorlevel 1 (
  echo %MSG_NO_CARGO%
  exit /b 1
)
echo %MSG_FIRST_RUN%
pushd "%~dp0"
cargo build --release
set "GF_RC=%errorlevel%"
popd
if not "%GF_RC%"=="0" (
  echo %MSG_BUILD_FAILED%
  exit /b 1
)
exit /b 0
