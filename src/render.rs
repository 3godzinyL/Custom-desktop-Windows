// Rysowanie (Direct2D + DirectWrite, programowo, pasami prosto do okna).
//
// Styl: indeks katalogu z Chromium (Times New Roman, linki #9e9eff) w ciemnej,
// old-schoolowej oprawie: podwojne ramki paneli z tytulem w linii (Norton
// Commander), drzewo z kropkowanymi liniami i [+]/[-] (Windows 98), granatowe
// zaznaczenie (pulpit Win95), segmentowe paski w kolorach CGA.
use std::collections::HashMap;

use windows::core::{w, Interface, Result, PCWSTR};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_END_CLOSED, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory, ID2D1Geometry,
    ID2D1RenderTarget, ID2D1SolidColorBrush, D2D1_ANTIALIAS_MODE_ALIASED,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_ROUNDED_RECT, D2D1_ELLIPSE,
    D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC;
use windows_numerics::{Matrix3x2, Vector2};

use crate::icons::{IconKind, IconState, Icons};
use crate::page::{Col, Layout, LinkStyle, Page, Rc, Region, Target, REGIONS, SCROLL_W};
use crate::stats;

const STRIP_PX: i32 = 256;
const COL_PAD: f32 = 32.0;
const TILE_ICON: f32 = 32.0;

pub const BG: u32 = 0x0A0A0A;
/// Link do folderu Pobrane obok [Otworz Eksplorator].
const DOWNLOADS: &str = "[Pobrane]";
const BAR_BG: u32 = 0x141414;
const FRAME: u32 = 0x4E4E4E;
const TEXT: u32 = 0xFFFFFF;
const LINK: u32 = 0x9E9EFF;
const DIM: u32 = 0x8C8C8C;
const ERR: u32 = 0xFF8A8A;
const NAVY: u32 = 0x000080;
/// Biezacy folder / zaznaczony folder: zolty, pogrubiony.
const YELLOW: u32 = 0xFFD84D;
const STRIPE: u32 = 0x111116;
const TAB_ON: u32 = 0x1E1E2E;
const HOVER: u32 = 0x16163A;
const CGA_GREEN: u32 = 0x55FF55;
const CGA_YELLOW: u32 = 0xFFFF55;
const CGA_RED: u32 = 0xFF5555;
const CGA_CYAN: u32 = 0x55FFFF;
const SEG_EMPTY: u32 = 0x232323;

fn rgb(c: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((c >> 16) & 0xFF) as f32 / 255.0,
        g: ((c >> 8) & 0xFF) as f32 / 255.0,
        b: (c & 0xFF) as f32 / 255.0,
        a: 1.0,
    }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }
}

fn wstr(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Stan interfejsu potrzebny do rysowania.
pub struct Ui {
    pub scroll: [f32; 4],
    pub hover: Target,
    pub selected: Target,
    pub drag: Option<Region>,
    pub index_count: u32,
}

struct Shapes {
    folder_back: ID2D1Geometry,
    folder_front: ID2D1Geometry,
    page: ID2D1Geometry,
    fold: ID2D1Geometry,
}

struct Brushes {
    text: ID2D1SolidColorBrush,
    link: ID2D1SolidColorBrush,
    dim: ID2D1SolidColorBrush,
    frame: ID2D1SolidColorBrush,
    bar: ID2D1SolidColorBrush,
    bg: ID2D1SolidColorBrush,
    navy: ID2D1SolidColorBrush,
    hover: ID2D1SolidColorBrush,
    err: ID2D1SolidColorBrush,
    cyan: ID2D1SolidColorBrush,
    yellow: ID2D1SolidColorBrush,
    stripe: ID2D1SolidColorBrush,
    tab_on: ID2D1SolidColorBrush,
}

pub struct Renderer {
    pub d2d: ID2D1Factory,
    pub dw: IDWriteFactory,
    f_body: IDWriteTextFormat,
    f_body_bold: IDWriteTextFormat,
    f_body_right: IDWriteTextFormat,
    f_bold_center: IDWriteTextFormat,
    f_h1: IDWriteTextFormat,
    f_tile: IDWriteTextFormat,
    f_mono: IDWriteTextFormat,
    f_mono_bold: IDWriteTextFormat,
    f_badge2: IDWriteTextFormat,
    f_badge3: IDWriteTextFormat,
    f_symbol: IDWriteTextFormat,
    pub line_h: f32,
    h1_h: f32,
    tile_line_h: f32,
    mono_h: f32,
    shapes: Shapes,
    dc_rt: Option<ID2D1DCRenderTarget>,
    bitmaps: HashMap<String, ID2D1Bitmap>,
    pub dpi: f32,
    ox: f32,
    oy: f32,
    clip_w: f32,
    clip_h: f32,
}

fn polygon(f: &ID2D1Factory, pts: &[(f32, f32)]) -> Result<ID2D1Geometry> {
    unsafe {
        let g = f.CreatePathGeometry()?;
        let sink = g.Open()?;
        sink.BeginFigure(Vector2 { X: pts[0].0, Y: pts[0].1 }, D2D1_FIGURE_BEGIN_FILLED);
        for p in &pts[1..] {
            sink.AddLine(Vector2 { X: p.0, Y: p.1 });
        }
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        g.cast()
    }
}

fn text_format(dw: &IDWriteFactory, family: PCWSTR, weight: DWRITE_FONT_WEIGHT, size: f32, wrap: bool) -> Result<IDWriteTextFormat> {
    unsafe {
        let f = dw.CreateTextFormat(family, None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("pl-PL"))?;
        f.SetWordWrapping(if wrap { DWRITE_WORD_WRAPPING_WRAP } else { DWRITE_WORD_WRAPPING_NO_WRAP })?;
        let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
        let sign = dw.CreateEllipsisTrimmingSign(&f)?;
        f.SetTrimming(&trimming, &sign)?;
        Ok(f)
    }
}

impl Renderer {
    pub fn new() -> Result<Renderer> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let times = w!("Times New Roman");
            let courier = w!("Courier New");
            let f_body = text_format(&dw, times, DWRITE_FONT_WEIGHT_NORMAL, 16.0, false)?;
            let f_body_bold = text_format(&dw, times, DWRITE_FONT_WEIGHT_BOLD, 16.0, false)?;
            let f_body_right = text_format(&dw, times, DWRITE_FONT_WEIGHT_NORMAL, 16.0, false)?;
            f_body_right.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
            let f_bold_center = text_format(&dw, times, DWRITE_FONT_WEIGHT_BOLD, 16.0, false)?;
            f_bold_center.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            let f_h1 = text_format(&dw, times, DWRITE_FONT_WEIGHT_BOLD, 30.0, false)?;
            let f_tile = text_format(&dw, times, DWRITE_FONT_WEIGHT_NORMAL, 14.0, true)?;
            f_tile.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            let f_mono = text_format(&dw, courier, DWRITE_FONT_WEIGHT_NORMAL, 13.0, false)?;
            let f_mono_bold = text_format(&dw, courier, DWRITE_FONT_WEIGHT_BOLD, 13.0, false)?;
            let f_badge2 = text_format(&dw, w!("Segoe UI"), DWRITE_FONT_WEIGHT_BOLD, 8.0, false)?;
            let f_badge3 = text_format(&dw, w!("Segoe UI"), DWRITE_FONT_WEIGHT_BOLD, 6.2, false)?;
            let f_symbol = text_format(&dw, w!("Segoe UI Symbol"), DWRITE_FONT_WEIGHT_NORMAL, 19.0, false)?;
            for f in [&f_badge2, &f_badge3, &f_symbol] {
                f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            }
            let shapes = Shapes {
                folder_back: polygon(&d2d, &[(1.0, 2.5), (6.0, 2.5), (7.5, 4.0), (15.0, 4.0), (15.0, 13.5), (1.0, 13.5)])?,
                folder_front: polygon(&d2d, &[(1.0, 6.0), (15.0, 6.0), (15.0, 13.5), (1.0, 13.5)])?,
                page: polygon(&d2d, &[(3.0, 1.0), (10.0, 1.0), (13.5, 4.5), (13.5, 15.0), (3.0, 15.0)])?,
                fold: polygon(&d2d, &[(10.0, 1.0), (10.0, 4.5), (13.5, 4.5)])?,
            };
            let mut r = Renderer {
                d2d,
                dw,
                f_body,
                f_body_bold,
                f_body_right,
                f_bold_center,
                f_h1,
                f_tile,
                f_mono,
                f_mono_bold,
                f_badge2,
                f_badge3,
                f_symbol,
                line_h: 18.4,
                h1_h: 34.5,
                tile_line_h: 16.1,
                mono_h: 15.0,
                shapes,
                dc_rt: None,
                bitmaps: HashMap::new(),
                dpi: 96.0,
                ox: 0.0,
                oy: 0.0,
                clip_w: 0.0,
                clip_h: 0.0,
            };
            r.line_h = r.line_height(&r.f_body.clone());
            r.h1_h = r.line_height(&r.f_h1.clone());
            r.tile_line_h = r.line_height(&r.f_tile.clone());
            r.mono_h = r.line_height(&r.f_mono.clone());
            Ok(r)
        }
    }

    fn line_height(&self, f: &IDWriteTextFormat) -> f32 {
        unsafe {
            let t = wstr("Ąg");
            self.dw
                .CreateTextLayout(&t, f, 1000.0, 1000.0)
                .ok()
                .and_then(|l| {
                    let mut m = DWRITE_TEXT_METRICS::default();
                    l.GetMetrics(&mut m).ok().map(|_| m.height)
                })
                .unwrap_or(18.4)
        }
    }

    fn measure_with(&self, s: &str, f: &IDWriteTextFormat) -> f32 {
        unsafe {
            let t = wstr(s);
            match self.dw.CreateTextLayout(&t, f, 100000.0, 100.0) {
                Ok(l) => {
                    let mut m = DWRITE_TEXT_METRICS::default();
                    let _ = l.GetMetrics(&mut m);
                    m.widthIncludingTrailingWhitespace
                }
                Err(_) => s.chars().count() as f32 * 7.5,
            }
        }
    }

    pub fn measure(&self, s: &str, bold: bool) -> f32 {
        self.measure_with(s, if bold { &self.f_bold_center } else { &self.f_body })
    }

    fn px(&self) -> f32 {
        96.0 / self.dpi
    }

    fn snap(&self, v: f32) -> f32 {
        let s = self.dpi / 96.0;
        (v * s).round() / s
    }

    // ---------- uklad ----------

    /// Uklad okna `w` x `h` DIP przy `dpi`. Krawedzie regionow sa wyrownane do pikseli,
    /// zeby przewijanie przez ScrollWindowEx pasowalo co do piksela.
    pub fn layout(&self, page: &mut Page, w: f32, h: f32, dpi: f32) -> Layout {
        let s = dpi / 96.0;
        let snap = |v: f32| (v * s).round() / s;
        let mut l = Layout { width: w, height: h, row_h: snap(self.line_h + 4.0), ..Default::default() };
        l.top = Rc { x: 0.0, y: 0.0, w, h: snap(34.0) };
        l.tabbar = Rc { x: 0.0, y: l.top.bottom(), w, h: snap(30.0) };
        // dol: (opcjonalnie) przyciski otwartych aplikacji, a pod nimi pasek wydajnosci na samym dole
        let row_h = snap(34.0);
        let mut slots: Vec<(f32, usize, f32)> = Vec::new();
        let (mut x, mut row) = (10.0, 0usize);
        for (title, _, _) in &page.apps {
            let bw = (self.measure(title, false) + 34.0).clamp(90.0, 210.0);
            if x + bw > w - 10.0 && x > 10.0 {
                row += 1;
                x = 10.0;
            }
            if row >= 3 {
                break;
            }
            slots.push((x, row, bw));
            x += bw + 4.0;
        }
        let rows = slots.last().map(|s| s.1 + 1).unwrap_or(0);
        let apps_h = rows as f32 * row_h;
        l.status = Rc { x: 0.0, y: snap(h - 38.0), w, h: h - snap(h - 38.0) };
        if apps_h > 0.0 {
            l.apps_row = Rc { x: 0.0, y: l.status.y - apps_h, w, h: apps_h };
            for (x, row, bw) in slots {
                l.apps.push(Rc { x, y: l.apps_row.y + row as f32 * row_h + 4.0, w: bw, h: row_h - 8.0 });
            }
        }
        // kolumna podpisow jezykow z lewej strony drzewa: najdluzsza nazwa w drzewie
        let mono = self.f_mono.clone();
        l.tree_label_w = page
            .tree
            .iter()
            .filter_map(|n| n.tech)
            .map(|t| self.measure_with(crate::profiles::LANGS[t].name, &mono) + 10.0)
            .fold(0.0, f32::max)
            .min(120.0);
        // zebatka ustawien w prawym gornym rogu
        l.gear = Rc { x: w - 36.0, y: (l.top.h - 26.0) / 2.0, w: 26.0, h: 26.0 };

        // gora: dyski | [Otworz Eksplorator] | Ulubione: ... ; akcje od prawej
        let cy = (l.top.h - self.line_h) / 2.0;
        let mut x = 12.0;
        for d in &page.drives {
            // szerokosc liczona pogrubionym fontem - biezacy link jest pogrubiony
            let dw = self.measure(&d.text, true) + 22.0;
            l.drives.push(Rc { x, y: cy, w: dw, h: self.line_h });
            x += dw + 14.0;
        }
        x += 10.0;
        let ew = self.measure(&page.explorer.text, false);
        l.explorer = Rc { x, y: cy, w: ew, h: self.line_h };
        x += ew + 14.0;
        let dlw = self.measure(DOWNLOADS, false);
        l.downloads = Rc { x, y: cy, w: dlw, h: self.line_h };
        x += dlw + 28.0 + self.measure_with("Ulubione:", &self.f_mono_bold) + 12.0;
        let mut right = l.gear.x - 14.0;
        for a in page.actions.iter().rev() {
            let aw = self.measure(&a.text, false);
            right -= aw;
            l.actions.insert(0, Rc { x: right, y: cy, w: aw, h: self.line_h });
            right -= 18.0;
        }
        for f in &page.favs {
            let fw = self.measure(&f.text, true) + 22.0;
            if x + fw > right {
                l.favs.push(Rc::default());
                continue;
            }
            l.favs.push(Rc { x, y: cy, w: fw, h: self.line_h });
            x += fw + 16.0;
        }

        // karty: [ikona nazwa  x]
        let ty = l.tabbar.y + 4.0;
        let th = l.tabbar.h - 6.0;
        let mut x = 12.0 + self.measure_with("Karty:", &self.f_mono_bold) + 12.0;
        for (k, t) in page.tabs.iter().enumerate() {
            let fixed = k == 0;
            let tw = (self.measure(&t.text, true) + 22.0 + if fixed { 10.0 } else { 28.0 }).min(260.0);
            if x + tw > w - 12.0 {
                l.tabs.push((Rc::default(), Rc::default()));
                continue;
            }
            let tab = Rc { x, y: ty, w: tw, h: th };
            let close = if fixed { Rc::default() } else { Rc { x: x + tw - 22.0, y: ty, w: 20.0, h: th } };
            l.tabs.push((tab, close));
            x += tw + 6.0;
        }

        let top = l.tabbar.bottom() + snap(10.0);
        // panele koncza sie nad rzedem aplikacji (a bez niego nad paskiem wydajnosci)
        let bottom = if l.apps_row.h > 0.0 { l.apps_row.y } else { l.status.y } - snap(8.0);
        // panel "Projekty na dysku" pod drzewem i lista folderow - panele nad nim
        // koncza sie wczesniej, zeby nic na niego nie nachodzilo
        let kinds_h = snap(76.0);
        let upper = bottom - kinds_h - snap(10.0);
        let tree_w = snap((w * 0.16).clamp(250.0, 340.0));
        l.tree_frame = Rc { x: snap(8.0), y: top, w: tree_w, h: upper - top };
        l.tree = inner(&l.tree_frame, s);

        let mx = l.tree_frame.right() + snap(12.0);
        let mw = w - mx - snap(8.0);
        // tytul, sciezka i (dla folderu) pasek jezykow z silnika projektow
        l.head = Rc { x: mx, y: top - 4.0, w: mw, h: snap(self.h1_h + self.line_h * 2.0 + 18.0) };
        let cy = l.head.bottom() + snap(10.0);
        let kinds_right;
        if page.search {
            let frame = Rc { x: mx, y: cy, w: mw, h: bottom - cy };
            l.files_frame = frame;
            l.table = inner(&frame, s);
            kinds_right = l.tree_frame.right();
        } else {
            let fw = snap((mw * 0.27).clamp(240.0, 420.0));
            l.folders_frame = Rc { x: mx, y: cy, w: fw, h: upper - cy };
            l.folders = inner(&l.folders_frame, s);
            l.files_frame = Rc { x: mx + fw + snap(10.0), y: cy, w: mw - fw - snap(10.0), h: bottom - cy };
            l.files = inner(&l.files_frame, s);
            kinds_right = l.folders_frame.right();
        }
        l.kinds_frame = Rc { x: l.tree_frame.x, y: bottom - kinds_h, w: kinds_right - l.tree_frame.x, h: kinds_h };
        // jezyki: ● Node.js 47 w dwoch rzedach (bez ramek)
        let (mut kx, mut ky) = (l.kinds_frame.x + 14.0, l.kinds_frame.y + 16.0);
        for (kind, n) in &page.kinds {
            let cw = 15.0 + self.measure(crate::profiles::short(*kind), true) + 6.0 + self.measure_with(&n.to_string(), &self.f_mono.clone());
            if kx + cw > l.kinds_frame.right() - 10.0 {
                kx = l.kinds_frame.x + 14.0;
                ky += 26.0;
                if ky + 22.0 > l.kinds_frame.bottom() {
                    break;
                }
            }
            l.kinds.push(Rc { x: kx, y: ky, w: cw, h: 22.0 });
            kx += cw + 22.0;
        }
        // okno ustawien na srodku
        if let Some(items) = &page.settings {
            let dw = 620.0;
            let dh = 44.0 + items.len() as f32 * 36.0 + 16.0;
            l.dialog = Rc { x: snap((w - dw) / 2.0), y: snap((h - dh) / 2.0), w: dw, h: dh };
            l.dialog_close = Rc { x: l.dialog.right() - 34.0, y: l.dialog.y + 4.0, w: 28.0, h: 24.0 };
            for (i, (_, kind)) in items.iter().enumerate() {
                let r = Rc { x: l.dialog.x + 12.0, y: l.dialog.y + 44.0 + i as f32 * 36.0, w: dw - 24.0, h: 32.0 };
                l.set_rows.push(r);
                l.set_tracks.push(match kind {
                    crate::page::SetKind::Slider(_) => Rc { x: r.x + 340.0, y: r.y + 10.0, w: 200.0, h: 12.0 },
                    _ => Rc::default(),
                });
            }
        }

        // siatka plikow
        l.tile_w = 104.0;
        l.tile_h = snap(TILE_ICON + 14.0 + self.tile_line_h * 2.0 + 10.0);
        l.cols = (((l.files.w - 16.0 - SCROLL_W) / l.tile_w).floor() as usize).max(1);
        let tile_rows = page.files.len().div_ceil(l.cols);
        l.content[Region::Tree as usize] = 8.0 + page.tree.len() as f32 * l.row_h;
        l.content[Region::Folders as usize] = 8.0 + page.folders.len() as f32 * l.row_h;
        l.content[Region::Files as usize] = 16.0 + tile_rows as f32 * l.tile_h;
        l.content[Region::Table as usize] = 8.0 + (page.table.len() + 1) as f32 * l.row_h;

        // kolumny tabeli wynikow
        if page.search {
            let (mut name_w, mut folder_w, mut size_w, mut date_w) = (0f32, 0f32, 0f32, 0f32);
            for r in page.table.iter_mut() {
                if r.name_w < 0.0 {
                    r.name_w = self.measure(&r.display, false);
                    r.folder_w = self.measure(&r.folder_s, false);
                    r.size_w = self.measure(&r.size_s, false);
                    r.date_w = self.measure(&r.date_s, false);
                }
                name_w = name_w.max(r.name_w);
                folder_w = folder_w.max(r.folder_w);
                size_w = size_w.max(r.size_w);
                date_w = date_w.max(r.date_w);
            }
            let tw = l.table.w - SCROLL_W;
            let sw = COL_PAD + size_w.max(self.measure("Rozmiar", true)) + 2.0;
            let dw = COL_PAD + date_w.max(self.measure("Data modyfikacji", true)) + 2.0;
            let nw = (24.0 + name_w).clamp(self.measure("Nazwa", true) + 26.0, (tw * 0.3).max(240.0)) + 2.0;
            let fw = (COL_PAD + folder_w).max(self.measure("Folder", true)).min((tw - nw - sw - dw).max(120.0)) + 2.0;
            let x0 = l.table.x + 6.0;
            l.name = Col { x: x0, w: nw };
            l.folder = Col { x: x0 + nw, w: fw };
            l.size = Col { x: x0 + nw + fw, w: sw };
            l.date = Col { x: x0 + nw + fw + sw, w: dw };
        }
        l
    }

    // ---------- cel rysowania ----------

    /// Maluje `area` (piksele okna) prosto do `hdc`, pasami po STRIP_PX: rysowanie
    /// programowe (bez sterownika GPU w procesie) i bez bufora na caly ekran.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_dc(&mut self, hdc: HDC, area: RECT, dpi: f32, page: &Page, l: &Layout, ui: &Ui, icons: &mut Icons) -> Result<()> {
        if self.dc_rt.is_none() || (self.dpi - dpi).abs() > 0.5 {
            self.drop_target();
            self.dpi = dpi;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                dpiX: dpi,
                dpiY: dpi,
                ..Default::default()
            };
            let rt = unsafe { self.d2d.CreateDCRenderTarget(&props)? };
            unsafe { rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE) };
            self.dc_rt = Some(rt);
        }
        let rt = self.dc_rt.clone().unwrap();
        let s = dpi / 96.0;
        let mut top = area.top;
        let result = (|| -> Result<()> {
            while top < area.bottom {
                let bottom = (top + STRIP_PX).min(area.bottom);
                let r = RECT { left: area.left, top, right: area.right, bottom };
                unsafe { rt.BindDC(hdc, &r)? };
                self.ox = area.left as f32 / s;
                self.oy = top as f32 / s;
                self.clip_w = (area.right - area.left) as f32 / s;
                self.clip_h = (bottom - top) as f32 / s;
                self.draw(&rt, page, l, ui, icons)?;
                top = bottom;
            }
            Ok(())
        })();
        self.ox = 0.0;
        self.oy = 0.0;
        result
    }

    /// Cala strona na dowolny cel (zrzut PNG).
    pub fn draw_full(&mut self, rt: &ID2D1RenderTarget, page: &Page, l: &Layout, ui: &Ui, icons: &mut Icons) -> Result<()> {
        self.ox = 0.0;
        self.oy = 0.0;
        self.clip_w = l.width;
        self.clip_h = l.height;
        self.draw(rt, page, l, ui, icons)
    }

    pub fn drop_target(&mut self) {
        self.dc_rt = None;
        self.bitmaps.clear();
    }

    pub fn grayscale_text(rt: &ID2D1RenderTarget) {
        unsafe { rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };
    }

    pub fn trim_bitmaps(&mut self) {
        if self.bitmaps.len() > 1500 {
            self.bitmaps.retain(|k, _| k.starts_with('.'));
        }
    }

    fn base(&self) -> Matrix3x2 {
        Matrix3x2::translation(-self.ox, -self.oy)
    }

    fn visible(&self, r: &Rc) -> bool {
        r.right() >= self.ox && r.x <= self.ox + self.clip_w && r.bottom() >= self.oy && r.y <= self.oy + self.clip_h
    }

    // ---------- rysowanie ----------

    fn draw(&mut self, rt: &ID2D1RenderTarget, page: &Page, l: &Layout, ui: &Ui, icons: &mut Icons) -> Result<()> {
        unsafe {
            rt.BeginDraw();
            rt.SetTransform(&self.base());
            rt.Clear(Some(&rgb(BG)));
            let b = Brushes {
                text: rt.CreateSolidColorBrush(&rgb(TEXT), None)?,
                link: rt.CreateSolidColorBrush(&rgb(LINK), None)?,
                dim: rt.CreateSolidColorBrush(&rgb(DIM), None)?,
                frame: rt.CreateSolidColorBrush(&rgb(FRAME), None)?,
                bar: rt.CreateSolidColorBrush(&rgb(BAR_BG), None)?,
                bg: rt.CreateSolidColorBrush(&rgb(BG), None)?,
                navy: rt.CreateSolidColorBrush(&rgb(NAVY), None)?,
                hover: rt.CreateSolidColorBrush(&rgb(HOVER), None)?,
                err: rt.CreateSolidColorBrush(&rgb(ERR), None)?,
                cyan: rt.CreateSolidColorBrush(&rgb(CGA_CYAN), None)?,
                yellow: rt.CreateSolidColorBrush(&rgb(YELLOW), None)?,
                stripe: rt.CreateSolidColorBrush(&rgb(STRIPE), None)?,
                tab_on: rt.CreateSolidColorBrush(&rgb(TAB_ON), None)?,
            };
            if self.visible(&l.top) {
                self.draw_top(rt, page, l, ui, &b, icons)?;
            }
            if self.visible(&l.tabbar) {
                self.draw_tabs(rt, page, l, ui, &b, icons)?;
            }
            if self.visible(&l.tree_frame) {
                self.draw_tree(rt, page, l, ui, &b, icons)?;
            }
            if self.visible(&l.head) {
                self.draw_head(rt, page, l, &b)?;
            }
            if page.search {
                if self.visible(&l.files_frame) {
                    self.draw_table(rt, page, l, ui, &b, icons)?;
                }
            } else {
                if self.visible(&l.folders_frame) {
                    self.draw_folders(rt, page, l, ui, &b, icons)?;
                }
                if self.visible(&l.files_frame) {
                    self.draw_files(rt, page, l, ui, &b, icons)?;
                }
            }
            if self.visible(&l.status) {
                self.draw_status(rt, page, l, ui, &b)?;
            }
            if !page.apps.is_empty() && self.visible(&l.apps_row) {
                self.draw_apps(rt, page, l, ui, &b, icons)?;
            }
            if self.visible(&l.kinds_frame) {
                self.draw_kinds(rt, page, l, ui, &b)?;
            }
            for r in REGIONS {
                let v = l.viewport(r);
                if v.w > 0.0 && self.visible(&v) {
                    self.draw_scrollbar(rt, l, ui, r)?;
                }
            }
            if page.settings.is_some() && self.visible(&l.dialog) {
                self.draw_settings(rt, page, l, ui, &b)?;
            }
            rt.EndDraw(None, None)
        }
    }

    // --- prymitywy ---

    fn hline(&self, rt: &ID2D1RenderTarget, x0: f32, x1: f32, y: f32, b: &ID2D1SolidColorBrush) {
        let (x0, x1, y) = (self.snap(x0), self.snap(x1), self.snap(y));
        unsafe { rt.FillRectangle(&rect(x0, y, x1 - x0, self.px()), b) };
    }

    fn vline(&self, rt: &ID2D1RenderTarget, x: f32, y0: f32, y1: f32, b: &ID2D1SolidColorBrush) {
        let (x, y0, y1) = (self.snap(x), self.snap(y0), self.snap(y1));
        unsafe { rt.FillRectangle(&rect(x, y0, self.px(), y1 - y0), b) };
    }

    /// Podwojna ramka z tytulem wpisanym w gorna linie.
    fn frame(&self, rt: &ID2D1RenderTarget, r: &Rc, title: &str, b: &Brushes) -> Result<()> {
        let p = self.px();
        for k in [0.0, 3.0 * p] {
            self.hline(rt, r.x + k, r.right() - k, r.y + k, &b.frame);
            self.hline(rt, r.x + k, r.right() - k, r.bottom() - p - k, &b.frame);
            self.vline(rt, r.x + k, r.y + k, r.bottom() - k, &b.frame);
            self.vline(rt, r.right() - p - k, r.y + k, r.bottom() - k, &b.frame);
        }
        if !title.is_empty() {
            let tw = self.measure_with(title, &self.f_mono_bold);
            unsafe {
                rt.FillRectangle(&rect(r.x + 12.0, r.y - 2.0, tw + 12.0, 4.0 * p + 4.0), &b.bg);
                let t = wstr(title);
                rt.DrawText(&t, &self.f_mono_bold, &rect(r.x + 18.0, r.y - self.mono_h / 2.0 + 1.5 * p, tw + 2.0, self.mono_h), &b.text, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn text(&self, rt: &ID2D1RenderTarget, s: &str, x: f32, y: f32, maxw: f32, f: &IDWriteTextFormat, brush: &ID2D1SolidColorBrush, underline: bool) -> Result<()> {
        unsafe {
            let t = wstr(s);
            if underline {
                let lay = self.dw.CreateTextLayout(&t, f, maxw.max(1.0), 200.0)?;
                lay.SetUnderline(true, DWRITE_TEXT_RANGE { startPosition: 0, length: t.len() as u32 })?;
                rt.DrawTextLayout(Vector2 { X: x, Y: y }, &lay, brush, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            } else {
                rt.DrawText(&t, f, &rect(x, y, maxw.max(1.0), 200.0), brush, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
            }
        }
        Ok(())
    }

    fn clip(&self, rt: &ID2D1RenderTarget, r: &Rc) {
        unsafe { rt.PushAxisAlignedClip(&rect(r.x, r.y, r.w, r.h), D2D1_ANTIALIAS_MODE_ALIASED) };
    }

    fn unclip(&self, rt: &ID2D1RenderTarget) {
        unsafe { rt.PopAxisAlignedClip() };
    }

    // --- pasek ulubionych ---

    fn draw_top(&mut self, rt: &ID2D1RenderTarget, page: &Page, l: &Layout, ui: &Ui, b: &Brushes, icons: &mut Icons) -> Result<()> {
        unsafe { rt.FillRectangle(&rect(l.top.x, l.top.y, l.top.w, l.top.h), &b.bar) };
        let p = self.px();
        self.hline(rt, 0.0, l.width, l.top.bottom() - p, &b.frame);
        self.hline(rt, 0.0, l.width, l.top.bottom() - 4.0 * p, &b.frame);
        let ly = (l.top.h - self.mono_h) / 2.0;
        // dyski, potem Eksplorator, potem ulubione
        for (i, (d, r)) in page.drives.iter().zip(&l.drives).enumerate() {
            self.icon_link(rt, d, r, ui.hover == Target::Drive(i), b, icons)?;
        }
        let e = &l.explorer;
        self.vline(rt, e.x - 12.0, l.top.y + 7.0, l.top.bottom() - 7.0, &b.frame);
        self.text(rt, &page.explorer.text, e.x, e.y, e.w + 4.0, &self.f_body.clone(), &b.link, ui.hover == Target::Explorer)?;
        let d = &l.downloads;
        self.text(rt, DOWNLOADS, d.x, d.y, d.w + 4.0, &self.f_body.clone(), &b.link, ui.hover == Target::Downloads)?;
        let fx = d.right() + 16.0;
        self.vline(rt, fx - 4.0, l.top.y + 7.0, l.top.bottom() - 7.0, &b.frame);
        self.text(rt, "Ulubione:", fx + 10.0, ly, 200.0, &self.f_mono_bold.clone(), &b.dim, false)?;
        for (i, (f, r)) in page.favs.iter().zip(&l.favs).enumerate() {
            if r.w > 0.0 {
                self.icon_link(rt, f, r, ui.hover == Target::Fav(i), b, icons)?;
            }
        }
        for (i, (a, r)) in page.actions.iter().zip(&l.actions).enumerate() {
            self.text(rt, &a.text, r.x, r.y, r.w + 4.0, &self.f_body.clone(), &b.link, ui.hover == Target::Action(i))?;
        }
        // zebatka ustawien
        let g = l.gear;
        let gb = if ui.hover == Target::Gear || page.settings.is_some() { &b.yellow } else { &b.dim };
        unsafe {
            let t = wstr("\u{2699}");
            rt.DrawText(&t, &self.f_symbol, &rect(g.x, g.y - 2.0, g.w, g.h + 4.0), gb, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        }
        Ok(())
    }

    /// Link z ikona 16 DIP; biezacy (Current) na zolto i pogrubiony.
    fn icon_link(&mut self, rt: &ID2D1RenderTarget, f: &crate::page::Link, r: &Rc, hot: bool, b: &Brushes, icons: &mut Icons) -> Result<()> {
        if let Some(ic) = &f.icon {
            self.draw_icon(rt, ic, r.x, r.y + (r.h - 16.0) / 2.0, 16.0, icons)?;
        }
        match f.style {
            LinkStyle::Current => self.text(rt, &f.text, r.x + 22.0, r.y, r.w - 20.0, &self.f_body_bold.clone(), &b.yellow, false),
            LinkStyle::Link => self.text(rt, &f.text, r.x + 22.0, r.y, r.w - 20.0, &self.f_body.clone(), &b.link, hot),
        }
    }

    // --- karty ---

    fn draw_tabs(&mut self, rt: &ID2D1RenderTarget, page: &Page, l: &Layout, ui: &Ui, b: &Brushes, icons: &mut Icons) -> Result<()> {
        let bar = l.tabbar;
        let p = self.px();
        self.hline(rt, 0.0, l.width, bar.bottom() - p, &b.frame);
        let ly = bar.y + (bar.h - self.mono_h) / 2.0;
        self.text(rt, "Karty:", 12.0, ly, 200.0, &self.f_mono_bold.clone(), &b.dim, false)?;
        if page.tabs.is_empty() {
            let x = 12.0 + self.measure_with("Karty:", &self.f_mono_bold) + 12.0;
            self.text(rt, "dwuklik na folderze w strukturze dodaje kartę", x, bar.y + (bar.h - self.line_h) / 2.0, 600.0, &self.f_body.clone(), &b.dim, false)?;
            return Ok(());
        }
        for (i, (t, (r, x))) in page.tabs.iter().zip(&l.tabs).enumerate() {
            if r.w <= 0.0 {
                continue;
            }
            let cur = t.style == LinkStyle::Current;
            // karta jak zakladka z Windows 3.1: obrys bez dolu, aktywna z jasniejszym tlem
            unsafe {
                rt.FillRectangle(&rect(r.x, r.y, r.w, r.h + 2.0 * p), if cur { &b.tab_on } else { &b.bar });
            }
            self.hline(rt, r.x, r.right(), r.y, &b.frame);
            self.vline(rt, r.x, r.y, r.bottom() + p, &b.frame);
            self.vline(rt, r.right() - p, r.y, r.bottom() + p, &b.frame);
            let ty = r.y + (r.h - self.line_h) / 2.0;
            self.draw_icon(rt, &IconKind::Folder, r.x + 5.0, r.y + (r.h - 16.0) / 2.0, 16.0, icons)?;
            let tw = r.w - 22.0 - if x.w > 0.0 { 28.0 } else { 6.0 };
            if cur {
                self.text(rt, &t.text, r.x + 25.0, ty, tw, &self.f_body_bold.clone(), &b.yellow, false)?;
            } else {
                self.text(rt, &t.text, r.x + 25.0, ty, tw, &self.f_body.clone(), &b.link, ui.hover == Target::Tab(i))?;
            }
            // [x] zamkniecia (karta Desktop jest stala)
            if x.w <= 0.0 {
                continue;
            }
            let xb = if ui.hover == Target::TabClose(i) { &b.err } else { &b.dim };
            self.text(rt, "×", x.x + 4.0, ty, 16.0, &self.f_body_bold.clone(), xb, false)?;
        }
        Ok(())
    }

    // --- drzewo ---

    fn draw_tree(&mut self, rt: &ID2D1RenderTarget, page: &Page, l: &Layout, ui: &Ui, b: &Brushes, icons: &mut Icons) -> Result<()> {
        self.frame(rt, &l.tree_frame, &page.tree_title, b)?;
        let v = l.tree;
        let scroll = ui.scroll[Region::Tree as usize];
        self.clip(rt, &v);
        let p = self.px();
        let first = ((scroll - 4.0) / l.row_h).floor().max(0.0) as usize;
        let last = (((scroll + v.h) / l.row_h).ceil() as usize + 1).min(page.tree.len());
        let lab_w = l.tree_label_w;
        let bx = |depth: u32| v.x + 6.0 + lab_w + depth as f32 * 18.0;
        for i in first..last {
            let n = &page.tree[i];
            let mut r = l.list_row(Region::Tree, i);
            r.y -= scroll;
            if !self.visible(&r) {
                continue;
            }
            let mid = self.snap(r.y + r.h / 2.0);
            // podpis jezyka z lewej strony (pelna nazwa, kolor jezyka), wyrownany do drzewa
            if let Some(t) = n.tech {
                let name = crate::profiles::LANGS[t].name;
                let mono = self.f_mono.clone();
                let w = self.measure_with(name, &mono);
                let br = self.lang_brush(rt, t)?;
                self.text(rt, name, v.x + lab_w - 4.0 - w, r.y + (r.h - self.mono_h) / 2.0, w + 2.0, &mono, &br, false)?;
            }
            // kropkowane linie (Windows 98): co drugi piksel, ostro, bez wygladzania
            let dots = |x0: f32, y0: f32, x1: f32, y1: f32| unsafe {
                let (x0p, y0p, x1p, y1p) = ((x0 / p).round() as i32, (y0 / p).round() as i32, (x1 / p).round() as i32, (y1 / p).round() as i32);
                if x0p == x1p {
                    for yy in (y0p..y1p).filter(|v| v % 2 == 0) {
                        rt.FillRectangle(&rect(x0p as f32 * p, yy as f32 * p, p, p), &b.dim);
                    }
                } else {
                    for xx in (x0p..x1p).filter(|v| v % 2 == 0) {
                        rt.FillRectangle(&rect(xx as f32 * p, y0p as f32 * p, p, p), &b.dim);
                    }
                }
            };
            for k in 0..n.depth.saturating_sub(1) {
                if n.guides & (1u64 << k) != 0 {
                    let x = bx(k) + 4.5;
                    dots(x, r.y, x, r.bottom());
                }
            }
            if n.depth > 0 {
                let x = bx(n.depth - 1) + 4.5;
                dots(x, r.y, x, if n.last { mid } else { r.bottom() });
                dots(x, mid, bx(n.depth) - 1.0, mid);
            }
            let x0 = bx(n.depth);
            if let Some(open) = n.toggle {
                // kwadracik [+] / [-]
                let (bxl, byt) = (self.snap(x0), self.snap(mid - 4.5));
                let sz = self.snap(9.0);
                unsafe { rt.FillRectangle(&rect(bxl, byt, sz, sz), &b.bg) };
                self.hline(rt, bxl, bxl + sz, byt, &b.dim);
                self.hline(rt, bxl, bxl + sz, byt + sz - p, &b.dim);
                self.vline(rt, bxl, byt, byt + sz, &b.dim);
                self.vline(rt, bxl + sz - p, byt, byt + sz, &b.dim);
                self.hline(rt, bxl + 2.0, bxl + sz - 2.0, mid, &b.text);
                if !open {
                    self.vline(rt, self.snap(bxl + sz / 2.0 - p / 2.0), byt + 2.0, byt + sz - 2.0, &b.text);
                }
            }
            let ix = x0 + 13.0;
            self.draw_icon(rt, &n.icon, ix, r.y + (r.h - 16.0) / 2.0, 16.0, icons)?;
            let tx = ix + 21.0;
            let tw = self.measure(&n.name, false);
            let ty = r.y + (r.h - self.line_h) / 2.0;
            let maxw = (v.right() - SCROLL_W - tx - 2.0).max(10.0);
            if n.current {
                self.text(rt, &n.name, tx, ty, maxw, &self.f_body_bold.clone(), &b.text, false)?;
            } else if ui.selected == Target::Tree(i) {
                unsafe { rt.FillRectangle(&rect(tx - 2.0, r.y + 1.0, tw.min(maxw) + 4.0, r.h - 2.0), &b.navy) };
                self.text(rt, &n.name, tx, ty, maxw, &self.f_body.clone(), &b.text, false)?;
            } else {
                self.text(rt, &n.name, tx, ty, maxw, &self.f_body.clone(), &b.link, ui.hover == Target::Tree(i))?;
            }
            if !n.note.is_empty() && tw + 12.0 < maxw {
                self.text(rt, &n.note, tx + tw + 8.0, ty, maxw - tw - 8.0, &self.f_body.clone(), &b.dim, false)?;
            }
        }
        if page.tree.is_empty() {
            self.text(rt, "(brak)", v.x + 8.0, v.y + 6.0, v.w, &self.f_body.clone(), &b.dim, false)?;
        }
        self.unclip(rt);
        Ok(())
    }

    // --- tytul + sciezka ---

    fn draw_head(&mut self, rt: &ID2D1RenderTarget, page: &Page, l: &Layout, b: &Brushes) -> Result<()> {
        let h = l.head;
        self.text(rt, &page.title, h.x + 2.0, h.y, h.w, &self.f_h1.clone(), &b.text, false)?;
        let y = h.y + self.h1_h + 2.0;
        match &page.message {
            Some((m, true)) => self.text(rt, m, h.x + 2.0, y, h.w, &self.f_body.clone(), &b.err, false)?,
            _ => self.text(rt, &page.subtitle, h.x + 2.0, y, h.w, &self.f_body.clone(), &b.dim, false)?,
        }
        // pasek jezykow (jak na GitHubie) + technologia / git / ostatnia zmiana
        let y2 = y + self.line_h + 4.0;
        let mut x = h.x + 2.0;
        if !page.langs.is_empty() {
            let bw = 220.0;
            let bh = self.snap(9.0);
            let by = self.snap(y2 + (self.line_h - bh) / 2.0);
            let mut sx = x;
            for (k, (_, color, frac)) in page.langs.iter().enumerate() {
                let w = if k + 1 == page.langs.len() { x + bw - sx } else { self.snap(bw * frac) };
                if w > 0.0 {
                    unsafe {
                        let c = rt.CreateSolidColorBrush(&rgb(*color), None)?;
                        rt.FillRectangle(&rect(sx, by, w, bh), &c);
                    }
                }
                sx += w;
            }
            x += bw + 14.0;
            for (name, color, frac) in &page.langs {
                let label = format!("{} {}%", name, (frac * 100.0).round() as u32);
                let lw = self.measure(&label, false);
                if x + lw + 16.0 > h.right() {
                    break;
                }
                unsafe {
                    let c = rt.CreateSolidColorBrush(&rgb(*color), None)?;
                    rt.FillRectangle(&rect(x, self.snap(y2 + (self.line_h - 8.0) / 2.0), 8.0, 8.0), &c);
                }
                self.text(rt, &label, x + 12.0, y2, lw + 2.0, &self.f_body.clone(), &b.text, false)?;
                x += lw + 26.0;
            }
        }
        if !page.head_info.is_empty() {
            self.text(rt, &page.head_info, x, y2, (h.right() - x).max(10.0), &self.f_body.clone(), &b.dim, false)?;
        }
        Ok(())
    }

    // --- foldery ---

    fn draw_folders(&mut self, rt: &ID2D1RenderTarget, page: &Page, l: &Layout, ui: &Ui, b: &Brushes, icons: &mut Icons) -> Result<()> {
        let title = format!("Projekty i foldery ({})", page.folders.len());
        self.frame(rt, &l.folders_frame, &title, b)?;
        let v = l.folders;
        let scroll = ui.scroll[Region::Folders as usize];
        self.clip(rt, &v);
        let first = ((scroll - 4.0) / l.row_h).floor().max(0.0) as usize;
        let last = (((scroll + v.h) / l.row_h).ceil() as usize + 1).min(page.folders.len());
        let mono = self.f_mono.clone();
        let date_w = self.measure_with("11 mies. temu", &mono);
        for i in first..last {
            let row = &page.folders[i];
            let mut r = l.list_row(Region::Folders, i);
            r.y -= scroll;
            if !self.visible(&r) {
                continue;
            }
            // naprzemienne tlo wierszy - lista rozni sie od drzewa jak listing z DOS-a
            if i % 2 == 1 {
                unsafe { rt.FillRectangle(&rect(r.x, r.y, r.w, r.h), &b.stripe) };
            }
            let sel = ui.selected == Target::Folder(i);
            if sel {
                unsafe { rt.FillRectangle(&rect(r.x, r.y, 3.0, r.h), &b.yellow) };
            }
            // folder | nazwa | "2 lata temu" | ● Jezyk - stale kolumny z prawej (rowna tabela jak na GitHubie)
            self.draw_icon(rt, &row.icon, r.x + 6.0, r.y + (r.h - 16.0) / 2.0, 16.0, icons)?;
            let tx = r.x + 28.0;
            let ty = r.y + (r.h - self.line_h) / 2.0;
            let mut right = r.right() - 6.0;
            if r.w >= 220.0 {
                if !row.ago.is_empty() && right - date_w > tx + 60.0 {
                    let aw = self.measure_with(&row.ago, &mono);
                    self.text(rt, &row.ago, right - aw, r.y + (r.h - self.mono_h) / 2.0, aw + 2.0, &mono, &b.dim, false)?;
                }
                right -= date_w + 12.0;
            }
            let maxw = (right - tx).max(20.0);
            if sel {
                self.text(rt, &row.display, tx, ty, maxw, &self.f_body_bold.clone(), &b.yellow, false)?;
            } else {
                self.text(rt, &row.display, tx, ty, maxw, &self.f_body.clone(), &b.link, ui.hover == Target::Folder(i))?;
            }
        }
        if page.folders.is_empty() {
            self.text(rt, "(brak podfolderów)", v.x + 8.0, v.y + 6.0, v.w, &self.f_body.clone(), &b.dim, false)?;
        }
        self.unclip(rt);
        Ok(())
    }

    /// Kropka jezyka w kolorze z GitHuba (srodek `cx`,`cy`). Zbyt ciemne kolory
    /// (C, Lua, Assembly...) rozjasnione, zeby byly widoczne na czarnym tle.
    fn lang_dot(&self, rt: &ID2D1RenderTarget, lang: usize, cx: f32, cy: f32) -> Result<()> {
        let br = self.lang_brush(rt, lang)?;
        unsafe {
            rt.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: 5.0, radiusY: 5.0 }, &br);
        }
        Ok(())
    }

    /// Kolor jezyka (GitHub); zbyt ciemne rozjasnione, zeby byly widoczne na czarnym tle.
    fn lang_brush(&self, rt: &ID2D1RenderTarget, lang: usize) -> Result<ID2D1SolidColorBrush> {
        let c = crate::profiles::LANGS[lang].color;
        let (r, g, bl) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
        let lum = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * bl as f32;
        let c = if lum < 90.0 { ((r + 0xB0) / 2) << 16 | ((g + 0xB0) / 2) << 8 | (bl + 0xB0) / 2 } else { c };
        unsafe { rt.CreateSolidColorBrush(&rgb(c), None) }
    }

    /// Wypukla ramka jak przycisk z Windows 95.
    fn bevel(&self, rt: &ID2D1RenderTarget, r: &Rc, pressed: bool, b: &Brushes) {
        let p = self.px();
        let (lt, rb) = if pressed { (&b.frame, &b.dim) } else { (&b.dim, &b.frame) };
        self.hline(rt, r.x, r.right(), r.y, lt);
        self.vline(rt, r.x, r.y, r.bottom(), lt);
        self.hline(rt, r.x, r.right(), r.bottom() - p, rb);
        self.vline(rt, r.right() - p, r.y, r.bottom(), rb);
    }

    // --- przyciski otwartych aplikacji ---

    fn draw_apps(&mut self, rt: &ID2D1RenderTarget, page: &Page, l: &Layout, ui: &Ui, b: &Brushes, icons: &mut Icons) -> Result<()> {
        let row = l.apps_row;
        unsafe { rt.FillRectangle(&rect(row.x, row.y, row.w, row.h), &b.bar) };
        for (i, ((title, icon, minimized), r)) in page.apps.iter().zip(&l.apps).enumerate() {
            let hot = ui.hover == Target::App(i);
            unsafe { rt.FillRectangle(&rect(r.x, r.y, r.w, r.h), if hot { &b.tab_on } else { &b.bg }) };
            self.bevel(rt, r, *minimized, b);
            self.draw_icon(rt, icon, r.x + 6.0, r.y + (r.h - 16.0) / 2.0, 16.0, icons)?;
            let brush = if *minimized { &b.dim } else { &b.text };
            self.text(rt, title, r.x + 27.0, r.y + (r.h - self.line_h) / 2.0, r.w - 32.0, &self.f_body.clone(), brush, hot)?;
        }
        Ok(())
    }

    // --- projekty na calym dysku ---

    fn draw_kinds(&mut self, rt: &ID2D1RenderTarget, page: &Page, l: &Layout, ui: &Ui, b: &Brushes) -> Result<()> {
        let f = l.kinds_frame;
        let total: usize = page.kinds.iter().map(|k| k.1).sum();
        let title = if page.scan.is_some() { "Projekty na dysku".to_string() } else { format!("Projekty na dysku ({})", total) };
        self.frame(rt, &f, &title, b)?;
        if let Some((pct, scanned)) = page.scan {
            let y = f.y + 18.0;
            let label = if pct > 0.0 {
                format!("Przeszukiwanie dysku: {}%", (pct * 100.0).round() as u32)
            } else {
                "Przeszukiwanie dysku…".to_string()
            };
            self.text(rt, &label, f.x + 14.0, y, 360.0, &self.f_body.clone(), &b.text, false)?;
            let w = self.measure(&label, false);
            let sx = f.x + 14.0 + w + 14.0;
            if pct > 0.0 {
                self.segments(rt, sx, y + 3.0, 20, pct)?;
            }
            let info = format!("{} plików przejrzanych", crate::app::group_digits(scanned));
            self.text(rt, &info, f.x + 14.0, y + self.line_h + 4.0, f.w - 28.0, &self.f_mono.clone(), &b.dim, false)?;
            return Ok(());
        }
        if page.kinds.is_empty() {
            self.text(rt, "(nie znaleziono projektów w indeksowanych folderach)", f.x + 14.0, f.y + 18.0, f.w - 28.0, &self.f_body.clone(), &b.dim, false)?;
        }
        // ● Node.js 79   ● C++ 91 ... - bez ramek, jak jezyki na GitHubie
        let mono = self.f_mono.clone();
        for (i, ((kind, n), r)) in page.kinds.iter().zip(&l.kinds).enumerate() {
            let active = page.kind_active == Some(*kind);
            let hot = ui.hover == Target::Kind(i);
            self.lang_dot(rt, *kind, r.x + 5.0, r.y + r.h / 2.0)?;
            let name = crate::profiles::short(*kind);
            let ty = r.y + (r.h - self.line_h) / 2.0;
            let nw = self.measure(name, active);
            if active {
                self.text(rt, name, r.x + 15.0, ty, nw + 2.0, &self.f_body_bold.clone(), &b.text, false)?;
            } else {
                self.text(rt, name, r.x + 15.0, ty, nw + 2.0, &self.f_body.clone(), &b.link, hot)?;
            }
            let cnt = n.to_string();
            let cw = self.measure_with(&cnt, &mono);
            self.text(rt, &cnt, r.x + 15.0 + nw + 6.0, r.y + (r.h - self.mono_h) / 2.0, cw + 2.0, &mono, if active { &b.text } else { &b.dim }, false)?;
        }
        Ok(())
    }

    // --- okno ustawien (jak okno dialogowe Windows 95) ---

    fn draw_settings(&mut self, rt: &ID2D1RenderTarget, page: &Page, l: &Layout, ui: &Ui, b: &Brushes) -> Result<()> {
        let Some(items) = &page.settings else { return Ok(()) };
        let d = l.dialog;
        unsafe {
            let shadow = rt.CreateSolidColorBrush(&rgb(0x000000), None)?;
            rt.FillRectangle(&rect(d.x + 6.0, d.y + 6.0, d.w, d.h), &shadow);
            rt.FillRectangle(&rect(d.x, d.y, d.w, d.h), &b.bar);
            rt.FillRectangle(&rect(d.x + 3.0, d.y + 3.0, d.w - 6.0, 28.0), &b.navy);
        }
        self.bevel(rt, &d, false, b);
        self.text(rt, "Ustawienia Indeksu", d.x + 12.0, d.y + 3.0 + (28.0 - self.line_h) / 2.0, d.w - 60.0, &self.f_body_bold.clone(), &b.text, false)?;
        let c = l.dialog_close;
        let cb = if ui.hover == Target::SetClose { &b.err } else { &b.text };
        self.text(rt, "×", c.x + 8.0, c.y + (c.h - self.line_h) / 2.0, 20.0, &self.f_body_bold.clone(), cb, false)?;
        for (i, ((label, kind), (r, track))) in items.iter().zip(l.set_rows.iter().zip(&l.set_tracks)).enumerate() {
            let ty = r.y + (r.h - self.line_h) / 2.0;
            if ui.hover == Target::SetItem(i) {
                unsafe { rt.FillRectangle(&rect(r.x, r.y, r.w, r.h), &b.tab_on) };
            }
            match kind {
                crate::page::SetKind::Toggle(on) => {
                    // pole wyboru jak w Win95
                    let bx = Rc { x: r.x + 6.0, y: r.y + (r.h - 16.0) / 2.0, w: 16.0, h: 16.0 };
                    unsafe { rt.FillRectangle(&rect(bx.x, bx.y, bx.w, bx.h), &b.bg) };
                    self.bevel(rt, &bx, true, b);
                    if *on {
                        unsafe { rt.FillRectangle(&rect(bx.x + 4.0, bx.y + 4.0, 8.0, 8.0), &b.yellow) };
                    }
                    self.text(rt, label, r.x + 32.0, ty, r.w - 40.0, &self.f_body.clone(), &b.text, false)?;
                }
                crate::page::SetKind::Slider(v) => {
                    self.text(rt, label, r.x + 32.0, ty, 300.0, &self.f_body.clone(), &b.text, false)?;
                    let t = track;
                    unsafe { rt.FillRectangle(&rect(t.x, t.y + 3.0, t.w, 6.0), &b.bg) };
                    self.bevel(rt, &Rc { x: t.x, y: t.y + 3.0, w: t.w, h: 6.0 }, true, b);
                    let kx = t.x + t.w * (*v as f32 / 100.0) - 5.0;
                    let knob = Rc { x: kx, y: t.y - 4.0, w: 10.0, h: 20.0 };
                    unsafe { rt.FillRectangle(&rect(knob.x, knob.y, knob.w, knob.h), &b.tab_on) };
                    self.bevel(rt, &knob, false, b);
                    self.text(rt, &format!("{}%", v), t.right() + 14.0, ty, 60.0, &self.f_mono_bold.clone(), &b.yellow, false)?;
                }
            }
        }
        Ok(())
    }

    // --- pliki: siatka ikon ---

    fn draw_files(&mut self, rt: &ID2D1RenderTarget, page: &Page, l: &Layout, ui: &Ui, b: &Brushes, icons: &mut Icons) -> Result<()> {
        let title = format!("Pliki ({})", page.files.len());
        self.frame(rt, &l.files_frame, &title, b)?;
        let v = l.files;
        let scroll = ui.scroll[Region::Files as usize];
        self.clip(rt, &v);
        let cols = l.cols.max(1);
        let first_row = ((scroll - 8.0) / l.tile_h).floor().max(0.0) as usize;
        let last_row = ((scroll + v.h) / l.tile_h).ceil() as usize + 1;
        let first = first_row * cols;
        let last = (last_row * cols).min(page.files.len());
        for i in first..last {
            let row = &page.files[i];
            let mut t = l.tile(i);
            t.y -= scroll;
            if !self.visible(&t) {
                continue;
            }
            let hot = ui.hover == Target::File(i);
            let sel = ui.selected == Target::File(i);
            if hot && !sel {
                unsafe { rt.FillRectangle(&rect(t.x + 2.0, t.y + 2.0, t.w - 4.0, t.h - 4.0), &b.hover) };
            }
            self.draw_icon(rt, &row.icon, t.x + (t.w - TILE_ICON) / 2.0, t.y + 6.0, TILE_ICON, icons)?;
            let nr = Rc { x: t.x + 4.0, y: t.y + TILE_ICON + 12.0, w: t.w - 8.0, h: self.tile_line_h * 2.0 + 1.0 };
            unsafe {
                let txt = wstr(&row.display);
                let lay = self.dw.CreateTextLayout(&txt, &self.f_tile, nr.w, nr.h)?;
                if sel {
                    let mut m = DWRITE_TEXT_METRICS::default();
                    lay.GetMetrics(&mut m)?;
                    let bw = m.width.min(nr.w) + 6.0;
                    rt.FillRectangle(&rect(nr.x + (nr.w - bw) / 2.0, nr.y, bw, m.height.min(nr.h) + 1.0), &b.navy);
                } else if hot {
                    lay.SetUnderline(true, DWRITE_TEXT_RANGE { startPosition: 0, length: txt.len() as u32 })?;
                }
                rt.PushAxisAlignedClip(&rect(nr.x - 3.0, nr.y, nr.w + 6.0, nr.h + 1.0), D2D1_ANTIALIAS_MODE_ALIASED);
                rt.DrawTextLayout(Vector2 { X: nr.x, Y: nr.y }, &lay, if sel { &b.text } else { &b.link }, D2D1_DRAW_TEXT_OPTIONS_NONE);
                rt.PopAxisAlignedClip();
            }
        }
        if page.files.is_empty() && page.message.is_none() {
            self.text(rt, "(brak plików)", v.x + 8.0, v.y + 6.0, v.w, &self.f_body.clone(), &b.dim, false)?;
        }
        self.unclip(rt);
        Ok(())
    }

    // --- tabela wynikow szukania ---

    fn draw_table(&mut self, rt: &ID2D1RenderTarget, page: &Page, l: &Layout, ui: &Ui, b: &Brushes, icons: &mut Icons) -> Result<()> {
        self.frame(rt, &l.files_frame, &format!("Pliki ({})", page.table.len()), b)?;
        let v = l.table;
        let scroll = ui.scroll[Region::Table as usize];
        self.clip(rt, &v);
        // naglowek kolumn
        let hy = v.y + 4.0 - scroll + 1.0;
        for (s, c) in [("Nazwa", l.name), ("Folder", l.folder), ("Rozmiar", l.size), ("Data modyfikacji", l.date)] {
            unsafe {
                let t = wstr(s);
                rt.DrawText(&t, &self.f_bold_center, &rect(c.x + 1.0, hy, c.w - 2.0, self.line_h), &b.text, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            }
        }
        let first = ((scroll - 4.0) / l.row_h).floor().max(1.0) as usize - 1;
        let last = (((scroll + v.h) / l.row_h).ceil() as usize + 1).min(page.table.len());
        for i in first..last {
            let r = &page.table[i];
            let mut rr = l.table_row(i);
            rr.y -= scroll;
            if !self.visible(&rr) {
                continue;
            }
            if ui.selected == Target::Row(i) {
                unsafe { rt.FillRectangle(&rect(rr.x, rr.y, rr.w, rr.h), &b.navy) };
            }
            let ty = rr.y + (rr.h - self.line_h) / 2.0;
            let sel = ui.selected == Target::Row(i);
            let lb = if sel { &b.text } else { &b.link };
            self.draw_icon(rt, &r.icon, l.name.x + 1.0, rr.y + (rr.h - 16.0) / 2.0, 16.0, icons)?;
            self.text(rt, &r.display, l.name.x + 25.0, ty, l.name.w - 27.0, &self.f_body.clone(), lb, ui.hover == Target::Row(i))?;
            self.text(rt, &r.folder_s, l.folder.x + COL_PAD, ty, l.folder.w - COL_PAD - 2.0, &self.f_body.clone(), lb, ui.hover == Target::RowFolder(i))?;
            unsafe {
                let t = wstr(&r.size_s);
                rt.DrawText(&t, &self.f_body_right, &rect(l.size.x + 1.0, ty, l.size.w - 2.0, self.line_h), &b.text, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
                let t = wstr(&r.date_s);
                rt.DrawText(&t, &self.f_body_right, &rect(l.date.x + 1.0, ty, l.date.w - 2.0, self.line_h), &b.text, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            }
        }
        if page.table.is_empty() {
            self.text(rt, "Brak plików.", v.x + 8.0, v.y + 8.0 + l.row_h, v.w, &self.f_body.clone(), &b.dim, false)?;
        }
        self.unclip(rt);
        Ok(())
    }

    // --- pasek stanu ---

    fn segments(&self, rt: &ID2D1RenderTarget, x: f32, y: f32, n: usize, frac: f32) -> Result<f32> {
        let filled = ((frac.clamp(0.0, 1.0) * n as f32).round()) as usize;
        let color = if frac < 0.75 { CGA_GREEN } else if frac < 0.9 { CGA_YELLOW } else { CGA_RED };
        unsafe {
            let on = rt.CreateSolidColorBrush(&rgb(color), None)?;
            let off = rt.CreateSolidColorBrush(&rgb(SEG_EMPTY), None)?;
            let (sw, gap, h) = (self.snap(6.0), self.snap(2.0), self.snap(13.0));
            let y = self.snap(y);
            for k in 0..n {
                let sx = self.snap(x + k as f32 * (sw + gap));
                rt.FillRectangle(&rect(sx, y, sw, h), if k < filled { &on } else { &off });
            }
            Ok(n as f32 * (sw + gap))
        }
    }

    fn draw_status(&mut self, rt: &ID2D1RenderTarget, page: &Page, l: &Layout, ui: &Ui, b: &Brushes) -> Result<()> {
        let s = l.status;
        let p = self.px();
        unsafe { rt.FillRectangle(&rect(s.x, s.y, s.w, s.h), &b.bar) };
        self.hline(rt, 0.0, l.width, s.y, &b.frame);
        self.hline(rt, 0.0, l.width, s.y + 3.0 * p, &b.frame);
        let ty = s.y + (s.h - self.mono_h) / 2.0 + 1.0;
        let by = s.y + (s.h - 13.0) / 2.0 + 1.0;
        let mut x = 12.0;
        let mono = self.f_mono.clone();
        let mono_b = self.f_mono_bold.clone();
        let st = &page.status;
        let item = |rt: &ID2D1RenderTarget, label: &str, frac: f32, n: usize, info: &str, x: &mut f32| -> Result<()> {
            self.text(rt, label, *x, ty, 200.0, &mono_b, &b.cyan, false)?;
            *x += self.measure_with(label, &mono_b) + 6.0;
            *x += self.segments(rt, *x, by, n, frac)? + 6.0;
            self.text(rt, info, *x, ty, 400.0, &mono, &b.text, false)?;
            *x += self.measure_with(info, &mono) + 14.0;
            self.vline(rt, *x - 7.0, s.y + 7.0, s.bottom() - 4.0, &b.frame);
            Ok(())
        };
        for (root, total, free) in &st.drives {
            let used = 1.0 - *free as f32 / (*total).max(1) as f32;
            let info = format!("{:>3}% {} wolne z {}", (used * 100.0).round() as u32, stats::gb(*free), stats::gb(*total));
            item(rt, root.trim_end_matches('\\'), used, 16, &info, &mut x)?;
        }
        item(rt, "CPU", st.cpu, 10, &format!("{:>3}%", (st.cpu * 100.0).round() as u32), &mut x)?;
        item(rt, "GPU", st.gpu, 10, &format!("{:>3}%", (st.gpu * 100.0).round() as u32), &mut x)?;
        let ram = st.ram_used as f32 / st.ram_total.max(1) as f32;
        let info = format!("{:>3}% {}/{}", (ram * 100.0).round() as u32, stats::gb(st.ram_used), stats::gb(st.ram_total));
        item(rt, "RAM", ram, 10, &info, &mut x)?;
        let own = format!(
            "Indeks: RAM {} · CPU {} · {} plików w indeksie",
            stats::mb(st.own),
            if st.own_cpu < 0.001 { "0%".to_string() } else { format!("{:.1}%", st.own_cpu * 100.0).replace('.', ",") },
            crate::app::group_digits(ui.index_count)
        );
        let ow = self.measure_with(&own, &mono);
        if x + ow + 12.0 < l.width {
            self.text(rt, &own, l.width - ow - 12.0, ty, ow + 4.0, &mono, &b.dim, false)?;
        }
        Ok(())
    }

    fn draw_scrollbar(&self, rt: &ID2D1RenderTarget, l: &Layout, ui: &Ui, r: Region) -> Result<()> {
        if let Some((ty, th)) = scrollbar_thumb(l, ui, r) {
            let v = l.viewport(r);
            let hot = ui.hover == Target::Scroll(r) || ui.drag == Some(r);
            unsafe {
                let tb = rt.CreateSolidColorBrush(&rgb(0x1A1A1A), None)?;
                rt.FillRectangle(&rect(v.right() - SCROLL_W, v.y, SCROLL_W, v.h), &tb);
                let c = rt.CreateSolidColorBrush(&rgb(if hot { 0x8A8A8A } else { 0x555555 }), None)?;
                rt.FillRectangle(&rect(v.right() - SCROLL_W + 2.0, ty, SCROLL_W - 4.0, th), &c);
            }
        }
        Ok(())
    }

    // --- ikony ---

    fn fill_shape(&self, rt: &ID2D1RenderTarget, g: &ID2D1Geometry, fill: u32, edge: u32, x: f32, y: f32, k: f32) -> Result<()> {
        unsafe {
            let m = Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: x - self.ox, M32: y - self.oy };
            rt.SetTransform(&m);
            let fb = rt.CreateSolidColorBrush(&rgb(fill), None)?;
            let eb = rt.CreateSolidColorBrush(&rgb(edge), None)?;
            rt.FillGeometry(g, &fb, None);
            rt.DrawGeometry(g, &eb, 0.75, None);
            rt.SetTransform(&self.base());
        }
        Ok(())
    }

    fn badge(&self, rt: &ID2D1RenderTarget, label: &str, bg: u32, fg: u32, x: f32, y: f32, k: f32) -> Result<()> {
        unsafe {
            let m = Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: x - self.ox, M32: y - self.oy };
            rt.SetTransform(&m);
            let b = rt.CreateSolidColorBrush(&rgb(bg), None)?;
            let f = rt.CreateSolidColorBrush(&rgb(fg), None)?;
            let rr = D2D1_ROUNDED_RECT { rect: rect(0.5, 1.5, 15.0, 13.0), radiusX: 2.5, radiusY: 2.5 };
            rt.FillRoundedRectangle(&rr, &b);
            let fmt = if label.chars().count() <= 2 { &self.f_badge2 } else { &self.f_badge3 };
            let t = wstr(label);
            rt.DrawText(&t, fmt, &rect(-2.0, 1.0, 20.0, 13.5), &f, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            rt.SetTransform(&self.base());
        }
        Ok(())
    }

    fn page_icon(&self, rt: &ID2D1RenderTarget, x: f32, y: f32, k: f32) -> Result<()> {
        self.fill_shape(rt, &self.shapes.page, 0xF8F8F7, 0x8C8C8C, x, y, k)?;
        self.fill_shape(rt, &self.shapes.fold, 0xD6D6D6, 0x8C8C8C, x, y, k)
    }

    /// Ikona w kwadracie `size` DIP (16 albo 32).
    fn draw_icon(&mut self, rt: &ID2D1RenderTarget, kind: &IconKind, x: f32, y: f32, size: f32, icons: &mut Icons) -> Result<()> {
        let k = size / 16.0;
        match kind {
            IconKind::Folder => {
                self.fill_shape(rt, &self.shapes.folder_back, 0xE2BE4F, 0xB8922E, x, y, k)?;
                self.fill_shape(rt, &self.shapes.folder_front, 0xF6DE7C, 0xC9A23A, x, y, k)?;
            }
            IconKind::File => self.page_icon(rt, x, y, k)?,
            IconKind::Drive => unsafe {
                let b = rt.CreateSolidColorBrush(&rgb(0xC8C8C8), None)?;
                let e = rt.CreateSolidColorBrush(&rgb(0x7A7A7A), None)?;
                let led = rt.CreateSolidColorBrush(&rgb(0x37C837), None)?;
                let rr = D2D1_ROUNDED_RECT { rect: rect(x + k, y + 5.0 * k, 14.0 * k, 7.0 * k), radiusX: 1.5 * k, radiusY: 1.5 * k };
                rt.FillRoundedRectangle(&rr, &b);
                rt.DrawRoundedRectangle(&rr, &e, 0.75, None);
                rt.FillRectangle(&rect(x + 11.0 * k, y + 9.0 * k, 2.0 * k, 1.5 * k), &led);
            },
            IconKind::Badge(bd) => self.badge(rt, bd.label, bd.bg, bd.fg, x, y, k)?,
            IconKind::System(key) => match icons.get(key) {
                IconState::Ready(px) => {
                    if !self.bitmaps.contains_key(key) {
                        let px = px.clone();
                        let props = D2D1_BITMAP_PROPERTIES {
                            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                            dpiX: 96.0,
                            dpiY: 96.0,
                        };
                        let bmp = unsafe { rt.CreateBitmap(D2D_SIZE_U { width: px.w, height: px.h }, Some(px.bgra.as_ptr() as *const _), px.w * 4, &props)? };
                        self.bitmaps.insert(key.clone(), bmp);
                    }
                    unsafe {
                        rt.DrawBitmap(&self.bitmaps[key], Some(&rect(x, y, size, size)), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
                    }
                }
                IconState::Unknown => {
                    let label: String = key.trim_start_matches('.').chars().take(3).collect::<String>().to_uppercase();
                    self.badge(rt, &label, 0x4A4A4A, 0xE8E8E8, x, y, k)?;
                }
                IconState::Pending | IconState::Failed => self.page_icon(rt, x, y, k)?,
            },
        }
        Ok(())
    }
}

/// Wnetrze ramki (pod tytulem), wyrownane do pikseli.
fn inner(frame: &Rc, s: f32) -> Rc {
    let snap = |v: f32| (v * s).round() / s;
    let x = snap(frame.x + 7.0);
    let y = snap(frame.y + 9.0);
    Rc { x, y, w: snap(frame.right() - 7.0) - x, h: snap(frame.bottom() - 6.0) - y }
}

/// Suwak regionu: (gora, wysokosc) w DIP okna albo None.
pub fn scrollbar_thumb(l: &Layout, ui: &Ui, r: Region) -> Option<(f32, f32)> {
    let v = l.viewport(r);
    let content = l.content[r as usize];
    if v.h <= 0.0 || content <= v.h + 0.5 {
        return None;
    }
    let th = (v.h * v.h / content).max(28.0);
    let max_scroll = content - v.h;
    let ty = v.y + (v.h - th) * (ui.scroll[r as usize] / max_scroll).clamp(0.0, 1.0);
    Some((ty, th))
}
