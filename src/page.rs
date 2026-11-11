// Model strony rysowanej przez renderer (niezalezny od Win32).
//
// Uklad:  [ulubione ................................ akcje]
//         [drzewo | tytul + sciezka                       ]
//         [drzewo | foldery | pliki (siatka ikon)         ]
//         [pasek stanu: dyski, CPU, RAM                   ]
// W trybie szukania: drzewo = pasujace foldery, srodek = tabela plikow.
use crate::fsx::Item;
use crate::icons::IconKind;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkTarget {
    Fav(usize),
    Drive(usize),
    Tab(usize),
    ToggleFav,
    Explorer,
    CloseSearch,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkStyle {
    Link,
    /// Ulubiony folder, w ktorym jestesmy.
    Current,
}

pub struct Link {
    pub text: String,
    pub icon: Option<IconKind>,
    pub target: LinkTarget,
    pub style: LinkStyle,
}

pub struct TreeNode {
    pub name: String,
    pub path: String,
    pub depth: u32,
    pub icon: IconKind,
    /// None = brak przycisku [+]/[-]
    pub toggle: Option<bool>,
    pub current: bool,
    /// Dodatkowy szary tekst (np. folder nadrzedny wyniku szukania).
    pub note: String,
    /// Bity poziomow, na ktorych pionowa linia drzewa biegnie dalej w dol.
    pub guides: u64,
    /// Ostatnie dziecko swojego rodzica (lacznik "└" zamiast "├").
    pub last: bool,
    /// Jezyk / technologia folderu (podpis z lewej strony drzewa).
    pub tech: Option<usize>,
}

/// Co jest pod kursorem / zaznaczone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    None,
    Fav(usize),
    Drive(usize),
    Explorer,
    /// [Pobrane] obok Eksploratora.
    Downloads,
    Tab(usize),
    TabClose(usize),
    Action(usize),
    Tree(usize),
    TreeToggle(usize),
    Folder(usize),
    File(usize),
    Row(usize),
    RowFolder(usize),
    Header(SortCol),
    Scroll(Region),
    App(usize),
    Kind(usize),
    Gear,
    SetItem(usize),
    SetClose,
}

impl Target {
    pub fn is_link(&self) -> bool {
        !matches!(self, Target::None | Target::Scroll(_))
    }
}

pub struct Row {
    pub item: Item,
    pub full: String,
    pub display: String,
    pub size_s: String,
    pub date_s: String,
    pub folder_s: String,
    pub icon: IconKind,
    /// Technologia folderu (silnik projektow) - indeks w profiles::LANGS.
    pub tech: Option<usize>,
    /// "3 dni temu" - ostatnia zmiana w folderze.
    pub ago: String,
    pub name_w: f32,
    pub folder_w: f32,
    pub size_w: f32,
    pub date_w: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortCol {
    Name,
    Folder,
    Size,
    Date,
}

/// Obszary przewijane niezaleznie.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Region {
    Tree = 0,
    Folders = 1,
    Files = 2,
    Table = 3,
}

pub const REGIONS: [Region; 4] = [Region::Tree, Region::Folders, Region::Files, Region::Table];

#[derive(Clone, Default)]
pub struct StatusLine {
    pub drives: Vec<(String, u64, u64)>,
    pub cpu: f32,
    pub gpu: f32,
    pub ram_used: u64,
    pub ram_total: u64,
    pub own: u64,
    pub own_cpu: f32,
}

pub struct Page {
    pub search: bool,
    pub drives: Vec<Link>,
    pub explorer: Link,
    pub favs: Vec<Link>,
    pub tabs: Vec<Link>,
    pub actions: Vec<Link>,
    pub tree_title: String,
    pub tree: Vec<TreeNode>,
    pub title: String,
    pub subtitle: String,
    /// Pasek jezykow biezacego folderu: (nazwa, kolor, udzial).
    pub langs: Vec<(String, u32, f32)>,
    /// Np. "Node.js · repo git · zmieniony 3 dni temu".
    pub head_info: String,
    /// Podsumowanie w dolnej ramce listy folderow ("Node 12 · Rust 5").
    pub folders_note: String,
    pub folders: Vec<Row>,
    pub files: Vec<Row>,
    pub table: Vec<Row>,
    pub message: Option<(String, bool)>,
    pub status: StatusLine,
    /// Przyciski otwartych aplikacji pod paskiem wydajnosci: (tytul, ikona, zminimalizowana).
    pub apps: Vec<(String, IconKind, bool)>,
    /// Panel "Projekty na dysku": None = skan trwa (procent, przejrzane pliki).
    pub scan: Option<(f32, u32)>,
    pub kinds: Vec<(usize, usize)>,
    pub kind_active: Option<usize>,
    /// Okno ustawien: (etykieta, wartosc) - przelacznik (0/1) albo suwak (0..100).
    pub settings: Option<Vec<(String, SetKind)>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SetKind {
    Toggle(bool),
    Slider(u8),
}

/// Prostokat w DIP (wspolrzedne okna).
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Rc {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rc {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Col {
    pub x: f32,
    pub w: f32,
}

#[derive(Default, Debug)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub top: Rc,
    pub drives: Vec<Rc>,
    pub explorer: Rc,
    pub downloads: Rc,
    pub favs: Vec<Rc>,
    /// Pasek kart (pod ulubionymi): (karta, przycisk zamkniecia).
    pub tabbar: Rc,
    pub tabs: Vec<(Rc, Rc)>,
    pub actions: Vec<Rc>,
    /// Ramka panelu drzewa i jego wnetrze (przewijane).
    pub tree_frame: Rc,
    pub tree: Rc,
    /// Szerokosc kolumny podpisow jezykow z lewej strony drzewa (0 = brak).
    pub tree_label_w: f32,
    pub head: Rc,
    pub folders_frame: Rc,
    pub folders: Rc,
    pub files_frame: Rc,
    pub files: Rc,
    pub table: Rc,
    pub status: Rc,
    pub row_h: f32,
    pub tile_w: f32,
    pub tile_h: f32,
    pub cols: usize,
    /// Wysokosc tresci kazdego regionu (do przewijania).
    pub content: [f32; 4],
    /// Panel projektow na dysku (pod drzewem i lista) i jego przyciski jezykow.
    pub kinds_frame: Rc,
    pub kinds: Vec<Rc>,
    /// Rzad przyciskow aplikacji pod paskiem wydajnosci.
    pub apps_row: Rc,
    pub apps: Vec<Rc>,
    pub gear: Rc,
    /// Okno ustawien: ramka, [x], wiersze (cala szerokosc) i tory suwakow.
    pub dialog: Rc,
    pub dialog_close: Rc,
    pub set_rows: Vec<Rc>,
    pub set_tracks: Vec<Rc>,
    pub name: Col,
    pub folder: Col,
    pub size: Col,
    pub date: Col,
}

impl Layout {
    pub fn viewport(&self, r: Region) -> Rc {
        match r {
            Region::Tree => self.tree,
            Region::Folders => self.folders,
            Region::Files => self.files,
            Region::Table => self.table,
        }
    }

    /// Prostokat wiersza drzewa / folderu (bez przewiniecia).
    pub fn list_row(&self, r: Region, i: usize) -> Rc {
        let v = self.viewport(r);
        Rc { x: v.x, y: v.y + 4.0 + i as f32 * self.row_h, w: v.w - SCROLL_W, h: self.row_h }
    }

    pub fn tile(&self, i: usize) -> Rc {
        let cols = self.cols.max(1);
        let (c, r) = (i % cols, i / cols);
        Rc { x: self.files.x + 8.0 + c as f32 * self.tile_w, y: self.files.y + 8.0 + r as f32 * self.tile_h, w: self.tile_w, h: self.tile_h }
    }

    /// Wiersz tabeli wynikow (0 = naglowek kolumn).
    pub fn table_row(&self, i: usize) -> Rc {
        Rc { x: self.name.x, y: self.table.y + 4.0 + self.row_h * (i as f32 + 1.0), w: self.date.x + self.date.w - self.name.x, h: self.row_h }
    }
}

pub const SCROLL_W: f32 = 10.0;
