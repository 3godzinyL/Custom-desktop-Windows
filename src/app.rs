// Stan aplikacji niezalezny od okna (okno moze zostac odtworzone po restarcie
// Explorera, a folder, historia i przewiniecia zostaja).
use std::collections::HashSet;
use std::sync::Arc;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Storage::FileSystem::{
    FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
    FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
    FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects, INFINITE};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::config::{same_path, Config};
use crate::fsx::{self, Item, COMPUTER};
use crate::icons::{self, IconKind, Icons};
use crate::page::{Layout, Link, LinkStyle, LinkTarget, Page, Rc, Region, Row, SortCol, StatusLine, Target, TreeNode, REGIONS, SCROLL_W};
use crate::render::{scrollbar_thumb, Renderer, Ui};
use crate::profiles;
use crate::search::{self, Find};
use crate::stats::Stats;
use crate::util::{self, cmp_logical, pcwstr, wide};

pub const WM_APP_DIR: u32 = WM_APP + 22;
const SEARCH_FILES: u32 = 400;
const SEARCH_DIRS: u32 = 200;
const WHEEL_STEP: f32 = 100.0;

struct Event(HANDLE);
unsafe impl Send for Event {}
unsafe impl Sync for Event {}
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Sledzenie zmian w biezacym folderze (bez podfolderow).
struct DirWatch {
    stop: Arc<Event>,
}

impl Drop for DirWatch {
    fn drop(&mut self) {
        unsafe {
            let _ = SetEvent(self.stop.0);
        }
    }
}

fn watch_dir(path: &str, notify: HWND) -> Option<DirWatch> {
    if path == COMPUTER {
        return None;
    }
    let w = wide(path);
    let change = unsafe {
        FindFirstChangeNotificationW(
            pcwstr(&w),
            false,
            FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_SIZE
                | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_ATTRIBUTES,
        )
    }
    .ok()?;
    let stop = Arc::new(Event(unsafe { CreateEventW(None, true, false, None) }.ok()?));
    let stop2 = stop.clone();
    let change_raw = change.0 as isize;
    let hwnd = notify.0 as isize;
    std::thread::Builder::new()
        .name("indeks-folder".into())
        .spawn(move || {
            let change = HANDLE(change_raw as *mut _);
            loop {
                let r = unsafe { WaitForMultipleObjects(&[change, stop2.0], false, INFINITE) };
                if r != WAIT_OBJECT_0 {
                    break;
                }
                unsafe {
                    let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_DIR, WPARAM(0), LPARAM(0));
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
                if unsafe { FindNextChangeNotification(change) }.is_err() {
                    break;
                }
            }
            unsafe {
                let _ = FindCloseChangeNotification(change);
            }
        })
        .ok()?;
    Some(DirWatch { stop })
}

pub struct SearchState {
    pub query: String,
    pub files: u32,
    pub dirs: u32,
    saved: [f32; 4],
}

pub struct App {
    pub cfg: Config,
    pub path: String,
    items: Vec<Item>,
    error: Option<String>,
    pub page: Page,
    pub layout: Layout,
    layout_dirty: bool,
    pub renderer: Renderer,
    pub icons: Icons,
    back: Vec<(String, [f32; 4])>,
    fwd: Vec<(String, [f32; 4])>,
    table_sort: (SortCol, bool),
    pub scroll: [f32; 4],
    pub view_w: f32,
    pub view_h: f32,
    pub dpi: f32,
    pub hover: Target,
    pub pressed: Target,
    pub selected: Target,
    pub search: Option<SearchState>,
    watch: Option<DirWatch>,
    pub notify: HWND,
    /// Przeciaganie suwaka: (region, y kursora na starcie, gora suwaka na starcie).
    pub drag: Option<(Region, f32, f32)>,
    /// Rosnie przy kazdej przebudowie strony (wtedy odmalowujemy calosc).
    pub gen: u64,
    expanded: HashSet<String>,
    /// Folder, wokol ktorego zbudowane jest drzewo. Klik w drzewie zmienia tylko
    /// zawartosc (path); drzewo zostaje w miejscu, a wybrany wezel jest podswietlony.
    tree_anchors: Vec<String>,
    /// Biezacy folder jest w drzewie rozwiniety, chyba ze ktos go zwinal.
    collapsed: HashSet<String>,
    /// Folder z ostatniego klikniecia (dla dwukliku = nowa karta).
    last_click: Option<(String, std::time::Instant)>,
    /// Filtr "projekty danego jezyka na dysku" (klik w panelu projektow).
    pub kind: Option<usize>,
    /// Panel projektow i miejsca (prawa czesc = okno silnika C#).
    pub disk: bool,
    /// Ten widok ma przejac panel przy najblizszym malowaniu.
    disk_claim: bool,
    /// Panel zabralo inne okno Indeksu.
    panel_lost: bool,
    /// Ostatnie polozenie panelu: (widok, x, y, szer, wys) w pikselach.
    panel_rect: Option<(isize, i32, i32, i32, i32)>,
    pub settings_open: bool,
    /// Pulpit: rozwiniety wiersz „bez optymalizacji" pod paskiem wydajnosci.
    pub opt_reveal: bool,
    /// Przeciagany suwak w ustawieniach.
    pub slider_drag: Option<usize>,
    /// Okna odpowiadajace przyciskom aplikacji.
    app_hwnds: Vec<isize>,
    pub stats: Stats,
}

pub fn group_digits(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{a0}');
        }
        out.push(c);
    }
    out
}

/// Polska odmiana: 1 folder, 2 foldery, 5 folderow.
fn pl(n: usize, one: &str, few: &str, many: &str) -> String {
    let w = if n == 1 {
        one
    } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
        few
    } else {
        many
    };
    format!("{} {}", group_digits(n as u32), w)
}

impl App {
    pub fn new(cfg: Config, notify: HWND) -> windows::core::Result<App> {
        let path = cfg.home.clone();
        let mut app = App {
            cfg,
            path,
            items: Vec::new(),
            error: None,
            page: Page {
                search: false,
                drives: Vec::new(),
                explorer: Link { text: String::new(), icon: None, target: LinkTarget::Explorer, style: LinkStyle::Link },
                favs: Vec::new(),
                tabs: Vec::new(),
                actions: Vec::new(),
                tree_title: String::new(),
                tree: Vec::new(),
                title: String::new(),
                subtitle: String::new(),
                langs: Vec::new(),
                head_info: String::new(),
                folders_note: String::new(),
                folders_title: String::new(),
                disk: false,
                folders: Vec::new(),
                files: Vec::new(),
                table: Vec::new(),
                message: None,
                status: StatusLine::default(),
                apps: Vec::new(),
                scan: None,
                kinds: Vec::new(),
                kind_active: None,
                settings: None,
                opt_reveal: false,
                opt_apps: Vec::new(),
            },
            layout: Layout::default(),
            layout_dirty: true,
            renderer: Renderer::new()?,
            icons: Icons::new(notify),
            back: Vec::new(),
            fwd: Vec::new(),
            table_sort: (SortCol::Name, true),
            scroll: [0.0; 4],
            view_w: 1200.0,
            view_h: 800.0,
            dpi: 96.0,
            hover: Target::None,
            pressed: Target::None,
            selected: Target::None,
            search: None,
            watch: None,
            notify,
            drag: None,
            gen: 0,
            expanded: HashSet::new(),
            tree_anchors: Vec::new(),
            collapsed: HashSet::new(),
            last_click: None,
            kind: None,
            disk: false,
            disk_claim: false,
            panel_lost: false,
            panel_rect: None,
            settings_open: false,
            opt_reveal: false,
            slider_drag: None,
            app_hwnds: Vec::new(),
            stats: Stats::new(),
        };
        app.load(true);
        Ok(app)
    }

    // ---------- dane ----------

    fn load(&mut self, rewatch: bool) {
        match fsx::list_dir(&self.path, self.cfg.show_protected) {
            Ok(items) => {
                self.items = items;
                self.error = None;
            }
            Err(e) => {
                self.items.clear();
                self.error = Some(format!("Nie można otworzyć folderu: {}", e.trim()));
            }
        }
        self.items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| cmp_logical(&a.name, &b.name)));
        if rewatch {
            self.watch = None;
            self.watch = watch_dir(&self.path, self.notify);
        }
        self.rebuild();
    }

    /// Odswiezenie po zmianie na dysku; nic nie robi, gdy lista sie nie zmienila.
    pub fn reload_if_changed(&mut self) -> bool {
        if self.search.is_some() {
            return false;
        }
        let Ok(mut items) = fsx::list_dir(&self.path, self.cfg.show_protected) else { return false };
        let key = |v: &Vec<Item>| -> Vec<(String, u64, u64)> {
            let mut k: Vec<_> = v.iter().map(|i| (i.name.clone(), i.size, i.mtime)).collect();
            k.sort();
            k
        };
        if key(&items) == key(&self.items) {
            return false;
        }
        let sel = self.selected_path();
        items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| cmp_logical(&a.name, &b.name)));
        self.items = items;
        self.rebuild();
        self.selected = sel.map(|p| self.target_for_path(&p)).unwrap_or(Target::None);
        true
    }

    fn make_row(&self, item: Item, full: String, is_drive: bool) -> Row {
        let display = if is_drive {
            match &item.label {
                Some(l) => format!("{}/ ({})", item.name.trim_end_matches('\\'), l),
                None => format!("{}/", item.name.trim_end_matches('\\')),
            }
        } else if item.is_dir {
            format!("{}/", item.name)
        } else {
            item.name.clone()
        };
        let icon = if is_drive { IconKind::Drive } else { icons::kind_for(&item.name, &item.ext(), item.is_dir, &full) };
        let size_s = if item.is_dir && !is_drive { String::new() } else { fsx::fmt_size(item.size) };
        let folder_s = item.folder.as_deref().map(fsx::display_path).unwrap_or_default();
        Row {
            date_s: fsx::fmt_time(item.mtime),
            size_s,
            folder_s,
            display,
            icon,
            tech: None,
            ago: String::new(),
            full,
            item,
            name_w: -1.0,
            folder_w: -1.0,
            size_w: 0.0,
            date_w: 0.0,
        }
    }

    /// Podfoldery do drzewa: z indeksu C++ (bez dysku), a poza indeksem z dysku.
    fn tree_dirs(&self, dir: &str) -> Vec<String> {
        let mut v = match search::child_dirs(dir) {
            Some(v) => v,
            None => fsx::list_dir(dir, self.cfg.show_protected)
                .map(|items| items.into_iter().filter(|i| i.is_dir).map(|i| i.name).collect())
                .unwrap_or_default(),
        };
        v.sort_by(|a, b| cmp_logical(a, b));
        v
    }

    /// Czy wezel drzewa pokazuje wszystkie podfoldery: kotwice domyslnie tak
    /// (chyba ze zwiniete), pozostale tylko po kliknieciu [+].
    fn tree_open(&self, key: &str, anchors: &[String]) -> bool {
        if anchors.iter().any(|a| a == key) {
            !self.collapsed.contains(key)
        } else {
            self.expanded.contains(key)
        }
    }

    fn tree_children(&self, dir: &str, depth: u32, guides: u64, anchors: &[String], here: &str, out: &mut Vec<TreeNode>) {
        let ldir = util::lower(dir.trim_end_matches('\\'));
        let full_open = self.tree_open(&ldir, anchors);
        let names: Vec<String> = if full_open {
            self.tree_dirs(dir)
        } else {
            // nastepne czlony sciezek kotwic pod `dir` - prawdziwa wielkosc liter ze sciezek
            let prefix = format!("{}\\", ldir);
            let mut v: Vec<String> = Vec::new();
            for a in self.anchor_paths() {
                if util::lower(a.trim_end_matches('\\')).starts_with(&prefix) {
                    if let Some(s) = a.trim_end_matches('\\').split('\\').nth(depth as usize) {
                        if !v.iter().any(|x| util::lower(x) == util::lower(s)) {
                            v.push(s.to_string());
                        }
                    }
                }
            }
            v.sort_by(|a, b| cmp_logical(a, b));
            v
        };
        let n = names.len();
        for (k, name) in names.into_iter().enumerate() {
            if out.len() > 4000 {
                return;
            }
            let full = fsx::join(dir, &name);
            let lf = util::lower(&full);
            let last = k + 1 == n;
            let below = anchors.iter().any(|a| a.starts_with(&format!("{}\\", lf)));
            let open = self.tree_open(&lf, anchors);
            out.push(TreeNode {
                name,
                path: full.clone(),
                depth,
                icon: IconKind::Folder,
                toggle: Some(open),
                current: here == lf,
                note: String::new(),
                guides,
                last,
                tech: self.tree_tech(&full),
            });
            if below || open {
                let g = if last { guides } else { guides | (1u64 << (depth - 1)) };
                self.tree_children(&full, depth + 1, g, anchors, here, out);
            }
        }
    }

/// Jezyk folderu do podpisu w drzewie - bez folderow nad domowym (Pulpit, profil:
    /// to zbiory projektow, nie projekty).
    fn tree_tech(&self, path: &str) -> Option<usize> {
        let lp = util::lower(path.trim_end_matches('\\'));
        let home = util::lower(self.cfg.home.trim_end_matches('\\'));
        if home == lp || home.starts_with(&format!("{}\\", lp)) {
            return None;
        }
        match profiles::get(path) {
            Some(Some(p)) => p.tech,
            _ => None,
        }
    }

    /// Foldery, wokol ktorych zbudowane jest drzewo (domyslnie biezacy).
    fn anchor_paths(&self) -> Vec<&str> {
        if self.tree_anchors.is_empty() {
            vec![self.path.as_str()]
        } else {
            self.tree_anchors.iter().map(|s| s.as_str()).collect()
        }
    }

    fn build_tree(&self) -> Vec<TreeNode> {
        // ksztalt drzewa wg kotwic (kazdy odwiedzony folder spoza drzewa dokłada swoja galaz,
        // nic sie nie zwija), podswietlenie wg biezacego folderu
        let anchors: Vec<String> = self.anchor_paths().iter().map(|a| util::lower(a.trim_end_matches('\\'))).collect();
        let here = util::lower(self.path.trim_end_matches('\\'));
        let mut out = Vec::new();
        let drives: Vec<String> = self.stats.drives.iter().map(|d| d.root.clone()).collect();
        for d in drives {
            let ld = util::lower(d.trim_end_matches('\\'));
            let active = anchors.iter().any(|a| *a == ld || a.starts_with(&format!("{}\\", ld)));
            let open = self.tree_open(&ld, &anchors);
            out.push(TreeNode {
                name: d.clone(),
                path: d.clone(),
                depth: 0,
                icon: IconKind::Drive,
                toggle: Some(open),
                current: here == ld,
                note: String::new(),
                guides: 0,
                last: true,
                tech: None,
            });
            if active || open {
                self.tree_children(&d, 1, 0, &anchors, &here, &mut out);
            }
        }
        out
    }

    /// Buduje strone z biezacego stanu.
    pub fn rebuild(&mut self) {
        let here = |p: &str| self.search.is_none() && same_path(p, &self.path);
        let style = |cur: bool| if cur { LinkStyle::Current } else { LinkStyle::Link };
        // gora: dyski, Eksplorator, ulubione (same dyski sa juz po lewej, wiec ich nie dublujemy)
        let drives: Vec<Link> = self
            .stats
            .drives
            .iter()
            .enumerate()
            .map(|(i, d)| Link { text: d.root.clone(), icon: Some(IconKind::Drive), target: LinkTarget::Drive(i), style: style(here(&d.root)) })
            .collect();
        let explorer = Link { text: "[Otwórz Eksplorator]".into(), icon: None, target: LinkTarget::Explorer, style: LinkStyle::Link };
        let mut favs = Vec::new();
        for (i, p) in self.cfg.pins.iter().enumerate() {
            if p.trim_end_matches('\\').len() <= 2 {
                continue;
            }
            favs.push(Link { text: fsx::last_name(p), icon: Some(IconKind::Folder), target: LinkTarget::Fav(i), style: style(here(p)) });
        }
        let tabs: Vec<Link> = self
            .tab_paths()
            .iter()
            .enumerate()
            .map(|(i, p)| Link {
                text: if p.trim_end_matches('\\').len() <= 2 { p.clone() } else { fsx::last_name(p) },
                icon: Some(IconKind::Folder),
                target: LinkTarget::Tab(i),
                style: style(here(p)),
            })
            .collect();
        let mut actions = Vec::new();
        let page_tree;
        let tree_title;
        let (title, subtitle);
        let (mut folders, mut files, mut table) = (Vec::new(), Vec::new(), Vec::new());
        let mut message = None;
        if let Some(s) = &self.search {
            actions.push(Link { text: "[Zamknij wyszukiwanie]".into(), icon: None, target: LinkTarget::CloseSearch, style: LinkStyle::Link });
            title = "Wyszukiwanie".to_string();
            let st = search::status();
            subtitle = format!(
                "Szukaj: {}|    {} · {}{}",
                s.query,
                pl(s.files as usize, "plik", "pliki", "plików"),
                pl(s.dirs as usize, "folder", "foldery", "folderów"),
                if st.ready { String::new() } else { " · indeksowanie trwa…".into() }
            );
            tree_title = format!("Foldery ({})", s.dirs);
            let (dirs, _) = search::search(&s.query, Find::Dirs, SEARCH_DIRS);
            page_tree = dirs
                .into_iter()
                .map(|h| TreeNode {
                    name: fsx::last_name(&h.path),
                    note: fsx::parent(&h.path).map(|p| fsx::display_path(&p)).unwrap_or_default(),
                    path: h.path,
                    depth: 0,
                    icon: IconKind::Folder,
                    toggle: None,
                    current: false,
                    guides: 0,
                    last: true,
                    tech: None,
                })
                .collect::<Vec<_>>();
            let (hits, _) = search::search(&s.query, Find::Files, SEARCH_FILES);
            for h in hits {
                let Some((is_dir, size, mtime, attrs)) = fsx::stat(&h.path) else { continue };
                let item = Item {
                    name: fsx::last_name(&h.path),
                    is_dir,
                    size,
                    mtime,
                    attrs,
                    label: None,
                    folder: fsx::parent(&h.path).filter(|p| !p.is_empty()),
                };
                table.push(self.make_row(item, h.path, false));
            }
            let (col, asc) = self.table_sort;
            if col != SortCol::Name || !asc {
                sort_rows(&mut table, col, asc);
            }
        } else if self.disk {
            // panel miejsca: z lewej wszystkie projekty od najwiekszych, reszte rysuje C#
            actions.push(Link { text: "[Zamknij panel]".into(), icon: None, target: LinkTarget::CloseSearch, style: LinkStyle::Link });
            let list: Vec<_> = crate::katalog::projects().unwrap_or_default().into_iter().filter(|p| self.kind.map(|k| p.kind == k).unwrap_or(true)).collect();
            let total: u64 = list.iter().map(|p| p.bytes).sum();
            title = "Projekty na dysku".to_string();
            subtitle = if list.is_empty() && crate::katalog::scanning() {
                "Silnik C++ przeszukuje dysk…".to_string()
            } else {
                format!("{}  ·  {} bez śmieci  ·  klik pokazuje szczegóły", pl(list.len(), "projekt", "projekty", "projektów"), fsx::fmt_size(total))
            };
            tree_title = "Struktura".to_string();
            page_tree = self.build_tree();
            for p in list {
                let item = Item { name: fsx::last_name(&p.path), is_dir: true, size: p.bytes, mtime: 0, attrs: 0, label: None, folder: fsx::parent(&p.path) };
                let mut row = self.make_row(item, p.path, false);
                row.tech = Some(p.kind);
                row.icon = profiles::icon(p.kind);
                row.ago = profiles::ago(p.newest);
                folders.push(row);
            }
        } else if let Some(kind) = self.kind {
            // projekty jednego jezyka z calego dysku
            actions.push(Link { text: "[Zamknij filtr]".into(), icon: None, target: LinkTarget::CloseSearch, style: LinkStyle::Link });
            let list = profiles::projects_of(kind);
            title = format!("Projekty: {}", profiles::LANGS[kind].name);
            subtitle = format!("{} na dysku  ·  klik otwiera projekt", pl(list.len(), "projekt", "projekty", "projektów"));
            tree_title = "Struktura".to_string();
            page_tree = self.build_tree();
            for p in list {
                let item = Item { name: fsx::last_name(&p), is_dir: true, size: 0, mtime: 0, attrs: 0, label: None, folder: fsx::parent(&p) };
                folders.push(self.make_row(item, p, false));
            }
        } else {
            if self.path != COMPUTER {
                let text = if self.cfg.is_pinned(&self.path) { "[− Usuń z ulubionych]" } else { "[+ Dodaj do ulubionych]" };
                actions.push(Link { text: text.into(), icon: None, target: LinkTarget::ToggleFav, style: LinkStyle::Link });
            }
            title = if self.path == COMPUTER {
                "Ten komputer".to_string()
            } else if self.path.trim_end_matches('\\').len() <= 2 {
                self.path.clone()
            } else {
                fsx::last_name(&self.path)
            };
            tree_title = "Struktura".to_string();
            page_tree = self.build_tree();
            let is_drive = self.path == COMPUTER;
            for it in self.items.clone() {
                let full = if is_drive { it.name.clone() } else { fsx::join(&self.path, &it.name) };
                let row = self.make_row(it, full, is_drive);
                if row.item.is_dir {
                    folders.push(row);
                } else {
                    files.push(row);
                }
            }
            subtitle = format!(
                "{}    ·  {}, {}",
                fsx::display_path(&self.path),
                pl(folders.len(), "folder", "foldery", "folderów"),
                pl(files.len(), "plik", "pliki", "plików")
            );
            if let Some(e) = &self.error {
                message = Some((e.clone(), true));
            }
        }
        let status = std::mem::take(&mut self.page.status);
        let folders_title = match (self.disk, self.kind) {
            (_, Some(k)) => format!("Projekty i foldery ({})", profiles::LANGS[k].name),
            (true, None) => "Projekty i foldery (wszystkie)".to_string(),
            (false, None) => format!("Projekty i foldery ({})", title),
        };
        self.page = Page {
            folders_title,
            disk: self.disk,
            search: self.search.is_some(),
            drives,
            explorer,
            favs,
            tabs,
            actions,
            tree_title,
            tree: page_tree,
            title,
            subtitle,
            langs: Vec::new(),
            head_info: String::new(),
            folders_note: String::new(),
            folders,
            files,
            table,
            message,
            status,
            apps: std::mem::take(&mut self.page.apps),
            scan: None,
            kinds: Vec::new(),
            kind_active: self.kind,
            // okno ustawien musi przetrwac przebudowe strony (wyniki projektow, zmiany w folderze...)
            settings: if self.settings_open { Some(self.settings_items()) } else { None },
            opt_reveal: self.opt_reveal,
            opt_apps: Vec::new(),
        };
        self.fill_status();
        self.fill_kinds();
        if self.settings_open {
            self.page.settings = Some(self.settings_items());
        }
        if self.search.is_none() {
            // silnik projektow: technologia i data zmiany kazdego podfolderu
            let mut want: Vec<String> = if self.disk { Vec::new() } else { self.page.folders.iter().map(|r| r.full.clone()).collect() };
            want.extend(self.page.tree.iter().filter(|n| n.toggle.is_some()).map(|n| n.path.clone()));
            if self.path != COMPUTER {
                want.push(self.path.clone());
            }
            profiles::request(&want);
            self.fill_profiles();
        }
        self.layout_dirty = true;
        self.gen += 1;
        self.hover = Target::None;
        self.icons.forget_per_file();
        self.renderer.trim_bitmaps();
    }

    /// Wpisuje gotowe profile (silnik projektow) do wierszy, naglowka i podsumowania.
    /// Zwraca true, gdy cos sie zmienilo.
    pub fn fill_profiles(&mut self) -> bool {
        if self.page.search {
            return false;
        }
        let mut changed = false;
        let mut counts: Vec<(usize, u32)> = Vec::new();
        let mut plain = 0u32;
        // w panelu dysku technologie i daty przychodza z katalogu
        let rows = if self.disk { &mut [][..] } else { &mut self.page.folders[..] };
        for r in rows.iter_mut() {
            let (tech, ago) = match profiles::get(&r.full) {
                Some(Some(p)) => {
                    match p.tech {
                        Some(t) => match counts.iter_mut().find(|c| c.0 == t) {
                            Some(c) => c.1 += 1,
                            None => counts.push((t, 1)),
                        },
                        None => plain += 1,
                    }
                    let when = if p.newest > 0 { p.newest } else { search::minutes(r.item.mtime) };
                    (p.tech, profiles::ago(when))
                }
                // poza indeksem: data samego folderu
                Some(None) => (None, profiles::ago(search::minutes(r.item.mtime))),
                None => (None, String::new()),
            };
            if r.ago != ago || r.tech != tech {
                changed = true;
            }
            // ikona technologii zamiast zwyklego folderu
            r.icon = match tech {
                Some(t) => profiles::icon(t),
                None => crate::icons::IconKind::Folder,
            };
            r.tech = tech;
            r.ago = ago;
        }
        // podpisy jezykow w drzewie
        let techs: Vec<Option<usize>> = self.page.tree.iter().map(|n| if n.toggle.is_some() { self.tree_tech(&n.path) } else { None }).collect();
        for (n, t) in self.page.tree.iter_mut().zip(techs) {
            if n.tech != t {
                n.tech = t;
                changed = true;
                self.layout_dirty = true;
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1));
        let mut note: Vec<String> = counts.iter().take(6).map(|(t, n)| format!("{} {}", profiles::LANGS[*t].name, n)).collect();
        if plain > 0 && !counts.is_empty() {
            note.push(format!("inne {}", plain));
        }
        let note = note.join(" · ");
        if note != self.page.folders_note {
            self.page.folders_note = note;
            changed = true;
        }
        // naglowek: jezyki biezacego folderu (pasek jak na GitHubie)
        if self.kind.is_some() || self.disk {
            self.page.langs.clear();
            self.page.head_info.clear();
        } else if let Some(Some(p)) = profiles::get(&self.path) {
            let total: u32 = p.langs.iter().map(|l| l.1).sum();
            let mut langs: Vec<(String, u32, f32)> = Vec::new();
            let mut rest = 0u32;
            for (i, (lang, n)) in p.langs.iter().enumerate() {
                if i < 6 {
                    let l = &profiles::LANGS[*lang];
                    langs.push((l.name.to_string(), l.color, *n as f32 / total.max(1) as f32));
                } else {
                    rest += n;
                }
            }
            if rest > 0 {
                langs.push(("inne".into(), 0x777777, rest as f32 / total.max(1) as f32));
            }
            let mut info = Vec::new();
            if let Some(t) = p.tech_name() {
                info.push(t.to_string());
            }
            if p.git {
                info.push("repo git".into());
            }
            info.push(pl(p.files as usize, "plik", "pliki", "plików"));
            if p.newest > 0 {
                info.push(format!("zmieniony {}", profiles::ago(p.newest)));
            }
            let info = info.join(" · ");
            if info != self.page.head_info || langs.len() != self.page.langs.len() {
                changed = true;
            }
            self.page.head_info = info;
            self.page.langs = langs;
        }
        if changed {
            self.layout_dirty = true;
        }
        changed
    }

    fn fill_status(&mut self) {
        let s = &self.stats;
        let opt = crate::optymalizator::savings();
        // rozwinieta zakladka optymalizacji: lista aplikacji z silnika; inna liczba wierszy =
        // nowy uklad (wysokosc zakladki, przyciski) - inaczej po powrocie na pulpit byla nieaktualna
        let rows_before = self.page.opt_apps.len();
        self.page.opt_apps = crate::optymalizator::usage()
            .into_iter()
            .map(|a| crate::page::OptApp {
                pid: a.pid,
                hwnd: a.hwnd,
                name: {
                    let n = a.exe.rsplit(['\\', '/']).next().unwrap_or(&a.exe).to_string();
                    let mut n = if a.procs > 1 { format!("{}  · {} proc.", n, a.procs) } else { n };
                    if a.hidden {
                        n.push_str("  · w zasobniku");
                    }
                    n
                },
                blocked: a.blocked,
                next_in: a.next_in,
                gpu: a.gpu,
                active: a.active,
                before_ram: a.before_ram,
                saved_ram: a.saved_ram,
                saved_cpu: a.saved_cpu,
                saved_gpu: a.saved_gpu,
                ws: a.ws,
                cpu: a.cpu,
                level: a.level,
                audio: a.audio,
            })
            .collect();
        if self.opt_reveal && self.page.opt_apps.len() != rows_before {
            self.layout_dirty = true;
            self.gen += 1;
        }
        self.page.status = StatusLine {
            drives: s.drives.iter().map(|d| (d.root.clone(), d.total, d.free)).collect(),
            cpu: s.cpu,
            gpu: s.gpu,
            ram_used: s.ram_used,
            ram_total: s.ram_total,
            own: s.own,
            own_cpu: s.own_cpu,
            opt_on: self.cfg.optimize,
            opt_saved: opt.saved_ram,
            opt_count: opt.count,
            opt_saved_cpu: opt.saved_cpu,
            opt_saved_gpu: opt.saved_gpu,
        };
    }

    /// Odswieza dane paska stanu. Zwraca true, gdy zmienila sie lista dyskow
    /// (wtedy cala strona zostala przebudowana).
    pub fn update_stats(&mut self) -> bool {
        let before: Vec<String> = self.stats.drives.iter().map(|d| d.root.clone()).collect();
        self.stats.update();
        let after: Vec<String> = self.stats.drives.iter().map(|d| d.root.clone()).collect();
        if before != after {
            self.rebuild();
            return true;
        }
        self.fill_status();
        false
    }

    pub fn ensure_layout(&mut self) {
        if self.layout_dirty || (self.layout.width - self.view_w).abs() > 0.5 || (self.layout.height - self.view_h).abs() > 0.5 {
            self.layout = self.renderer.layout(&mut self.page, self.view_w, self.view_h, self.dpi);
            self.layout_dirty = false;
        }
        self.clamp_scroll();
    }

    pub fn ui(&self) -> Ui {
        Ui {
            scroll: self.scroll,
            hover: self.hover,
            selected: self.selected,
            drag: self.drag.map(|d| d.0),
            index_count: search::status().count,
        }
    }

    /// Pozycja ulubionego `i` (jak na ekranie) w `cfg.pins`. Lista pomija przypiete dyski,
    /// wiec indeksy sie roznia - bez tego klik otwieral sasiedni folder.
    pub fn fav_pin(&self, i: usize) -> Option<usize> {
        match self.page.favs.get(i).map(|f| &f.target) {
            Some(LinkTarget::Fav(k)) => Some(*k),
            _ => None,
        }
    }

    /// Pulpit: scroll w pustym miejscu rozwija / chowa zakladke optymalizacji.
    pub fn toggle_opt_reveal(&mut self, show: bool) -> bool {
        if self.opt_reveal == show || !self.cfg.optimize {
            return false;
        }
        self.opt_reveal = show;
        self.page.opt_reveal = show;
        self.layout_dirty = true;
        self.gen += 1;
        true
    }

    // ---------- nawigacja ----------

    pub fn navigate(&mut self, path: &str) {
        if self.search.is_none() && same_path(path, &self.path) && path.is_empty() == self.path.is_empty() {
            return;
        }
        let saved = self.search.as_ref().map(|s| s.saved).unwrap_or(self.scroll);
        self.back.push((self.path.clone(), saved));
        self.fwd.clear();
        self.go(path.to_string(), [0.0; 4]);
    }

    fn go(&mut self, path: String, scroll: [f32; 4]) {
        // Struktura nie przeskakuje (drzewo, karty, ulubione, projekty na dysku, wstecz):
        // folder widoczny w drzewie tylko sie podswietla; niewidoczny dokłada swoja galaz,
        // a reszta drzewa i przewiniecie zostaja - przewijamy tylko tyle, zeby go bylo widac.
        let in_tree = self.build_tree().iter().any(|n| same_path(&n.path, &path));
        let tree_scroll = self.scroll[Region::Tree as usize];
        if self.tree_anchors.is_empty() {
            self.tree_anchors.push(self.path.clone());
        }
        if !in_tree {
            self.tree_anchors.retain(|a| !same_path(a, &path));
            self.tree_anchors.push(path.clone());
            if self.tree_anchors.len() > 16 {
                self.tree_anchors.remove(0);
            }
        }
        self.search = None;
        self.kind = None;
        self.disk = false;
        self.path = path;
        self.scroll = scroll;
        self.scroll[Region::Tree as usize] = tree_scroll;
        self.selected = Target::None;
        self.load(true);
        self.ensure_layout();
        if let Some(i) = self.page.tree.iter().position(|n| n.current) {
            self.ensure_visible(Target::Tree(i));
        }
    }

    pub fn back(&mut self) {
        if self.disk {
            self.set_disk(false);
            return;
        }
        if self.kind.is_some() {
            self.set_kind(None);
            return;
        }
        if self.search.is_some() {
            self.close_search();
            return;
        }
        if let Some((p, s)) = self.back.pop() {
            self.fwd.push((self.path.clone(), self.scroll));
            self.go(p, s);
        }
    }

    pub fn forward(&mut self) {
        if let Some((p, s)) = self.fwd.pop() {
            self.back.push((self.path.clone(), self.scroll));
            self.go(p, s);
        }
    }

    pub fn up(&mut self) {
        if let Some(parent) = fsx::parent(&self.path) {
            let from = self.path.clone();
            self.navigate(&parent);
            let t = self.target_for_path(&from);
            self.select_top(t);
        }
    }

    pub fn home(&mut self) {
        let h = self.cfg.home.clone();
        self.navigate(&h);
    }

    /// Przejscie do folderu i zaznaczenie w nim elementu.
    /// Folder - wejdz; plik - wejdz do jego folderu i zaznacz go (np. nowy plik z Pobranych).
    pub fn open_path(&mut self, path: &str) {
        match fsx::stat(path) {
            Some((false, ..)) => {
                if let Some(folder) = fsx::parent(path) {
                    self.reveal(&folder, path);
                }
            }
            _ => self.navigate(path),
        }
    }

    pub fn reveal(&mut self, folder: &str, full: &str) {
        self.navigate(folder);
        let t = self.target_for_path(full);
        self.select_top(t);
    }

    pub fn current_dir(&self) -> Option<String> {
        if self.path == COMPUTER {
            None
        } else {
            Some(self.path.clone())
        }
    }

    fn target_for_path(&self, full: &str) -> Target {
        if let Some(i) = self.page.folders.iter().position(|r| same_path(&r.full, full)) {
            return Target::Folder(i);
        }
        if let Some(i) = self.page.files.iter().position(|r| same_path(&r.full, full)) {
            return Target::File(i);
        }
        Target::None
    }

    pub fn path_of(&self, t: Target) -> Option<(String, bool)> {
        match t {
            Target::Folder(i) => self.page.folders.get(i).map(|r| (r.full.clone(), true)),
            Target::File(i) => self.page.files.get(i).map(|r| (r.full.clone(), false)),
            Target::Row(i) | Target::RowFolder(i) => self.page.table.get(i).map(|r| (r.full.clone(), r.item.is_dir)),
            Target::Tree(i) | Target::TreeToggle(i) => self.page.tree.get(i).map(|n| (n.path.clone(), true)),
            _ => None,
        }
    }

    fn selected_path(&self) -> Option<String> {
        self.path_of(self.selected).map(|p| p.0)
    }

    // ---------- wyszukiwanie ----------

    pub fn search_input(&mut self, ch: char) {
        match &mut self.search {
            Some(s) => s.query.push(ch),
            None => {
                if ch == ' ' {
                    return;
                }
                self.search = Some(SearchState { query: ch.to_string(), files: 0, dirs: 0, saved: self.scroll });
            }
        }
        self.run_search();
    }

    pub fn search_backspace(&mut self) {
        let empty = match &mut self.search {
            Some(s) => {
                s.query.pop();
                s.query.trim().is_empty()
            }
            None => return,
        };
        if empty {
            self.close_search();
        } else {
            self.run_search();
        }
    }

    pub fn run_search(&mut self) {
        if let Some(s) = &mut self.search {
            s.files = search::search(&s.query, Find::Files, 0).1;
            s.dirs = search::search(&s.query, Find::Dirs, 0).1;
        }
        self.scroll[Region::Tree as usize] = 0.0;
        self.scroll[Region::Table as usize] = 0.0;
        self.rebuild();
        self.selected = if self.page.table.is_empty() { Target::None } else { Target::Row(0) };
    }

    pub fn close_search(&mut self) {
        if let Some(s) = self.search.take() {
            self.scroll = s.saved;
            self.selected = Target::None;
            self.rebuild();
        }
    }

    pub fn sort_table(&mut self, col: SortCol) {
        self.table_sort = if self.table_sort.0 == col { (col, !self.table_sort.1) } else { (col, matches!(col, SortCol::Name | SortCol::Folder)) };
        let (col, asc) = self.table_sort;
        sort_rows(&mut self.page.table, col, asc);
        self.layout_dirty = true;
        self.gen += 1;
        self.selected = Target::None;
    }

    // ---------- ulubione / drzewo ----------

    pub fn toggle_fav(&mut self, path: &str) {
        self.cfg.toggle_pin(path);
        self.rebuild();
    }

    pub fn toggle_tree(&mut self, i: usize) {
        let Some(n) = self.page.tree.get(i) else { return };
        let key = util::lower(n.path.trim_end_matches('\\'));
        let is_anchor = self.anchor_paths().iter().any(|a| util::lower(a.trim_end_matches('\\')) == key);
        let set = if is_anchor { &mut self.collapsed } else { &mut self.expanded };
        if !set.remove(&key) {
            set.insert(key);
        }
        self.rebuild();
    }

    // ---------- karty ----------

    /// Karty: pierwsza zawsze Pulpit (stala, bez [x]), potem karty uzytkownika.
    pub fn tab_paths(&self) -> Vec<String> {
        let desk = crate::config::desktop_dir();
        let mut v = vec![desk.clone()];
        v.extend(self.cfg.tabs.iter().filter(|t| !same_path(t, &desk)).cloned());
        v
    }

    pub fn add_tab(&mut self, path: &str) {
        if path.is_empty() || self.tab_paths().iter().any(|t| same_path(t, path)) {
            return;
        }
        self.cfg.tabs.push(path.to_string());
        if self.cfg.tabs.len() > 16 {
            self.cfg.tabs.remove(0);
        }
        self.cfg.save();
        self.rebuild();
    }

    pub fn close_tab(&mut self, i: usize) {
        let Some(p) = self.tab_paths().get(i).cloned().filter(|_| i > 0) else { return };
        if let Some(k) = self.cfg.tabs.iter().position(|t| same_path(t, &p)) {
            self.cfg.tabs.remove(k);
            self.cfg.save();
            self.rebuild();
        }
    }

    /// Dwuklik: folder z pierwszego klikniecia (drzewo albo lista) trafia do kart.
    /// Pierwsze klikniecie juz go otworzylo, wiec cel pod kursorem mogl sie zmienic.
    /// Dwuklik na folderze: zwraca folder do otwarcia w nowym oknie. Pierwsze klikniecie
    /// juz do niego weszlo - cofamy to, zeby ten widok zostal tam, gdzie byl.
    pub fn double_click(&mut self, t: Target) -> Option<String> {
        let recent = self.last_click.take().filter(|(_, at)| at.elapsed().as_millis() < 700).map(|(p, _)| p);
        let path = recent.or_else(|| match t {
            Target::Tree(_) | Target::Folder(_) => self.path_of(t).filter(|p| p.1).map(|p| p.0),
            _ => None,
        })?;
        if same_path(&self.path, &path) && !self.back.is_empty() {
            self.back();
        }
        Some(path)
    }

    // ---------- aktywacja ----------

    /// Klikniecie: folder - wejdz, plik - otworz (przez Explorera, jak z pulpitu).
    pub fn activate(&mut self, t: Target, in_explorer: bool) {
        match t {
            Target::Fav(i) => {
                if let Some(p) = self.fav_pin(i).and_then(|k| self.cfg.pins.get(k).cloned()) {
                    self.navigate(&p);
                }
            }
            Target::Drive(i) => {
                if let Some(d) = self.stats.drives.get(i).map(|d| d.root.clone()) {
                    self.navigate(&d);
                }
            }
            Target::Explorer => crate::shell::explorer_folder(&self.current_dir().unwrap_or_default()),
            Target::Downloads => {
                if let Some(p) = crate::config::downloads_dir() {
                    if in_explorer { crate::shell::explorer_folder(&p) } else { self.navigate(&p) }
                }
            }
            Target::Tab(i) => {
                if let Some(p) = self.tab_paths().get(i).cloned() {
                    self.navigate(&p);
                }
            }
            Target::TabClose(i) => self.close_tab(i),
            Target::App(i) => self.activate_app(i),
            Target::Kind(i) => {
                let k = self.page.kinds.get(i).map(|k| k.0);
                self.set_kind(k);
            }
            Target::Gear => {
                let open = !self.settings_open;
                self.toggle_settings(open);
            }
            Target::Disk => {
                let on = !self.disk || self.panel_lost;
                self.set_disk(on);
            }
            Target::OptLimit(i) => {
                if let Some(a) = self.page.opt_apps.get(i) {
                    crate::optymalizator::force_limit(a.pid);
                }
            }
            Target::OptMinimize(i) => {
                if let Some(a) = self.page.opt_apps.get(i).filter(|a| a.hwnd != 0) {
                    crate::winlist::minimize(windows::Win32::Foundation::HWND(a.hwnd as *mut _));
                }
            }
            Target::SetItem(i) => {
                if matches!(self.page.settings.as_ref().and_then(|s| s.get(i)), Some((_, crate::page::SetKind::Toggle(_)))) {
                    self.set_setting(i, None);
                }
            }
            Target::SetClose => self.toggle_settings(false),
            Target::Action(i) => match self.page.actions.get(i).map(|a| a.target) {
                Some(LinkTarget::ToggleFav) => {
                    let p = self.path.clone();
                    self.toggle_fav(&p);
                }
                Some(LinkTarget::Explorer) => crate::shell::explorer_folder(&self.current_dir().unwrap_or_default()),
                Some(LinkTarget::CloseSearch) => {
                    if self.disk {
                        self.set_disk(false);
                    } else if self.kind.is_some() {
                        self.set_kind(None);
                    } else {
                        self.close_search();
                    }
                }
                _ => {}
            },
            Target::TreeToggle(i) => self.toggle_tree(i),
            Target::Tree(i) => {
                if let Some(n) = self.page.tree.get(i) {
                    let p = n.path.clone();
                    self.last_click = Some((p.clone(), std::time::Instant::now()));
                    if in_explorer {
                        crate::shell::explorer_folder(&p);
                    } else {
                        self.navigate(&p);
                    }
                }
            }
            Target::RowFolder(i) => {
                if let Some(r) = self.page.table.get(i) {
                    let (folder, full) = (r.item.folder.clone().unwrap_or_default(), r.full.clone());
                    self.reveal(&folder, &full);
                }
            }
            Target::Folder(_) | Target::File(_) | Target::Row(_) => {
                let Some((full, is_dir)) = self.path_of(t) else { return };
                // w panelu dysku klik pokazuje projekt po prawej zamiast wchodzic do folderu
                if self.disk && !in_explorer {
                    self.selected = t;
                    crate::katalog::panel_project(&full);
                    return;
                }
                if is_dir {
                    self.last_click = Some((full.clone(), std::time::Instant::now()));
                }
                match (is_dir, in_explorer) {
                    (true, false) => self.navigate(&full),
                    (true, true) => crate::shell::explorer_folder(&full),
                    (false, false) => crate::shell::open(&full),
                    (false, true) => crate::shell::explorer_select(&full),
                }
            }
            Target::Header(c) => self.sort_table(c),
            _ => {}
        }
    }

    // ---------- przewijanie ----------

    pub fn max_scroll(&self, r: Region) -> f32 {
        (self.layout.content[r as usize] - self.layout.viewport(r).h).max(0.0)
    }

    pub fn clamp_scroll(&mut self) {
        for r in REGIONS {
            self.scroll[r as usize] = self.scroll[r as usize].clamp(0.0, self.max_scroll(r));
        }
    }

    pub fn region_at(&self, x: f32, y: f32) -> Option<Region> {
        let l = &self.layout;
        for (r, f) in [(Region::Tree, l.tree_frame), (Region::Folders, l.folders_frame), (Region::Files, l.files_frame)] {
            if f.contains(x, y) {
                return Some(if r == Region::Files && self.page.search { Region::Table } else { r });
            }
        }
        None
    }

    pub fn wheel(&mut self, r: Region, notches: f32) {
        self.scroll[r as usize] -= notches * WHEEL_STEP;
        self.clamp_scroll();
    }

    pub fn scroll_to_thumb(&mut self, r: Region, thumb_top: f32) {
        if let Some((_, th)) = scrollbar_thumb(&self.layout, &self.ui(), r) {
            let v = self.layout.viewport(r);
            let range = (v.h - th).max(1.0);
            self.scroll[r as usize] = ((thumb_top - v.y) / range).clamp(0.0, 1.0) * self.max_scroll(r);
        }
    }

    // ---------- zaznaczenie z klawiatury ----------

    fn region_of(t: Target) -> Option<Region> {
        match t {
            Target::Tree(_) | Target::TreeToggle(_) => Some(Region::Tree),
            Target::Folder(_) => Some(Region::Folders),
            Target::File(_) => Some(Region::Files),
            Target::Row(_) | Target::RowFolder(_) => Some(Region::Table),
            _ => None,
        }
    }

    /// Prostokat elementu w tresci regionu (bez przewiniecia).
    fn content_rect(&self, t: Target) -> Option<Rc> {
        let l = &self.layout;
        match t {
            Target::Tree(i) | Target::TreeToggle(i) => Some(l.list_row(Region::Tree, i)),
            Target::Folder(i) => Some(l.list_row(Region::Folders, i)),
            Target::File(i) => Some(l.tile(i)),
            Target::Row(i) | Target::RowFolder(i) => Some(l.table_row(i)),
            _ => None,
        }
    }

    /// Prostokat celu w oknie (z przewinieciem) - do odmalowania tylko jego.
    pub fn target_rect(&self, t: Target) -> Option<Rc> {
        let l = &self.layout;
        match t {
            Target::Fav(i) => l.favs.get(i).copied(),
            Target::Action(i) => l.actions.get(i).copied(),
            Target::Drive(i) => l.drives.get(i).copied(),
            Target::App(i) => l.apps.get(i).copied(),
            Target::Kind(i) => l.kinds.get(i).copied(),
            Target::Gear => Some(l.gear),
            Target::Disk => Some(l.kinds_title),
            Target::OptLimit(i) => l.opt_limit.get(i).copied(),
            Target::OptMinimize(i) => l.opt_min.get(i).copied(),
            Target::SetItem(_) | Target::SetClose => Some(l.dialog),
            Target::Explorer => Some(l.explorer),
            Target::Downloads => Some(l.downloads),
            Target::Tab(i) | Target::TabClose(i) => l.tabs.get(i).map(|t| t.0),
            Target::Scroll(r) => {
                let v = l.viewport(r);
                Some(Rc { x: v.right() - SCROLL_W, y: v.y, w: SCROLL_W, h: v.h })
            }
            _ => {
                let r = Self::region_of(t)?;
                let mut rc = self.content_rect(t)?;
                rc.y -= self.scroll[r as usize];
                let v = l.viewport(r);
                let y0 = rc.y.max(v.y);
                let y1 = rc.bottom().min(v.bottom());
                if y1 <= y0 {
                    return None;
                }
                Some(Rc { x: rc.x.max(v.x), y: y0, w: rc.w.min(v.w), h: y1 - y0 })
            }
        }
    }

    pub fn ensure_visible(&mut self, t: Target) {
        self.ensure_layout();
        let (Some(r), Some(rc)) = (Self::region_of(t), self.content_rect(t)) else { return };
        let v = self.layout.viewport(r);
        let s = &mut self.scroll[r as usize];
        let top = rc.y - v.y;
        if top - *s < 4.0 {
            *s = top - 4.0;
        } else if rc.bottom() - v.y - *s > v.h - 4.0 {
            *s = rc.bottom() - v.y - v.h + 4.0;
        }
        self.clamp_scroll();
    }

    /// Zaznacza i przewija tak, zeby element byl na gorze panelu.
    pub fn select_top(&mut self, t: Target) {
        self.selected = t;
        self.ensure_layout();
        if let (Some(r), Some(rc)) = (Self::region_of(t), self.content_rect(t)) {
            let v = self.layout.viewport(r);
            self.scroll[r as usize] = rc.y - v.y - 4.0;
            self.clamp_scroll();
        }
    }

    pub fn select(&mut self, t: Target) {
        self.selected = t;
        if t != Target::None {
            self.ensure_visible(t);
        }
    }

    /// Strzalki: lista folderow <-> siatka plikow (lewo/prawo), gora/dol w obrebie.
    pub fn move_selection(&mut self, dx: isize, dy: isize) {
        let (nf, nfi, nt) = (self.page.folders.len() as isize, self.page.files.len() as isize, self.page.table.len() as isize);
        let cols = self.layout.cols.max(1) as isize;
        let t = match self.selected {
            Target::Row(i) => Target::Row((i as isize + dy).clamp(0, (nt - 1).max(0)) as usize),
            Target::Folder(i) => {
                if dx > 0 && nfi > 0 {
                    Target::File(0)
                } else {
                    Target::Folder((i as isize + dy).clamp(0, (nf - 1).max(0)) as usize)
                }
            }
            Target::File(i) => {
                let i = i as isize;
                if dx < 0 && i % cols == 0 && nf > 0 {
                    Target::Folder(((self.scroll[Region::Folders as usize] / self.layout.row_h.max(1.0)) as isize).clamp(0, nf - 1) as usize)
                } else if dx != 0 {
                    Target::File((i + dx).clamp(0, nfi - 1) as usize)
                } else {
                    let j = i + dy * cols;
                    if j < 0 && nf > 0 && dy == -1 {
                        Target::Folder((nf - 1) as usize)
                    } else {
                        Target::File(j.clamp(0, nfi - 1) as usize)
                    }
                }
            }
            _ => {
                if self.page.search && nt > 0 {
                    Target::Row(0)
                } else if nf > 0 {
                    Target::Folder(0)
                } else if nfi > 0 {
                    Target::File(0)
                } else {
                    Target::None
                }
            }
        };
        self.select(t);
    }

    /// Ile wierszy (lub rzedow ikon) miesci sie na ekranie w regionie zaznaczenia.
    pub fn page_step(&self) -> isize {
        let l = &self.layout;
        match Self::region_of(self.selected) {
            Some(Region::Files) => ((l.files.h / l.tile_h.max(1.0)) as isize - 1).max(1),
            Some(r) => ((l.viewport(r).h / l.row_h.max(1.0)) as isize - 2).max(1),
            None => 1,
        }
    }

    pub fn select_edge(&mut self, end: bool) {
        let t = match self.selected {
            Target::Folder(_) => Target::Folder(if end { self.page.folders.len().saturating_sub(1) } else { 0 }),
            Target::File(_) => Target::File(if end { self.page.files.len().saturating_sub(1) } else { 0 }),
            Target::Row(_) => Target::Row(if end { self.page.table.len().saturating_sub(1) } else { 0 }),
            _ => return,
        };
        self.select(t);
    }

    // ---------- trafienie kursorem ----------

    pub fn hit(&self, x: f32, y: f32) -> Target {
        let l = &self.layout;
        // okno ustawien przechwytuje klikniecia; klik obok je zamyka
        if self.page.settings.is_some() {
            if l.dialog_close.contains(x, y) || !l.dialog.contains(x, y) {
                return Target::SetClose;
            }
            return l.set_rows.iter().position(|r| r.contains(x, y)).map(Target::SetItem).unwrap_or(Target::None);
        }
        if l.gear.contains(x, y) {
            return Target::Gear;
        }
        if let Some(i) = l.apps.iter().position(|r| r.contains(x, y)) {
            return Target::App(i);
        }
        // rozwinieta zakladka optymalizacji przechwytuje klikniecia (jest na wierzchu)
        if self.page.opt_reveal && l.opt_panel.w > 0.0 && l.opt_panel.contains(x, y) {
            if let Some(i) = l.opt_limit.iter().position(|r| r.w > 0.0 && r.contains(x, y)) {
                return Target::OptLimit(i);
            }
            if let Some(i) = l.opt_min.iter().position(|r| r.w > 0.0 && r.contains(x, y)) {
                return Target::OptMinimize(i);
            }
            return Target::None;
        }
        if l.kinds_title.contains(x, y) {
            return Target::Disk;
        }
        if let Some(i) = l.kinds.iter().position(|r| r.contains(x, y)) {
            return Target::Kind(i);
        }
        if l.fav_frame.contains(x, y) {
            return l.favs.iter().position(|r| r.w > 0.0 && r.contains(x, y)).map(Target::Fav).unwrap_or(Target::None);
        }
        if l.top.contains(x, y) {
            if let Some(i) = l.actions.iter().position(|r| r.contains(x, y)) {
                return Target::Action(i);
            }
            if let Some(i) = l.drives.iter().position(|r| r.contains(x, y)) {
                return Target::Drive(i);
            }
            if l.explorer.contains(x, y) {
                return Target::Explorer;
            }
            if l.downloads.contains(x, y) {
                return Target::Downloads;
            }
            return Target::None;
        }
        if l.tabbar.contains(x, y) {
            if let Some(i) = l.tabs.iter().position(|t| t.1.contains(x, y)) {
                return Target::TabClose(i);
            }
            if let Some(i) = l.tabs.iter().position(|t| t.0.contains(x, y)) {
                return Target::Tab(i);
            }
            return Target::None;
        }
        let ui = self.ui();
        for r in REGIONS {
            let v = l.viewport(r);
            if v.w <= 0.0 || !v.contains(x, y) {
                continue;
            }
            if x >= v.right() - SCROLL_W && scrollbar_thumb(l, &ui, r).is_some() {
                return Target::Scroll(r);
            }
            let cy = y + self.scroll[r as usize];
            let row = ((cy - v.y - 4.0) / l.row_h).floor();
            match r {
                Region::Tree => {
                    if row < 0.0 {
                        return Target::None;
                    }
                    let i = row as usize;
                    let Some(n) = self.page.tree.get(i) else { return Target::None };
                    let x0 = v.x + 6.0 + self.layout.tree_label_w + n.depth as f32 * 18.0;
                    if n.toggle.is_some() && x >= x0 - 3.0 && x < x0 + 12.0 {
                        return Target::TreeToggle(i);
                    }
                    return Target::Tree(i);
                }
                Region::Folders => {
                    if row >= 0.0 && (row as usize) < self.page.folders.len() {
                        return Target::Folder(row as usize);
                    }
                }
                Region::Files => {
                    let col = ((x - v.x - 8.0) / l.tile_w).floor();
                    let trow = ((cy - v.y - 8.0) / l.tile_h).floor();
                    if col >= 0.0 && trow >= 0.0 && (col as usize) < l.cols {
                        let i = trow as usize * l.cols + col as usize;
                        if i < self.page.files.len() {
                            return Target::File(i);
                        }
                    }
                }
                Region::Table => {
                    if row < 1.0 {
                        if row >= 0.0 {
                            for (c, col) in [(SortCol::Name, l.name), (SortCol::Folder, l.folder), (SortCol::Size, l.size), (SortCol::Date, l.date)] {
                                if x >= col.x && x < col.x + col.w {
                                    return Target::Header(c);
                                }
                            }
                        }
                        return Target::None;
                    }
                    let i = row as usize - 1;
                    if i < self.page.table.len() {
                        return if x >= l.folder.x && x < l.folder.x + l.folder.w { Target::RowFolder(i) } else { Target::Row(i) };
                    }
                }
            }
            return Target::None;
        }
        Target::None
    }

    // ---------- projekty na dysku ----------

    /// Panel pod drzewem: postep indeksowania albo liczby projektow wg jezyka.
    pub fn fill_kinds(&mut self) {
        match profiles::project_counts() {
            Some(k) => {
                self.page.scan = None;
                self.page.kinds = k;
            }
            None => {
                let st = search::status();
                let pct = if self.cfg.last_index > 0 { (st.scanned as f32 / self.cfg.last_index as f32).min(0.99) } else { 0.0 };
                self.page.scan = Some((if st.ready { 1.0 } else { pct }, st.scanned));
                self.page.kinds.clear();
            }
        }
        self.page.kind_active = self.kind;
        profiles::scan_projects();
    }

    /// Indeks gotowy: zapamietaj liczbe plikow (procent postepu przy nastepnym starcie).
    pub fn index_finished(&mut self) {
        let n = search::status().scanned;
        if n > 0 && (self.cfg.last_index == 0 || n.abs_diff(self.cfg.last_index) > self.cfg.last_index / 20) {
            self.cfg.last_index = n;
            self.cfg.save();
        }
    }

    pub fn set_kind(&mut self, kind: Option<usize>) {
        self.kind = if self.kind == kind { None } else { kind };
        self.search = None;
        self.selected = Target::None;
        self.scroll[Region::Folders as usize] = 0.0;
        self.scroll[Region::Files as usize] = 0.0;
        if self.disk {
            crate::katalog::panel_language(self.kind);
        }
        self.rebuild();
    }

    /// Otwiera / zamyka panel projektow i miejsca.
    pub fn set_disk(&mut self, on: bool) {
        self.disk = on;
        self.disk_claim = on;
        self.kind = None;
        self.search = None;
        self.selected = Target::None;
        self.scroll[Region::Folders as usize] = 0.0;
        if on {
            crate::katalog::panel_language(None);
        }
        self.rebuild();
    }

    /// Dopasowuje okno panelu C# do ukladu tego widoku. Wolane przy malowaniu.
    pub fn sync_panel(&mut self, view: HWND) {
        if self.disk_claim {
            self.disk_claim = false;
            crate::katalog::panel_claim(view, self.notify);
            self.panel_rect = None;
        }
        if !crate::katalog::panel_owned_by(view) {
            self.panel_lost = self.disk;
            return;
        }
        self.panel_lost = false;
        // okno ustawien rysuje Rust, wiec panel nie moze go zaslaniac
        if self.disk && !self.settings_open {
            let (s, r) = (self.dpi / 96.0, self.layout.files_frame);
            let px = |v: f32| (v * s).round() as i32;
            let rect = (view.0 as isize, px(r.x), px(r.y), px(r.right()) - px(r.x), px(r.bottom()) - px(r.y));
            if self.panel_rect != Some(rect) && crate::katalog::panel_place(view, rect.1, rect.2, rect.3, rect.4) {
                self.panel_rect = Some(rect);
            }
        } else if self.panel_rect.take().is_some() {
            crate::katalog::panel_hide(view);
        }
    }

    // ---------- otwarte aplikacje ----------

    /// Odswieza przyciski aplikacji. Zwraca true, gdy cos sie zmienilo.
    pub fn update_apps(&mut self) -> bool {
        let wins = if self.cfg.apps_row { crate::winlist::list() } else { Vec::new() };
        let apps: Vec<(String, IconKind, bool)> = wins
            .iter()
            .map(|w| (w.title.clone(), if w.exe.is_empty() { IconKind::File } else { IconKind::System(w.exe.clone()) }, w.minimized))
            .collect();
        let hwnds: Vec<isize> = wins.iter().map(|w| w.hwnd).collect();
        let same = hwnds == self.app_hwnds && apps.len() == self.page.apps.len() && apps.iter().zip(&self.page.apps).all(|(a, b)| a.0 == b.0 && a.2 == b.2);
        if same {
            return false;
        }
        let rows_changed = apps.is_empty() != self.page.apps.is_empty();
        self.app_hwnds = hwnds;
        self.page.apps = apps;
        self.layout_dirty = true;
        if rows_changed {
            self.gen += 1;
        }
        true
    }

    // ---------- ustawienia ----------

    pub fn settings_items(&self) -> Vec<(String, crate::page::SetKind)> {
        use crate::page::SetKind::{Slider, Toggle};
        let c = &self.cfg;
        vec![
            ("Nakładka nad pełnym ekranem (CPU / GPU / RAM)".into(), Toggle(c.overlay)),
            ("Widoczność nakładki".into(), Slider(c.overlay_alpha)),
            ("Dyski na nakładce".into(), Toggle(c.overlay_disks)),
            ("Widoczność dysków".into(), Slider(c.disks_alpha)),
            ("Klik w puste miejsce pulpitu chowa wszystkie okna".into(), Toggle(c.click_hides)),
            ("Klawisz Windows minimalizuje okno na pełnym ekranie".into(), Toggle(c.win_minimizes)),
            ("Przyciski otwartych aplikacji na dole".into(), Toggle(c.apps_row)),
            ("Nowy plik w Pobranych otwiera się w Indeksie".into(), Toggle(c.downloads_open)),
            ("Nakładka także nad oknem zmaksymalizowanym".into(), Toggle(c.overlay_maximized)),
            ("Nakładkę rysuje silnik C# (wyłączone = Rust)".into(), Toggle(c.overlay_cs)),
            ("Ogranicz zasoby bezczynnych aplikacji (EcoQoS)".into(), Toggle(c.optimize)),
            ("Uruchamiaj Indeks razem z Windows".into(), Toggle(crate::autostart::enabled())),
        ]
    }

    pub fn toggle_settings(&mut self, open: bool) {
        self.settings_open = open;
        self.page.settings = if open { Some(self.settings_items()) } else { None };
        // prostokat okna ustawien (i jego wiersze) liczy uklad - bez tego okno mialo
        // rozmiar 0 do najblizszego przypadkowego przeliczenia i nie bylo go widac
        self.layout_dirty = true;
        self.ensure_layout();
        self.gen += 1;
    }

    /// Zmiana ustawienia `i`: przelacznik - odwraca, suwak - `value` (0..100).
    pub fn set_setting(&mut self, i: usize, value: Option<u8>) {
        let c = &mut self.cfg;
        match (i, value) {
            (0, _) => c.overlay = !c.overlay,
            (1, Some(v)) => c.overlay_alpha = v,
            (2, _) => c.overlay_disks = !c.overlay_disks,
            (3, Some(v)) => c.disks_alpha = v,
            (4, _) => c.click_hides = !c.click_hides,
            (5, _) => c.win_minimizes = !c.win_minimizes,
            (6, _) => c.apps_row = !c.apps_row,
            (7, _) => c.downloads_open = !c.downloads_open,
            (8, _) => c.overlay_maximized = !c.overlay_maximized,
            (9, _) => c.overlay_cs = !c.overlay_cs,
            (10, _) => {
                c.optimize = !c.optimize;
                if !c.optimize {
                    crate::optymalizator::disable_all();
                }
                self.opt_reveal = false;
            }
            (11, _) => crate::autostart::set(!crate::autostart::enabled()),
            _ => return,
        }
        if value.is_none() || self.slider_drag.is_none() {
            self.cfg.save();
        }
        if i == 6 {
            self.update_apps();
            self.gen += 1;
        }
        self.page.settings = Some(self.settings_items());
    }

    /// Suwak `i` ustawiony wg polozenia kursora `x` (DIP).
    pub fn slider_at(&mut self, i: usize, x: f32) {
        if let Some(t) = self.layout.set_tracks.get(i).copied() {
            if t.w > 0.0 {
                let v = (((x - t.x) / t.w).clamp(0.0, 1.0) * 100.0).round() as u8;
                self.set_setting(i, Some(v));
            }
        }
    }

    pub fn activate_app(&self, i: usize) {
        if let Some(h) = self.app_hwnds.get(i) {
            crate::winlist::activate(*h);
        }
    }

    pub fn toggle_protected(&mut self) {
        self.cfg.show_protected = !self.cfg.show_protected;
        self.cfg.save();
        self.load(false);
    }

    pub fn set_home_here(&mut self) {
        if let Some(d) = self.current_dir() {
            self.cfg.home = d;
            self.cfg.save();
        }
    }

    pub fn log_state(&self) {
        util::log(&format!("folder: {} ({} pozycji)", self.path, self.items.len()));
    }
}

fn sort_rows(rows: &mut [Row], col: SortCol, asc: bool) {
    rows.sort_by(|a, b| {
        let o = match col {
            SortCol::Name => cmp_logical(&a.item.name, &b.item.name),
            SortCol::Folder => cmp_logical(&a.folder_s, &b.folder_s),
            SortCol::Size => a.item.size.cmp(&b.item.size),
            SortCol::Date => a.item.mtime.cmp(&b.item.mtime),
        };
        if asc { o } else { o.reverse() }
    });
}
