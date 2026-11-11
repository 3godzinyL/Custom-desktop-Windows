// Indeks - lekki pulpit dla Windows 10/11.
//
//   indeks.exe              pulpit (zastepuje ikony pulpitu Windows)
//   indeks.exe --okno       to samo w zwyklym oknie (do testow)
//   indeks.exe [--okno] --folder SCIEZKA   start w podanym folderze
//   indeks.exe --zamknij    zamyka dzialajacy Indeks i przywraca ikony pulpitu
//   indeks.exe --przywroc   jak --zamknij + wylacza autostart + wymusza ikony pulpitu
//   indeks.exe --autostart-wl / --autostart-wyl
//   indeks.exe --zrzut plik.png [--folder X] [--szukaj Q] [--szer W] [--wys H] [--dpi D]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod config;
mod downloads;
mod desktop;
mod fsx;
mod icons;
mod overlay;
mod page;
mod profiles;
mod render;
mod search;
mod shell;
mod snapshot;
mod stats;
mod util;
mod win;
mod winlist;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsWindow, PostMessageW, WM_CLOSE};

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Pokazuje nakladke (bez aktywacji, przezroczysta dla klikniec) na 4 s - do sprawdzenia wygladu.
fn test_overlay() {
    let Ok(mut a) = app::App::new(config::Config::load(), windows::Win32::Foundation::HWND::default()) else { return };
    a.update_stats();
    std::thread::sleep(std::time::Duration::from_millis(1000));
    a.update_stats();
    if let Some(mut ov) = overlay::Overlay::new() {
        let desk = unsafe { windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow() };
        if let Err(e) = ov.show(desk, false, &a.page.status, &a.cfg, &a.renderer.d2d, &a.renderer.dw) {
            eprintln!("nakladka: {}", e.message());
        }
        std::thread::sleep(std::time::Duration::from_secs(4));
        ov.hide();
    }
}

fn test_search(args: &[String]) {
    let roots: Vec<String> = if args.is_empty() {
        vec![std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".into())]
    } else {
        args.to_vec()
    };
    let cfg = config::Config::load();
    let t0 = std::time::Instant::now();
    search::start(roots, cfg.skip.clone(), windows::Win32::Foundation::HWND::default());
    while !search::status().ready {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let st = search::status();
    println!("indeks: {} wpisow, {:.1}s, {:.1} MB", st.count, t0.elapsed().as_secs_f32(), st.memory as f64 / 1048576.0);
    for q in ["server.js", "pivot", "PULPIT", "pulpit", "ZŁOTE", "złote", "cargo toml", "a"] {
        let t = std::time::Instant::now();
        let (hits, total) = search::search(q, search::Find::All, 50);
        println!("'{}': {} trafien, {:.2} ms", q, total, t.elapsed().as_secs_f64() * 1000.0);
        for h in hits.iter().take(3) {
            println!("    {}{}", h.path, if h.is_dir { "\\" } else { "" });
        }
    }
}

/// Zamyka dzialajacy Indeks (on sam przywraca ikony pulpitu). Czeka do 10 s.
fn close_running() -> bool {
    unsafe {
        let Ok(ctl) = FindWindowW(win::CTL_CLASS, win::CTL_TITLE_DESKTOP) else { return false };
        if ctl.is_invalid() {
            return false;
        }
        let _ = PostMessageW(Some(ctl), WM_CLOSE, WPARAM(0), LPARAM(0));
        for _ in 0..100 {
            if !IsWindow(Some(ctl)).as_bool() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        false
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let first = args.first().map(|s| s.as_str()).unwrap_or("");
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    match first {
        "--test-szukaj" => test_search(&args[1..]),
        "--test-nakladki" => test_overlay(),
        // nowe okno Indeksu w dzialajacym pulpicie (wspolny indeks); bez pulpitu - osobny proces okna
        "--nowe-okno" => {
            let path = args.get(1).cloned().unwrap_or_else(config::desktop_dir);
            match unsafe { FindWindowW(win::CTL_CLASS, win::CTL_TITLE_DESKTOP) } {
                Ok(ctl) if !ctl.is_invalid() => win::send_copy(ctl, win::COPY_WINDOW, &path),
                _ => {
                    if let Err(e) = win::run(win::Mode::Window, Some(path)) {
                        util::log(&format!("okno: {}", e.message()));
                    }
                }
            }
        }
        "--zrzut" => {
            let o = snapshot::Opts {
                out: args.get(1).cloned().unwrap_or_else(|| "indeks.png".into()),
                folder: arg_value(&args, "--folder"),
                query: arg_value(&args, "--szukaj"),
                index: args.iter().any(|a| a == "--indeks"),
                width: arg_value(&args, "--szer").and_then(|v| v.parse().ok()).unwrap_or(2560),
                height: arg_value(&args, "--wys").and_then(|v| v.parse().ok()).unwrap_or(1400),
                dpi: arg_value(&args, "--dpi").and_then(|v| v.parse().ok()).unwrap_or(144.0),
                hover_row: arg_value(&args, "--najedz").and_then(|v| v.parse().ok()),
                select: arg_value(&args, "--zaznacz").and_then(|v| v.parse().ok()),
                settings: args.iter().any(|a| a == "--ustawienia"),
                tree_click: arg_value(&args, "--klik-drzewo").and_then(|v| v.parse().ok()),
                demo: args.iter().any(|a| a == "--demo"),
                kind: arg_value(&args, "--rodzaj").and_then(|v| v.parse().ok()),
            };
            if let Err(e) = snapshot::run(o) {
                util::log(&format!("zrzut: {}", e.message()));
                std::process::exit(1);
            }
        }
        "--zamknij" => {
            close_running();
        }
        "--przywroc" => {
            close_running();
            autostart::set(false);
            desktop::set_icons_hidden(false);
            let mut cfg = config::Config::load();
            cfg.icons_were_hidden = None;
            cfg.save();
        }
        "--uruchom" => {
            if let Some(f) = args.get(1) {
                shell::launch_debug(f);
            }
        }
        "--autostart-wl" => autostart::set(true),
        "--autostart-wyl" => autostart::set(false),
        _ => {
            let window = first == "--okno";
            let name = if window { w!("Local\\Indeks.Okno") } else { w!("Local\\Indeks.Pulpit") };
            let title = if window { win::CTL_TITLE_WINDOW } else { win::CTL_TITLE_DESKTOP };
            let open = || unsafe {
                let m = CreateMutexW(None, false, name);
                (m, GetLastError() == ERROR_ALREADY_EXISTS)
            };
            let (mut _mutex, mut exists) = open();
            // poprzednia instancja wlasnie sie zamyka (np. aktualizacja po --zamknij):
            // okna juz nie ma, a nazwa jeszcze istnieje - poczekaj do 5 s
            for _ in 0..25 {
                if !exists || unsafe { FindWindowW(win::CTL_CLASS, title) }.is_ok() {
                    break;
                }
                if let Ok(h) = _mutex {
                    unsafe {
                        let _ = windows::Win32::Foundation::CloseHandle(h);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
                (_mutex, exists) = open();
            }
            if exists {
                // juz dziala: podany folder/plik (np. nowy z Pobranych) albo folder domowy w istniejacym
                if let Ok(ctl) = unsafe { FindWindowW(win::CTL_CLASS, title) } {
                    match arg_value(&args, "--folder") {
                        Some(p) => win::send_open(ctl, &p),
                        None => unsafe {
                            let _ = PostMessageW(Some(ctl), win::WM_APP_SHOW, WPARAM(0), LPARAM(0));
                        },
                    }
                }
                return;
            }
            let mode = if window { win::Mode::Window } else { win::Mode::Desktop };
            if let Err(e) = win::run(mode, arg_value(&args, "--folder")) {
                util::log(&format!("start: {}", e.message()));
            }
        }
    }
}
