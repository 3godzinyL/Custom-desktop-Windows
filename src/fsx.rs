// Listowanie folderow i formatowanie jak w indeksie katalogu Chromium
// ("8.0 kB", "18.12.2025, 00:04:37").
use windows::core::PCWSTR;
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::Storage::FileSystem::{
    FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
    GetDiskFreeSpaceExW, GetFileAttributesExW, GetFileExInfoStandard, GetLogicalDrives,
    GetVolumeInformationW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN,
    FILE_ATTRIBUTE_SYSTEM, FIND_FIRST_EX_LARGE_FETCH, WIN32_FILE_ATTRIBUTE_DATA,
    WIN32_FIND_DATAW,
};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

use crate::util::{self, pcwstr, wide};

/// Pseudo-sciezka widoku "Ten komputer" (lista dyskow).
pub const COMPUTER: &str = "";

#[derive(Clone, Debug)]
pub struct Item {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
    #[allow(dead_code)]
    pub attrs: u32,
    /// Dla dyskow: etykieta woluminu.
    pub label: Option<String>,
    /// Dla wynikow wyszukiwania: folder nadrzedny.
    pub folder: Option<String>,
}

impl Item {
    pub fn ext(&self) -> String {
        if self.is_dir {
            return String::new();
        }
        match self.name.rfind('.') {
            Some(i) if i > 0 => util::lower(&self.name[i + 1..]),
            _ => String::new(),
        }
    }

}

pub fn ft_u64(ft: &FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('\\') {
        format!("{}{}", dir, name)
    } else {
        format!("{}\\{}", dir, name)
    }
}

/// Folder nadrzedny; dla "C:\" zwraca widok komputera, dla niego None.
pub fn parent(path: &str) -> Option<String> {
    if path == COMPUTER {
        return None;
    }
    let t = path.trim_end_matches('\\');
    match t.rfind('\\') {
        Some(i) => {
            let p = &t[..i];
            if p.len() == 2 && p.ends_with(':') {
                Some(format!("{}\\", p))
            } else if p.is_empty() || p.ends_with('\\') {
                Some(COMPUTER.to_string())
            } else {
                Some(p.to_string())
            }
        }
        None => Some(COMPUTER.to_string()),
    }
}

/// Sciezka do wyswietlenia w tytule ("C:\dev\").
pub fn display_path(path: &str) -> String {
    if path == COMPUTER {
        return "Ten komputer".to_string();
    }
    if path.ends_with('\\') {
        path.to_string()
    } else {
        format!("{}\\", path)
    }
}

pub fn last_name(path: &str) -> String {
    if path == COMPUTER {
        return "Ten komputer".to_string();
    }
    let t = path.trim_end_matches('\\');
    match t.rfind('\\') {
        Some(i) => t[i + 1..].to_string(),
        None => format!("{}\\", t),
    }
}

pub fn list_dir(path: &str, show_protected: bool) -> Result<Vec<Item>, String> {
    if path == COMPUTER {
        return Ok(list_drives());
    }
    let pattern = wide(&join(path, "*"));
    let mut fd = WIN32_FIND_DATAW::default();
    let h = unsafe {
        FindFirstFileExW(
            pcwstr(&pattern),
            FindExInfoBasic,
            &mut fd as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let h = match h {
        Ok(h) => h,
        Err(e) => {
            // pusty folder (np. pusty dysk) zwraca ERROR_FILE_NOT_FOUND
            if e.code().0 as u32 == 0x80070002 {
                return Ok(Vec::new());
            }
            return Err(e.message().to_string());
        }
    };
    let mut out = Vec::new();
    loop {
        let name = util::from_wide(&fd.cFileName);
        let attrs = fd.dwFileAttributes;
        let protected = attrs & FILE_ATTRIBUTE_HIDDEN.0 != 0 && attrs & FILE_ATTRIBUTE_SYSTEM.0 != 0;
        if name != "." && name != ".." && (show_protected || !protected) {
            out.push(Item {
                name,
                is_dir: attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
                size: ((fd.nFileSizeHigh as u64) << 32) | fd.nFileSizeLow as u64,
                mtime: ft_u64(&fd.ftLastWriteTime),
                attrs,
                label: None,
                folder: None,
            });
        }
        if unsafe { FindNextFileW(h, &mut fd) }.is_err() {
            break;
        }
    }
    unsafe {
        let _ = FindClose(h);
    }
    Ok(out)
}

pub fn list_drives() -> Vec<Item> {
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        let wroot = wide(&root);
        let mut label = [0u16; 128];
        let label = unsafe {
            if GetVolumeInformationW(pcwstr(&wroot), Some(&mut label), None, None, None, None).is_ok() {
                Some(util::from_wide(&label)).filter(|s| !s.is_empty())
            } else {
                None
            }
        };
        let mut total = 0u64;
        unsafe {
            let _ = GetDiskFreeSpaceExW(pcwstr(&wroot), None, Some(&mut total), None);
        }
        out.push(Item {
            name: root,
            is_dir: true,
            size: total,
            mtime: 0,
            attrs: FILE_ATTRIBUTE_DIRECTORY.0,
            label,
            folder: None,
        });
    }
    out
}

/// Atrybuty pojedynczego pliku (dla wynikow wyszukiwania).
pub fn stat(path: &str) -> Option<(bool, u64, u64, u32)> {
    let w = wide(path);
    let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
    unsafe {
        GetFileAttributesExW(PCWSTR(w.as_ptr()), GetFileExInfoStandard, &mut data as *mut _ as *mut _).ok()?;
    }
    let size = ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64;
    Some((
        data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
        size,
        ft_u64(&data.ftLastWriteTime),
        data.dwFileAttributes,
    ))
}

pub fn fmt_size(b: u64) -> String {
    if b < 1024 {
        return format!("{} B", b);
    }
    const UNITS: [&str; 5] = ["kB", "MB", "GB", "TB", "PB"];
    let mut v = b as f64 / 1024.0;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 100.0 {
        format!("{:.0} {}", v, UNITS[i])
    } else {
        format!("{:.1} {}", v, UNITS[i])
    }
}

pub fn fmt_time(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    let f = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        if FileTimeToSystemTime(&f, &mut utc).is_err() {
            return String::new();
        }
        if SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            local = utc;
        }
    }
    format!(
        "{}.{:02}.{}, {:02}:{:02}:{:02}",
        local.wDay, local.wMonth, local.wYear, local.wHour, local.wMinute, local.wSecond
    )
}
