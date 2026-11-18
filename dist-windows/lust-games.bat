@echo off
rem the list of games next to this file (the folder "games"), in a window of its own.
rem if something is wrong, the text stays on the screen
cd /d "%~dp0"
lust.exe
if errorlevel 1 (
  echo.
  echo lust ended with an error, see above.
  pause
)
