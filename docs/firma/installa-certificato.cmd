@echo off
rem Rende attendibile su questo PC il certificato con cui e' firmato POPMSG.exe.
rem Avvialo come amministratore: tasto destro, "Esegui come amministratore".
cd /d "%~dp0"
certutil -addstore -f Root POPMSG-firma.cer
certutil -addstore -f TrustedPublisher POPMSG-firma.cer
echo.
echo Fatto: POPMSG e' ora un editore attendibile su questo PC.
pause
