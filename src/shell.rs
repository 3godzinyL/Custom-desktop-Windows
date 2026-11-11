// Akcje powloki Windows: otwieranie, Eksplorator, natywne menu kontekstowe.
//
// Wazne: okno Indeksu jest dzieckiem okna Explorera, wiec kolejki wejscia sa
// polaczone. Nic, co moze dlugo trwac, nie moze blokowac watku UI - inaczej
// zawieslibysmy tez pulpit/pasek zadan. Dlatego otwieranie idzie w osobnym watku.
use std::cell::RefCell;

use windows::core::{Interface, BSTR, PCSTR, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, IDispatch, IServiceProvider, CLSCTX_ALL,
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    IContextMenu, IContextMenu2, IContextMenu3, IShellBrowser, IShellDispatch2, IShellFolder,
    IShellFolderViewDual, IShellWindows, SHBindToParent, SHParseDisplayName, ShellExecuteExW,
    ShellWindows, CMF_EXTENDEDVERBS, CMF_NORMAL, CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFO,
    CMINVOKECOMMANDINFOEX, CSIDL_DESKTOP, GCS_VERBW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
    SID_STopLevelBrowser, SVGIO_BACKGROUND, SWC_DESKTOP, SWFO_NEEDDISPATCH,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, AppendMenuW, CreatePopupMenu, DestroyMenu, InsertMenuW,
    MF_BYPOSITION, MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, SW_SHOWNORMAL, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenuEx, ASFW_ANY,
};

use crate::util::{self, pcwstr, wide};

/// Uruchamia przez samego Explorera (IShellDispatch2::ShellExecute obiektu pulpitu):
/// program startuje dokladnie jak po dwukliku na zwyklym pulpicie - rodzicem jest
/// explorer.exe, z jego srodowiskiem, rozmiarem i polozeniem okien, a nie Indeks.
/// Gdy to sie nie uda, zapasowo zwykly ShellExecuteEx.
pub fn launch(file: &str, args: Option<&str>, dir: Option<&str>) {
    let (file, args, dir) = (file.to_string(), args.map(str::to_string), dir.map(str::to_string));
    let trace = std::env::var_os("INDEKS_TRACE").is_some();
    if trace {
        util::log(&format!("launch: {}", file));
    }
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        match via_explorer_steps(&file, args.as_deref(), dir.as_deref(), trace) {
            Ok(()) => util::log(&format!("uruchomiono przez Explorera: {}", file)),
            Err(e) => {
                util::log(&format!("uruchamianie przez Explorera nieudane ({}), zapasowo ShellExecuteEx", e.message()));
                shell_execute(&file, args.as_deref(), dir.as_deref());
            }
        }
    });
}

/// Diagnostyka: `indeks.exe --uruchom PLIK` - ta sama droga, synchronicznie, z logiem krokow.
pub fn launch_debug(file: &str) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let t0 = std::time::Instant::now();
        let r = via_explorer_steps(file, None, crate::fsx::parent(file).as_deref(), true);
        util::log(&format!("--uruchom {}: {:?} po {} ms", file, r.map_err(|e| e.message()), t0.elapsed().as_millis()));
    }
}

unsafe fn via_explorer_steps(file: &str, args: Option<&str>, dir: Option<&str>, trace: bool) -> windows::core::Result<()> {
    let t0 = std::time::Instant::now();
    let step = |s: &str| {
        if trace {
            util::log(&format!("  krok {} ({} ms)", s, t0.elapsed().as_millis()));
        }
    };
    let sw: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
    step("ShellWindows");
    let loc = VARIANT::from(CSIDL_DESKTOP as i32);
    let root = VARIANT::default();
    let mut hwnd = 0i32;
    let disp = sw.FindWindowSW(&loc, &root, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)?;
    step("FindWindowSW");
    let sp: IServiceProvider = disp.cast()?;
    let browser: IShellBrowser = sp.QueryService(&SID_STopLevelBrowser)?;
    let view = browser.QueryActiveShellView()?;
    step("QueryActiveShellView");
    let bg: IDispatch = view.GetItemObject(SVGIO_BACKGROUND)?;
    step("GetItemObject");
    let fvd: IShellFolderViewDual = bg.cast()?;
    let shell: IShellDispatch2 = fvd.Application()?.cast()?;
    step("Application");
    let v = |s: Option<&str>| VARIANT::from(BSTR::from(s.unwrap_or("")));
    let r = shell.ShellExecute(&BSTR::from(file), &v(args), &v(dir), &v(None), &VARIANT::from(SW_SHOWNORMAL.0));
    step("ShellExecute");
    r
}

unsafe fn shell_execute(file: &str, args: Option<&str>, dir: Option<&str>) {
    let wfile = wide(file);
    let wargs = args.map(wide);
    let wdir = dir.map(wide);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpFile: pcwstr(&wfile),
        lpParameters: wargs.as_ref().map(|v| pcwstr(v)).unwrap_or(PCWSTR::null()),
        lpDirectory: wdir.as_ref().map(|v| pcwstr(v)).unwrap_or(PCWSTR::null()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    if let Err(e) = ShellExecuteExW(&mut info) {
        util::log(&format!("otwieranie {}: {}", file, e.message()));
    }
}

/// Chowa wszystkie okna (jak Win+M) - klik w puste miejsce pulpitu.
pub fn minimize_all() {
    std::thread::spawn(|| unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let r = (|| -> windows::core::Result<()> {
            let sw: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
            let mut hwnd = 0i32;
            let disp = sw.FindWindowSW(&VARIANT::from(CSIDL_DESKTOP as i32), &VARIANT::default(), SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)?;
            let sp: IServiceProvider = disp.cast()?;
            let browser: IShellBrowser = sp.QueryService(&SID_STopLevelBrowser)?;
            let bg: IDispatch = browser.QueryActiveShellView()?.GetItemObject(SVGIO_BACKGROUND)?;
            let shell: IShellDispatch2 = bg.cast::<IShellFolderViewDual>()?.Application()?.cast()?;
            shell.MinimizeAll()
        })();
        if let Err(e) = r {
            util::log(&format!("chowanie okien: {}", e.message()));
        }
    });
}

/// Otwiera plik/folder domyslna aplikacja (jak dwuklik na pulpicie).
pub fn open(path: &str) {
    let dir = crate::fsx::parent(path).filter(|p| !p.is_empty());
    launch(path, None, dir.as_deref());
}

/// Okno Eksploratora w folderze (pusta sciezka = Ten komputer).
pub fn explorer_folder(path: &str) {
    let p = if path.is_empty() { "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}".to_string() } else { path.to_string() };
    launch("explorer.exe", Some(&format!("\"{}\"", p)), None);
}

pub fn explorer_select(path: &str) {
    launch("explorer.exe", Some(&format!("/select,\"{}\"", path)), None);
}

pub fn terminal_here(dir: &str) {
    if dir.is_empty() {
        return;
    }
    // Windows Terminal, jesli jest; inaczej PowerShell w tym folderze
    let wt = std::env::var("LOCALAPPDATA").map(|l| format!("{}\\Microsoft\\WindowsApps\\wt.exe", l)).unwrap_or_default();
    if std::path::Path::new(&wt).exists() {
        launch(&wt, Some(&format!("-d \"{}\"", dir)), Some(dir));
    } else {
        launch("powershell.exe", Some("-NoExit"), Some(dir));
    }
}

// ---------- menu kontekstowe ----------

thread_local! {
    static ACTIVE: RefCell<Option<IContextMenu>> = const { RefCell::new(None) };
}

/// Przekazuje wiadomosci menu do rozszerzen powloki (podmenu "Wyslij do", "Otworz za pomoca").
pub fn forward_menu_msg(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    ACTIVE.with(|a| {
        let a = a.borrow();
        let cm = a.as_ref()?;
        unsafe {
            if let Ok(cm3) = cm.cast::<IContextMenu3>() {
                let mut res = LRESULT(0);
                if cm3.HandleMenuMsg2(msg, wp, lp, Some(&mut res)).is_ok() {
                    return Some(res);
                }
            } else if let Ok(cm2) = cm.cast::<IContextMenu2>() {
                if cm2.HandleMenuMsg(msg, wp, lp).is_ok() {
                    return Some(LRESULT(0));
                }
            }
        }
        None
    })
}

/// Wlasna pozycja menu: (id, tekst, zaznaczona, aktywna). id >= 0x8000.
pub struct Extra {
    pub id: u32,
    pub text: String,
    pub checked: bool,
    pub enabled: bool,
}

pub const EXTRA_BASE: u32 = 0x8000;

pub enum MenuResult {
    None,
    Extra(u32),
    /// Powloka chciala "Zmien nazwe" - tego nie obsluzy obcy host.
    Rename,
    /// Domyslne "Otworz".
    Open,
    Done,
}

/// Menu kontekstowe Windows dla pliku/folderu + wlasne pozycje na gorze.
pub fn context_menu(hwnd: HWND, path: &str, pt: POINT, extras: &[Extra]) -> MenuResult {
    unsafe {
        let wpath = wide(path);
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        if SHParseDisplayName(pcwstr(&wpath), None, &mut pidl, 0, None).is_err() || pidl.is_null() {
            return popup_only(hwnd, pt, extras);
        }
        let result = (|| -> windows::core::Result<MenuResult> {
            let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
            let folder: IShellFolder = SHBindToParent(pidl, Some(&mut child as *mut _ as *mut _))?;
            let cm: IContextMenu = folder.GetUIObjectOf(hwnd, &[child as *const ITEMIDLIST], None)?;
            let menu = CreatePopupMenu()?;
            let mut flags = CMF_NORMAL;
            if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                flags |= CMF_EXTENDEDVERBS;
            }
            cm.QueryContextMenu(menu, 0, 1, 0x7FFF, flags).ok()?;
            if !extras.is_empty() {
                let _ = InsertMenuW(menu, 0, MF_BYPOSITION | MF_SEPARATOR, 0, PCWSTR::null());
                for e in extras.iter().rev() {
                    let t = wide(&e.text);
                    let mut f = MF_BYPOSITION | MF_STRING;
                    if e.checked {
                        f |= MF_CHECKED;
                    }
                    if !e.enabled {
                        f |= MF_GRAYED;
                    }
                    let _ = InsertMenuW(menu, 0, f, e.id as usize, pcwstr(&t));
                }
            }
            ACTIVE.with(|a| *a.borrow_mut() = Some(cm.clone()));
            let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0 as u32;
            ACTIVE.with(|a| *a.borrow_mut() = None);
            let res = if cmd == 0 {
                MenuResult::None
            } else if cmd >= EXTRA_BASE {
                MenuResult::Extra(cmd)
            } else {
                let verb = verb_of(&cm, cmd - 1);
                if verb.eq_ignore_ascii_case("rename") {
                    MenuResult::Rename
                } else if verb.eq_ignore_ascii_case("open") {
                    // "Otworz" jak dwuklik: przez Explorera (albo wejscie do folderu)
                    MenuResult::Open
                } else {
                    let info = CMINVOKECOMMANDINFOEX {
                        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                        fMask: CMIC_MASK_PTINVOKE | 0x4000, // 0x4000 = CMIC_MASK_UNICODE
                        hwnd,
                        lpVerb: PCSTR((cmd - 1) as usize as *const u8),
                        lpVerbW: PCWSTR((cmd - 1) as usize as *const u16),
                        nShow: SW_SHOWNORMAL.0,
                        ptInvoke: pt,
                        ..Default::default()
                    };
                    let _ = AllowSetForegroundWindow(ASFW_ANY);
                    if let Err(e) = cm.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO) {
                        util::log(&format!("menu {} ({}): {}", path, verb, e.message()));
                    }
                    MenuResult::Done
                }
            };
            let _ = DestroyMenu(menu);
            Ok(res)
        })();
        CoTaskMemFree(Some(pidl as *const _));
        result.unwrap_or(MenuResult::None)
    }
}

unsafe fn verb_of(cm: &IContextMenu, id: u32) -> String {
    let mut buf = [0u16; 64];
    if cm.GetCommandString(id as usize, GCS_VERBW, None, windows::core::PSTR(buf.as_mut_ptr() as *mut u8), buf.len() as u32).is_ok() {
        util::from_wide(&buf)
    } else {
        String::new()
    }
}

/// Zwykle menu tylko z wlasnymi pozycjami (puste miejsce, linki u gory).
pub fn popup_only(hwnd: HWND, pt: POINT, extras: &[Extra]) -> MenuResult {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return MenuResult::None };
        for e in extras {
            if e.id == 0 {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                continue;
            }
            let t = wide(&e.text);
            let mut f = MF_STRING;
            if e.checked {
                f |= MF_CHECKED;
            }
            if !e.enabled {
                f |= MF_GRAYED;
            }
            let _ = AppendMenuW(menu, f, e.id as usize, pcwstr(&t));
        }
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0 as u32;
        let _ = DestroyMenu(menu);
        if cmd == 0 {
            MenuResult::None
        } else {
            MenuResult::Extra(cmd)
        }
    }
}

pub fn sep() -> Extra {
    Extra { id: 0, text: String::new(), checked: false, enabled: true }
}

pub fn item(id: u32, text: &str) -> Extra {
    Extra { id, text: text.to_string(), checked: false, enabled: true }
}
