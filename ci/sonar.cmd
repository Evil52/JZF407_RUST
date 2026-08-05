@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0sonar.ps1" %*
exit /b %ERRORLEVEL%
