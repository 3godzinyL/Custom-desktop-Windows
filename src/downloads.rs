// Folder Pobrane: nowy, kompletny plik (z dowolnego zrodla - przegladarka, Discord,
// Telegram, torrent...) pokazujemy w Indeksie. Nie zmienia nic w systemie - tylko
// obserwuje folder. Plik jest "gotowy", gdy zniknie jego plik tymczasowy i przestanie rosnac.
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_READ, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_MODE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::util::{self, pcwstr, wide};

/// Do okna kontrolera: w kolejce sa gotowe pliki (`take()`).
pub const WM_APP_DOWNLOAD: u32 = WM_APP + 24;

static READY: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Gotowe pliki od ostatniego wywolania.
pub fn take() -> Vec<String> {
    std::mem::take(&mut *READY.lock().unwrap())
}

/// Pliki w trakcie pobierania (Chrome/Edge, Firefox, Opera, uTorrent, aria2...).
fn temporary(name: &str) -> bool {
    let l = name.to_lowercase();
    const EXT: [&str; 10] = [".crdownload", ".part", ".partial", ".tmp", ".download", ".opdownload", ".!ut", ".!qb", ".aria2", ".dtapart"];
    EXT.iter().any(|e| l.ends_with(e)) || l.starts_with("~$") || l.starts_with('.') || l.starts_with("unconfirmed ") || l == "desktop.ini"
}

pub fn start(dir: String, notify: HWND) {
    let hwnd = notify.0 as isize;
    let (tx, rx) = mpsc::channel::<String>();
    let d = dir.clone();
    let _ = std::thread::Builder::new().name("indeks-pobrane".into()).spawn(move || watch(&d, tx));
    let _ = std::thread::Builder::new().name("indeks-pobrane-gotowe".into()).spawn(move || settle(rx, hwnd));
}

/// Nowe nazwy w folderze (dodane albo przemianowane z pliku tymczasowego).
fn watch(dir: &str, tx: mpsc::Sender<String>) {
    let w = wide(dir);
    let h = unsafe {
        CreateFileW(
            pcwstr(&w),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    };
    let Ok(h) = h else {
        util::log(&format!("pobrane: nie moge sledzic {}", dir));
        return;
    };
    let mut buf = vec![0u32; 4096]; // 16 kB, wyrownane do DWORD
    loop {
        let mut bytes = 0u32;
        let ok = unsafe {
            ReadDirectoryChangesW(h, buf.as_mut_ptr() as *mut _, (buf.len() * 4) as u32, false, FILE_NOTIFY_CHANGE_FILE_NAME, Some(&mut bytes), None, None)
        };
        if ok.is_err() {
            util::log(&format!("pobrane: sledzenie przerwane: {:?}", ok));
            break;
        }
        let base = buf.as_ptr() as *const u8;
        let mut off = 0usize;
        while bytes > 0 {
            let info = unsafe { &*(base.add(off) as *const FILE_NOTIFY_INFORMATION) };
            let len = info.FileNameLength as usize / 2;
            let name = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(info.FileName.as_ptr(), len) });
            // 1 = dodany, 5 = nowa nazwa po zmianie (np. "x.crdownload" -> "x.zip")
            if matches!(info.Action.0, 1 | 5) && !temporary(&name) && !name.contains('\\') {
                let _ = tx.send(format!("{}\\{}", dir.trim_end_matches('\\'), name));
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

/// Czy nikt juz nie pisze do pliku (otwarcie bez wspoldzielenia zapisu sie udaje).
fn closed(path: &str) -> bool {
    let w = wide(path);
    match unsafe { CreateFileW(pcwstr(&w), FILE_GENERIC_READ.0, FILE_SHARE_MODE(FILE_SHARE_READ.0), None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) } {
        Ok(h) => {
            unsafe {
                let _ = CloseHandle(h);
            }
            true
        }
        Err(_) => false,
    }
}

/// Czeka, az plik bedzie kompletny: istnieje, ma rozmiar > 0, nie ma obok jego pliku
/// tymczasowego (Firefox tworzy pusty "x.zip" + "x.zip.part"), rozmiar stoi i nikt nie pisze.
fn settle(rx: mpsc::Receiver<String>, hwnd: isize) {
    let mut pending: Vec<(String, Instant, u64, u32)> = Vec::new();
    loop {
        let wait = if pending.is_empty() { Duration::from_secs(3600) } else { Duration::from_millis(700) };
        match rx.recv_timeout(wait) {
            Ok(p) => {
                if !pending.iter().any(|x| x.0.eq_ignore_ascii_case(&p)) {
                    pending.push((p, Instant::now(), u64::MAX, 0));
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return,
        }
        let mut done = Vec::new();
        pending.retain_mut(|(path, since, last, stable)| {
            if since.elapsed() > Duration::from_secs(600) {
                return false; // 10 min bez konca - rezygnujemy
            }
            let Ok(md) = std::fs::metadata(&*path) else { return since.elapsed() < Duration::from_secs(5) };
            if md.is_dir() {
                return false;
            }
            let busy = ["part", "crdownload", "download", "partial"].iter().any(|e| std::path::Path::new(&format!("{}.{}", path, e)).exists());
            let size = md.len();
            if busy || size == 0 || size != *last {
                *last = size;
                *stable = 0;
                return true;
            }
            *stable += 1;
            if *stable >= 2 && closed(path) {
                done.push(path.clone());
                return false;
            }
            true
        });
        if !done.is_empty() {
            READY.lock().unwrap().extend(done);
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_DOWNLOAD, WPARAM(0), LPARAM(0));
            }
        }
    }
}
