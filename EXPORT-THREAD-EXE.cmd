@echo off
chcp 65001 >nul
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\export-thread-exe.ps1" %*
set "EXPORT_EXIT=%ERRORLEVEL%"
if not "%EXPORT_EXIT%"=="0" echo Export failed. See the error above.
pause
exit /b %EXPORT_EXIT%
