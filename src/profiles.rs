// Silnik projektow: w tle rozpoznaje, czym jest folder (Node, Rust, C++, Python...),
// kiedy byl ostatnio zmieniany i z jakich jezykow sie sklada.
//
// Dane liczy C++ (ix_profile_dir) prosto z indeksu w pamieci - bez czytania dysku,
// wiec nawet duze repo to milisekundy. Watek w tle, wynik w pamieci podrecznej,
// UI dostaje komunikat i odmalowuje liste.
use std::collections::HashMap;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{IsWindow, PostMessageW, WM_APP};

use crate::search::{self, IxProfile};
use crate::util;

pub const WM_APP_PROFILE: u32 = WM_APP + 23;

pub struct Lang {
    pub name: &'static str,
    /// Kolor jak na pasku jezykow GitHuba.
    pub color: u32,
    /// Jezyk programowania (a nie znaczniki/skrypty).
    /// Rozszerzenie plakietki (icons::badge_for_ext).
    pub ext: &'static str,
    pub code: bool,
}

const fn l(name: &'static str, color: u32, ext: &'static str, code: bool) -> Lang {
    Lang { name, color, ext, code }
}

/// Kolejnosc = identyfikatory z cpp/search.cpp (EXT_LANGS). 31 = Node.js (z package.json).
pub static LANGS: [Lang; 32] = [
    l("JavaScript", 0xF1E05A, "js", true),
    l("TypeScript", 0x3178C6, "ts", true),
    l("Rust", 0xDEA584, "rs", true),
    l("C++", 0xF34B7D, "cpp", true),
    l("C", 0x555555, "c", true),
    l("C#", 0x178600, "cs", true),
    l("Python", 0x3572A5, "py", true),
    l("Go", 0x00ADD8, "go", true),
    l("Java", 0xB07219, "java", true),
    l("Kotlin", 0xA97BFF, "kt", true),
    l("PHP", 0x4F5D95, "php", true),
    l("Ruby", 0x701516, "rb", true),
    l("HTML", 0xE34C26, "html", false),
    l("CSS", 0x563D7C, "css", false),
    l("Shell", 0x89E051, "sh", false),
    l("Lua", 0x000080, "lua", true),
    l("Dart", 0x00B4AB, "dart", true),
    l("Swift", 0xF05138, "swift", true),
    l("MQL", 0x4A90D9, "mq5", true),
    l("Pine Script", 0x2962FF, "pine", true),
    l("Vue", 0x41B883, "vue", true),
    l("Svelte", 0xFF3E00, "svelte", true),
    l("SQL", 0xE38C00, "sql", false),
    l("Assembly", 0x6E4C13, "asm", true),
    l("Zig", 0xEC915C, "zig", true),
    l("R", 0x198CE7, "r", true),
    l("Scala", 0xC22D40, "scala", true),
    l("Haskell", 0x5E5086, "hs", true),
    l("Julia", 0xA270BA, "jl", true),
    l("WebAssembly", 0x04133B, "wasm", true),
    l("Shadery", 0x5686A5, "glsl", true),
    l("Node.js", 0x339933, "node", true),
];

const NODE: usize = 31;

// znaczniki z cpp/search.cpp (marker_of)
const M_PACKAGE: u32 = 1 << 0;
const M_CARGO: u32 = 1 << 1;
const M_CMAKE: u32 = 1 << 2;
const M_SLN: u32 = 1 << 3;
const M_VCXPROJ: u32 = 1 << 4;
const M_CSPROJ: u32 = 1 << 5;
const M_PYTHON: u32 = 1 << 6;
const M_GO: u32 = 1 << 7;
const M_JAVA: u32 = 1 << 8;
const M_PHP: u32 = 1 << 9;
const M_RUBY: u32 = 1 << 10;
const M_DART: u32 = 1 << 11;
const M_GIT: u32 = 1 << 12;
const M_TSCONFIG: u32 = 1 << 13;
const M_HTML: u32 = 1 << 14;

pub struct Profile {
    /// Glowna technologia (indeks w LANGS).
    pub tech: Option<usize>,
    /// Jezyki od najczestszego: (indeks w LANGS, liczba plikow).
    pub langs: Vec<(usize, u32)>,
    /// Najnowszy plik w poddrzewie (minuty od 1601), 0 = brak plikow.
    pub newest: u32,
    pub files: u32,
    pub git: bool,
}

impl Profile {
    pub fn tech_name(&self) -> Option<&'static str> {
        self.tech.map(|t| LANGS[t].name)
    }
}

fn detect(p: &IxProfile) -> Profile {
    let m = p.markers;
    let n = |i: usize| p.lang[i];
    let by_markers = if m & M_CARGO != 0 {
        Some(2)
    } else if m & M_PACKAGE != 0 {
        Some(if m & M_TSCONFIG != 0 || n(1) > n(0) { 1 } else { NODE })
    } else if m & M_CSPROJ != 0 || (m & M_SLN != 0 && m & (M_VCXPROJ | M_CMAKE) == 0 && n(5) >= n(3) + n(4)) {
        Some(5)
    } else if m & (M_CMAKE | M_VCXPROJ | M_SLN) != 0 {
        Some(if n(4) > n(3) { 4 } else { 3 })
    } else if m & M_PYTHON != 0 {
        Some(6)
    } else if m & M_GO != 0 {
        Some(7)
    } else if m & M_JAVA != 0 {
        Some(if n(9) > n(8) { 9 } else { 8 })
    } else if m & M_PHP != 0 {
        Some(10)
    } else if m & M_RUBY != 0 {
        Some(11)
    } else if m & M_DART != 0 {
        Some(16)
    } else {
        None
    };
    // bez znacznikow: najczestszy jezyk programowania, a jak go brak - HTML/skrypty
    let tech = by_markers.or_else(|| {
        let best_code = (0..31).filter(|&i| LANGS[i].code && n(i) >= 2).max_by_key(|&i| n(i));
        best_code.or_else(|| {
            if m & M_HTML != 0 {
                Some(12)
            } else {
                (0..31).filter(|&i| n(i) >= 1).max_by_key(|&i| n(i))
            }
        })
    });
    let mut langs: Vec<(usize, u32)> = (0..31).filter(|&i| n(i) > 0).map(|i| (i, n(i))).collect();
    langs.sort_by(|a, b| b.1.cmp(&a.1));
    Profile { tech, langs, newest: p.newest, files: p.files, git: m & M_GIT != 0 }
}

enum Slot {
    Pending,
    Done(Option<Arc<Profile>>, Instant),
}

struct Engine {
    cache: Mutex<HashMap<String, Slot>>,
    tx: Mutex<Sender<String>>,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// Okna kontrolerow (pulpit + dodatkowe okna Indeksu), ktore dostaja WM_APP_PROFILE.
static NOTIFY: Mutex<Vec<isize>> = Mutex::new(Vec::new());

/// Dodatkowe okno Indeksu tez dostaje gotowe profile i wynik skanu projektow.
pub fn listen(notify: HWND) {
    let mut v = NOTIFY.lock().unwrap();
    v.retain(|&h| unsafe { IsWindow(Some(HWND(h as *mut _))).as_bool() });
    v.push(notify.0 as isize);
}

fn post_all(wp: usize) {
    for &h in NOTIFY.lock().unwrap().iter() {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_APP_PROFILE, WPARAM(wp), LPARAM(0));
        }
    }
}

pub fn start(notify: HWND) {
    listen(notify);
    let (tx, rx) = channel::<String>();
    if ENGINE.set(Engine { cache: Mutex::new(HashMap::new()), tx: Mutex::new(tx) }).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("indeks-projekty".into())
        .spawn(move || {
            let e = ENGINE.get().unwrap();
            let post = || post_all(0);
            let mut last_post = Instant::now();
            let mut dirty = false;
            loop {
                let key = match rx.recv_timeout(Duration::from_millis(120)) {
                    Ok(k) => k,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if dirty {
                            post();
                            dirty = false;
                        }
                        continue;
                    }
                    Err(_) => break,
                };
                // do konca indeksowania nie ma czego liczyc
                while !search::index_ready() {
                    std::thread::sleep(Duration::from_millis(250));
                }
                let prof = search::profile(&key).map(|p| Arc::new(detect(&p)));
                e.cache.lock().unwrap().insert(key, Slot::Done(prof, Instant::now()));
                dirty = true;
                if last_post.elapsed() > Duration::from_millis(150) {
                    post();
                    dirty = false;
                    last_post = Instant::now();
                }
            }
        })
        .ok();
}

/// Zamawia profile folderow (pomija te juz policzone niedawno albo w kolejce).
pub fn request(paths: &[String]) {
    let Some(e) = ENGINE.get() else { return };
    let mut cache = e.cache.lock().unwrap();
    let tx = e.tx.lock().unwrap();
    for p in paths {
        let key = util::lower(p.trim_end_matches('\\'));
        let fresh = match cache.get(&key) {
            Some(Slot::Pending) => true,
            Some(Slot::Done(_, at)) => at.elapsed() < Duration::from_secs(120),
            None => false,
        };
        if !fresh {
            cache.insert(key.clone(), Slot::Pending);
            let _ = tx.send(key);
        }
    }
}

/// Some(None) = folder poza indeksem / bez danych; None = jeszcze liczone.
pub fn get(path: &str) -> Option<Option<Arc<Profile>>> {
    let e = ENGINE.get()?;
    let key = util::lower(path.trim_end_matches('\\'));
    match e.cache.lock().unwrap().get(&key) {
        Some(Slot::Done(p, _)) => Some(p.clone()),
        _ => None,
    }
}

/// "3 dni temu" itp. (minuty od 1601).
pub fn ago(min: u32) -> String {
    if min == 0 {
        return String::new();
    }
    let d = search::now_minutes().saturating_sub(min);
    let days = d / 1440;
    match d {
        0..=1 => "przed chwilą".into(),
        2..=59 => format!("{} min temu", d),
        60..=1439 => format!("{} godz. temu", d / 60),
        _ if days == 1 => "wczoraj".into(),
        _ if days < 30 => format!("{} dni temu", days),
        _ if days < 365 => format!("{} mies. temu", days / 30),
        _ => {
            let y = days / 365;
            let w = if y == 1 {
                "rok"
            } else if (2..=4).contains(&(y % 10)) && !(12..=14).contains(&(y % 100)) {
                "lata"
            } else {
                "lat"
            };
            format!("{} {} temu", y, w)
        }
    }
}

/// Krotka nazwa jezyka na etykiecie przy folderze.
/// Ikona technologii na liscie "Projekty i foldery": plakietka jak przy plikach
/// (JS, PY, RS, C#...), Node.js - zielona NJS, reszta - kod w kolorze jezyka.
pub fn icon(lang: usize) -> crate::icons::IconKind {
    use crate::icons::{Badge, IconKind};
    let l = &LANGS[lang];
    if lang == 31 {
        return IconKind::Badge(Badge { label: "NJS", bg: 0x339933, fg: 0xFFFFFF });
    }
    match crate::icons::badge_for_ext(l.ext) {
        Some(b) => IconKind::Badge(b),
        None => IconKind::Badge(Badge { label: "</>", bg: l.color, fg: 0xFFFFFF }),
    }
}

pub fn short(lang: usize) -> &'static str {
    match lang {
        0 => "JavaScript",
        19 => "Pine",
        29 => "WASM",
        30 => "Shadery",
        23 => "ASM",
        _ => LANGS[lang].name,
    }
}

// ---------- projekty na calym dysku (panel pod drzewem) ----------

struct Scan {
    at: Option<Instant>,
    running: bool,
    /// (jezyk, sciezki projektow) od najliczniejszego.
    groups: Vec<(usize, Vec<String>)>,
}

static SCAN: Mutex<Scan> = Mutex::new(Scan { at: None, running: false, groups: Vec::new() });

/// Uruchamia skan w tle, jesli jeszcze go nie bylo albo ma ponad 10 minut.
pub fn scan_projects() {
    {
        let mut s = SCAN.lock().unwrap();
        let stale = s.at.map(|t| t.elapsed() > Duration::from_secs(600)).unwrap_or(true);
        if s.running || !stale {
            return;
        }
        s.running = true;
    }
    std::thread::Builder::new()
        .name("indeks-skan-projektow".into())
        .spawn(|| {
            while !search::index_ready() {
                std::thread::sleep(Duration::from_millis(300));
            }
            let t0 = Instant::now();
            let found = search::find_projects();
            let mut groups: Vec<(usize, Vec<String>)> = Vec::new();
            for (path, kind) in found {
                match groups.iter_mut().find(|g| g.0 == kind) {
                    Some(g) => g.1.push(path),
                    None => groups.push((kind, vec![path])),
                }
            }
            for g in groups.iter_mut() {
                g.1.sort_by(|a, b| util::cmp_logical(&crate::fsx::last_name(a), &crate::fsx::last_name(b)));
            }
            groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
            let total: usize = groups.iter().map(|g| g.1.len()).sum();
            util::log(&format!("projekty: {} na dysku w {} ms", total, t0.elapsed().as_millis()));
            {
                let mut s = SCAN.lock().unwrap();
                s.groups = groups;
                s.at = Some(Instant::now());
                s.running = false;
            }
            post_all(1);
        })
        .ok();
}

/// Nowy katalog z silnika C++: okna odswiezaja panel projektow.
pub fn notify_catalog() {
    post_all(1);
}

/// None = skan jeszcze trwa; Some = (jezyk, liczba projektow) od najliczniejszego.
/// Katalog z silnika C++ ma pierwszenstwo; szybki skan indeksu dziala do jego powstania.
pub fn project_counts() -> Option<Vec<(usize, usize)>> {
    if let Some(list) = crate::katalog::projects() {
        let mut counts: Vec<(usize, usize)> = Vec::new();
        for p in &list {
            match counts.iter_mut().find(|c| c.0 == p.kind) {
                Some(c) => c.1 += 1,
                None => counts.push((p.kind, 1)),
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1));
        return Some(counts);
    }
    let s = SCAN.lock().unwrap();
    s.at.map(|_| s.groups.iter().map(|g| (g.0, g.1.len())).collect())
}

pub fn projects_of(kind: usize) -> Vec<String> {
    if let Some(list) = crate::katalog::projects() {
        let mut v: Vec<String> = list.into_iter().filter(|p| p.kind == kind).map(|p| p.path).collect();
        v.sort_by(|a, b| util::cmp_logical(&crate::fsx::last_name(a), &crate::fsx::last_name(b)));
        return v;
    }
    let s = SCAN.lock().unwrap();
    s.groups.iter().find(|g| g.0 == kind).map(|g| g.1.clone()).unwrap_or_default()
}