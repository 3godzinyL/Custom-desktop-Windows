@echo off
rem Indeks - uruchomienie pulpitu.
rem Najpierw wersja zainstalowana (%LOCALAPPDATA%\Programs\Indeks), a bez instalacji - zbudowana.
rem Gdy Indeks juz dziala, tylko wraca do folderu domowego.
setlocal
set "EXE=%LOCALAPPDATA%\Programs\Indeks\indeks.exe"
if not exist "%EXE%" set "EXE=%~dp0target\release\indeks.exe"
if not exist "%EXE%" (
  echo Nie znaleziono indeks.exe - uruchom najpierw INSTALUJ.cmd
  pause
  exit /b 1
)
start "" "%EXE%"
