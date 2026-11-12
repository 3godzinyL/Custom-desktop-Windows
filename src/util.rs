use std::io::Write;
use std::path::PathBuf;

use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DEFAULT};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(w: &[u16]) -> String {
    let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..n])
}

pub fn pcwstr(w: &[u16]) -> PCWSTR {
    PCWSTR(w.as_ptr())
}

pub fn known_folder(id: &GUID) -> Option<String> {
    unsafe {
        let p: PWSTR = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

fn env_dir(var: &str) -> PathBuf {
    let base = std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir());
    let dir = base.join("Indeks");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// %APPDATA%\Indeks - konfiguracja (przypiete foldery, ustawienia).
pub fn config_dir() -> PathBuf {
    env_dir("APPDATA")
}

/// %LOCALAPPDATA%\Indeks - log.
pub fn local_dir() -> PathBuf {
    env_dir("LOCALAPPDATA")
}

/// Krotki log diagnostyczny (przycinany przy starcie do ~256 kB).
pub fn log(msg: &str) {
    let path = local_dir().join("indeks.log");
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 256 * 1024 {
            let _ = std::fs::remove_file(&path);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] {}", t, msg);
    }
    if cfg!(debug_assertions) {
        eprintln!("{}", msg);
    }
}

/// Porownanie nazw jak w Eksploratorze (liczby naturalnie, bez wielkosci liter).
pub fn cmp_logical(a: &str, b: &str) -> std::cmp::Ordering {
    let wa = wide(a);
    let wb = wide(b);
    let r = unsafe { windows::Win32::UI::Shell::StrCmpLogicalW(pcwstr(&wa), pcwstr(&wb)) };
    r.cmp(&0)
}

pub fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// Dzieli tekst na wiersze konfiguracji `klucz=wartosc`.
pub fn kv_lines(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|l| {
        let l = l.trim_start_matches('\u{feff}').trim();
        if l.is_empty() || l.starts_with('#') {
            return None;
        }
        let (k, v) = l.split_once('=')?;
        Some((k.trim(), v.trim()))
    })
}

/// Zrzuty do README (`--zrzut --demo`, zmienna INDEKS_DEMO): nazwy, ktore na publicznym
/// zdjeciu wygladalyby zle, sa pomijane w listach. W zwyklej pracy nic nie jest ukrywane.
pub fn demo_hidden(name: &str) -> bool {
    const WORDS: [&str; 21] = [
        "steal", "cheat", "token", "grab", "virus", "wirus", "inject", "keydrop", "loader", "hack", "crack",
        "keylog", "spoof", "bypass", "exploit", "malware", "nuker", "miner", "rat_", "obf", "_load",
    ];
    if std::env::var_os("INDEKS_DEMO").is_none() {
        return false;
    }
    let n = name.to_lowercase();
    WORDS.iter().any(|w| n.contains(w))
}
