# Indeks – własny pulpit Windows

Pulpit w stylu indeksu katalogu z Chromium (Times New Roman, linki `#9e9eff`) w ciemnej,
old-schoolowej oprawie: podwójne ramki paneli z tytułem w linii (Norton Commander),
drzewo z kropkowanymi liniami i `[+]/[−]` (Windows 98), granatowe zaznaczenie (pulpit
Win95), segmentowe paski w kolorach CGA. Bez Electrona i bez przeglądarki.

```
┌ Ulubione: [Desktop] [huber] [C:\] ─────────── [+ Dodaj do ulubionych] [Otwórz Eksplorator] ┐
├─ Struktura ─────┬─ Desktop  (C:\Users\huber\Desktop\ · 143 foldery, 177 plików) ─────────┤
│ [+] C:\         ├─ Foldery (143) ──────┬─ Pliki (177) ────────────────────────────────────┤
│  └[+] Users     │ $$$/                 │  [ikona] [ikona] [ikona] ...                     │
│     └[+] huber  │ 0.2 miner/           │   nazwa   nazwa   nazwa                          │
│        └ Desktop│ ...                  │                                                  │
├─────────────────┴──────────────────────┴──────────────────────────────────────────────────┤
│ C: ■■■■■■■■■■□□ 61% 412 GB wolne │ CPU ■■□□□ 7% │ RAM ■■■■■□□ 58% │      Indeks 30 MB    │
└───────────────────────────────────────────────────────────────────────────────────────────┘
```

## Jak to działa

- **Rust + Win32**, rysowanie **Direct2D/DirectWrite programowo** (bez sterownika GPU w procesie),
  pasami po 256 px prosto do okna; odmalowywane są tylko zmienione elementy, przewijanie
  przesuwa piksele panelu (`ScrollWindowEx`).
- Okno Indeksu jest **dzieckiem okna pulpitu Explorera** (`WorkerW`/`Progman` z
  `SHELLDLL_DefView`) i leży nad ikonami: Win+D, „Pokaż pulpit” i menu Start go nie chowają,
  nie ma go na pasku zadań ani w Alt+Tab. Udostępnianie **ekranu** (np. Discord) pokazuje go
  jak zwykły pulpit.
- **Programy startują przez samego Explorera** (`IShellDispatch2::ShellExecute` obiektu pulpitu):
  rodzicem jest `explorer.exe`, więc okna otwierają się w tym samym rozmiarze i miejscu co ze
  zwykłego pulpitu.
- Ikony pulpitu Windows są wyłączane oficjalnym przełącznikiem („Widok → Pokaż ikony pulpitu”).
  **Pliki na pulpicie nie są ruszane**. Po zamknięciu ikony wracają do stanu sprzed startu.
- Zmiana rozdzielczości / restart Explorera: Indeks sam się podpina i dopasowuje.
- **C++** (`cpp/search.cpp`): indeks nazw w pamięci (~14 MB / 345 tys. plików), wyszukiwanie
  wielowątkowe (2–6 ms, także polskie znaki), oraz **drzewo z lewej** – podfoldery rozwijane prosto
  z indeksu (`ix_lookup`, `ix_child_dirs`), bez dotykania dysku. W trybie szukania lewy panel
  pokazuje pasujące foldery, środek pliki.
- Pasek stanu (dyski, CPU, RAM, pamięć Indeksu) liczony co 2 s **tylko gdy pulpit jest na wierzchu**.
- **Silnik projektów** (`src/profiles.rs` + `ix_profile_dir` w C++): w tle, z indeksu w pamięci (bez dysku),
  rozpoznaje technologię każdego folderu (znaczniki `package.json`, `Cargo.toml`, `CMakeLists.txt`,
  `*.csproj`, `pyproject.toml`… albo przewaga plików kodu) i datę ostatniej zmiany w całym poddrzewie.
  Lista folderów: ikona folderu + plakietka technologii + nazwa + „3 dni temu”; nagłówek folderu:
  pasek języków jak na GitHubie, technologia, repo git, liczba plików.
- Górny pasek: dyski → `[Otwórz Eksplorator]` → `[Pobrane]` → ulubione; pod nim **karty**: pierwsza
  zawsze Desktop (stała, bez `×`), dalej własne (prawy klik na folderze → „Dodaj jako kartę”,
  `×` zamyka, zapisane w `karta=` w ustawieniach).
- **Okna Indeksu**: dwuklik na folderze (drzewo albo lista) otwiera go w osobnym oknie w naszym
  stylu (zwykłe okno: minimalizuj / maksymalizuj / zamknij, tytuł „folder - Indeks”), a pulpit
  wraca tam, gdzie był przed pierwszym kliknięciem. To samo: prawy klik → „Otwórz w nowym oknie
  Indeksu”. Każde okno to wątek w procesie pulpitu — wspólny indeks i silnik projektów, ~12 MB na
  okno; zamknięcie okna kończy tylko jego wątek. Okna widać w rzędzie aplikacji na dole.
- Drzewo: klik w folder **nie przebudowuje drzewa** — drzewo zostaje w miejscu (ta sama struktura
  i przewinięcie), wybrany folder świeci na biało pogrubiony, zmieniają się tylko lista folderów
  i pliki. Drzewo przebudowuje się dopiero przy przejściu do folderu, którego w nim nie widać.
- Lista folderów jak tabela: folder, nazwa, „2 lata temu” (stała kolumna), `● Język` (kolorowa
  kropka jak na GitHubie, stała kolumna z prawej; ciemne kolory GitHuba rozjaśnione).
- Dolny pasek z prawej: zużycie samego Indeksu — RAM (pamięć prywatna) i CPU procesu — oraz liczba
  plików w indeksie.
- **Pobrane**: Indeks obserwuje folder Pobrane (tylko odczyt zmian, nic nie zmienia w systemie).
  Nowy kompletny plik z dowolnego źródła (przeglądarka, Discord, Telegram, torrent…) — po zniknięciu
  pliku tymczasowego `.crdownload`/`.part`/`.tmp` i gdy rozmiar przestanie rosnąć — pulpit przechodzi do
  Pobranych z tym plikiem zaznaczonym; gdy pulpit jest zakryty i nic nie gra na pełnym ekranie,
  otwiera się okno Indeksu (albo istniejące przechodzi do pliku) i wychodzi na wierzch.
  Wyłączysz w ustawieniach (`pobrane_otwieraj=`).
- **Projekty na dysku** (pod drzewem i listą): najpierw postęp skanu %, potem liczba projektów
  wg języka z całego indeksu (profil + przypięte; dysk D: dodasz przez `indeksuj=D:\`).
  Klik w język = lista tych projektów w środkowym panelu; Esc / `[Zamknij filtr]` wraca.
  Nie liczy: korzenia indeksu, AppData i folderów z kropką, podfolderów projektu (do 3 poziomów).
  Folder z ≥4 projektami w środku (np. Pulpit z package.json) to kontener, nie projekt;
  cały `…MQL5` to jeden projekt MQL; ≥20 projektów jednego rodzaju w folderze (vcpkg/ports) = jeden.
- Dół: **przyciski otwartych aplikacji** (jak Alt+Tab, także zminimalizowane — klik przywraca okno),
  a pod nimi na samym dole pasek wydajności: dyski, CPU, **GPU** (PDH „GPU Engine 3D”), RAM.
- **Klik w puste miejsce pulpitu chowa wszystkie okna** (jak „Pokaż pulpit”, opcja).
- **Nakładka nad pełnym ekranem**: gdy okno na wierzchu zajmuje cały monitor (gra, film), w lewym
  dolnym rogu półprzezroczyste CPU/GPU/RAM (domyślnie 40%) i osobno dyski (15%). Okno warstwowe,
  przezroczyste dla kliknięć (WS_EX_TRANSPARENT + HTTRANSPARENT), nigdy się nie aktywuje — klik
  trafia do gry. Działa w grach bez ramki i w „pełnym ekranie” z optymalizacją Windows; nad
  ekskluzywnym pełnym ekranem (DirectX exclusive) Windows nie pokazuje żadnych innych okien.
- **Silnik nakładki w C#** (`csharp/Nakladka.cs` → `indeks-nakladka.exe`, kompiluje go build.rs przez
  `csc.exe` z .NET Framework 4): osobny proces uruchamiany przez pulpit z `--rodzic PID` (kończy się
  razem z Indeksem), co 0,5 s i przy każdej zmianie okna na wierzchu sprawdza pełny ekran **albo okno
  zmaksymalizowane** (`nakladka_maks=`), wraca na wierzch warstwy TOPMOST, ustawienia czyta z indeks.cfg.
  ~25 MB. Gdy go nie ma albo padnie, nakładkę rysuje jak dawniej Rust. Podgląd: `indeks-nakladka.exe --zrzut plik.png`.
- Drzewo pamięta wszystkie odwiedzone gałęzie (karty, ulubione, projekty na dysku, wstecz) — nic się
  nie zwija, przewija się tylko tyle, żeby było widać bieżący folder.
- Przyciski otwartych aplikacji zawijają się do kolejnych rzędów (maks. 3).
- **Zębatka** w prawym górnym rogu: ustawienia (nakładka, suwaki widoczności, dyski, klik chowa
  okna, klawisz Windows minimalizuje okno pełnoekranowe — domyślnie wyłączone, przyciski aplikacji,
  autostart).

## Obsługa

| Akcja | Jak |
|---|---|
| Wejście do folderu / otwarcie pliku | klik |
| Folder / plik w Eksploratorze | Ctrl+klik, środkowy przycisk, `[Otwórz Eksplorator]`, Ctrl+E |
| Wstecz / dalej / wyżej | Backspace, Alt+←/→, boczne przyciski myszy, Alt+↑ |
| Drzewo | klik w nazwę = przejdź, `[+]/[−]` = rozwiń wszystkie podfoldery |
| Szukanie | po prostu pisz; Esc zamyka, Enter otwiera |
| Ulubione | `[+ Dodaj do ulubionych]`, Ctrl+D, prawy klik na ulubionym = usuń/przesuń |
| Zaznaczanie | strzałki (lista folderów ↔ siatka plików), PgUp/PgDn, Home/End, Enter |
| Menu Windows dla pliku | prawy klik (pełne menu powłoki) |
| Pliki systemowe | Ctrl+H |
| Schowanie wszystkich okien | klik w puste miejsce pulpitu |
| Ustawienia | zębatka w prawym górnym rogu; Esc zamyka |
| Projekty wg języka | klik w język w „Projekty na dysku”; Esc zamyka filtr |
| Przywrócenie okna | klik w jego przycisk na samym dole |
| Zamknięcie | prawy klik w puste miejsce albo ikona w zasobniku |

## Pliki

- `INSTALUJ.cmd` – buduje, kopiuje do `%LOCALAPPDATA%\Programs\Indeks`, włącza autostart, uruchamia.
- `PRZYWROC_PULPIT.cmd` – zamyka Indeks, wyłącza autostart, włącza ikony pulpitu.
- Ustawienia: `%APPDATA%\Indeks\indeks.cfg` (`dom=`, `przypiety=`, `indeksuj=`, `pomijaj=`,
  `nakladka=`, `nakladka_widocznosc=`, `nakladka_dyski=`, `dyski_widocznosc=`, `klik_chowa_okna=`,
  `win_minimalizuje=`, `pasek_aplikacji=`, `pobrane_otwieraj=`).
- Log: `%LOCALAPPDATA%\Indeks\indeks.log`.

## Polecenia

```
indeks.exe [--folder X]          pulpit
indeks.exe --okno [--folder X]   w zwykłym oknie (testy)
indeks.exe --zamknij             zamknij (przywraca ikony)
indeks.exe --przywroc            zamknij + wyłącz autostart + pokaż ikony
indeks.exe --zrzut plik.png [--folder X] [--szukaj Q] [--szer W] [--wys H] [--dpi D]
             [--indeks] [--rodzaj N] [--ustawienia] [--klik-drzewo N]   (N = numer języka / wiersza)
indeks.exe --okno --folder PLIK  okno z folderem pliku i zaznaczonym plikiem (działające okno dostaje go przez WM_COPYDATA)
indeks.exe --nowe-okno ŚCIEŻKA   nowe okno Indeksu w działającym pulpicie (np. do skrótu); bez pulpitu - osobny proces
indeks.exe --test-nakladki       pokazuje nakładkę na 4 s (bez aktywacji, do sprawdzenia wyglądu)
indeks.exe --uruchom PLIK        test uruchamiania przez Explorera (log kroków)
indeks.exe --test-szukaj         pomiar indeksu i wyszukiwania
```

## Zmierzone (10.11.2026, profil ~346 tys. plików)

- pamięć prywatna ~34–40 MB łącznie z indeksem, 22 wątki, w bezczynności 0% CPU,
- najechanie myszą ~3,5 ms, strzałki ~3,5 ms, pełne odmalowanie ekranu 60–80 ms (tylko przy zmianie folderu),
- uruchomienie pliku przez Explorera ~150–500 ms (tyle co dwuklik na zwykłym pulpicie).

## Budowanie

`cargo build --release` (Rust 1.88+, MSVC Build Tools – kompilują też C++).
