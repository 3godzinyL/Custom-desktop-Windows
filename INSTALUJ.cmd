@echo off
rem Indeks - instalacja jako pulpit (bez uprawnien administratora).
rem Kopiuje indeks.exe do %LOCALAPPDATA%\Programs\Indeks, wlacza autostart
rem (HKCU\...\Run) i uruchamia. Pliki na pulpicie NIE sa ruszane.
setlocal
set "DEST=%LOCALAPPDATA%\Programs\Indeks"
cd /d "%~dp0"

where cargo >nul 2>nul
if %errorlevel%==0 (
  echo Buduje wersje release...
  cargo build --release
  if errorlevel 1 goto :blad
)
if not exist "target\release\indeks.exe" goto :blad

if exist "%DEST%\indeks.exe" "%DEST%\indeks.exe" --zamknij
if not exist "%DEST%" mkdir "%DEST%"
copy /y "target\release\indeks.exe" "%DEST%\indeks.exe" >nul
if errorlevel 1 goto :blad
rem silnik nakladki w C# (opcjonalny - bez niego nakladka dziala w Rust)
if exist "target\release\indeks-nakladka.exe" copy /y "target\release\indeks-nakladka.exe" "%DEST%\indeks-nakladka.exe" >nul

"%DEST%\indeks.exe" --autostart-wl
start "" "%DEST%\indeks.exe"
echo.
echo Gotowe. Indeks dziala jako pulpit i bedzie startowal razem z Windows.
echo  - menu: prawy klik na pulpicie albo ikona folderu w zasobniku
echo  - powrot do zwyklego pulpitu: PRZYWROC_PULPIT.cmd
echo.
pause
exit /b 0

:blad
echo.
echo Instalacja nie powiodla sie (brak target\release\indeks.exe albo blad kopiowania).
pause
exit /b 1
