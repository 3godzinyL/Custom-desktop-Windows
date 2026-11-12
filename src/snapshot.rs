// Zrzut strony do PNG bez pokazywania okna (ten sam kod rysujacy co na pulpicie).
// Uzycie: indeks.exe --zrzut plik.png [--folder SCIEZKA] [--szukaj TEKST]
//                   [--szer PX] [--wys PX] [--dpi DPI] [--najedz N]
use windows::core::Result;
use windows::Win32::Foundation::{GENERIC_WRITE, HWND};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::D2D1_RENDER_TARGET_PROPERTIES;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppPBGRA,
    IWICImagingFactory, WICBitmapCacheOnLoad, WICBitmapEncoderNoCache,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use crate::app::App;
use crate::page::Target;
use crate::config::Config;
use crate::render::Renderer;
use crate::search;
use crate::util::{pcwstr, wide};

pub struct Opts {
    pub out: String,
    pub folder: Option<String>,
    pub query: Option<String>,
    /// Zbuduj indeks i poczekaj na silnik projektow (plakietki, jezyki).
    pub index: bool,
    pub width: u32,
    pub height: u32,
    pub dpi: f32,
    pub hover_row: Option<usize>,
    /// Rozwinieta zakladka optymalizacji (dwa przejscia silnika, nic nie jest dlawione).
    pub opt: bool,
    pub select: Option<usize>,
    /// Pokaz okno ustawien.
    pub settings: bool,
    /// Filtr projektow wg jezyka (indeks w profiles::LANGS).
    pub kind: Option<usize>,
    /// Klik w wiersz drzewa (sprawdzenie, ze drzewo nie przeskakuje).
    pub tree_click: Option<usize>,
    /// Bez przyciskow otwartych aplikacji (prywatne tytuly okien) - zrzuty do README.
    pub demo: bool,
    /// Panel "Projekty na dysku" (lista z katalogu; prawa czesc rysuje indeks-panel.exe --zrzut).
    pub disk: bool,
}

pub fn run(o: Opts) -> Result<()> {
    let mut cfg = Config::load();
    if let Some(f) = &o.folder {
        cfg.home = f.clone();
    }
    if o.query.is_some() || o.index {
        search::start(cfg.roots.clone(), cfg.skip.clone(), HWND::default());
        let t0 = std::time::Instant::now();
        while !search::status().ready && t0.elapsed().as_secs() < 120 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    crate::profiles::start(HWND::default());
    crate::katalog::start_light(HWND::default());
    let mut app = App::new(cfg, HWND::default())?;
    if o.disk {
        app.fill_kinds();
        app.set_disk(true);
    }
    if let Some(q) = &o.query {
        for ch in q.chars() {
            app.search_input(ch);
        }
    }
    if o.index {
        // silnik projektow liczy w tle - czekamy, az wyniki przestana sie zmieniac
        let t0 = std::time::Instant::now();
        let mut quiet = 0;
        while t0.elapsed().as_secs() < 15 && quiet < 5 {
            std::thread::sleep(std::time::Duration::from_millis(150));
            if app.fill_profiles() { quiet = 0 } else { quiet += 1 }
        }
        // skan projektow na calym indeksie (panel "Projekty na dysku")
        app.fill_kinds();
        let t0 = std::time::Instant::now();
        while crate::profiles::project_counts().is_none() && t0.elapsed().as_secs() < 30 {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        app.fill_kinds();
        if o.kind.is_some() {
            app.set_kind(o.kind);
        }
    }
    if !o.demo {
        app.update_apps();
    }
    if o.settings {
        app.toggle_settings(true);
    }
    if o.opt {
        app.cfg.optimize = true;
        crate::optymalizator::tick(true, 2.0);
        std::thread::sleep(std::time::Duration::from_millis(1000));
        crate::optymalizator::tick(true, 1.0);
        app.update_stats();
        app.toggle_opt_reveal(true);
    }
    app.view_w = o.width as f32 * 96.0 / o.dpi;
    app.view_h = o.height as f32 * 96.0 / o.dpi;
    app.ensure_layout();
    app.dpi = o.dpi;
    app.ensure_layout();
    if let Some(i) = o.hover_row {
        app.hover = Target::File(i);
    }
    if let Some(i) = o.tree_click {
        app.activate(Target::Tree(i), false);
        app.ensure_layout();
    }
    if let Some(i) = o.select {
        app.select(Target::Folder(i));
    }
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let bmp = wic.CreateBitmap(o.width, o.height, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: o.dpi,
            dpiY: o.dpi,
            ..Default::default()
        };
        let rt = app.renderer.d2d.CreateWicBitmapRenderTarget(&bmp, &props)?;
        Renderer::grayscale_text(&rt);
        // pierwsze rysowanie zamawia ikony systemowe; czekamy na nie i rysujemy jeszcze raz
        let ui = app.ui();
        app.renderer.draw_full(&rt, &app.page, &app.layout, &ui, &mut app.icons)?;
        let t0 = std::time::Instant::now();
        loop {
            app.icons.drain();
            if app.icons.pending() == 0 || t0.elapsed().as_secs() > 8 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        let ui = app.ui();
        app.renderer.draw_full(&rt, &app.page, &app.layout, &ui, &mut app.icons)?;

        let wout = wide(&o.out);
        let stream = wic.CreateStream()?;
        stream.InitializeFromFilename(pcwstr(&wout), GENERIC_WRITE.0)?;
        let enc = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        enc.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        enc.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.ok_or_else(windows::core::Error::empty)?;
        frame.Initialize(None)?;
        frame.SetSize(o.width, o.height)?;
        let mut fmt = GUID_WICPixelFormat32bppPBGRA;
        frame.SetPixelFormat(&mut fmt)?;
        frame.WriteSource(&bmp, std::ptr::null())?;
        frame.Commit()?;
        enc.Commit()?;
    }
    Ok(())
}
