// Optymalizacja zasobów tła. Podział: Rust decyduje (mózg), osobny program C robi czarną
// robotę (indeks-optymalizator.exe: próbkowanie + dławienie EcoQoS/priorytet/RAM), C# pokazuje.
// Rust i C gadają na żywo przez potok (stdin/stdout dziecka).
//
// Dla każdej aplikacji z oknem liczymy, jak długo jest aktywna (na pierwszym planie). Gdy
// znika z pierwszego planu i zostaje zminimalizowana, rusza odliczanie: po ~15% jej czasu
// aktywności ograniczamy o jeden stopień (EcoQoS + niższy priorytet), po kolejnych 15% mocniej
// (priorytet bezczynny + oczyszczenie RAM). Powrót na pierwszy plan natychmiast zdejmuje
// ograniczenia. Nie ruszamy pierwszego planu, powłoki, systemu ani Indeksu.
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::Instant;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::SystemInformation::GetSystemInfo;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

const NORMAL_PRIORITY: u32 = 0x0000_0020;
const BELOW_NORMAL_PRIORITY: u32 = 0x0000_4000;
const IDLE_PRIORITY: u32 = 0x0000_0040;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Minimalny czas, po którym w ogóle zaczynamy ograniczać (krótkie przełączenie się nie liczy).
const MIN_BUDGET_S: f32 = 45.0;
/// Każdy stopień ograniczenia to kolejne 15% dotychczasowego czasu aktywności.
const STEP_FRACTION: f32 = 0.15;
const MAX_LEVEL: u8 = 3;

/// Połączenie z osobnym programem C (potok): Rust pisze komendę, czyta jedną linię odpowiedzi.
struct Worker {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    fn spawn() -> Option<Worker> {
        let exe = std::env::current_exe().ok()?.with_file_name("indeks-optymalizator.exe");
        if !exe.exists() {
            crate::util::log("optymalizator: brak indeks-optymalizator.exe");
            return None;
        }
        let mut child = Command::new(exe)
            .args(["--rodzic", &std::process::id().to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = BufReader::new(child.stdout.take()?);
        Some(Worker { _child: child, stdin, stdout })
    }

    /// Jedna komenda -> jedna linia odpowiedzi (None, gdy potok padł).
    fn cmd(&mut self, line: &str) -> Option<String> {
        self.stdin.write_all(line.as_bytes()).ok()?;
        self.stdin.write_all(b"\n").ok()?;
        self.stdin.flush().ok()?;
        let mut s = String::new();
        if self.stdout.read_line(&mut s).ok()? == 0 {
            return None;
        }
        Some(s.trim_end().to_string())
    }

    /// (ws, cpu100, denied). denied = proces podniesiony / innego użytkownika.
    fn sample(&mut self, pid: u32) -> Option<(u64, u64, bool)> {
        let r = self.cmd(&format!("s {}", pid))?;
        let mut it = r.split_whitespace();
        let ws = it.next()?.parse().ok()?;
        let cpu = it.next()?.parse().ok()?;
        let den = it.next()? == "1";
        Some((ws, cpu, den))
    }

    fn eco(&mut self, pid: u32, on: bool) {
        self.cmd(&format!("e {} {}", pid, on as u8));
    }
    fn prio(&mut self, pid: u32, cls: u32) {
        self.cmd(&format!("p {} {}", pid, cls));
    }
    fn trim(&mut self, pid: u32) -> u64 {
        self.cmd(&format!("t {}", pid)).and_then(|r| r.trim().parse().ok()).unwrap_or(0)
    }
    fn restore(&mut self, pid: u32) {
        self.cmd(&format!("r {}", pid));
    }
}

struct AppState {
    exe: String,
    hwnd: isize,
    active_secs: f32,
    idle_since: Option<Instant>,
    level: u8,
    saved_ws: u64,
    ws_now: u64,
    last_cpu100: u64,
    /// Średnie zużycie CPU, gdy był aktywny (ułamek wszystkich rdzeni).
    avg_cpu: f32,
    seen: bool,
}

/// Jedna aplikacja do pokazania w rozwijanej zakładce optymalizacji.
#[derive(Clone)]
pub struct AppUsage {
    pub pid: u32,
    pub hwnd: isize,
    pub exe: String,
    pub ws: u64,
    pub cpu: f32,
    pub level: u8,
}

#[derive(Clone, Copy, Default)]
pub struct Savings {
    pub saved_ram: u64,
    pub count: u32,
    pub would_cpu: f32,
}

struct Engine {
    apps: HashMap<u32, AppState>,
    cores: f32,
    last: Savings,
    worker: Option<Worker>,
    usage: Vec<AppUsage>,
    /// Aplikacje ograniczone ręcznie („Ogranicz teraz") - trzymane w ryzach, aż wrócą na wierzch.
    forced: HashSet<u32>,
}

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

fn cores() -> f32 {
    let mut si = Default::default();
    unsafe { GetSystemInfo(&mut si) };
    si.dwNumberOfProcessors.max(1) as f32
}

/// Procesy, których nigdy nie dotykamy (powłoka, system, nasze okna).
fn protected(exe: &str) -> bool {
    let name = exe.rsplit(['\\', '/']).next().unwrap_or(exe).to_ascii_lowercase();
    matches!(
        name.as_str(),
        "" | "explorer.exe" | "dwm.exe" | "csrss.exe" | "winlogon.exe" | "services.exe" | "lsass.exe"
            | "sihost.exe" | "fontdrvhost.exe" | "taskmgr.exe" | "shellexperiencehost.exe"
            | "searchhost.exe" | "startmenuexperiencehost.exe" | "textinputhost.exe" | "applicationframehost.exe"
            | "indeks.exe" | "indeks-panel.exe" | "indeks-nakladka.exe" | "indeks-katalog.exe" | "indeks-optymalizator.exe"
    )
}

fn foreground_pid() -> u32 {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return 0;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        pid
    }
}

/// Jedno przejście pętli: `enabled` z ustawień, `dt` = sekundy od poprzedniego.
pub fn tick(enabled: bool, dt: f32) -> Savings {
    let mut guard = ENGINE.lock().unwrap();
    let e = guard.get_or_insert_with(|| Engine { apps: HashMap::new(), cores: cores(), last: Savings::default(), worker: None, usage: Vec::new(), forced: HashSet::new() });
    if !enabled {
        if !e.apps.is_empty() || e.worker.is_some() {
            restore_all(e);
            e.worker = None; // zamknięcie stdin -> C sam przywraca i kończy
        }
        e.last = Savings::default();
        e.usage.clear();
        write_state(e);
        return e.last;
    }
    if e.worker.is_none() {
        e.worker = Worker::spawn();
    }
    if e.worker.is_none() {
        return Savings::default();
    }
    let fg = foreground_pid();
    let own = std::process::id();
    let mut shown: HashMap<u32, bool> = HashMap::new();
    let mut exe_of: HashMap<u32, String> = HashMap::new();
    let mut hwnd_of: HashMap<u32, isize> = HashMap::new();
    for w in crate::winlist::list() {
        let h = HWND(w.hwnd as *mut _);
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
        if pid == 0 || pid == own {
            continue;
        }
        *shown.entry(pid).or_insert(false) |= !w.minimized;
        exe_of.entry(pid).or_insert(w.exe);
        // do „Minimalizuj" wolimy okno widoczne (nie zminimalizowane)
        let slot = hwnd_of.entry(pid).or_insert(w.hwnd);
        if !w.minimized {
            *slot = w.hwnd;
        }
    }
    for s in e.apps.values_mut() {
        s.seen = false;
    }
    let dt_100ns = (dt as f64 * 1e7).max(1.0);
    let cores_f = e.cores;
    for (&pid, &visible) in &shown {
        let exe = exe_of.get(&pid).cloned().unwrap_or_default();
        if protected(&exe) {
            continue;
        }
        let Some((ws, cpu100, denied)) = e.worker.as_mut().and_then(|w| w.sample(pid)) else {
            continue;
        };
        let hwnd = hwnd_of.get(&pid).copied().unwrap_or(0);
        let st = e.apps.entry(pid).or_insert_with(|| AppState {
            exe: exe.clone(),
            hwnd,
            active_secs: 0.0,
            idle_since: None,
            level: 0,
            saved_ws: 0,
            ws_now: ws,
            last_cpu100: cpu100,
            avg_cpu: 0.0,
            seen: true,
        });
        st.seen = true;
        st.ws_now = ws;
        st.hwnd = hwnd;
        if denied {
            continue;
        }
        let d_cpu = cpu100.saturating_sub(st.last_cpu100) as f64;
        let inst = (d_cpu / dt_100ns / cores_f as f64).clamp(0.0, 1.0) as f32;
        st.last_cpu100 = cpu100;
        st.avg_cpu = st.avg_cpu * 0.8 + inst * 0.2;

        let active = pid == fg;
        let forced = e.forced.contains(&pid);
        let target = if active {
            st.active_secs += dt;
            st.idle_since = None;
            0
        } else if forced {
            // „Ogranicz teraz": od razu i mocno, aż wróci na pierwszy plan
            2
        } else if visible {
            st.idle_since = None;
            0
        } else {
            let idle = st.idle_since.get_or_insert_with(Instant::now).elapsed().as_secs_f32();
            let budget = (st.active_secs * STEP_FRACTION).max(MIN_BUDGET_S);
            ((idle / budget).floor() as u8).min(MAX_LEVEL)
        };
        if active {
            e.forced.remove(&pid);
        }
        if (target == 0 && st.level > 0) || target > st.level {
            if let Some(w) = e.worker.as_mut() {
                set_level(w, st, pid, target);
            }
        }
    }
    e.apps.retain(|_, s| s.seen);
    let alive: HashSet<u32> = e.apps.keys().copied().collect();
    e.forced.retain(|p| alive.contains(p));

    let mut saved = 0u64;
    let mut count = 0u32;
    let mut would = 0.0f32;
    let mut usage: Vec<AppUsage> = Vec::new();
    for (&pid, s) in e.apps.iter() {
        if s.level > 0 {
            saved += s.saved_ws;
            count += 1;
            would += s.avg_cpu;
        }
        // pokazujemy w zakładce to, co realnie coś waży lub jest ograniczone
        if s.level > 0 || s.ws_now > 48 * 1024 * 1024 || s.avg_cpu > 0.01 {
            usage.push(AppUsage { pid, hwnd: s.hwnd, exe: s.exe.clone(), ws: s.ws_now, cpu: s.avg_cpu, level: s.level });
        }
    }
    usage.sort_by(|a, b| (b.ws + (b.cpu * 4e9) as u64).cmp(&(a.ws + (a.cpu * 4e9) as u64)));
    usage.truncate(60);
    e.usage = usage;
    e.last = Savings { saved_ram: saved, count, would_cpu: would };
    write_state(e);
    e.last
}

/// Ustawia stopień ograniczenia i wykonuje różnicę przez osobny program C.
fn set_level(w: &mut Worker, st: &mut AppState, pid: u32, level: u8) {
    if level == st.level {
        return;
    }
    if level == 0 {
        w.restore(pid);
        w.prio(pid, NORMAL_PRIORITY);
        st.level = 0;
        st.saved_ws = 0;
        return;
    }
    w.eco(pid, true);
    if level >= 2 {
        w.prio(pid, IDLE_PRIORITY);
        st.saved_ws += w.trim(pid);
    } else {
        w.prio(pid, BELOW_NORMAL_PRIORITY);
    }
    if level >= 3 {
        st.saved_ws += w.trim(pid);
    }
    st.level = level;
}

fn restore_all(e: &mut Engine) {
    if let Some(w) = e.worker.as_mut() {
        for (&pid, st) in e.apps.iter() {
            if st.level > 0 {
                w.restore(pid);
            }
        }
    }
    e.apps.clear();
}

/// Przywraca wszystkim pełne zasoby (wyłączenie opcji, zamknięcie pulpitu).
pub fn disable_all() {
    let mut guard = ENGINE.lock().unwrap();
    if let Some(e) = guard.as_mut() {
        restore_all(e);
        e.worker = None;
        e.last = Savings::default();
        e.usage.clear();
        write_state(e);
    }
}

pub fn savings() -> Savings {
    ENGINE.lock().unwrap().as_ref().map(|e| e.last).unwrap_or_default()
}

/// Lista aplikacji do rozwijanej zakładki (najcięższe pierwsze).
pub fn usage() -> Vec<AppUsage> {
    ENGINE.lock().unwrap().as_ref().map(|e| e.usage.clone()).unwrap_or_default()
}

/// „Ogranicz teraz": natychmiast ogranicza daną aplikację (trzyma, aż wróci na pierwszy plan).
pub fn force_limit(pid: u32) {
    let mut guard = ENGINE.lock().unwrap();
    if let Some(e) = guard.as_mut() {
        e.forced.insert(pid);
        if let (Some(w), Some(st)) = (e.worker.as_mut(), e.apps.get_mut(&pid)) {
            set_level(w, st, pid, 2);
        }
    }
}

/// Plik stanu dla panelu C# (%LOCALAPPDATA%\Indeks\optymalizator.stan).
/// V<on> <saved> <count> <would_cpu_promile>\nA<pid> <exe> <ws> <cpu_promile> <level>
fn write_state(e: &Engine) {
    let dir = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Indeks");
    let file = dir.join("optymalizator.stan");
    let mut s = String::new();
    let on = e.worker.is_some();
    s.push_str(&format!("V\t{}\t{}\t{}\t{}\n", on as u8, e.last.saved_ram, e.last.count, (e.last.would_cpu * 1000.0) as u32));
    for a in &e.usage {
        let name = a.exe.rsplit(['\\', '/']).next().unwrap_or(&a.exe);
        s.push_str(&format!("A\t{}\t{}\t{}\t{}\t{}\n", a.pid, name, a.ws, (a.cpu * 1000.0) as u32, a.level));
    }
    let tmp = file.with_extension("stan.tmp");
    if std::fs::write(&tmp, s).is_ok() {
        let _ = std::fs::rename(&tmp, &file);
    }
}
