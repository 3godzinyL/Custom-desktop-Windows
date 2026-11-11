# Indeks

### Pulpit Windows dla ludzi, którzy mają za dużo plików i projektów

Indeks zamienia pulpit Windows w szybki katalog całego komputera. Zamiast przeklikiwać się przez kolejne okna Eksploratora, przeglądasz foldery, pliki i projekty z jednego miejsca — z wyszukiwaniem, drzewem katalogów, kartami, ulubionymi i informacjami o technologiach.

To natywna aplikacja Windows napisana w Rust. Rysuje własny interfejs bez Electrona i osadzonej przeglądarki, a pliki pozostawia w ich dotychczasowych lokalizacjach.

![Zrzut ekranu aplikacji Indeks](https://cdn.discordapp.com/attachments/703557136793337926/1569912531881820210/image.png?ex=6af52e7b&is=6af3dcfb&hm=fd0185c34b4ff9f27a5514c011677fb0aca7d4c161645711b6d2b76776e87f23)

## Interfejs i kierunek wizualny

Interfejs łączy gęstość klasycznego menedżera plików z informacjami przydatnymi podczas pracy z wieloma projektami. Ciemne tło utrzymuje czytelność przez cały dzień, a cienkie obramowania porządkują widok bez marnowania miejsca.

- **Pasek nawigacji u góry** zawiera skróty do dysków, Eksploratora, Pobranych, ulubionych i otwartych kart.
- **Drzewo folderów po lewej** pokazuje strukturę katalogów i pozwala szybko przeskoczyć do dowolnej gałęzi.
- **Lista projektów i folderów** prezentuje nazwy, datę ostatniej zmiany oraz rozpoznaną technologię.
- **Siatka plików** wykorzystuje ikony Windows, dzięki czemu typy plików są rozpoznawalne bez czytania rozszerzeń.
- **Pasek na dole** pokazuje otwarte aplikacje oraz zużycie zasobów komputera i samego Indeksu.

Wizualnie Indeks nawiązuje do gęstych katalogów znanych z Chromium, ramek paneli w stylu Norton Commandera, drzewa folderów z ery Windows 98 i wyraźnego zaznaczenia Windows 95. Kolorowe etykiety języków i segmentowe wskaźniki CGA dodają informacyjnych akcentów do spokojnej, ciemnej palety.

### Ikony i kolory technologii

Indeks pobiera ikony plików i folderów z powłoki Windows, więc widok pozostaje zgodny z typami i skojarzeniami znanymi z systemu. Foldery projektów otrzymują dodatkową plakietkę wykrytej technologii. W tabeli język sygnalizuje kolorowa kropka, a w nagłówku projektu pasek udziału języków — podobnie jak w podsumowaniu repozytorium na GitHubie.

Rozpoznawane są między innymi Rust, C/C++, C#, JavaScript, TypeScript, Python, HTML, CSS, Shell i MQL. Technologia wynika z plików charakterystycznych dla projektu, takich jak `Cargo.toml`, `CMakeLists.txt`, `*.csproj`, `package.json` czy `pyproject.toml`, albo z przewagi plików danego języka.

## Co można zrobić

### Znaleźć plik lub folder

Wyszukiwanie działa na zbudowanym indeksie nazw, także dla polskich znaków. Wyniki pojawiają się w folderach i plikach bez ponownego przeszukiwania dysku. Drzewo katalogów korzysta z tego samego indeksu, więc rozwijanie folderów nie wymaga osobnego skanowania systemu plików.

### Pracować z projektami

Indeks rozpoznaje foldery projektów, użyte języki i czas ostatniej zmiany w poddrzewie. Podsumowanie języków obejmuje projekty znalezione w indeksie. Kliknięcie języka filtruje listę; `Esc` zamyka filtr. Można też dodać kolejne korzenie, na przykład `D:\`, w ustawieniach.

Skan pomija między innymi `AppData`, foldery zaczynające się od kropki i podfoldery rozpoznanego projektu. Rozpoznaje również kontenery zawierające wiele projektów, takie jak katalog z wieloma paczkami.

### Odzyskać miejsce na dysku

Kliknięcie nagłówka **Projekty na dysku** otwiera panel z dwoma widokami: **Wszystkie projekty** (rozmiary i udział języków) oraz **Czyszczenie dysku** (`node_modules`, `target`, `Debug`/`Release`, `bin`/`obj`, Temp, pliki robocze Claude'a i Codexa, pamięć podręczna pakietów, logi — od najstarszych).

Pracę dzielą trzy silniki: **C++** (`indeks-katalog.exe`) raz przeszukuje dysk i zapisuje katalog, **Rust** w tle nasłuchuje tylko nazw nowych folderów i zleca przeliczenie jednego projektu, a **C#** (`indeks-panel.exe`) wyświetla gotowy katalog. Usuwanie jest trwałe, wymaga potwierdzenia, obejmuje wyłącznie pozycje z katalogu i nie wchodzi w dowiązania. Historia rozmów, pamięć, ustawienia i klucze narzędzi AI nie są skanowane jako śmieci.

### Nawigować po wielu miejscach naraz

Karta Pulpit pozostaje zawsze dostępna. Kolejne karty można dodać z menu kontekstowego folderu. Ulubione zapewniają szybki dostęp do często używanych lokalizacji. Każdy folder można otworzyć w osobnym oknie Indeksu, a otwarte aplikacje przywrócić z dolnego paska.

### Otwierać pliki po swojemu

Kliknięcie pliku otwiera go w skojarzonej aplikacji. `Ctrl`+klik, środkowy przycisk myszy lub polecenie z menu kontekstowego otwierają lokalizację w Eksploratorze. Menu prawego przycisku udostępnia polecenia powłoki Windows.

### Śledzić pobrane pliki

Indeks obserwuje folder Pobrane. Gdy pobieranie się zakończy — zniknie plik tymczasowy, a rozmiar przestanie rosnąć — może przejść do nowego pliku i go zaznaczyć. Zachowanie można wyłączyć w ustawieniach (`pobrane_otwieraj=`).

### Widzieć użycie zasobów

Pasek wydajności pokazuje użycie dysków, CPU, GPU i RAM. Dane są odświeżane co dwie sekundy, gdy pulpit jest widoczny. Osobna nakładka może wyświetlać CPU, GPU, RAM i dyski nad zmaksymalizowanym lub pełnoekranowym oknem; jej widoczność można regulować w ustawieniach.

## Jak działa

Indeks łączy kilka natywnych komponentów:

| Część | Rola |
|---|---|
| **Rust** | Aplikacja Windows, interfejs, nawigacja, konfiguracja, statystyki i obsługa okien. |
| **Direct2D + DirectWrite** | Programowe rysowanie interfejsu bez osadzonej przeglądarki. |
| **C++** | Wielowątkowy indeks nazw plików i folderów, wyszukiwanie oraz odczyt relacji między katalogami. |
| **C# / .NET Framework 4** | Opcjonalny, osobny proces nakładki monitorującej okno na wierzchu. |
| **Win32 i Windows Shell** | Integracja z pulpitem, ikonami, menu kontekstowym, Eksploratorem i uruchamianiem plików. |

Okno pulpitu Indeksu jest osadzone w warstwie pulpitu Explorera (`WorkerW`/`Progman`). Nie pojawia się na pasku zadań ani w `Alt`+`Tab`. Systemowe ikony pulpitu są ukrywane oficjalnym przełącznikiem Windows i wracają po zamknięciu aplikacji; same pliki nie są przenoszone ani modyfikowane.

Pliki są indeksowane w pamięci. Wyszukiwanie i rozwijanie drzewa korzystają z indeksu, a silnik projektów ustala technologie i daty zmian bez dodatkowego skanowania dysku. Indeks dopasowuje rozmiar okna po zmianie rozdzielczości i ponownie podpina się po restarcie Explorera.

## Skróty i obsługa

| Akcja | Sterowanie |
|---|---|
| Wejście do folderu / otwarcie pliku | Kliknięcie |
| Otworzenie w Eksploratorze | `Ctrl`+klik, środkowy przycisk, `[Otwórz Eksplorator]` lub `Ctrl`+`E` |
| Wstecz / dalej / poziom wyżej | `Backspace`, `Alt`+`←` / `Alt`+`→`, boczne przyciski myszy, `Alt`+`↑` |
| Wyszukiwanie | Zacznij pisać; `Enter` otwiera wynik, `Esc` zamyka wyszukiwanie |
| Rozwinięcie drzewa | `[+]` / `[−]`; kliknięcie nazwy przechodzi do folderu |
| Ulubione | `[+ Dodaj do ulubionych]` lub `Ctrl`+`D` |
| Zaznaczanie na liście | Strzałki, `PgUp` / `PgDn`, `Home` / `End`, `Enter` |
| Menu Windows dla pliku | Prawy przycisk myszy |
| Pokaż pliki systemowe | `Ctrl`+`H` |
| Filtr projektów według języka | Kliknij język; `Esc` zamyka filtr |
| Zamknięcie aplikacji | Prawy klik na pustym miejscu pulpitu lub ikona w zasobniku |

## Instalacja i uruchamianie

Wymagane środowisko do kompilacji: **Rust 1.88 lub nowszy** oraz **MSVC Build Tools** z kompilatorem C++.

```powershell
cargo build --release
```

Gotowy program znajduje się w `target\release`. `INSTALUJ.cmd` buduje aplikację, kopiuje ją do `%LOCALAPPDATA%\Programs\Indeks`, konfiguruje autostart i uruchamia. `START.bat` uruchamia wersję zainstalowaną lub — jeśli jej nie ma — kompilację z `target\release`.

| Plik | Działanie |
|---|---|
| `INSTALUJ.cmd` | Buduje, instaluje i uruchamia Indeks oraz włącza autostart. |
| `START.bat` | Uruchamia zainstalowaną wersję lub wersję z `target\release`. |
| `STOP.bat` | Zamyka Indeks, jego okna i proces nakładki; przywraca ikony pulpitu. |
| `PRZYWROC_PULPIT.cmd` | Zamyka Indeks, wyłącza autostart i przywraca ikony pulpitu. |

## Ustawienia i pliki

Konfiguracja znajduje się w `%APPDATA%\Indeks\indeks.cfg`, a log w `%LOCALAPPDATA%\Indeks\indeks.log`.

Plik konfiguracji obsługuje m.in. korzeń (`dom=`), przypięte lokalizacje (`przypiety=`), dodatkowe ścieżki indeksowania (`indeksuj=`), pomijane ścieżki (`pomijaj=`), opcje nakładki (`nakladka=`, `nakladka_widocznosc=`, `nakladka_dyski=`, `dyski_widocznosc=`), zachowanie okien (`klik_chowa_okna=`, `win_minimalizuje=`), pasek aplikacji (`pasek_aplikacji=`) i reakcję na pobrania (`pobrane_otwieraj=`). Ustawienia wizualne i zachowanie nakładki można też zmienić z poziomu zębatki w prawym górnym rogu.

## Polecenia

```text
indeks.exe [--folder X]              Uruchamia pulpit.
indeks.exe --okno [--folder X]       Uruchamia zwykłe okno (np. do testów).
indeks.exe --zamknij                  Zamyka Indeks i przywraca ikony pulpitu.
indeks.exe --przywroc                 Zamyka, wyłącza autostart i przywraca ikony.
indeks.exe --nowe-okno ŚCIEŻKA        Otwiera nową kartę okna Indeksu dla ścieżki.
indeks.exe --test-szukaj              Mierzy indeks i wyszukiwanie.
indeks.exe --test-nakladki            Pokazuje nakładkę przez 4 sekundy.
indeks.exe --uruchom PLIK             Testuje uruchomienie pliku przez Explorera.
indeks.exe --zrzut plik.png [opcje]   Zapisuje zrzut widoku; dostępne m.in. --folder,
                                      --szukaj, --szer, --wys, --dpi, --indeks,
                                      --rodzaj, --ustawienia, --klik-drzewo.
indeks.exe --okno --folder PLIK       Otwiera folder pliku i zaznacza plik w działającym
                                      oknie Indeksu (przez WM_COPYDATA).
```

`indeks-nakladka.exe` jest budowany przez `build.rs` przy użyciu `csc.exe` z .NET Framework 4. Uruchamia się jako osobny proces powiązany z Indeksem, a ustawienia czyta z `indeks.cfg`. Jeśli proces nakładki jest niedostępny, Indeks może narysować nakładkę samodzielnie. Tryb `--zrzut plik.png` pozwala podejrzeć jej wygląd.

## Wydajność

Pomiary projektu dla indeksu około **346 tys. plików** (10.11.2026):

- około **34–40 MB pamięci prywatnej** łącznie z indeksem;
- **22 wątki** i bliskie zeru użycie CPU w bezczynności;
- wyszukiwanie nazw w około **2–6 ms**;
- reakcja na najechanie myszą lub strzałkę około **3,5 ms**;
- pełne odmalowanie ekranu przy zmianie folderu około **60–80 ms**;
- uruchomienie pliku przez Explorera około **150–500 ms**.

Wartości zależą od sprzętu i wielkości indeksu. Nakładka okienkowa działa nad pełnoekranowymi oknami bez ramek i w trybie pełnoekranowym z optymalizacją Windows. Windows nie wyświetla innych okien nad ekskluzywnym pełnym ekranem DirectX.
