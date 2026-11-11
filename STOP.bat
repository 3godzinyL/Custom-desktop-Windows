@echo off
rem Indeks - zatrzymanie: zamyka pulpit Indeksu razem z jego oknami i przywraca ikony Windows.
rem Pliki na pulpicie nie sa ruszane. Autostart zostaje bez zmian (wylacza go PRZYWROC_PULPIT.cmd).
setlocal
set "EXE=%LOCALAPPDATA%\Programs\Indeks\indeks.exe"
if not exist "%EXE%" set "EXE=%~dp0target\release\indeks.exe"
if exist "%EXE%" "%EXE%" --zamknij
rem silnik nakladki (C#) konczy sie sam razem z Indeksem - to tylko na wszelki wypadek
taskkill /im indeks-nakladka.exe /f >nul 2>nul
exit /b 0
