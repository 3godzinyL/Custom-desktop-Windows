@echo off
rem Indeks - powrot do zwyklego pulpitu Windows.
rem Zamyka Indeks, wylacza autostart i wlacza ikony pulpitu.
rem Pliki na pulpicie nigdy nie sa ruszane - Indeks tylko je wyswietla.
setlocal
set "EXE=%LOCALAPPDATA%\Programs\Indeks\indeks.exe"
if not exist "%EXE%" set "EXE=%~dp0target\release\indeks.exe"

if exist "%EXE%" (
  "%EXE%" --przywroc
) else (
  echo Nie znaleziono indeks.exe - przywracam recznie.
  taskkill /im indeks.exe /f >nul 2>nul
  reg delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v Indeks /f >nul 2>nul
  reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced" /v HideIcons /t REG_DWORD /d 0 /f >nul
  echo Ikony pojawia sie po ponownym zalogowaniu albo: prawy klik na pulpicie - Widok - Pokaz ikony pulpitu.
)
echo.
echo Zwykly pulpit przywrocony. Autostart Indeksu wylaczony.
pause
