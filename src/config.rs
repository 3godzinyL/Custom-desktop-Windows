// Ustawienia w %APPDATA%\Indeks\indeks.cfg (zwykly tekst klucz=wartosc,
// mozna edytowac recznie). Zapis atomowy: plik .tmp + zamiana.
use std::path::PathBuf;

use crate::util;

#[derive(Clone, Debug)]
pub struct Config {
    /// Folder startowy (domyslnie prawdziwy Pulpit uzytkownika).
    pub home: String,
    /// Foldery przypiete u gory (niebieskie linki).
    pub pins: Vec<String>,
    /// Otwarte karty (dwuklik na folderze w drzewie).
    pub tabs: Vec<String>,
    /// Korzenie indeksu wyszukiwania.
    pub roots: Vec<String>,
    /// Nazwy folderow pomijanych przy indeksowaniu (bez wielkosci liter).
    pub skip: Vec<String>,
    /// Pokazuj pliki ukryte+systemowe (np. desktop.ini).
    pub show_protected: bool,
    /// Stan ikon pulpitu Windows sprzed uruchomienia Indeksu (do przywrocenia).
    pub icons_were_hidden: Option<bool>,
    /// Nakladka CPU/GPU/RAM nad oknem na pelnym ekranie i jej widocznosc (%).
    pub overlay: bool,
    pub overlay_alpha: u8,
    /// Dyski na nakladce i ich osobna widocznosc (%).
    pub overlay_disks: bool,
    pub disks_alpha: u8,
    /// Klik w puste miejsce pulpitu chowa wszystkie okna.
    pub click_hides: bool,
    /// Klawisz Windows minimalizuje okno na pelnym ekranie (np. gre bez ramki).
    pub win_minimizes: bool,
    /// Rzad przyciskow otwartych aplikacji pod paskiem wydajnosci.
    pub apps_row: bool,
    /// Nowy plik w Pobranych otwiera sie w Indeksie (pulpit + okno, gdy pulpit zakryty).
    pub downloads_open: bool,
    /// Nakladka takze nad zmaksymalizowanym oknem (nie tylko pelny ekran).
    pub overlay_maximized: bool,
    /// Nakladke rysuje silnik C# (indeks-nakladka.exe); false = nakladka w Rust.
    pub overlay_cs: bool,
    /// Liczba plikow z poprzedniego indeksowania (do procentu postepu).
    pub last_index: u32,
    /// Ograniczanie zasobow bezczynnych aplikacji (EcoQoS). Domyslnie wylaczone.
    pub optimize: bool,
}

pub const DEFAULT_SKIP: &[&str] = &[
    "node_modules", "AppData", "target", "__pycache__", "venv", "$Recycle.Bin",
    "System Volume Information", "bower_components", "obj",
];

impl Config {
    pub fn path() -> PathBuf {
        util::config_dir().join("indeks.cfg")
    }

    pub fn default_home() -> String {
        util::known_folder(&windows::Win32::UI::Shell::FOLDERID_Desktop)
            .unwrap_or_else(|| "C:\\".to_string())
    }

    pub fn load() -> Config {
        let mut c = Config {
            home: Self::default_home(),
            pins: Vec::new(),
            tabs: Vec::new(),
            roots: Vec::new(),
            skip: Vec::new(),
            show_protected: false,
            icons_were_hidden: None,
            overlay: true,
            overlay_alpha: 40,
            overlay_disks: true,
            disks_alpha: 15,
            click_hides: true,
            win_minimizes: false,
            apps_row: true,
            downloads_open: true,
            overlay_maximized: true,
            overlay_cs: true,
            last_index: 0,
            optimize: false,
        };
        let mut had_file = false;
        let mut skip_set = false;
        if let Ok(text) = std::fs::read_to_string(Self::path()) {
            had_file = true;
            for (k, v) in util::kv_lines(&text) {
                match k {
                    "dom" if !v.is_empty() => c.home = v.to_string(),
                    "przypiety" if !v.is_empty() => c.pins.push(v.to_string()),
                    "karta" if !v.is_empty() => c.tabs.push(v.to_string()),
                    "indeksuj" if !v.is_empty() => c.roots.push(v.to_string()),
                    "pomijaj" => {
                        skip_set = true;
                        if !v.is_empty() {
                            c.skip.push(v.to_string());
                        }
                    }
                    "pokaz_systemowe" => c.show_protected = v == "1",
                    "ikony_przed" => c.icons_were_hidden = Some(v == "1"),
                    "nakladka" => c.overlay = v == "1",
                    "nakladka_widocznosc" => c.overlay_alpha = v.parse().unwrap_or(40),
                    "nakladka_dyski" => c.overlay_disks = v == "1",
                    "dyski_widocznosc" => c.disks_alpha = v.parse().unwrap_or(15),
                    "klik_chowa_okna" => c.click_hides = v == "1",
                    "win_minimalizuje" => c.win_minimizes = v == "1",
                    "pasek_aplikacji" => c.apps_row = v == "1",
                    "pobrane_otwieraj" => c.downloads_open = v == "1",
                    "nakladka_maks" => c.overlay_maximized = v == "1",
                    "nakladka_cs" => c.overlay_cs = v == "1",
                    "ostatni_indeks" => c.last_index = v.parse().unwrap_or(0),
                    "optymalizuj" => c.optimize = v == "1",
                    _ => {}
                }
            }
        }
        if !skip_set {
            c.skip = DEFAULT_SKIP.iter().map(|s| s.to_string()).collect();
        }
        if c.roots.is_empty() {
            if let Ok(profile) = std::env::var("USERPROFILE") {
                c.roots.push(profile);
            }
        }
        if !had_file {
            c.pins.push(c.home.clone());
            if let Ok(profile) = std::env::var("USERPROFILE") {
                c.pins.push(profile);
            }
            c.pins.push("C:\\".to_string());
            c.save();
        }
        c
    }

    pub fn save(&self) {
        let mut s = String::new();
        s.push_str("# Indeks - ustawienia. Mozna edytowac recznie (Indeks musi byc zamkniety).\n");
        s.push_str("# dom = folder startowy; przypiety = link u gory; indeksuj = korzen wyszukiwania;\n");
        s.push_str("# pomijaj = nazwa folderu pomijana w indeksie (foldery zaczynajace sie od kropki sa pomijane zawsze).\n");
        s.push_str(&format!("dom={}\n", self.home));
        for p in &self.pins {
            s.push_str(&format!("przypiety={}\n", p));
        }
        for k in &self.tabs {
            s.push_str(&format!("karta={}\n", k));
        }
        for r in &self.roots {
            s.push_str(&format!("indeksuj={}\n", r));
        }
        if self.skip.is_empty() {
            s.push_str("pomijaj=\n");
        }
        for k in &self.skip {
            s.push_str(&format!("pomijaj={}\n", k));
        }
        s.push_str(&format!("pokaz_systemowe={}\n", self.show_protected as u8));
        s.push_str(&format!("nakladka_maks={}\nnakladka_cs={}\n", self.overlay_maximized as u8, self.overlay_cs as u8));
        s.push_str(&format!("nakladka={}\nnakladka_widocznosc={}\nnakladka_dyski={}\ndyski_widocznosc={}\n", self.overlay as u8, self.overlay_alpha, self.overlay_disks as u8, self.disks_alpha));
        s.push_str(&format!("klik_chowa_okna={}\nwin_minimalizuje={}\npasek_aplikacji={}\npobrane_otwieraj={}\noptymalizuj={}\nostatni_indeks={}\n", self.click_hides as u8, self.win_minimizes as u8, self.apps_row as u8, self.downloads_open as u8, self.optimize as u8, self.last_index));
        if let Some(h) = self.icons_were_hidden {
            s.push_str(&format!("ikony_przed={}\n", h as u8));
        }
        let path = Self::path();
        let tmp = path.with_extension("cfg.tmp");
        if std::fs::write(&tmp, s.as_bytes()).is_ok() {
            if std::fs::rename(&tmp, &path).is_err() {
                let _ = std::fs::copy(&tmp, &path);
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    pub fn is_pinned(&self, path: &str) -> bool {
        self.pins.iter().any(|p| same_path(p, path))
    }

    pub fn toggle_pin(&mut self, path: &str) {
        if self.is_pinned(path) {
            self.pins.retain(|p| !same_path(p, path));
        } else {
            self.pins.push(path.to_string());
        }
        self.save();
    }
}

pub fn same_path(a: &str, b: &str) -> bool {
    let a = a.trim_end_matches('\\');
    let b = b.trim_end_matches('\\');
    a.eq_ignore_ascii_case(b) || util::lower(a) == util::lower(b)
}

/// Pulpit uzytkownika (pierwsza, stala karta).
pub fn desktop_dir() -> String {
    Config::default_home()
}

/// Folder Pobrane (link obok [Otworz Eksplorator], obserwowanie nowych plikow).
pub fn downloads_dir() -> Option<String> {
    util::known_folder(&windows::Win32::UI::Shell::FOLDERID_Downloads)
}
