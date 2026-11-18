@echo off
rem for an AI that builds games: writes the settings of Claude Desktop for lust into the file
rem lust-dev-line.txt and opens it. the games are the folder "games" beside lust.exe; in the
rem chat you only say which game (a new name makes a new one).
setlocal
cd /d "%~dp0"
if not exist "%~dp0games" mkdir "%~dp0games"
set EXE=%~dp0lust.exe
set EXE=%EXE:\=\\%
> lust-dev-line.txt echo {
>> lust-dev-line.txt echo   "mcpServers": {
>> lust-dev-line.txt echo     "lust": { "command": "%EXE%", "args": ["dev"] }
>> lust-dev-line.txt echo   }
>> lust-dev-line.txt echo }
start notepad lust-dev-line.txt
echo.
echo Claude Desktop: Settings, Developer, Edit Config. Put the text of lust-dev-line.txt there
echo (if "mcpServers" is already there, add only the line with "lust" into it), save, and
echo quit Claude Desktop fully (the icon by the clock, Quit), then start it again.
echo One window of lust opens. In a chat: "open the game bomberman" or any new name.
pause
