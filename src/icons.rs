// Ikony wierszy:
//  - foldery / plik bez rozszerzenia / "katalog glowny" / dysk: wektorowe jak w Chromium,
//  - pliki kodu (js, ts, rs, py, cpp, ...): plakietki jezykow,
//  - reszta: ikona systemowa Windows dla rozszerzenia (exe/lnk/ico: dla konkretnego pliku),
//  - rozszerzenia nieznane systemowi: szara plakietka z rozszerzeniem.
// Ikony systemowe pobiera watek w tle (COM STA); UI dostaje gotowe piksele BGRA.
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Controls::IImageList;
use windows::Win32::UI::Shell::{
    SHGetFileInfoW, SHGetImageList, SHFILEINFOW, SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DispatchMessageW, PeekMessageW, PostMessageW, TranslateMessage, MSG, PM_REMOVE, WM_APP,
};

use crate::util::{pcwstr, wide};

pub const WM_APP_ICON: u32 = WM_APP + 21;

#[derive(Clone, Copy, Debug)]
pub struct Badge {
    pub label: &'static str,
    pub bg: u32,
    pub fg: u32,
}

#[derive(Clone, Debug)]
pub enum IconKind {
    Folder,
    File,
    Drive,
    Badge(Badge),
    /// Ikona systemowa: klucz cache (".pdf" albo pelna sciezka exe/lnk).
    System(String),
}

const fn b(label: &'static str, bg: u32, fg: u32) -> Badge {
    Badge { label, bg, fg }
}

const WHITE: u32 = 0xFFFFFF;
const BLACK: u32 = 0x000000;

pub fn badge_for_ext(ext: &str) -> Option<Badge> {
    Some(match ext {
        "js" | "mjs" | "cjs" => b("JS", 0xF7DF1E, BLACK),
        "jsx" => b("JSX", 0x61DAFB, BLACK),
        "ts" | "mts" | "cts" => b("TS", 0x3178C6, WHITE),
        "tsx" => b("TSX", 0x3178C6, WHITE),
        "rs" => b("RS", 0xDEA584, BLACK),
        "py" | "pyw" | "pyi" => b("PY", 0x3776AB, 0xFFD43B),
        "ipynb" => b("NB", 0xF37626, WHITE),
        "whl" => b("WHL", 0x3776AB, 0xFFD43B),
        "c" => b("C", 0xA8B9CC, BLACK),
        "h" => b("H", 0xA8B9CC, BLACK),
        "cpp" | "cc" | "cxx" | "c++" => b("C++", 0x00599C, WHITE),
        "hpp" | "hh" | "hxx" | "inl" => b("H++", 0x00599C, WHITE),
        "cs" => b("C#", 0x68217A, WHITE),
        "go" => b("GO", 0x00ADD8, WHITE),
        "java" => b("JV", 0xB07219, WHITE),
        "class" | "jar" => b("JAR", 0xB07219, WHITE),
        "kt" | "kts" => b("KT", 0x7F52FF, WHITE),
        "swift" => b("SW", 0xF05138, WHITE),
        "rb" => b("RB", 0xCC342D, WHITE),
        "php" => b("PHP", 0x777BB4, WHITE),
        "lua" => b("LUA", 0x000080, WHITE),
        "sh" | "bash" | "zsh" => b("SH", 0x4EAA25, WHITE),
        "ps1" | "psm1" | "psd1" => b("PS", 0x012456, WHITE),
        "bat" | "cmd" => b("BAT", 0xC1F12E, BLACK),
        "json" | "jsonc" | "json5" | "jsonl" | "ndjson" => b("{}", 0xCBCB41, BLACK),
        "toml" => b("TML", 0x9C4221, WHITE),
        "yaml" | "yml" => b("YML", 0xCB171E, WHITE),
        "xml" | "xsd" | "xaml" => b("XML", 0x0060AC, WHITE),
        "md" | "markdown" | "mdx" => b("MD", 0x519ABA, WHITE),
        "html" | "htm" | "xhtml" => b("HTM", 0xE34C26, WHITE),
        "css" => b("CSS", 0x1572B6, WHITE),
        "scss" | "sass" | "less" => b("SCS", 0xCC6699, WHITE),
        "vue" => b("VUE", 0x41B883, WHITE),
        "svelte" => b("SV", 0xFF3E00, WHITE),
        "sql" | "sqlite" | "db" => b("SQL", 0xE38C00, BLACK),
        "wasm" | "wat" => b("WA", 0x654FF0, WHITE),
        "mq4" | "mq5" | "mqh" => b("MQ", 0x4A90D9, WHITE),
        "ex4" | "ex5" => b("EX5", 0x2B5797, WHITE),
        "pine" => b("PIN", 0x2962FF, WHITE),
        "dart" => b("DRT", 0x00B4AB, WHITE),
        "zig" => b("ZIG", 0xEC915C, BLACK),
        "r" => b("R", 0x198CE7, WHITE),
        "jl" => b("JL", 0x9558B2, WHITE),
        "hs" => b("HS", 0x5E5086, WHITE),
        "scala" => b("SC", 0xDC322F, WHITE),
        "asm" | "s" => b("ASM", 0x6E4C13, WHITE),
        "gradle" => b("GR", 0x02303A, WHITE),
        "lock" => b("LCK", 0x8A8A8A, BLACK),
        "map" => b("MAP", 0x8A8A8A, BLACK),
        "node" => b("ND", 0x339933, WHITE),
        "env" => b("ENV", 0xECD53F, BLACK),
        "cfg" | "conf" | "ini" | "properties" => b("CFG", 0x6D8086, WHITE),
        "proto" => b("PB", 0x4285F4, WHITE),
        "graphql" | "gql" => b("GQL", 0xE10098, WHITE),
        "tf" => b("TF", 0x7B42BC, WHITE),
        "glsl" | "hlsl" | "wgsl" | "frag" | "vert" => b("GL", 0x5586A4, WHITE),
        "csv" | "tsv" => b("CSV", 0x217346, WHITE),
        "log" => b("LOG", 0x5A5A5A, WHITE),
        _ => return None,
    })
}

fn badge_for_name(name: &str) -> Option<Badge> {
    Some(match name.to_ascii_lowercase().as_str() {
        ".gitignore" | ".gitattributes" | ".gitmodules" => b("GIT", 0xF05032, WHITE),
        ".env" | ".env.local" | ".env.example" => b("ENV", 0xECD53F, BLACK),
        "dockerfile" | ".dockerignore" => b("DK", 0x2496ED, WHITE),
        "makefile" | "cmakelists.txt" => b("MK", 0x427819, WHITE),
        ".npmrc" | ".nvmrc" => b("NPM", 0xCB3837, WHITE),
        ".editorconfig" | ".prettierrc" | ".eslintrc" => b("CFG", 0x6D8086, WHITE),
        _ => return None,
    })
}

/// Rozszerzenia, dla ktorych ikona zalezy od konkretnego pliku.
fn per_file(ext: &str) -> bool {
    matches!(ext, "exe" | "lnk" | "ico" | "url" | "scr" | "cpl" | "msc" | "appref-ms" | "cur" | "ani")
}

pub fn kind_for(name: &str, ext: &str, is_dir: bool, full_path: &str) -> IconKind {
    if is_dir {
        return IconKind::Folder;
    }
    if let Some(bd) = badge_for_name(name) {
        return IconKind::Badge(bd);
    }
    if ext.is_empty() {
        return IconKind::File;
    }
    if let Some(bd) = badge_for_ext(ext) {
        return IconKind::Badge(bd);
    }
    if per_file(ext) {
        return IconKind::System(full_path.to_string());
    }
    IconKind::System(format!(".{}", ext))
}

pub struct Pixels {
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>,
}

pub enum IconState {
    Pending,
    Ready(Arc<Pixels>),
    /// System nie zna rozszerzenia - rysujemy plakietke.
    Unknown,
    Failed,
}

pub struct Icons {
    cache: HashMap<String, IconState>,
    tx: Sender<String>,
    rx: Receiver<(String, IconState)>,
    /// Ile ikon z tego zestawu jeszcze czeka (dla zrzutu ekranu).
    pending: usize,
}

impl Icons {
    pub fn new(notify: HWND) -> Icons {
        let (tx, req_rx) = channel::<String>();
        let (res_tx, rx) = channel::<(String, IconState)>();
        let hwnd = notify.0 as isize;
        std::thread::Builder::new()
            .name("indeks-ikony".into())
            .spawn(move || worker(req_rx, res_tx, hwnd))
            .ok();
        Icons { cache: HashMap::new(), tx, rx, pending: 0 }
    }

    pub fn get(&mut self, key: &str) -> &IconState {
        if !self.cache.contains_key(key) {
            self.cache.insert(key.to_string(), IconState::Pending);
            self.pending += 1;
            let _ = self.tx.send(key.to_string());
        }
        &self.cache[key]
    }

    /// Odbiera gotowe ikony. Zwraca true, jesli cos doszlo.
    pub fn drain(&mut self) -> bool {
        let mut any = false;
        while let Ok((k, st)) = self.rx.try_recv() {
            self.pending = self.pending.saturating_sub(1);
            self.cache.insert(k, st);
            any = true;
        }
        any
    }

    pub fn pending(&self) -> usize {
        self.pending
    }

    /// Ikony plikow zaleza od sciezki - czyscimy je przy zmianie folderu,
    /// zeby pamiec nie rosla bez konca.
    pub fn forget_per_file(&mut self) {
        if self.cache.len() > 2000 {
            self.cache.retain(|k, v| k.starts_with('.') || matches!(v, IconState::Pending));
        }
    }
}

fn worker(rx: Receiver<String>, tx: Sender<(String, IconState)>, hwnd: isize) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let wic: Option<IWICImagingFactory> =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok() };
    // SHIL_EXTRALARGE = 48 px: przy 150% skali rysujemy 24 px (dokladnie 1/2 - ostro)
    let list: Option<IImageList> = unsafe { SHGetImageList(2).ok() };
    let generic = sys_index(".zz-indeks-nieznane", true);
    // Watek STA musi pompowac komunikaty: funkcje powloki (np. dla .lnk) potrafia
    // utworzyc tu ukryte okno i czekac na komunikat - bez pompy watek wisi na zawsze.
    // Kolejka: najpierw ikony po rozszerzeniu (szybkie), potem konkretne pliki.
    let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    loop {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if queue.is_empty() {
            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(k) => queue.push_back(k),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            }
        }
        while let Ok(k) = rx.try_recv() {
            queue.push_back(k);
        }
        let pos = queue.iter().position(|k| k.starts_with('.')).unwrap_or(0);
        let Some(key) = queue.remove(pos) else { continue };
        let by_ext = key.starts_with('.');
        let st = match sys_index(&key, by_ext) {
            None => IconState::Failed,
            Some(i) if by_ext && Some(i) == generic => IconState::Unknown,
            Some(i) => match (&list, &wic) {
                (Some(l), Some(w)) => pixels(l, w, i).map(|p| IconState::Ready(Arc::new(p))).unwrap_or(IconState::Failed),
                _ => IconState::Failed,
            },
        };
        if tx.send((key, st)).is_err() {
            break;
        }
        // UI odbiera wszystkie gotowe ikony naraz, a odmalowanie i tak sie laczy
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_ICON, WPARAM(0), LPARAM(0));
        }
    }
}

fn sys_index(key: &str, by_ext: bool) -> Option<i32> {
    let w = wide(key);
    let mut info = SHFILEINFOW::default();
    let flags = if by_ext { SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES } else { SHGFI_SYSICONINDEX };
    let r = unsafe {
        SHGetFileInfoW(
            pcwstr(&w),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    if r == 0 {
        None
    } else {
        Some(info.iIcon)
    }
}

fn pixels(list: &IImageList, wic: &IWICImagingFactory, index: i32) -> Option<Pixels> {
    unsafe {
        let hicon = list.GetIcon(index, 0x1 /* ILD_TRANSPARENT */).ok()?;
        let res = (|| -> windows::core::Result<Pixels> {
            let bmp = wic.CreateBitmapFromHICON(hicon)?;
            let conv = wic.CreateFormatConverter()?;
            conv.Initialize(&bmp, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
            let (mut w, mut h) = (0u32, 0u32);
            conv.GetSize(&mut w, &mut h)?;
            let mut bgra = vec![0u8; (w * h * 4) as usize];
            conv.CopyPixels(std::ptr::null(), w * 4, &mut bgra)?;
            Ok(Pixels { w, h, bgra })
        })();
        let _ = DestroyIcon(hicon);
        res.ok()
    }
}
