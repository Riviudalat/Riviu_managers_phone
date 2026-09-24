@echo off
setlocal
cd /d "%~dp0"
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start-windows.ps1" %*
set "RIVIU_EXIT=%ERRORLEVEL%"
if not "%RIVIU_EXIT%"=="0" echo Riviu stopped with code %RIVIU_EXIT%. See the message above.
pause
exit /b %RIVIU_EXIT%
