// Indeks wyszukiwania: dane i dopasowanie w C++ (cpp/search.cpp), tutaj
// skanowanie dysku w tle (niski priorytet I/O) i sledzenie zmian na zywo.
use std::ffi::c_char;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    FindNextFileW, ReadDirectoryChangesW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
    FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FIND_FIRST_EX_LARGE_FETCH, OPEN_EXISTING, WIN32_FIND_DATAW,
};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::fsx;
use crate::util::{self, pcwstr, wide};

#[repr(C)]
struct IxIndex {
    _p: [u8; 0],
}

const IX_DIR: u32 = 1;
const IX_NONE: u32 = u32::MAX;

extern "C" {
    fn ix_create() -> *mut IxIndex;
    fn ix_add(ix: *mut IxIndex, parent: u32, name: *const c_char, len: u32, flags: u32, mtime: u32) -> u32;
    fn ix_touch(ix: *mut IxIndex, id: u32, mtime: u32);
    fn ix_profile_dir(ix: *mut IxIndex, id: u32, out: *mut IxProfile) -> u32;
    fn ix_find_projects(ix: *mut IxIndex, out_ids: *mut u32, out_kind: *mut u8, max: u32) -> u32;
    fn ix_find_child(ix: *mut IxIndex, parent: u32, name: *const c_char, len: u32) -> u32;
    fn ix_remove(ix: *mut IxIndex, id: u32);
    fn ix_rename(ix: *mut IxIndex, id: u32, name: *const c_char, len: u32);
    fn ix_path(ix: *mut IxIndex, id: u32, buf: *mut c_char, cap: u32) -> u32;
    fn ix_flags(ix: *mut IxIndex, id: u32) -> u32;
    fn ix_count(ix: *mut IxIndex) -> u32;
    fn ix_memory(ix: *mut IxIndex) -> u64;
    fn ix_compact(ix: *mut IxIndex);
    fn ix_search(ix: *mut IxIndex, q: *const c_char, qlen: u32, mode: u32, out: *mut u32, max: u32, total: *mut u32) -> u32;
    fn ix_lookup(ix: *mut IxIndex, path: *const c_char, len: u32) -> u32;
    fn ix_child_dirs(ix: *mut IxIndex, id: u32, out: *mut u32, max: u32) -> u32;
    fn ix_name(ix: *mut IxIndex, id: u32, buf: *mut c_char, cap: u32) -> u32;
}

/// Uchwyt indeksu C++. Indeks zyje do konca procesu; wszystkie funkcje C++
/// same pilnuja blokad, wiec wspoldzielenie miedzy watkami jest bezpieczne.
struct Index(*mut IxIndex);
unsafe impl Send for Index {}
unsafe impl Sync for Index {}

impl Index {
    fn add(&self, parent: u32, name: &str, dir: bool, mtime: u32) -> u32 {
        unsafe { ix_add(self.0, parent, name.as_ptr() as _, name.len() as u32, if dir { IX_DIR } else { 0 }, mtime) }
    }
    fn find_child(&self, parent: u32, name: &str) -> u32 {
        unsafe { ix_find_child(self.0, parent, name.as_ptr() as _, name.len() as u32) }
    }
    fn remove(&self, id: u32) {
        unsafe { ix_remove(self.0, id) }
    }
    fn rename(&self, id: u32, name: &str) {
        unsafe { ix_rename(self.0, id, name.as_ptr() as _, name.len() as u32) }
    }
    fn path(&self, id: u32) -> String {
        let mut buf = vec![0u8; 512];
        let n = unsafe { ix_path(self.0, id, buf.as_mut_ptr() as _, buf.len() as u32) } as usize;
        if n > buf.len() {
            buf.resize(n, 0);
            unsafe { ix_path(self.0, id, buf.as_mut_ptr() as _, buf.len() as u32) };
        }
        buf.truncate(n);
        String::from_utf8_lossy(&buf).into_owned()
    }
    fn is_dir(&self, id: u32) -> bool {
        unsafe { ix_flags(self.0, id) & IX_DIR != 0 }
    }
}

struct Root {
    path: String,
    id: u32,
}

struct Engine {
    ix: Index,
    skip: Vec<String>,
    roots: Mutex<Vec<Root>>,
    scanned: AtomicU32,
    ready: AtomicBool,
    notify: AtomicIsize,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

pub const WM_APP_INDEX: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 20;

pub struct Hit {
    pub path: String,
    pub is_dir: bool,
}

pub struct Status {
    pub count: u32,
    pub ready: bool,
    pub memory: u64,
    /// Ile wpisow skaner juz obejrzal (postep pierwszego indeksowania).
    pub scanned: u32,
}

fn engine() -> Option<&'static Engine> {
    ENGINE.get()
}

/// Startuje indeksowanie w tle. Wywolac raz.
pub fn start(roots: Vec<String>, skip: Vec<String>, notify: HWND) {
    let raw = unsafe { ix_create() };
    if raw.is_null() {
        util::log("indeks: brak pamieci na indeks");
        return;
    }
    let e = Engine {
        ix: Index(raw),
        skip: skip.iter().map(|s| util::lower(s)).collect(),
        roots: Mutex::new(Vec::new()),
        scanned: AtomicU32::new(0),
        ready: AtomicBool::new(false),
        notify: AtomicIsize::new(notify.0 as isize),
    };
    if ENGINE.set(e).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("indeks-skan".into())
        .spawn(move || {
            unsafe {
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
            }
            let e = engine().unwrap();
            let t0 = std::time::Instant::now();
            util::log(&format!("indeks: start, korzenie: {:?}", roots));
            let mut started = Vec::new();
            for r in roots {
                // pomijaj korzenie zawarte w juz dodanych
                let rl = util::lower(r.trim_end_matches('\\'));
                if started.iter().any(|s: &String| rl.starts_with(&format!("{}\\", s)) || rl == *s) {
                    continue;
                }
                started.push(rl);
                if let Some(id) = crawl_root(e, &r) {
                    let idx = {
                        let mut roots = e.roots.lock().unwrap();
                        roots.push(Root { path: r.clone(), id });
                        roots.len() - 1
                    };
                    std::thread::Builder::new()
                        .name("indeks-zmiany".into())
                        .spawn(move || watch_root(idx))
                        .ok();
                }
                notify_ui(e);
            }
            unsafe { ix_compact(e.ix.0) };
            e.ready.store(true, Ordering::SeqCst);
            util::log(&format!(
                "indeks gotowy: {} wpisow w {:.1}s, {:.1} MB",
                unsafe { ix_count(e.ix.0) },
                t0.elapsed().as_secs_f32(),
                unsafe { ix_memory(e.ix.0) } as f64 / 1048576.0
            ));
            notify_ui(e);
        })
        .ok();
}

fn notify_ui(e: &Engine) {
    let h = e.notify.load(Ordering::SeqCst);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_APP_INDEX, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn status() -> Status {
    match engine() {
        Some(e) => Status {
            count: unsafe { ix_count(e.ix.0) },
            ready: e.ready.load(Ordering::SeqCst),
            memory: unsafe { ix_memory(e.ix.0) },
            scanned: e.scanned.load(Ordering::Relaxed),
        },
        None => Status { count: 0, ready: false, memory: 0, scanned: 0 },
    }
}

#[derive(Clone, Copy)]
pub enum Find {
    All = 0,
    Dirs = 1,
    Files = 2,
}

pub fn search(query: &str, mode: Find, max: u32) -> (Vec<Hit>, u32) {
    let Some(e) = engine() else { return (Vec::new(), 0) };
    let mut ids = vec![0u32; max.max(1) as usize];
    let mut total = 0u32;
    let n = unsafe {
        ix_search(e.ix.0, query.as_ptr() as _, query.len() as u32, mode as u32, ids.as_mut_ptr(), max, &mut total)
    } as usize;
    let hits = ids[..n]
        .iter()
        .map(|&id| Hit { path: e.ix.path(id), is_dir: e.ix.is_dir(id) })
        .collect();
    (hits, total)
}

/// Profil poddrzewa liczony w C++ (cpp/search.h: ix_profile).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IxProfile {
    pub files: u32,
    pub dirs: u32,
    pub newest: u32,
    pub markers: u32,
    pub lang: [u32; 32],
}

/// FILETIME -> minuty od 1601 (tak trzyma to indeks C++).
pub fn minutes(ft: u64) -> u32 {
    (ft / 600_000_000) as u32
}

pub fn now_minutes() -> u32 {
    let ft = unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() };
    minutes(fsx::ft_u64(&ft))
}

/// Profil folderu z indeksu (jezyki, znaczniki projektu, najnowszy plik).
/// None, gdy indeks nie jest gotowy albo folder lezy poza nim.
pub fn profile(path: &str) -> Option<IxProfile> {
    let e = engine()?;
    if !e.ready.load(Ordering::SeqCst) {
        return None;
    }
    let id = unsafe { ix_lookup(e.ix.0, path.as_ptr() as _, path.len() as u32) };
    if id == IX_NONE {
        return None;
    }
    let mut p = IxProfile { files: 0, dirs: 0, newest: 0, markers: 0, lang: [0; 32] };
    (unsafe { ix_profile_dir(e.ix.0, id, &mut p) } != 0).then_some(p)
}

/// Korzenie projektow w calym indeksie: (sciezka, jezyk wg profiles::LANGS).
pub fn find_projects() -> Vec<(String, usize)> {
    let Some(e) = engine() else { return Vec::new() };
    let max = 20_000u32;
    let mut ids = vec![0u32; max as usize];
    let mut kinds = vec![0u8; max as usize];
    let n = unsafe { ix_find_projects(e.ix.0, ids.as_mut_ptr(), kinds.as_mut_ptr(), max) }.min(max) as usize;
    (0..n).map(|i| (e.ix.path(ids[i]), kinds[i] as usize)).collect()
}

pub fn index_ready() -> bool {
    engine().map(|e| e.ready.load(Ordering::SeqCst)).unwrap_or(false)
}

/// Podfoldery prosto z indeksu w pamieci (C++), bez dotykania dysku.
/// None, gdy sciezka lezy poza indeksem (wtedy drzewo czyta dysk).
pub fn child_dirs(path: &str) -> Option<Vec<String>> {
    let e = engine()?;
    if !e.ready.load(Ordering::SeqCst) {
        return None;
    }
    let id = unsafe { ix_lookup(e.ix.0, path.as_ptr() as _, path.len() as u32) };
    if id == IX_NONE {
        return None;
    }
    let mut ids = vec![0u32; 512];
    let n = unsafe { ix_child_dirs(e.ix.0, id, ids.as_mut_ptr(), ids.len() as u32) } as usize;
    if n > ids.len() {
        ids.resize(n, 0);
        unsafe { ix_child_dirs(e.ix.0, id, ids.as_mut_ptr(), ids.len() as u32) };
    }
    let mut buf = vec![0u8; 512];
    let names = ids[..n]
        .iter()
        .map(|&c| {
            let len = unsafe { ix_name(e.ix.0, c, buf.as_mut_ptr() as _, buf.len() as u32) } as usize;
            String::from_utf8_lossy(&buf[..len.min(buf.len())]).into_owned()
        })
        .filter(|n: &String| !crate::util::demo_hidden(n))
        .collect();
    Some(names)
}

#[derive(PartialEq)]
enum DirKind {
    /// Ukryty systemowy - poza indeksem (jak w liscie folderow).
    Hidden,
    /// Sama nazwa w indeksie, bez zawartosci (node_modules, .git, junction...),
    /// zeby drzewo z indeksu pokazywalo komplet podfolderow.
    Leaf,
    Crawl,
}

fn dir_kind(e: &Engine, name: &str, attrs: u32) -> DirKind {
    if attrs & FILE_ATTRIBUTE_HIDDEN.0 != 0 && attrs & FILE_ATTRIBUTE_SYSTEM.0 != 0 {
        return DirKind::Hidden;
    }
    // junction / symlink - unikamy petli i duplikatow
    if name.starts_with('.') || name.starts_with('$') || attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return DirKind::Leaf;
    }
    let l = util::lower(name);
    if e.skip.iter().any(|s| *s == l) {
        DirKind::Leaf
    } else {
        DirKind::Crawl
    }
}

fn crawl_root(e: &Engine, root: &str) -> Option<u32> {
    let path = root.trim_end_matches('\\');
    let path = if path.len() == 2 { format!("{}\\", path) } else { path.to_string() };
    let st = fsx::stat(&path)?;
    if !st.0 {
        return None;
    }
    let id = e.ix.add(IX_NONE, &path, true, minutes(st.2));
    if id == IX_NONE {
        return None;
    }
    crawl_dir(e, &path, id);
    Some(id)
}

/// Skanuje poddrzewo (BFS, bez rekurencji na stosie).
fn crawl_dir(e: &Engine, path: &str, id: u32) {
    let mut queue = std::collections::VecDeque::new();
    queue.push_back((path.to_string(), id));
    let mut fd = WIN32_FIND_DATAW::default();
    while let Some((dir, did)) = queue.pop_front() {
        let pattern = wide(&fsx::join(&dir, "*"));
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
        let Ok(h) = h else { continue };
        loop {
            let name = util::from_wide(&fd.cFileName);
            let attrs = fd.dwFileAttributes;
            let mt = minutes(fsx::ft_u64(&fd.ftLastWriteTime));
            if name != "." && name != ".." {
                if attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
                    match dir_kind(e, &name, attrs) {
                        DirKind::Crawl => {
                            let cid = e.ix.add(did, &name, true, mt);
                            if cid != IX_NONE {
                                queue.push_back((fsx::join(&dir, &name), cid));
                            }
                        }
                        DirKind::Leaf => {
                            e.ix.add(did, &name, true, mt);
                        }
                        DirKind::Hidden => {}
                    }
                } else if !(attrs & FILE_ATTRIBUTE_HIDDEN.0 != 0 && attrs & FILE_ATTRIBUTE_SYSTEM.0 != 0) {
                    e.ix.add(did, &name, false, mt);
                }
                let n = e.scanned.fetch_add(1, Ordering::Relaxed);
                if n % 100_000 == 99_999 {
                    notify_ui(e);
                }
            }
            if unsafe { FindNextFileW(h, &mut fd) }.is_err() {
                break;
            }
        }
        unsafe {
            let _ = FindClose(h);
        }
    }
}

/// Znajduje identyfikator dla sciezki wzglednej wzgledem korzenia.
fn resolve(e: &Engine, root_id: u32, rel: &str) -> u32 {
    let mut id = root_id;
    for part in rel.split('\\').filter(|p| !p.is_empty()) {
        id = e.ix.find_child(id, part);
        if id == IX_NONE {
            break;
        }
    }
    id
}

fn rel_skipped(e: &Engine, rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('\\').collect();
    // ostatni czlon moze byc plikiem - sprawdzamy tylko foldery nadrzedne
    // (sam folder sprawdza `dir_kind` przy dodawaniu)
    parts[..parts.len().saturating_sub(1)].iter().any(|p| {
        p.starts_with('.') || p.starts_with('$') || e.skip.contains(&util::lower(p))
    })
}

fn add_rel(e: &Engine, root_path: &str, root_id: u32, rel: &str) {
    let (parent_rel, name) = match rel.rfind('\\') {
        Some(i) => (&rel[..i], &rel[i + 1..]),
        None => ("", rel),
    };
    let pid = resolve(e, root_id, parent_rel);
    if pid == IX_NONE || e.ix.find_child(pid, name) != IX_NONE {
        return;
    }
    let full = fsx::join(root_path.trim_end_matches('\\'), rel);
    let Some((is_dir, _, mtime, attrs)) = fsx::stat(&full) else { return };
    let mt = minutes(mtime);
    if is_dir {
        match dir_kind(e, name, attrs) {
            DirKind::Crawl => {
                let id = e.ix.add(pid, name, true, mt);
                if id != IX_NONE {
                    crawl_dir(e, &full, id);
                }
            }
            DirKind::Leaf => {
                e.ix.add(pid, name, true, mt);
            }
            DirKind::Hidden => {}
        }
    } else if !(attrs & FILE_ATTRIBUTE_HIDDEN.0 != 0 && attrs & FILE_ATTRIBUTE_SYSTEM.0 != 0) {
        e.ix.add(pid, name, false, mt);
    }
}

fn watch_root(idx: usize) {
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
    let e = engine().unwrap();
    let root_path = e.roots.lock().unwrap()[idx].path.clone();
    let w = wide(&root_path);
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
        util::log(&format!("indeks: nie moge sledzic {}", root_path));
        return;
    };
    let mut buf = vec![0u32; 16 * 1024]; // 64 kB, wyrownane do DWORD
    let mut pending_old: Option<(u32, String)> = None;
    loop {
        let mut bytes = 0u32;
        let ok = unsafe {
            ReadDirectoryChangesW(
                h,
                buf.as_mut_ptr() as *mut _,
                (buf.len() * 4) as u32,
                true,
                FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE,
                Some(&mut bytes),
                None,
                None,
            )
        };
        if ok.is_err() {
            util::log(&format!("indeks: sledzenie {} przerwane: {:?}", root_path, ok));
            break;
        }
        let root_id = e.roots.lock().unwrap()[idx].id;
        if bytes == 0 {
            // przepelnienie bufora - przeskanuj korzen od nowa
            e.ix.remove(root_id);
            if let Some(id) = crawl_root(e, &root_path) {
                e.roots.lock().unwrap()[idx].id = id;
            }
            continue;
        }
        let base = buf.as_ptr() as *const u8;
        let mut off = 0usize;
        loop {
            let info = unsafe { &*(base.add(off) as *const FILE_NOTIFY_INFORMATION) };
            let name_len = info.FileNameLength as usize / 2;
            let name_ptr = info.FileName.as_ptr();
            let rel = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(name_ptr, name_len) });
            if !rel_skipped(e, &rel) {
                match info.Action.0 {
                    1 => add_rel(e, &root_path, root_id, &rel),
                    2 => {
                        let id = resolve(e, root_id, &rel);
                        if id != IX_NONE && id != root_id {
                            e.ix.remove(id);
                        }
                    }
                    4 => {
                        let id = resolve(e, root_id, &rel);
                        pending_old = if id != IX_NONE && id != root_id { Some((id, rel.clone())) } else { None };
                    }
                    5 => {
                        let parent_of = |p: &str| p.rfind('\\').map(|i| util::lower(&p[..i])).unwrap_or_default();
                        let new_name = rel.rsplit('\\').next().unwrap_or(&rel).to_string();
                        match pending_old.take() {
                            Some((old, old_rel)) if parent_of(&old_rel) == parent_of(&rel) => {
                                e.ix.rename(old, &new_name)
                            }
                            Some((old, _)) => {
                                e.ix.remove(old);
                                add_rel(e, &root_path, root_id, &rel);
                            }
                            None => add_rel(e, &root_path, root_id, &rel),
                        }
                    }
                    3 => {
                        // zmiana zawartosci pliku: tylko data (dla "ostatnio zmieniany")
                        let id = resolve(e, root_id, &rel);
                        if id != IX_NONE {
                            unsafe { ix_touch(e.ix.0, id, now_minutes()) };
                        }
                    }
                    _ => {}
                }
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
