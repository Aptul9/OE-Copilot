@echo off
cd /d "%~dp0"
rem pythonw.exe = niente finestra di console: con python.exe se ne aprirebbe una
rem seconda accanto a quella dell'app. Gli errori finiscono in debug.log/crash.log.
set "PYW=%~dp0.venv\Scripts\pythonw.exe"
if not exist "%PYW%" set "PYW=%LOCALAPPDATA%\Programs\Python\Python312\pythonw.exe"
if not exist "%PYW%" set "PYW=pythonw"
start "" "%PYW%" "%~dp0avvia.pyw"

