// Integracja z pulpitem Windows.
//
// Okno Indeksu staje sie oknem-dzieckiem tego samego okna Explorera, ktore
// trzyma ikony pulpitu (Progman albo WorkerW z SHELLDLL_DefView), i lezy nad
// nimi w kolejnosci Z. Dzieki temu:
//  - Win+D / "Pokaz pulpit" / menu Start nie chowaja Indeksu (to jest pulpit),
//  - nie ma osobnego okna na pasku zadan ani w Alt+Tab,
//  - po zamknieciu/awarii Indeksu zostaje zwykly pulpit Windows.
// Ikony Windows sa wylaczane oficjalnym przelacznikiem FWF_NOICONS (to samo co
// "Widok > Pokaz ikony pulpitu"); pliki na pulpicie nie sa ruszane.
use windows::core::{w, Interface, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MapWindowPoints, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::Com::{CoCreateInstance, IServiceProvider, CLSCTX_ALL};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    IFolderView2, IShellBrowser, IShellWindows, ShellWindows, CSIDL_DESKTOP, FWF_NOICONS,
    SID_STopLevelBrowser, SWC_DESKTOP, SWFO_NEEDDISPATCH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, GetClassNameW, IsWindowVisible, SendMessageW, WM_COMMAND,
};

use crate::util;

/// Okno najwyzszego poziomu Explorera, ktore zawiera SHELLDLL_DefView.
pub fn find_host() -> Option<HWND> {
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let out = &mut *(l.0 as *mut Option<HWND>);
        let mut cls = [0u16; 32];
        let n = GetClassNameW(h, &mut cls) as usize;
        let cls = String::from_utf16_lossy(&cls[..n]);
        if cls == "Progman" || cls == "WorkerW" {
            if let Ok(dv) = FindWindowExW(Some(h), None, w!("SHELLDLL_DefView"), PCWSTR::null()) {
                if !dv.is_invalid() {
                    *out = Some(h);
                    return false.into();
                }
            }
        }
        true.into()
    }
    let mut out: Option<HWND> = None;
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

pub fn defview(host: HWND) -> Option<HWND> {
    unsafe { FindWindowExW(Some(host), None, w!("SHELLDLL_DefView"), PCWSTR::null()).ok() }
        .filter(|h| !h.is_invalid())
}

/// Obszar roboczy monitora glownego (bez paska zadan) w ukladzie klienta `host`.
pub fn work_rect_in(host: Option<HWND>) -> RECT {
    unsafe {
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let mut r = mi.rcWork;
        if let Some(h) = host {
            let mut pts = [POINT { x: r.left, y: r.top }, POINT { x: r.right, y: r.bottom }];
            MapWindowPoints(None, Some(h), &mut pts);
            r = RECT { left: pts[0].x, top: pts[0].y, right: pts[1].x, bottom: pts[1].y };
        }
        r
    }
}

fn folder_view() -> windows::core::Result<IFolderView2> {
    unsafe {
        let sw: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        let loc = VARIANT::from(CSIDL_DESKTOP as i32);
        let root = VARIANT::default();
        let mut hwnd = 0i32;
        let disp = sw.FindWindowSW(&loc, &root, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)?;
        let sp: IServiceProvider = disp.cast()?;
        let browser: IShellBrowser = sp.QueryService(&SID_STopLevelBrowser)?;
        let view = browser.QueryActiveShellView()?;
        view.cast()
    }
}

/// Czy ikony pulpitu Windows sa teraz ukryte.
pub fn icons_hidden() -> Option<bool> {
    if let Ok(fv) = folder_view() {
        if let Ok(f) = unsafe { fv.GetCurrentFolderFlags() } {
            return Some(f & FWF_NOICONS.0 as u32 != 0);
        }
    }
    let host = find_host()?;
    let dv = defview(host)?;
    let lv = unsafe { FindWindowExW(Some(dv), None, w!("SysListView32"), PCWSTR::null()).ok()? };
    Some(!unsafe { IsWindowVisible(lv) }.as_bool())
}

/// Wlacza/wylacza ikony pulpitu Windows (odwracalne, zapamietywane przez Explorer).
pub fn set_icons_hidden(hide: bool) -> bool {
    if icons_hidden() == Some(hide) {
        return true;
    }
    if let Ok(fv) = folder_view() {
        let flags = if hide { FWF_NOICONS.0 as u32 } else { 0 };
        if unsafe { fv.SetCurrentFolderFlags(FWF_NOICONS.0 as u32, flags) }.is_ok() {
            return true;
        }
    }
    // Zapasowo: polecenie menu "Pokaz ikony pulpitu" (przelacznik).
    if let Some(dv) = find_host().and_then(defview) {
        unsafe {
            SendMessageW(dv, WM_COMMAND, Some(WPARAM(0x7402)), Some(LPARAM(0)));
        }
        let ok = icons_hidden() == Some(hide);
        if !ok {
            util::log("pulpit: nie udalo sie przelaczyc ikon");
        }
        return ok;
    }
    false
}
