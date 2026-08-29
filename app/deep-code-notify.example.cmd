@echo off
REM Managed by OpenMicro Settings. Remove this line before editing; marked files may be replaced on update.
REM Copy to %%USERPROFILE%%\.deepcode\openmicro-notify.cmd and set the
REM `notify` key in %%USERPROFILE%%\.deepcode\settings.json to that path.
set "OPENMICRO_BIN=C:\Program Files\OpenMicro\OpenMicro.exe"

set "light_status="
if /I "%STATUS%"=="completed" set "light_status=success"
if /I "%STATUS%"=="failed" set "light_status=error"
if not defined light_status exit /b 0

set "session_title=%TITLE%"
if not defined session_title set "session_title=default"
"%OPENMICRO_BIN%" status "%light_status%" "deep-code:%session_title%" >nul 2>&1
exit /b 0
