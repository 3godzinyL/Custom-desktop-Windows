// Katalog projektow i smieci. Podzial pracy:
//   C++  (indeks-katalog.exe) - raz przeszukuje caly dysk i zapisuje katalog.tsv
//   Rust (ten plik)           - w tle slucha tylko nazw nowych folderow/plikow i zleca
//                               przeliczenie pojedynczego projektu
//   C#   (indeks-panel.exe)   - wyswietla gotowy katalog
use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetDriveTypeW, GetLogicalDriveStringsW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, IsWindow, MoveWindow, SetParent, SetWindowLongPtrW, ShowWindow, GWL_STYLE, SW_HIDE, SW_SHOWNA, WS_CHILD, WS_CLIPSIBLINGS, WS_POPUP,
};

use crate::util::{self, pcwstr, wide};

/// WM_COPYDATA od panelu do Rusta ("skanuj", "wczytaj").
pub const COPY_PANEL: usize = 0x494E445A;
/// WM_COPYDATA od Rusta do panelu.
const COPY_TO_PANEL: usize = 0x50414E4C;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Pelny skan odswieza sie sam, gdy katalog jest starszy (Temp i pamiec podreczna rosna po cichu).
const FULL_SCAN_DAYS: u64 = 7;
/// Ile czekac po ostatniej zmianie w folderze, zanim policzymy go od nowa (npm install trwa).
const QUIET: Duration = Duration::from_secs(25);

#[derive(Clone)]
pub struct Project {
    pub path: String,
    pub kind: usize,
    pub bytes: u64,
    pub newest: u32,
}

static PROJECTS: Mutex<Option<Vec<Project>>> = Mutex::new(None);
static SCANNING: AtomicBool = AtomicBool::new(false);
static CTL: AtomicIsize = AtomicIsize::new(0);
static PANEL_OWNER: AtomicIsize = AtomicIsize::new(0);
static PANEL_CTL: AtomicIsize = AtomicIsize::new(0);
static PANEL: AtomicIsize = AtomicIsize::new(0);
static PANEL_START: Mutex<Option<Instant>> = Mutex::new(None);
static PENDING: Mutex<Option<(HashSet<String>, Instant)>> = Mutex::new(None);

fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Indeks")
}

fn catalog_file() -> PathBuf {
    data_dir().join("katalog.tsv")
}

fn sibling(name: &str) -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.with_file_name(name);
    p.exists().then_some(p)
}

/// Wczytuje projekty z katalogu (rekordy P). Reszte czyta panel C#.
fn load() -> bool {
    let Ok(text) = std::fs::read_to_string(catalog_file()) else { return false };
    let mut list = Vec::new();
    for line in text.lines() {
        if !line.starts_with("P\t") {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 8 {
            continue;
        }
        let kind = f[2].parse().unwrap_or(usize::MAX);
        if kind < crate::profiles::LANGS.len() {
            list.push(Project { path: f[1].to_string(), kind, bytes: f[3].parse().unwrap_or(0), newest: f[5].parse().unwrap_or(0) });
        }
    }
    list.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    *PROJECTS.lock().unwrap() = Some(list);
    true
}

/// None = katalogu jeszcze nie ma (pierwszy skan trwa).
pub fn projects() -> Option<Vec<Project>> {
    PROJECTS.lock().unwrap().clone()
}

pub fn scanning() -> bool {
    SCANNING.load(Ordering::SeqCst)
}

/// Nowy katalog na dysku: lista z lewej i panel czytaja go od nowa.
fn reloaded() {
    load();
    crate::profiles::notify_catalog();
    panel_send("wczytaj");
}

/// Wynik pracy panelu (usuwanie) albo jego prosba o nowy skan.
pub fn from_panel(cmd: &str) {
    match cmd {
        "wczytaj" => {
            load();
            crate::profiles::notify_catalog();
        }
        "skanuj" => full_scan(),
        // okno panelu powstalo: widok moze je wstawic (malowanie po WM_APP_PROFILE)
        "gotowy" => {
            let ctl = PANEL_CTL.load(Ordering::SeqCst);
            if ctl != 0 {
                panel_send(&format!("wlasciciel\t{}", ctl));
            }
            crate::profiles::notify_catalog();
        }
        _ => {}
    }
}

pub fn full_scan() {
    if SCANNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(exe) = sibling("indeks-katalog.exe") else {
        SCANNING.store(false, Ordering::SeqCst);
        util::log("katalog: brak indeks-katalog.exe");
        return;
    };
    let _ = std::thread::Builder::new().name("indeks-katalog".into()).spawn(move || {
        let t0 = Instant::now();
        let ok = std::process::Command::new(exe).arg("--pelny").creation_flags(CREATE_NO_WINDOW).status().map(|s| s.success()).unwrap_or(false);
        util::log(&format!("katalog: pelny skan {} w {} s", if ok { "gotowy" } else { "nieudany" }, t0.elapsed().as_secs()));
        SCANNING.store(false, Ordering::SeqCst);
        reloaded();
    });
}

fn catalog_stale() -> bool {
    match std::fs::metadata(catalog_file()).and_then(|m| m.modified()) {
        Ok(t) => t.elapsed().map(|e| e.as_secs() > FULL_SCAN_DAYS * 86400).unwrap_or(false),
        Err(_) => true,
    }
}

/// Samodzielne okno i zrzuty: tylko gotowy katalog, bez skanow i obserwatora.
pub fn start_light(ctl: HWND) {
    CTL.store(ctl.0 as isize, Ordering::SeqCst);
    load();
}

/// Start silnikow: wczytanie gotowego katalogu, pierwszy skan (gdy trzeba), obserwator i panel.
pub fn start(ctl: HWND) {
    CTL.store(ctl.0 as isize, Ordering::SeqCst);
    load();
    let _ = std::thread::Builder::new().name("indeks-katalog-tlo".into()).spawn(|| {
        // start Windows i indeks wyszukiwania maja pierwszenstwo
        std::thread::sleep(Duration::from_secs(20));
        start_panel();
        if catalog_stale() {
            full_scan();
        }
        watch_drives();
        loop {
            std::thread::sleep(Duration::from_secs(5));
            flush_pending();
            if catalog_stale() {
                full_scan();
            }
        }
    });
}

// ---------- obserwator nowych folderow ----------

fn fixed_drives() -> Vec<String> {
    let mut buf = [0u16; 256];
    let n = unsafe { GetLogicalDriveStringsW(Some(&mut buf)) } as usize;
    buf[..n.min(256)]
        .split(|c| *c == 0)
        .filter(|s| !s.is_empty())
        .filter_map(|s| {
            let root: Vec<u16> = s.iter().copied().chain(std::iter::once(0)).collect();
            (unsafe { GetDriveTypeW(windows::core::PCWSTR(root.as_ptr())) } == 3).then(|| String::from_utf16_lossy(s))
        })
        .collect()
}

fn watch_drives() {
    for d in fixed_drives() {
        let _ = std::thread::Builder::new().name("indeks-obserwator".into()).spawn(move || watch_drive(&d));
    }
}

/// Nazwy, ktorych pojawienie sie moze oznaczac nowy projekt albo nowe smieci.
fn interesting(name: &str) -> bool {
    matches!(
        name,
        "package.json" | "cargo.toml" | "cmakelists.txt" | "pyproject.toml" | "requirements.txt" | "go.mod" | "pom.xml" | "build.gradle"
            | "composer.json" | "pubspec.yaml" | ".git" | "node_modules" | "target" | "__pycache__" | ".venv" | "venv" | "dist" | ".next" | "bin" | "obj"
            | "debug" | "release" | ".vs"
    ) || name.ends_with(".sln")
        || name.ends_with(".csproj")
        || name.ends_with(".vcxproj")
}

/// Sciezki, w ktorych zdarzenia nie maja znaczenia: wnetrza smieci i dane programow.
fn ignored(path: &str) -> bool {
    const SKIP: [&str; 12] = [
        "\\node_modules\\", "\\target\\", "\\.git\\", "\\appdata\\", "\\windows\\", "\\$recycle.bin\\", "\\program files", "\\programdata\\",
        "\\__pycache__\\", "\\.venv\\", "\\obj\\", "\\system volume information\\",
    ];
    SKIP.iter().any(|s| path.contains(s))
}

fn watch_drive(root: &str) {
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
    let w = wide(root);
    let h = unsafe {
        CreateFileW(pcwstr(&w), FILE_LIST_DIRECTORY.0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None)
    };
    let Ok(h) = h else {
        util::log(&format!("katalog: nie moge obserwowac {}", root));
        return;
    };
    let mut buf = vec![0u32; 16 * 1024];
    loop {
        let mut bytes = 0u32;
        // tylko nazwy: zapisy do plikow nie budza tego watku
        let ok = unsafe {
            ReadDirectoryChangesW(h, buf.as_mut_ptr() as *mut _, (buf.len() * 4) as u32, true, FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_FILE_NAME, Some(&mut bytes), None, None)
        };
        if ok.is_err() {
            break;
        }
        if bytes == 0 {
            continue;
        }
        let base = buf.as_ptr() as *const u8;
        let mut off = 0usize;
        loop {
            let info = unsafe { &*(base.add(off) as *const FILE_NOTIFY_INFORMATION) };
            // 1 = dodany, 5 = nowa nazwa, 2 = usuniety (zniknal projekt albo smieci)
            if matches!(info.Action.0, 1 | 2 | 5) {
                let rel = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(info.FileName.as_ptr(), info.FileNameLength as usize / 2) });
                note_change(root, &rel);
            }
            if info.NextEntryOffset == 0 {
                break;
            }
            off += info.NextEntryOffset as usize;
        }
    }
    unsafe {
        let _ = CloseHandle(h);
    }
}

fn note_change(root: &str, rel: &str) {
    let lowered = util::lower(rel);
    let name = lowered.rsplit('\\').next().unwrap_or(&lowered);
    if !interesting(name) {
        return;
    }
    let check = format!("\\{}", lowered);
    let parent_part = check.rsplit_once('\\').map(|p| format!("{}\\", p.0)).unwrap_or_default();
    if ignored(&parent_part) {
        return;
    }
    let Some(parent) = rel.rsplit_once('\\').map(|p| p.0) else { return };
    let dir = format!("{}{}", root, parent);
    let mut pending = PENDING.lock().unwrap();
    let slot = pending.get_or_insert_with(|| (HashSet::new(), Instant::now()));
    slot.0.insert(dir);
    slot.1 = Instant::now();
}

/// Po chwili ciszy zleca silnikowi C++ przeliczenie tylko zmienionych folderow.
fn flush_pending() {
    if scanning() {
        return;
    }
    let dirs = {
        let mut p = PENDING.lock().unwrap();
        match p.as_ref() {
            Some((_, at)) if at.elapsed() >= QUIET => p.take().map(|x| x.0).unwrap_or_default(),
            _ => return,
        }
    };
    if dirs.is_empty() || !catalog_file().exists() {
        return;
    }
    let Some(exe) = sibling("indeks-katalog.exe") else { return };
    let list = data_dir().join("katalog.odswiez");
    let text: String = dirs.iter().map(|d| format!("{}\n", d)).collect();
    if std::fs::write(&list, text).is_err() {
        return;
    }
    let ok = std::process::Command::new(exe).arg("--odswiez").arg(&list).creation_flags(CREATE_NO_WINDOW).status().map(|s| s.success()).unwrap_or(false);
    util::log(&format!("katalog: odswiezono {} folderow ({})", dirs.len(), ok));
    if ok {
        reloaded();
    }
}

// ---------- panel C# ----------

fn panel_title() -> Vec<u16> {
    wide(&format!("Indeks.Panel.{}", std::process::id()))
}

/// Uchwyt trzymany po pierwszym znalezieniu: FindWindow widzi tylko okna najwyzszego
/// poziomu, a osadzony panel jest dzieckiem widoku.
fn panel_hwnd() -> Option<HWND> {
    let known = PANEL.load(Ordering::SeqCst);
    if known != 0 && unsafe { IsWindow(Some(HWND(known as *mut _))).as_bool() } {
        return Some(HWND(known as *mut _));
    }
    let t = panel_title();
    let found = unsafe { FindWindowW(None, pcwstr(&t)) }.ok().filter(|h| !h.is_invalid());
    PANEL.store(found.map(|h| h.0 as isize).unwrap_or(0), Ordering::SeqCst);
    found
}

fn start_panel() {
    if panel_hwnd().is_some() {
        return;
    }
    // proces potrzebuje chwili na utworzenie okna; malowanie pyta o nie wiele razy
    {
        let mut last = PANEL_START.lock().unwrap();
        if last.map(|t| t.elapsed() < Duration::from_secs(15)).unwrap_or(false) {
            return;
        }
        *last = Some(Instant::now());
    }
    let Some(exe) = sibling("indeks-panel.exe") else {
        util::log("panel: brak indeks-panel.exe");
        return;
    };
    let ctl = CTL.load(Ordering::SeqCst);
    let started = std::process::Command::new(exe).args(["--rodzic", &std::process::id().to_string(), "--sterownik", &ctl.to_string()]).spawn();
    if let Err(e) = started {
        util::log(&format!("panel: nie wystartowal: {}", e));
    }
}

fn panel_send(text: &str) {
    if let Some(h) = panel_hwnd() {
        crate::win::send_copy(h, COPY_TO_PANEL, text);
    }
}

/// Panel przechodzi do tego okna Indeksu (`ctl` dostaje polecenia "otworz folder").
pub fn panel_claim(view: HWND, ctl: HWND) {
    PANEL_OWNER.store(view.0 as isize, Ordering::SeqCst);
    PANEL_CTL.store(ctl.0 as isize, Ordering::SeqCst);
    start_panel();
    panel_send(&format!("wlasciciel\t{}", ctl.0 as isize));
}

pub fn panel_owned_by(view: HWND) -> bool {
    PANEL_OWNER.load(Ordering::SeqCst) == view.0 as isize
}

/// Ustawia panel jako dziecko widoku w podanym prostokacie (piksele).
pub fn panel_place(view: HWND, x: i32, y: i32, w: i32, h: i32) -> bool {
    let Some(p) = panel_hwnd() else {
        start_panel();
        return false;
    };
    unsafe {
        SetWindowLongPtrW(p, GWL_STYLE, (WS_CHILD | WS_CLIPSIBLINGS).0 as isize);
        let _ = SetParent(p, Some(view));
        let _ = MoveWindow(p, x, y, w, h, true);
        let _ = ShowWindow(p, SW_SHOWNA);
    }
    true
}

/// Chowa panel i odpina go od widoku (zamkniecie okna nie moze zniszczyc panelu).
pub fn panel_hide(view: HWND) {
    if !panel_owned_by(view) {
        return;
    }
    let Some(p) = panel_hwnd() else { return };
    unsafe {
        if IsWindow(Some(p)).as_bool() {
            let _ = ShowWindow(p, SW_HIDE);
            let _ = SetParent(p, None);
            SetWindowLongPtrW(p, GWL_STYLE, WS_POPUP.0 as isize);
        }
    }
}

pub fn panel_project(path: &str) {
    panel_send(&format!("projekt\t{}", path));
}

pub fn panel_language(kind: Option<usize>) {
    panel_send(&format!("jezyk\t{}", kind.map(|k| k as i64).unwrap_or(-1)));
}
