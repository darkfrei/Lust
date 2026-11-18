@echo off
rem for an AI that builds a game: writes the line for the settings of Claude Desktop
rem (claude_desktop_config.json, the part "mcpServers") into the file lust-dev-line.txt and opens it.
rem you type the folder of the game; the AI writes its lua files there and nowhere else.
setlocal
cd /d "%~dp0"
set /p GAME=Folder of the game (for example D:\games\pong, it is made if it is not there): 
if "%GAME%"=="" goto :eof
if not exist "%GAME%" mkdir "%GAME%"
set EXE=%~dp0lust.exe
set EXE=%EXE:\=\\%
set DIR=%GAME:\=\\%
> lust-dev-line.txt echo "lust": { "command": "%EXE%", "args": ["dev", "%DIR%"] }
if not exist "%GAME%\world.lua" > "%GAME%\world.lua" echo return { types = {}, entities = {} }
start notepad lust-dev-line.txt
echo.
echo The line is in lust-dev-line.txt: put it into "mcpServers" in the settings of Claude Desktop,
echo then restart Claude Desktop. A window with your game opens when the AI starts lust.
pause
