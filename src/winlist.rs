// Otwarte aplikacje (jak Alt+Tab) do rzedu przyciskow pod paskiem wydajnosci
// oraz wykrywanie okna na pelnym ekranie (nakladka, klawisz Windows).
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, PartialEq)]
pub struct AppWin {
    pub hwnd: isize,
    pub title: String,
    pub exe: String,
    pub minimized: bool,
}

fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// Okna, ktore nie sa "aplikacjami" (pulpit, pasek zadan, Start, nasze).
fn shell_class(c: &str) -> bool {
    matches!(
        c,
        "WorkerW" | "Progman" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "IndeksWidok" | "IndeksKontroler"
            | "IndeksNakladka" | "Windows.UI.Core.CoreWindow" | "NotifyIconOverflowWindow"
    )
}

fn exe_of(h: HWND) -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 512];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(p);
        if ok { String::from_utf16_lossy(&buf[..n as usize]) } else { String::new() }
    }
}

pub fn list() -> Vec<AppWin> {
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let out = &mut *(l.0 as *mut Vec<AppWin>);
        if !IsWindowVisible(h).as_bool() || GetWindow(h, GW_OWNER).map(|o| !o.is_invalid()).unwrap_or(false) {
            return true.into();
        }
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 && ex & WS_EX_APPWINDOW.0 == 0 {
            return true.into();
        }
        // ukryte "w tle" okna aplikacji ze Sklepu
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
        // okna Indeksu (dodatkowe okna eksploratora) tez sa aplikacjami na liscie
        let cls = class_of(h);
        if cloaked != 0 || (shell_class(&cls) && cls != "IndeksWidok") {
            return true.into();
        }
        let mut t = [0u16; 256];
        let n = GetWindowTextW(h, &mut t) as usize;
        if n == 0 {
            return true.into();
        }
        out.push(AppWin { hwnd: h.0 as isize, title: String::from_utf16_lossy(&t[..n]), exe: exe_of(h), minimized: IsIconic(h).as_bool() });
        true.into()
    }
    let mut out = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// Przywraca i aktywuje okno (klik w przycisk aplikacji).
pub fn activate(hwnd: isize) {
    unsafe {
        let h = HWND(hwnd as *mut _);
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        if SetForegroundWindow(h).as_bool() {
            return;
        }
        // Windows nie oddaje pierwszego planu procesowi w tle - na chwile laczymy
        // kolejke wejscia z watkiem okna na wierzchu (jak Alt+Tab)
        let fg = GetForegroundWindow();
        let fg_tid = GetWindowThreadProcessId(fg, None);
        let me = windows::Win32::System::Threading::GetCurrentThreadId();
        let attached = fg_tid != 0 && fg_tid != me && windows::Win32::System::Threading::AttachThreadInput(me, fg_tid, true).as_bool();
        let _ = BringWindowToTop(h);
        let ok = SetForegroundWindow(h).as_bool();
        if attached {
            let _ = windows::Win32::System::Threading::AttachThreadInput(me, fg_tid, false);
        }
        if !ok {
            SwitchToThisWindow(h, true);
        }
    }
}

/// Okno na pierwszym planie, jesli zajmuje caly monitor (gra, film, prezentacja).
pub fn fullscreen_foreground() -> Option<HWND> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() || !IsWindowVisible(fg).as_bool() || IsIconic(fg).as_bool() || shell_class(&class_of(fg)) {
            return None;
        }
        let mut r = RECT::default();
        GetWindowRect(fg, &mut r).ok()?;
        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        GetMonitorInfoW(mon, &mut mi).ok().ok()?;
        let m = mi.rcMonitor;
        (r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom).then_some(fg)
    }
}

/// Cel nakladki: okno na wierzchu na pelnym ekranie (false) albo zmaksymalizowane /
/// zakrywajace obszar roboczy (true = nakladka nad paskiem zadan).
pub fn overlay_target(maximized: bool) -> Option<(HWND, bool)> {
    if let Some(h) = fullscreen_foreground() {
        return Some((h, false));
    }
    if !maximized {
        return None;
    }
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() || !IsWindowVisible(fg).as_bool() || IsIconic(fg).as_bool() || shell_class(&class_of(fg)) {
            return None;
        }
        if IsZoomed(fg).as_bool() {
            return Some((fg, true));
        }
        let mut r = RECT::default();
        GetWindowRect(fg, &mut r).ok()?;
        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        GetMonitorInfoW(mon, &mut mi).ok().ok()?;
        let w = mi.rcWork;
        (r.left <= w.left && r.top <= w.top && r.right >= w.right && r.bottom >= w.bottom).then_some((fg, true))
    }
}

pub fn is_start_menu(h: HWND) -> bool {
    let c = class_of(h);
    c == "Windows.UI.Core.CoreWindow" || c == "Shell_TrayWnd"
}

pub fn minimize(h: HWND) {
    unsafe {
        let _ = ShowWindowAsync(h, SW_MINIMIZE);
    }
}
