// Okna: widok (rysuje strone) i ukryty kontroler (ikona w zasobniku, pilnowanie
// podpiecia do pulpitu, restart Explorera, zamykanie z przywroceniem pulpitu).
use std::cell::{Cell, RefCell};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, ClientToScreen, EndPaint, InvalidateRect, HDC, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
    VK_BACK, VK_CONTROL, VK_DOWN, VK_END, VK_ESCAPE, VK_F5, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_UP,
};
use windows::Win32::UI::Shell::{
    SHGetStockIconInfo, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
    NOTIFYICONDATAW, SHGSI_ICON, SHGSI_SMALLICON, SHSTOCKICONINFO, SIID_FOLDER,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{self, App};
use crate::config::Config;
use crate::page::{Rc, Target, REGIONS};
use crate::shell::{self, item, sep, Extra, MenuResult};
use crate::util::{self, wide};
use crate::{autostart, desktop, icons, search};

pub const CTL_CLASS: PCWSTR = w!("IndeksKontroler");
const VIEW_CLASS: PCWSTR = w!("IndeksWidok");

pub const WM_APP_TRAY: u32 = WM_APP + 1;
pub const WM_APP_SHOW: u32 = WM_APP + 2;
/// Tytuly okien kontrolera - zeby drugi start trafil do wlasciwej instancji.
pub const CTL_TITLE_DESKTOP: PCWSTR = w!("Indeks");
pub const CTL_TITLE_WINDOW: PCWSTR = w!("Indeks.Okno");
/// WM_COPYDATA: otworz folder / pokaz plik (dwTypeData).
const COPY_OPEN: usize = 0x494E4458;
/// WM_COPYDATA: otworz sciezke w nowym oknie Indeksu (w procesie pulpitu).
pub const COPY_WINDOW: usize = 0x494E4459;
/// COPYDATASTRUCT (bez dodatkowej cechy crate windows).
#[repr(C)]
struct CopyData {
    data: usize,
    len: u32,
    ptr: *const u8,
}

const WM_MOUSELEAVE: u32 = 0x02A3;

const T_CHECK: usize = 1;
const T_RELOAD: usize = 2;
const T_REATTACH: usize = 3;
const T_STATS: usize = 4;

const ID_PIN: u32 = 0x8001;
const ID_EXPLORER: u32 = 0x8002;
const ID_TERMINAL: u32 = 0x8003;
const ID_REVEAL: u32 = 0x8004;
const ID_GOTO: u32 = 0x8005;
const ID_HOME_HERE: u32 = 0x8006;
const ID_REFRESH: u32 = 0x8007;
const ID_PROTECTED: u32 = 0x8008;
const ID_QUIT: u32 = 0x8009;
const ID_UNPIN: u32 = 0x8010;
const ID_LEFT: u32 = 0x8011;
const ID_RIGHT: u32 = 0x8012;
const ID_AUTOSTART: u32 = 0x8013;
const ID_SETTINGS: u32 = 0x8014;
const ID_HOME: u32 = 0x8015;
const ID_WINDOW: u32 = 0x8016;
const ID_TAB: u32 = 0x8017;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Desktop,
    Window,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static MODE: Cell<Mode> = const { Cell::new(Mode::Desktop) };
    static OPT_LAST: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
    static CTL: Cell<isize> = const { Cell::new(0) };
    static VIEW: Cell<isize> = const { Cell::new(0) };
    static HOST: Cell<isize> = const { Cell::new(0) };
    static ICONS_HOST: Cell<isize> = const { Cell::new(0) };
    static HOOK: Cell<isize> = const { Cell::new(0) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    static QUITTING: Cell<bool> = const { Cell::new(false) };
    static PAINTS: Cell<u32> = const { Cell::new(0) };
    static OVERLAY: RefCell<Option<crate::overlay::Overlay>> = const { RefCell::new(None) };
    /// Ostatnie okno na pelnym ekranie (dla klawisza Windows).
    static LAST_FS: Cell<isize> = const { Cell::new(0) };
    /// Silnik nakladki w C# (osobny proces) - gdy dziala, nakladka w Rust jest wylaczona.
    static CS_OVERLAY: RefCell<Option<std::process::Child>> = const { RefCell::new(None) };
    /// Ostatni tytul okna (tryb okna: "Folder - Indeks").
    static TITLE: RefCell<String> = const { RefCell::new(String::new()) };
}

fn h(v: isize) -> HWND {
    HWND(v as *mut _)
}

fn view() -> Option<HWND> {
    let v = VIEW.with(|c| c.get());
    if v != 0 && unsafe { IsWindow(Some(h(v))) }.as_bool() {
        Some(h(v))
    } else {
        None
    }
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| match a.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

/// Wysyla do dzialajacej instancji (WM_COPYDATA) sciezke do otwarcia.
pub fn send_open(ctl: HWND, path: &str) {
    send_copy(ctl, COPY_OPEN, path);
}

pub fn send_copy(ctl: HWND, kind: usize, path: &str) {
    let w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let cd = CopyData { data: kind, len: (w.len() * 2) as u32, ptr: w.as_ptr() as *const u8 };
    unsafe {
        let _ = SendMessageTimeoutW(ctl, WM_COPYDATA, WPARAM(0), LPARAM(&cd as *const _ as isize), SMTO_ABORTIFHUNG, 3000, None);
    }
}

/// Nowy kompletny plik w Pobranych: pulpit pokazuje go zaznaczonego; gdy pulpit jest
/// zakryty, a na wierzchu nie ma nic pelnoekranowego (gra), otwiera sie okno Indeksu.
fn on_download(full: &str) {
    if !with_app(|a| a.cfg.downloads_open).unwrap_or(false) {
        return;
    }
    util::log(&format!("pobrane: {}", full));
    with_app(|a| a.open_path(full));
    invalidate();
    if crate::winlist::fullscreen_foreground().is_none() && !desktop_visible() {
        show_in_window(full);
    }
}

/// Uruchamia silnik nakladki w C# (indeks-nakladka.exe obok indeks.exe). Sam sledzi
/// okna na pelnym ekranie / zmaksymalizowane i czyta ustawienia z indeks.cfg; konczy sie
/// razem z Indeksem (--rodzic PID).
fn start_cs_overlay() {
    let Ok(exe) = std::env::current_exe() else { return };
    let cs = exe.with_file_name("indeks-nakladka.exe");
    if !cs.exists() {
        return;
    }
    match std::process::Command::new(&cs).args(["--rodzic", &std::process::id().to_string()]).spawn() {
        Ok(child) => {
            util::log("nakladka: silnik C# uruchomiony");
            CS_OVERLAY.with(|c| *c.borrow_mut() = Some(child));
        }
        Err(e) => util::log(&format!("nakladka: silnik C# nie wystartowal: {}", e)),
    }
}

/// Wlacza / wylacza silnik C# wg ustawienia; po awarii uruchamia go ponownie (co 30 s).
fn manage_cs_overlay(want: bool) {
    thread_local! {
        static LAST_TRY: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
    }
    let alive = cs_overlay_alive();
    if want && !alive {
        if LAST_TRY.with(|c| c.get().map(|t| t.elapsed().as_secs() >= 30).unwrap_or(true)) {
            LAST_TRY.with(|c| c.set(Some(std::time::Instant::now())));
            start_cs_overlay();
        }
    } else if !want && alive {
        // C# sam sie konczy po odczytaniu nakladka_cs=0; nasz proces dobijamy dla pewnosci
        CS_OVERLAY.with(|c| {
            if let Some(mut ch) = c.borrow_mut().take() {
                let _ = ch.kill();
                util::log("nakladka: silnik C# wylaczony w ustawieniach - rysuje Rust");
            }
        });
    }
}

/// Czy silnik C# dziala (jesli padl - nakladke przejmuje Rust).
fn cs_overlay_alive() -> bool {
    use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, w!("Local\\Indeks.Nakladka")) } {
        Ok(h) => {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(h);
            }
            true
        }
        Err(_) => false,
    }
}

fn invalidate() {
    if let Some(v) = view() {
        unsafe {
            let _ = InvalidateRect(Some(v), None, false);
        }
    }
}

fn dpi_of(hwnd: HWND) -> f32 {
    let d = unsafe { GetDpiForWindow(hwnd) };
    if d == 0 { 96.0 } else { d as f32 }
}

fn lp_xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32)
}

fn key_down(vk: u16) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

// ---------- start ----------

pub fn register_classes() -> windows::core::Result<()> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let cursor = LoadCursorW(None, IDC_ARROW)?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(view_proc),
            hInstance: hinst.into(),
            lpszClassName: VIEW_CLASS,
            hCursor: cursor,
            style: CS_DBLCLKS,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(ctl_proc),
            hInstance: hinst.into(),
            lpszClassName: CTL_CLASS,
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
    Ok(())
}

pub fn run(mode: Mode, start_folder: Option<String>) -> windows::core::Result<()> {
    run_inner(mode, start_folder, false)
}

/// Dodatkowe okno Indeksu (dwuklik na folderze, nowy plik w Pobranych): osobny watek
/// w tym samym procesie - wspolny indeks i silnik projektow, wlasny stan i widok okna
/// (APP/VIEW/MODE sa thread_local). Zamkniecie okna konczy tylko jego watek.
pub fn open_window(path: String) {
    let _ = std::thread::Builder::new().name("indeks-okno".into()).spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED | windows::Win32::System::Com::COINIT_DISABLE_OLE1DDE);
        }
        if let Err(e) = run_inner(Mode::Window, Some(path), true) {
            util::log(&format!("okno: {}", e.message()));
        }
    });
}

/// Okno Indeksu juz otwarte (w tym procesie albo osobnym --okno): przejdz w nim do sciezki;
/// inaczej otworz nowe.
fn show_in_window(path: &str) {
    match unsafe { FindWindowW(CTL_CLASS, CTL_TITLE_WINDOW) } {
        Ok(ctl) if !ctl.is_invalid() => send_open(ctl, path),
        _ => open_window(path.to_string()),
    }
}

fn run_inner(mode: Mode, start_folder: Option<String>, extra: bool) -> windows::core::Result<()> {
    MODE.with(|m| m.set(mode));
    register_classes()?;
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let ctl = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CTL_CLASS,
            if mode == Mode::Window { CTL_TITLE_WINDOW } else { CTL_TITLE_DESKTOP },
            WS_POPUP,
            0, 0, 0, 0,
            None,
            None,
            Some(hinst.into()),
            None,
        )?;
        CTL.with(|c| c.set(ctl.0 as isize));
        TASKBAR_CREATED.with(|c| c.set(RegisterWindowMessageW(w!("TaskbarCreated"))));

        let mut cfg = Config::load();
        if mode == Mode::Desktop && cfg.icons_were_hidden.is_none() {
            cfg.icons_were_hidden = Some(desktop::icons_hidden().unwrap_or(false));
            cfg.save();
        }
        let roots = {
            let mut r = cfg.roots.clone();
            for p in &cfg.pins {
                if p.trim_end_matches('\\').len() > 2 {
                    r.push(p.clone());
                }
            }
            r
        };
        if extra {
            // indeks i silnik projektow juz dzialaja w watku pulpitu
            crate::profiles::listen(ctl);
        } else {
            if std::env::var_os("INDEKS_BEZ_INDEKSU").is_none() {
                search::start(roots, cfg.skip.clone(), ctl);
            }
            crate::profiles::start(ctl);
            // skanowanie i obserwator tylko w pulpicie; samodzielne okno czyta gotowy katalog
            if mode == Mode::Desktop {
                crate::katalog::start(ctl);
            } else {
                crate::katalog::start_light(ctl);
            }
        }
        let mut app = App::new(cfg, ctl)?;
        // w oknie folder/plik otwieramy dopiero, gdy okno ma rozmiar (przewiniecie do pliku)
        let start_file = if mode == Mode::Window { start_folder.clone() } else { None };
        if let (Some(f), Mode::Desktop) = (&start_folder, mode) {
            app.open_path(f);
        }
        app.log_state();
        APP.with(|a| *a.borrow_mut() = Some(app));

        match mode {
            Mode::Desktop => {
                add_tray(ctl);
                ensure_attached();
                if view().is_none() {
                    util::log("pulpit: brak okna pulpitu Explorera - ponowie probe");
                }
                SetTimer(Some(ctl), T_CHECK, 2000, None);
                // nowe pliki w Pobranych (dowolne zrodlo) - pokazywane w Indeksie
                if let Some(d) = crate::config::downloads_dir() {
                    crate::downloads::start(d, ctl);
                }
                let hook = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(foreground_hook), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
                HOOK.with(|c| c.set(hook.0 as isize));
            }
            Mode::Window => {
                let v = create_view(None)?;
                let _ = SetFocus(Some(v));
                if let Some(f) = start_file {
                    sync_view_size(v);
                    with_app(|a| a.open_path(&f));
                    let _ = InvalidateRect(Some(v), None, false);
                    crate::winlist::activate(v.0 as isize);
                }
                // INDEKS_DYSK: okno startuje od razu w panelu projektow (skroty, testy)
                if !extra && std::env::var_os("INDEKS_DYSK").is_some() {
                    sync_view_size(v);
                    with_app(|a| a.set_disk(true));
                    let _ = InvalidateRect(Some(v), None, false);
                }
            }
        }
        // pasek stanu (CPU/GPU/RAM/dyski) - liczony gdy pulpit jest widoczny albo nakladka nad pelnym ekranem
        if mode == Mode::Desktop {
            OVERLAY.with(|o| *o.borrow_mut() = crate::overlay::Overlay::new());
            manage_cs_overlay(with_app(|a| a.cfg.overlay_cs).unwrap_or(true));
        }
        with_app(|a| a.update_apps());
        SetTimer(Some(ctl), T_STATS, 2000, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

fn create_view(parent: Option<HWND>) -> windows::core::Result<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let hwnd = match parent {
            Some(host) => {
                let r = desktop::work_rect_in(Some(host));
                CreateWindowExW(
                    WS_EX_NOPARENTNOTIFY,
                    VIEW_CLASS,
                    w!("Indeks"),
                    WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
                    r.left, r.top, r.right - r.left, r.bottom - r.top,
                    Some(host),
                    None,
                    Some(hinst.into()),
                    None,
                )?
            }
            None => {
                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    VIEW_CLASS,
                    w!("Indeks"),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN,
                    CW_USEDEFAULT, CW_USEDEFAULT, 1400, 900,
                    None,
                    None,
                    Some(hinst.into()),
                    None,
                )?;
                let s = dpi_of(hwnd) / 96.0;
                let _ = SetWindowPos(hwnd, None, 0, 0, (1400.0 * s) as i32, (900.0 * s) as i32, SWP_NOMOVE | SWP_NOZORDER);
                hwnd
            }
        };
        VIEW.with(|c| c.set(hwnd.0 as isize));
        Ok(hwnd)
    }
}

/// Pilnuje, zeby widok byl dzieckiem aktualnego okna pulpitu, na wierzchu
/// i na calym obszarze roboczym. Wywolywane co 2 s i po zdarzeniach systemu.
fn ensure_attached() {
    if MODE.with(|m| m.get()) != Mode::Desktop || QUITTING.with(|q| q.get()) {
        return;
    }
    let Some(host) = desktop::find_host() else { return };
    HOST.with(|c| c.set(host.0 as isize));
    unsafe {
        let v = match view() {
            Some(v) => v,
            None => match create_view(Some(host)) {
                Ok(v) => {
                    util::log("pulpit: widok utworzony");
                    v
                }
                Err(e) => {
                    util::log(&format!("pulpit: nie moge utworzyc widoku: {}", e.message()));
                    return;
                }
            },
        };
        if GetParent(v).ok() != Some(host) {
            let _ = SetParent(v, Some(host));
            util::log("pulpit: widok przepiety do nowego okna pulpitu");
        }
        let want = desktop::work_rect_in(None);
        let mut have = RECT::default();
        let _ = GetWindowRect(v, &mut have);
        let top_child = GetWindow(host, GW_CHILD).ok();
        if have != want || top_child != Some(v) || !IsWindowVisible(v).as_bool() {
            let r = desktop::work_rect_in(Some(host));
            let _ = SetWindowPos(v, Some(HWND_TOP), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
        if ICONS_HOST.with(|c| c.get()) != host.0 as isize {
            ICONS_HOST.with(|c| c.set(host.0 as isize));
            desktop::set_icons_hidden(true);
        }
    }
}

unsafe extern "system" fn foreground_hook(_h: HWINEVENTHOOK, _ev: u32, hwnd: HWND, _o: i32, _c: i32, _t: u32, _ms: u32) {
    // Po Win+D / kliknieciu w pulpit: klawiatura od razu trafia do Indeksu,
    // a pasek stanu dostaje swieze dane
    if hwnd.0 as isize != 0 && hwnd.0 as isize == HOST.with(|c| c.get()) {
        if let Some(v) = view() {
            let _ = SetFocus(Some(v));
        }
        update_stats(true);
        return;
    }
    // klawisz Windows nad gra na pelnym ekranie: gra sie minimalizuje (opcja)
    if crate::winlist::is_start_menu(hwnd) && with_app(|a| a.cfg.win_minimizes).unwrap_or(false) {
        let prev = LAST_FS.with(|c| c.replace(0));
        if prev != 0 {
            crate::winlist::minimize(h(prev));
        }
    }
    LAST_FS.with(|c| c.set(crate::winlist::fullscreen_foreground().map(|w| w.0 as isize).unwrap_or(0)));
    update_stats(false);
}

/// Czy pulpit jest teraz na wierzchu (inaczej nie liczymy i nie malujemy statystyk).
fn desktop_visible() -> bool {
    if MODE.with(|m| m.get()) == Mode::Window {
        return true;
    }
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return true;
        }
        let mut cls = [0u16; 32];
        let n = GetClassNameW(fg, &mut cls) as usize;
        let cls = String::from_utf16_lossy(&cls[..n]);
        matches!(cls.as_str(), "WorkerW" | "Progman" | "Shell_TrayWnd" | "IndeksKontroler")
    }
}

/// Co 2 s i przy zmianie okna na wierzchu: statystyki, nakladka nad pelnym ekranem,
/// przyciski aplikacji. Nic nie liczy, gdy pulpit zakryty i nie ma pelnego ekranu.
fn update_stats(force: bool) {
    let (max, want_cs) = with_app(|a| (a.cfg.overlay_maximized, a.cfg.overlay_cs)).unwrap_or((true, true));
    let fs = crate::winlist::overlay_target(max);
    if MODE.with(|m| m.get()) == Mode::Desktop {
        manage_cs_overlay(want_cs);
    }
    // optymalizacja bezczynnych aplikacji liczy sie na pulpicie, nawet gdy jest zakryty
    // (INDEKS_OPT_TEST pozwala policzyc ja tez w oknie --okno do zrzutow zakladki)
    if MODE.with(|m| m.get()) == Mode::Desktop || std::env::var_os("INDEKS_OPT_TEST").is_some() {
        let enabled = with_app(|a| a.cfg.optimize).unwrap_or(false);
        let dt = OPT_LAST.with(|c| {
            let now = std::time::Instant::now();
            let dt = c.get().map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(2.0);
            c.set(Some(now));
            dt
        })
        .clamp(0.5, 30.0);
        crate::optymalizator::tick(enabled, dt);
    }
    let overlay_on = with_app(|a| a.cfg.overlay).unwrap_or(false);
    let desk = force || desktop_visible();
    let show_overlay = overlay_on && fs.is_some() && MODE.with(|m| m.get()) == Mode::Desktop && !cs_overlay_alive();
    if !desk && !show_overlay {
        OVERLAY.with(|o| o.borrow_mut().as_mut().map(|ov| ov.hide()));
        return;
    }
    let gen0 = with_app(|a| a.gen).unwrap_or(0);
    let rebuilt = with_app(|a| a.update_stats()).unwrap_or(false);
    OVERLAY.with(|o| {
        if let Some(ov) = o.borrow_mut().as_mut() {
            match fs {
                Some((w, work)) if show_overlay => {
                    with_app(|a| {
                        if let Err(e) = ov.show(w, work, &a.page.status, &a.cfg, &a.renderer.d2d, &a.renderer.dw) {
                            util::log(&format!("nakladka: {}", e.message()));
                        }
                    });
                }
                _ => ov.hide(),
            }
        }
    });
    if !desk {
        return;
    }
    with_app(|a| a.update_apps());
    let gen1 = with_app(|a| a.gen).unwrap_or(0);
    if rebuilt || gen0 != gen1 {
        invalidate();
    } else if let Some(v) = view() {
        let s = dpi_of(v) / 96.0;
        if let Some((st, ap)) = with_app(|a| (a.layout.status, a.layout.apps_row)) {
            unsafe {
                let _ = InvalidateRect(Some(v), Some(&px_rect(&st, s)), false);
                if ap.w > 0.0 {
                    let _ = InvalidateRect(Some(v), Some(&px_rect(&ap, s)), false);
                }
            }
        }
    }
}

fn add_tray(ctl: HWND) {
    unsafe {
        let mut sii = SHSTOCKICONINFO { cbSize: std::mem::size_of::<SHSTOCKICONINFO>() as u32, ..Default::default() };
        let _ = SHGetStockIconInfo(SIID_FOLDER, SHGSI_ICON | SHGSI_SMALLICON, &mut sii);
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: ctl,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_APP_TRAY,
            hIcon: sii.hIcon,
            ..Default::default()
        };
        let tip = wide("Indeks – pulpit (prawy klik: menu)");
        for (i, c) in tip.iter().take(nid.szTip.len() - 1).enumerate() {
            nid.szTip[i] = *c;
        }
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

fn remove_tray(ctl: HWND) {
    unsafe {
        let nid = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: ctl, uID: 1, ..Default::default() };
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

/// Zamyka Indeks. `restore` = przywroc ikony pulpitu Windows do stanu sprzed startu.
fn shutdown(restore: bool) {
    if QUITTING.with(|q| q.replace(true)) {
        return;
    }
    // zdejmij ograniczenia z aplikacji, zanim Indeks zniknie
    crate::optymalizator::disable_all();
    let ctl = h(CTL.with(|c| c.get()));
    unsafe {
        let _ = KillTimer(Some(ctl), T_CHECK);
        let hook = HOOK.with(|c| c.get());
        if hook != 0 {
            let _ = UnhookWinEvent(HWINEVENTHOOK(hook as *mut _));
        }
        remove_tray(ctl);
        if MODE.with(|m| m.get()) == Mode::Desktop && restore {
            let before = with_app(|a| a.cfg.icons_were_hidden).flatten().unwrap_or(false);
            desktop::set_icons_hidden(before);
            with_app(|a| {
                a.cfg.icons_were_hidden = None;
                a.cfg.save();
            });
            util::log("pulpit: przywrocono ikony Windows");
        }
        if let Some(v) = view() {
            let _ = DestroyWindow(v);
        }
        let _ = DestroyWindow(ctl);
    }
}

// ---------- kontroler ----------

unsafe extern "system" fn ctl_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == TASKBAR_CREATED.with(|c| c.get()) && msg != 0 {
        // Explorer wystartowal od nowa: nowe okno pulpitu i nowy zasobnik
        util::log("pulpit: Explorer uruchomiony ponownie");
        if MODE.with(|m| m.get()) == Mode::Desktop {
            add_tray(hwnd);
            ICONS_HOST.with(|c| c.set(0));
            SetTimer(Some(hwnd), T_REATTACH, 1500, None);
        }
        return LRESULT(0);
    }
    match msg {
        WM_APP_TRAY => {
            let ev = (lp.0 & 0xFFFF) as u32;
            if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP || ev == WM_CONTEXTMENU {
                tray_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_APP_SHOW => {
            with_app(|a| a.home());
            if let Some(v) = view() {
                let _ = SetFocus(Some(v));
            }
            invalidate();
            LRESULT(0)
        }
        m if m == icons::WM_APP_ICON => {
            if with_app(|a| a.icons.drain()).unwrap_or(false) {
                invalidate_icons();
            }
            LRESULT(0)
        }
        m if m == search::WM_APP_INDEX => {
            if with_app(|a| {
                if a.search.is_some() {
                    let sel = a.selected;
                    let scroll = a.scroll;
                    a.run_search();
                    a.selected = match sel {
                        Target::Row(i) if i < a.page.table.len() => sel,
                        _ => Target::None,
                    };
                    a.scroll = scroll;
                    true
                } else {
                    // postep indeksowania w panelu "Projekty na dysku"
                    if search::status().ready {
                        a.index_finished();
                    }
                    a.fill_kinds();
                    true
                }
            })
            .unwrap_or(false)
            {
                invalidate();
            }
            LRESULT(0)
        }
        m if m == crate::downloads::WM_APP_DOWNLOAD => {
            if let Some(full) = crate::downloads::take().pop() {
                on_download(&full);
            }
            LRESULT(0)
        }
        WM_COPYDATA => {
            let cd = unsafe { &*(lp.0 as *const CopyData) };
            if cd.data == crate::katalog::COPY_PANEL && !cd.ptr.is_null() {
                let s = unsafe { std::slice::from_raw_parts(cd.ptr as *const u16, cd.len as usize / 2) };
                crate::katalog::from_panel(String::from_utf16_lossy(s).trim_end_matches('\0'));
            } else if cd.data == COPY_WINDOW && !cd.ptr.is_null() {
                let s = unsafe { std::slice::from_raw_parts(cd.ptr as *const u16, cd.len as usize / 2) };
                open_window(String::from_utf16_lossy(s).trim_end_matches('\0').to_string());
            } else if cd.data == COPY_OPEN && !cd.ptr.is_null() {
                let s = unsafe { std::slice::from_raw_parts(cd.ptr as *const u16, cd.len as usize / 2) };
                let path = String::from_utf16_lossy(s).trim_end_matches('\0').to_string();
                with_app(|a| a.open_path(&path));
                if MODE.with(|m| m.get()) == Mode::Window {
                    if let Some(v) = view() {
                        crate::winlist::activate(v.0 as isize);
                    }
                }
                invalidate();
            }
            LRESULT(1)
        }
        m if m == crate::profiles::WM_APP_PROFILE => {
            // wp=1: skan projektow na dysku gotowy
            let changed = with_app(|a| {
                // nowy katalog: lista projektow w panelu tez sie zmienia
                if wp.0 == 1 && a.disk {
                    a.rebuild();
                }
                a.fill_kinds();
                a.fill_profiles() || wp.0 == 1
            });
            if changed.unwrap_or(false) {
                invalidate();
            }
            LRESULT(0)
        }
        m if m == app::WM_APP_DIR => {
            SetTimer(Some(hwnd), T_RELOAD, 300, None);
            LRESULT(0)
        }
        WM_TIMER => {
            match wp.0 {
                T_STATS => update_stats(false),
                T_RELOAD => {
                    let _ = KillTimer(Some(hwnd), T_RELOAD);
                    if with_app(|a| a.reload_if_changed()).unwrap_or(false) {
                        invalidate();
                    }
                }
                T_REATTACH => {
                    let _ = KillTimer(Some(hwnd), T_REATTACH);
                    ensure_attached();
                }
                T_CHECK => ensure_attached(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE | WM_DPICHANGED => {
            ensure_attached();
            invalidate();
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_QUERYENDSESSION => LRESULT(1),
        WM_ENDSESSION => {
            if wp.0 != 0 {
                // przy wylogowaniu: jesli Indeks i tak wystartuje sam, nie migamy ikonami
                shutdown(!autostart::enabled());
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            shutdown(true);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn tray_menu(ctl: HWND) {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(ctl);
    }
    let extras = vec![
        item(ID_HOME, "Folder domowy"),
        Extra { id: ID_AUTOSTART, text: "Uruchamiaj z Windows".into(), checked: autostart::enabled(), enabled: true },
        item(ID_SETTINGS, "Edytuj ustawienia (plik)"),
        sep(),
        item(ID_QUIT, "Zamknij Indeks (przywraca zwykły pulpit)"),
    ];
    let res = shell::popup_only(ctl, pt, &extras);
    unsafe {
        let _ = PostMessageW(Some(ctl), WM_NULL, WPARAM(0), LPARAM(0));
    }
    if let MenuResult::Extra(id) = res {
        handle_command(id, None);
    }
}

// ---------- widok ----------

unsafe extern "system" fn view_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        if let Some(r) = shell::forward_menu_msg(msg, wp, lp) {
            return r;
        }
    }
    match msg {
        WM_PAINT => {
            // rcPaint to prostokat obejmujacy CALY region (np. wiersz po lewej +
            // suwak po prawej = caly ekran), wiec malujemy kazdy prostokat regionu
            let rects = update_rects(hwnd);
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if rects.is_empty() || rects.len() > 24 {
                paint(hwnd, hdc, ps.rcPaint);
            } else {
                for r in rects {
                    paint(hwnd, hdc, r);
                }
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            sync_view_size(hwnd);
            invalidate();
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let r = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            with_app(|a| a.renderer.drop_target());
            invalidate();
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => {
            let _ = SetFocus(Some(hwnd));
            LRESULT(MA_ACTIVATE as isize)
        }
        WM_MOUSEMOVE => {
            let (x, y) = dip(hwnd, lp);
            let b = before();
            let changed = with_app(|a| {
                if let Some(i) = a.slider_drag {
                    a.slider_at(i, x);
                    return true;
                }
                if let Some((r, start_y, start_thumb)) = a.drag {
                    a.scroll_to_thumb(r, start_thumb + (y - start_y));
                    return true;
                }
                let t = a.hit(x, y);
                if t != a.hover {
                    a.hover = t;
                    return true;
                }
                false
            })
            .unwrap_or(false);
            let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            let _ = TrackMouseEvent(&mut tme);
            if changed {
                refresh(hwnd, b);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let b = before();
            if with_app(|a| std::mem::replace(&mut a.hover, Target::None) != Target::None).unwrap_or(false) {
                refresh(hwnd, b);
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            if (lp.0 & 0xFFFF) as u32 == HTCLIENT {
                let link = with_app(|a| a.hover.is_link()).unwrap_or(false);
                if let Ok(c) = LoadCursorW(None, if link { IDC_HAND } else { IDC_ARROW }) {
                    SetCursor(Some(c));
                }
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_LBUTTONDBLCLK => {
            // dwuklik na folderze (drzewo / lista) = nowe okno Indeksu z tym folderem;
            // tu widok wraca tam, gdzie byl przed pierwszym kliknieciem
            let (x, y) = dip(hwnd, lp);
            let path = with_app(|a| {
                let t = a.hit(x, y);
                a.pressed = Target::None;
                a.double_click(t)
            })
            .flatten();
            if let Some(p) = path {
                open_window(p);
            }
            invalidate();
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let _ = SetFocus(Some(hwnd));
            SetCapture(hwnd);
            let (x, y) = dip(hwnd, lp);
            let b = before();
            with_app(|a| {
                let t = a.hit(x, y);
                a.pressed = t;
                if let Target::Scroll(r) = t {
                    let ui = a.ui();
                    if let Some((ty, th)) = crate::render::scrollbar_thumb(&a.layout, &ui, r) {
                        if y >= ty && y < ty + th {
                            a.drag = Some((r, y, ty));
                        } else {
                            // klik w tor: skok o strone
                            let page = a.layout.viewport(r).h * 0.9;
                            a.scroll[r as usize] += if y < ty { -page } else { page };
                            a.clamp_scroll();
                        }
                    }
                }
                if matches!(t, Target::Folder(_) | Target::File(_) | Target::Row(_)) {
                    a.selected = t;
                }
                // suwak w ustawieniach: chwyc i przeciagaj
                if let Target::SetItem(i) = t {
                    if a.layout.set_tracks.get(i).map(|r| r.w > 0.0).unwrap_or(false) {
                        a.slider_drag = Some(i);
                        a.slider_at(i, x);
                    }
                }
            });
            refresh(hwnd, b);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            let (x, y) = dip(hwnd, lp);
            let ctrl = key_down(VK_CONTROL.0);
            let b = before();
            let hide_all = with_app(|a| {
                if a.slider_drag.take().is_some() {
                    a.cfg.save();
                    return false;
                }
                if a.drag.take().is_some() {
                    return false;
                }
                let t = a.hit(x, y);
                if t != a.pressed {
                    return false;
                }
                a.pressed = Target::None;
                // klik w puste miejsce pulpitu chowa wszystkie okna (opcja)
                if t == Target::None {
                    return a.cfg.click_hides && MODE.with(|m| m.get()) == Mode::Desktop;
                }
                a.activate(t, ctrl);
                false
            });
            if hide_all.unwrap_or(false) {
                shell::minimize_all();
            }
            refresh(hwnd, b);
            LRESULT(0)
        }
        WM_MBUTTONUP => {
            let (x, y) = dip(hwnd, lp);
            with_app(|a| {
                let t = a.hit(x, y);
                if matches!(t, Target::Folder(_) | Target::File(_) | Target::Row(_) | Target::Tree(_)) {
                    a.activate(t, true);
                }
            });
            LRESULT(0)
        }
        WM_XBUTTONUP => {
            let button = (wp.0 >> 16) & 0xFFFF;
            with_app(|a| if button == 1 { a.back() } else { a.forward() });
            invalidate();
            LRESULT(1)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) & 0xFFFF) as i16 as f32 / 120.0;
            let b = before();
            let reveal = with_app(|a| {
                let mut changed = false;
                if let Some((x, y)) = cursor_dip(hwnd) {
                    match a.region_at(x, y) {
                        Some(r) => a.wheel(r, delta),
                        // scroll w pustym miejscu: w dol rozwija zakladke optymalizacji, w gore chowa
                        None => changed = a.toggle_opt_reveal(delta < 0.0),
                    }
                    a.hover = a.hit(x, y);
                }
                changed
            })
            .unwrap_or(false);
            if reveal {
                invalidate();
            } else {
                refresh(hwnd, b);
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let (x, y) = dip(hwnd, lp);
            let (px, py) = lp_xy(lp);
            let mut pt = POINT { x: px, y: py };
            let _ = ClientToScreen(hwnd, &mut pt);
            let t = with_app(|a| a.hit(x, y)).unwrap_or(Target::None);
            context_menu(hwnd, t, pt);
            LRESULT(0)
        }
        WM_CONTEXTMENU => {
            // klawisz menu / Shift+F10: menu dla zaznaczonego elementu
            if lp.0 == -1 {
                let (t, at) = with_app(|a| (a.selected, a.target_rect(a.selected))).unwrap_or((Target::None, None));
                let s = dpi_of(hwnd) / 96.0;
                let r = at.unwrap_or(Rc { x: 40.0, y: 80.0, w: 0.0, h: 0.0 });
                let mut pt = POINT { x: ((r.x + 24.0) * s) as i32, y: ((r.y + r.h / 2.0) * s) as i32 };
                let _ = ClientToScreen(hwnd, &mut pt);
                context_menu(hwnd, t, pt);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let b = before();
            if on_key(wp.0 as u16, msg == WM_SYSKEYDOWN) {
                refresh(hwnd, b);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_CHAR => {
            let c = wp.0 as u32;
            if c >= 0x20 && c != 0x7F && !key_down(VK_CONTROL.0) {
                if let Some(ch) = char::from_u32(c) {
                    with_app(|a| a.search_input(ch));
                    invalidate();
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            with_app(|a| a.renderer.drop_target());
            // panel C# jest dzieckiem widoku: odpiety przezyje zamkniecie tego okna
            crate::katalog::panel_hide(hwnd);
            if VIEW.with(|c| c.get()) == hwnd.0 as isize {
                VIEW.with(|c| c.set(0));
            }
            if MODE.with(|m| m.get()) == Mode::Window {
                let ctl = h(CTL.with(|c| c.get()));
                let _ = DestroyWindow(ctl);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ---------- odswiezanie czesciowe ----------
//
// Pelne malowanie ekranu 4K na CPU kosztuje kilkadziesiat ms, wiec przy
// najechaniu / zaznaczeniu odmalowujemy tylko zmienione elementy, a przy
// przewijaniu panelu przesuwamy jego piksele (ScrollWindowEx) i domalowujemy pas.

struct Before {
    gen: u64,
    scroll: [f32; 4],
    selected: Target,
    hover: Target,
}

fn before() -> Option<Before> {
    with_app(|a| Before { gen: a.gen, scroll: a.scroll, selected: a.selected, hover: a.hover })
}

fn px_rect(r: &Rc, s: f32) -> RECT {
    RECT {
        left: (r.x * s).floor() as i32 - 1,
        top: (r.y * s).floor() as i32 - 1,
        right: (r.right() * s).ceil() as i32 + 1,
        bottom: (r.bottom() * s).ceil() as i32 + 1,
    }
}

/// Odmalowuje tylko to, co zmienilo sie od `b`.
fn refresh(hwnd: HWND, b: Option<Before>) {
    let Some(b) = b else {
        invalidate();
        return;
    };
    let s = dpi_of(hwnd) / 96.0;
    let plan = with_app(|a| {
        if a.gen != b.gen {
            return None;
        }
        let mut scrolls = Vec::new();
        // suwaki do odmalowania: tylko przewiniete / najechane / przeciagane panele
        let mut bars = Vec::new();
        for t in [b.hover, a.hover] {
            if let Target::Scroll(r) = t {
                bars.push(r);
            }
        }
        if let Some((r, _, _)) = a.drag {
            bars.push(r);
        }
        for r in REGIONS {
            let i = r as usize;
            // przewijanie o pelne piksele - wtedy przesuniete piksele pasuja 1:1
            a.scroll[i] = (a.scroll[i] * s).round() / s;
            let dy = ((a.scroll[i] - b.scroll[i]) * s).round() as i32;
            if dy != 0 {
                bars.push(r);
                let v = a.layout.viewport(r);
                let clip = RECT {
                    left: (v.x * s).round() as i32,
                    top: (v.y * s).round() as i32,
                    right: (v.right() * s).round() as i32,
                    bottom: (v.bottom() * s).round() as i32,
                };
                scrolls.push((dy, clip, (v.h * s) as i32));
            }
        }
        let mut rects = Vec::new();
        for t in [b.selected, a.selected, b.hover, a.hover] {
            if let Some(r) = a.target_rect(t) {
                rects.push(px_rect(&r, s));
            }
        }
        for r in bars {
            if let Some(rc) = a.target_rect(Target::Scroll(r)) {
                rects.push(px_rect(&rc, s));
            }
        }
        Some((scrolls, rects))
    })
    .flatten();
    let Some((scrolls, rects)) = plan else {
        invalidate();
        return;
    };
    unsafe {
        for (dy, clip, h) in scrolls {
            if dy.abs() >= h {
                let _ = InvalidateRect(Some(hwnd), Some(&clip), false);
            } else {
                let _ = ScrollWindowEx(hwnd, 0, -dy, Some(&clip), Some(&clip), None, None, SW_INVALIDATE);
            }
        }
        for r in &rects {
            let _ = InvalidateRect(Some(hwnd), Some(r), false);
        }
    }
}

/// Doszly ikony systemowe: odmaluj panele z ikonami (bez paskow i naglowka).
fn invalidate_icons() {
    let Some(v) = view() else { return };
    let s = dpi_of(v) / 96.0;
    let rects = with_app(|a| {
        let l = &a.layout;
        [l.tree, l.folders, l.files, l.table, l.top].iter().filter(|r| r.w > 0.0).map(|r| px_rect(r, s)).collect::<Vec<_>>()
    });
    for r in rects.unwrap_or_default() {
        unsafe {
            let _ = InvalidateRect(Some(v), Some(&r), false);
        }
    }
}

/// Prostokaty regionu do odmalowania (przed BeginPaint).
fn update_rects(hwnd: HWND) -> Vec<RECT> {
    use windows::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject, GetRegionData, GetUpdateRgn, RGNDATA, RGNDATAHEADER};
    let mut out = Vec::new();
    unsafe {
        let rgn = CreateRectRgn(0, 0, 0, 0);
        if GetUpdateRgn(hwnd, rgn, false).0 > 1 {
            let size = GetRegionData(rgn, 0, None);
            if size as usize >= std::mem::size_of::<RGNDATAHEADER>() {
                let mut buf = vec![0u64; (size as usize).div_ceil(8)];
                let data = buf.as_mut_ptr() as *mut RGNDATA;
                if GetRegionData(rgn, size, Some(data)) == size {
                    let n = (*data).rdh.nCount as usize;
                    let first = (data as *const u8).add(std::mem::size_of::<RGNDATAHEADER>()) as *const RECT;
                    out.extend_from_slice(std::slice::from_raw_parts(first, n));
                }
            }
        }
        let _ = DeleteObject(rgn.into());
    }
    out
}

/// Pozycja kursora w DIP okna.
fn cursor_dip(hwnd: HWND) -> Option<(f32, f32)> {
    let mut pt = POINT::default();
    unsafe {
        GetCursorPos(&mut pt).ok()?;
        let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
    }
    let s = 96.0 / dpi_of(hwnd);
    Some((pt.x as f32 * s, pt.y as f32 * s))
}

fn dip(hwnd: HWND, lp: LPARAM) -> (f32, f32) {
    let (x, y) = lp_xy(lp);
    let s = 96.0 / dpi_of(hwnd);
    (x as f32 * s, y as f32 * s)
}

/// Rozmiar i DPI widoku do aplikacji juz przy zmianie rozmiaru, nie dopiero przy
/// malowaniu - przewiniecie do pliku liczy sie wtedy dla prawdziwego ukladu.
fn sync_view_size(hwnd: HWND) {
    let dpi = dpi_of(hwnd);
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let (pw, ph) = ((rc.right - rc.left).max(1) as f32, (rc.bottom - rc.top).max(1) as f32);
    with_app(|a| {
        a.dpi = dpi;
        a.view_w = pw * 96.0 / dpi;
        a.view_h = ph * 96.0 / dpi;
        a.ensure_layout();
    });
}

fn paint(hwnd: HWND, hdc: HDC, area: RECT) {
    with_app(|a| {
        a.ensure_layout();
        a.sync_panel(hwnd);
    });
    if MODE.with(|m| m.get()) == Mode::Window {
        if let Some(t) = with_app(|a| a.page.title.clone()) {
            if TITLE.with(|c| *c.borrow() != t) {
                let w = util::wide(&format!("{} - Indeks", t));
                unsafe {
                    let _ = SetWindowTextW(hwnd, util::pcwstr(&w));
                }
                TITLE.with(|c| *c.borrow_mut() = t);
            }
        }
    }
    let dpi = dpi_of(hwnd);
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let (pw, ph) = ((rc.right - rc.left).max(1) as u32, (rc.bottom - rc.top).max(1) as u32);
    let t0 = std::time::Instant::now();
    let retry = with_app(|a| {
        a.dpi = dpi;
        a.view_w = pw as f32 * 96.0 / dpi;
        a.view_h = ph as f32 * 96.0 / dpi;
        a.ensure_layout();
        let ui = a.ui();
        if let Err(e) = a.renderer.paint_dc(hdc, area, dpi, &a.page, &a.layout, &ui, &mut a.icons) {
            util::log(&format!("render: {:?} {}", e.code(), e.message()));
            a.renderer.drop_target();
            return true;
        }
        false
    })
    .unwrap_or(false);
    let ms = t0.elapsed().as_secs_f32() * 1000.0;
    let n = PAINTS.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    if n <= 3 || ms > 60.0 || std::env::var_os("INDEKS_LOG_MALOWANIE").is_some() {
        util::log(&format!("render: malowanie {}x{} px w {:.1} ms", area.right - area.left, area.bottom - area.top, ms));
    }
    if retry {
        invalidate();
    }
}

fn on_key(vk: u16, sys: bool) -> bool {
    let alt = sys || key_down(VK_MENU.0);
    let ctrl = key_down(VK_CONTROL.0);
    let res = with_app(|a| {
        let searching = a.search.is_some();
        match vk {
            v if v == VK_LEFT.0 && alt => a.back(),
            v if v == VK_RIGHT.0 && alt => a.forward(),
            v if v == VK_UP.0 && alt => a.up(),
            v if v == VK_HOME.0 && alt => a.home(),
            v if v == VK_BACK.0 => {
                if searching {
                    a.search_backspace()
                } else {
                    a.back()
                }
            }
            v if v == VK_ESCAPE.0 && a.settings_open => a.toggle_settings(false),
            v if v == VK_ESCAPE.0 && a.kind.is_some() => a.set_kind(None),
            v if v == VK_ESCAPE.0 => {
                if searching {
                    a.close_search()
                } else {
                    a.selected = Target::None
                }
            }
            v if v == VK_UP.0 => a.move_selection(0, -1),
            v if v == VK_DOWN.0 => a.move_selection(0, 1),
            v if v == VK_LEFT.0 => a.move_selection(-1, 0),
            v if v == VK_RIGHT.0 => a.move_selection(1, 0),
            v if v == VK_PRIOR.0 => {
                let n = a.page_step();
                a.move_selection(0, -n)
            }
            v if v == VK_NEXT.0 => {
                let n = a.page_step();
                a.move_selection(0, n)
            }
            v if v == VK_HOME.0 => a.select_edge(false),
            v if v == VK_END.0 => a.select_edge(true),
            v if v == VK_RETURN.0 => {
                let t = a.selected;
                a.activate(t, alt || ctrl);
            }
            v if v == VK_F5.0 => {
                a.reload_if_changed();
            }
            0x48 if ctrl => a.toggle_protected(), // Ctrl+H
            0x45 if ctrl => shell::explorer_folder(&a.current_dir().unwrap_or_default()), // Ctrl+E
            0x44 if ctrl => {
                // Ctrl+D: dodaj / usun z ulubionych
                if let Some(p) = a.current_dir() {
                    a.toggle_fav(&p);
                }
            }
            _ => return false,
        }
        true
    });
    res.unwrap_or(false)
}

fn context_menu(hwnd: HWND, t: Target, pt: POINT) {
    let info = with_app(|a| {
        if matches!(t, Target::Folder(_) | Target::File(_) | Target::Row(_) | Target::RowFolder(_) | Target::Tree(_)) {
            a.selected = t;
        }
        let path = a.path_of(t);
        let pinned = path.as_ref().map(|p| a.cfg.is_pinned(&p.0)).unwrap_or(false);
        let folder = match t {
            Target::Row(i) | Target::RowFolder(i) => a.page.table.get(i).and_then(|r| r.item.folder.clone()),
            _ => None,
        };
        (path, pinned, folder)
    });
    let Some((path, pinned, folder)) = info else { return };
    invalidate();
    if let Target::Fav(k) = t {
        fav_menu(hwnd, k, pt);
        invalidate();
        return;
    }
    let Some((path, is_dir)) = path else {
        background_menu(hwnd, pt);
        invalidate();
        return;
    };
    let mut extras = Vec::new();
    if is_dir {
        extras.push(item(ID_WINDOW, "Otwórz w nowym oknie Indeksu"));
        extras.push(item(ID_TAB, "Dodaj jako kartę"));
        extras.push(item(ID_PIN, if pinned { "Usuń z ulubionych" } else { "Dodaj do ulubionych" }));
        extras.push(item(ID_EXPLORER, "Otwórz w Eksploratorze"));
        extras.push(item(ID_TERMINAL, "Terminal tutaj"));
    } else {
        extras.push(item(ID_REVEAL, "Pokaż w Eksploratorze"));
    }
    if folder.is_some() {
        extras.push(item(ID_GOTO, "Przejdź do folderu w Indeksie"));
    }
    match shell::context_menu(hwnd, &path, pt, &extras) {
        MenuResult::Extra(ID_PIN) => {
            with_app(|a| a.toggle_fav(&path));
        }
        MenuResult::Extra(ID_WINDOW) => open_window(path.clone()),
        MenuResult::Extra(ID_TAB) => {
            with_app(|a| a.add_tab(&path));
        }
        MenuResult::Extra(ID_EXPLORER) => shell::explorer_folder(&path),
        MenuResult::Extra(ID_TERMINAL) => shell::terminal_here(&path),
        MenuResult::Extra(ID_REVEAL) | MenuResult::Rename => shell::explorer_select(&path),
        MenuResult::Extra(ID_GOTO) => {
            if let Some(f) = folder {
                with_app(|a| a.reveal(&f, &path));
            }
        }
        MenuResult::Open => {
            if is_dir {
                with_app(|a| a.navigate(&path));
            } else {
                shell::open(&path);
            }
        }
        _ => {}
    }
    invalidate();
}

fn fav_menu(hwnd: HWND, k: usize, pt: POINT) {
    let extras = vec![item(ID_UNPIN, "Usuń z ulubionych"), item(ID_LEFT, "Przesuń w lewo"), item(ID_RIGHT, "Przesuń w prawo"), sep(), item(ID_EXPLORER, "Otwórz w Eksploratorze")];
    if let MenuResult::Extra(id) = shell::popup_only(hwnd, pt, &extras) {
        with_app(|a| {
            // k = pozycja na ekranie; w cfg.pins sa jeszcze (ukryte) dyski
            let (Some(cur), prev, next) = (a.fav_pin(k), k.checked_sub(1).and_then(|j| a.fav_pin(j)), a.fav_pin(k + 1)) else { return };
            match id {
                ID_UNPIN => {
                    a.cfg.pins.remove(cur);
                }
                ID_LEFT => {
                    if let Some(j) = prev {
                        a.cfg.pins.swap(cur, j);
                    }
                }
                ID_RIGHT => {
                    if let Some(j) = next {
                        a.cfg.pins.swap(cur, j);
                    }
                }
                ID_EXPLORER => {
                    if let Some(p) = a.cfg.pins.get(cur) {
                        shell::explorer_folder(p);
                    }
                }
                _ => {}
            }
            a.cfg.save();
            a.rebuild();
        });
    }
}

fn background_menu(hwnd: HWND, pt: POINT) {
    let (dir, pinned, prot) = with_app(|a| {
        let d = a.current_dir();
        let pinned = d.as_ref().map(|p| a.cfg.is_pinned(p)).unwrap_or(false);
        (d, pinned, a.cfg.show_protected)
    })
    .unwrap_or((None, false, false));
    let has_dir = dir.is_some();
    let mut extras = vec![
        Extra { id: ID_PIN, text: if pinned { "Usuń ten folder z ulubionych".into() } else { "Dodaj ten folder do ulubionych".into() }, checked: false, enabled: has_dir },
        Extra { id: ID_HOME_HERE, text: "Ustaw ten folder jako domowy".into(), checked: false, enabled: has_dir },
        sep(),
        Extra { id: ID_EXPLORER, text: "Otwórz Eksplorator tutaj\tCtrl+E".into(), checked: false, enabled: true },
        Extra { id: ID_TERMINAL, text: "Terminal tutaj".into(), checked: false, enabled: has_dir },
        item(ID_REFRESH, "Odśwież\tF5"),
        Extra { id: ID_PROTECTED, text: "Pokaż pliki systemowe\tCtrl+H".into(), checked: prot, enabled: true },
        sep(),
    ];
    if MODE.with(|m| m.get()) == Mode::Desktop {
        extras.push(Extra { id: ID_AUTOSTART, text: "Uruchamiaj z Windows".into(), checked: autostart::enabled(), enabled: true });
    }
    extras.push(item(ID_QUIT, "Zamknij Indeks (przywraca zwykły pulpit)"));
    if let MenuResult::Extra(id) = shell::popup_only(hwnd, pt, &extras) {
        handle_command(id, dir);
    }
}

fn handle_command(id: u32, dir: Option<String>) {
    match id {
        ID_PIN => {
            if let Some(d) = dir {
                with_app(|a| a.toggle_fav(&d));
            }
        }
        ID_HOME_HERE => {
            with_app(|a| a.set_home_here());
        }
        ID_HOME => {
            with_app(|a| a.home());
        }
        ID_EXPLORER => shell::explorer_folder(&dir.unwrap_or_default()),
        ID_TERMINAL => {
            if let Some(d) = dir {
                shell::terminal_here(&d);
            }
        }
        ID_REFRESH => {
            with_app(|a| a.reload_if_changed());
        }
        ID_PROTECTED => {
            with_app(|a| a.toggle_protected());
        }
        ID_AUTOSTART => {
            let on = !autostart::enabled();
            autostart::set(on);
        }
        ID_SETTINGS => {
            let p = Config::path();
            shell::launch("notepad.exe", Some(&format!("\"{}\"", p.display())), None);
        }
        ID_QUIT => unsafe {
            let ctl = h(CTL.with(|c| c.get()));
            let _ = PostMessageW(Some(ctl), WM_CLOSE, WPARAM(0), LPARAM(0));
        },
        _ => {}
    }
    invalidate();
}
