// Optymalizacja zasobów tła. Podział: Rust decyduje (mózg), osobny program C robi czarną
// robotę (indeks-optymalizator.exe: pomiar + dławienie EcoQoS/priorytet/RAM), C# pokazuje.
// Rust i C gadają na żywo przez potok (stdin/stdout dziecka).
//
// Aplikacja = proces okna + drzewo jego potomków (renderery, GPU, helpery) - liczymy i
// dławimy całą grupę, bo okno często waży mało, a podprocesy najwięcej. Aplikacje bez okna
// (schowane do zasobnika: Discord, Steam...) też są grupami - tło od razu.
// Pomiar: jedno zapytanie do jądra o wszystkie procesy (jak Menedżer zadań) - widać też gry
// z anty-cheatem; dławić się ich nie da (system odmawia) i wtedy pokazujemy „chroniona".
//
// Stopnie (licznik rusza, gdy aplikacja traci pierwszy plan):
//  - zminimalizowana / bez okna: po max(45 s, 15% czasu aktywności) stopień 1 (EcoQoS +
//    niższy priorytet), po kolejnym takim odcinku 2 (bezczynny + RAM), potem 3 (ponowny trim);
//  - widoczna, ale nie na wierzchu: po max(3 min, 30% aktywności) tylko stopień 1;
//  - gra dźwięk (miernik szczytu, nie sam otwarty strumień): najwyżej stopień 1.
// Powrót na pierwszy plan natychmiast zdejmuje ograniczenia. Nie ruszamy powłoki, systemu,
// konsol/serwerów/buildów ani Indeksu.
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::SystemInformation::GetSystemInfo;
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Minimalny czas, po którym w ogóle zaczynamy ograniczać (krótkie przełączenie się nie liczy).
const MIN_BUDGET_S: f32 = 45.0;
/// Każdy stopień ograniczenia to kolejne 15% dotychczasowego czasu aktywności.
const STEP_FRACTION: f32 = 0.15;
/// Okno widoczne, ale przykryte: tylko stopień 1, po dłuższym czasie.
const COVERED_MIN_S: f32 = 180.0;
const COVERED_FRACTION: f32 = 0.30;
const MAX_LEVEL: u8 = 3;
/// Aplikacja bez okna wchodzi do optymalizacji od tylu MB prywatnego RAM całej grupy.
const HIDDEN_MIN_RAM: u64 = 150 * 1024 * 1024;
/// Tak długo po ostatnim słyszalnym dźwięku grupa ma najwyżej stopień 1.
const AUDIO_HOLD: Duration = Duration::from_secs(60);

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

    /// Wszystkie procesy jednym zapytaniem (C: NtQuerySystemInformation).
    fn procs(&mut self) -> Option<Procs> {
        let r = self.cmd("l")?;
        let mut info = HashMap::new();
        let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
        for e in r.split('\t') {
            let f: Vec<&str> = e.splitn(7, ' ').collect();
            if f.len() < 7 {
                continue;
            }
            let num = |i: usize| f[i].parse::<u64>().ok();
            let (Some(pid), Some(ppid), Some(start), Some(session), Some(ws), Some(cpu)) = (num(0), num(1), num(2), num(3), num(4), num(5)) else { continue };
            let (pid, ppid) = (pid as u32, ppid as u32);
            info.insert(pid, Proc { ppid, start, session: session as u32, ws, cpu, name: f[6].to_string() });
            if ppid != pid {
                kids.entry(ppid).or_default().push(pid);
            }
        }
        if info.is_empty() {
            return None;
        }
        Some(Procs { info, kids })
    }

    /// (procesy z otwartym dźwiękiem, procesy, które teraz realnie słychać).
    fn audio(&mut self) -> HashSet<u32> {
        let r = self.cmd("a").unwrap_or_default();
        let v: Vec<u32> = r.split_whitespace().filter_map(|p| p.parse().ok()).collect();
        v.chunks(2).filter(|c| c.len() == 2 && c[1] == 1).map(|c| c[0]).collect()
    }

    /// Pełne ścieżki exe (puste = brak dostępu).
    fn paths(&mut self, pids: &[u32]) -> Vec<String> {
        let mut line = String::from("P");
        for p in pids {
            line.push_str(&format!(" {}", p));
        }
        let r = self.cmd(&line).unwrap_or_default();
        let mut out: Vec<String> = r.split('\t').map(|s| s.to_string()).collect();
        out.resize(pids.len(), String::new());
        out
    }

    /// Stopień dla wielu procesów naraz: (zwolnione bajty, ilu procesom system pozwolił).
    fn level_many(&mut self, level: u8, pids: &[u32]) -> (u64, usize) {
        if pids.is_empty() {
            return (0, 0);
        }
        let mut line = format!("L {}", level);
        for p in pids {
            line.push_str(&format!(" {}", p));
        }
        let r = self.cmd(&line).unwrap_or_default();
        let mut it = r.split_whitespace();
        let freed = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        let ok = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        (freed, ok)
    }
}

struct Proc {
    ppid: u32,
    start: u64,
    session: u32,
    /// Prywatny zbiór roboczy (jak kolumna „Pamięć" w Menedżerze zadań).
    ws: u64,
    /// Czas CPU (100 ns), jądro + użytkownik.
    cpu: u64,
    name: String,
}

/// Migawka procesów i dzieci każdego pid.
struct Procs {
    info: HashMap<u32, Proc>,
    kids: HashMap<u32, Vec<u32>>,
}

/// Jeden proces należący do aplikacji (proces okna albo jego potomek).
struct Member {
    last_cpu100: u64,
    /// Stopień, który C już na nim ustawił (nowe dzieci dostają stopień grupy).
    level: u8,
}

struct AppState {
    exe: String,
    hwnd: isize,
    active_secs: f32,
    /// Od kiedy nie jest na pierwszym planie.
    idle_since: Option<Instant>,
    level: u8,
    /// RAM całej grupy tuż przed ograniczeniem (do „teraz vs bez optymalizacji").
    ws_before: u64,
    ws_now: u64,
    /// CPU teraz (ułamek wszystkich rdzeni) i średnia sprzed ograniczenia.
    cpu_now: f32,
    avg_cpu: f32,
    last_audible: Option<Instant>,
    /// System odmówił (anty-cheat / admin) - tylko pomiar.
    blocked: bool,
    /// Okno widoczne (przykryte) - licznik tylko do stopnia 1.
    visible: bool,
    /// Bez okna (zasobnik).
    hidden: bool,
    last_trim: Option<Instant>,
    ws_after_trim: u64,
    members: HashMap<u32, Member>,
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
    /// Liczba procesów w grupie (okno + potomkowie).
    pub procs: u32,
    /// Słychać ją teraz (lub przed chwilą) - najwyżej EcoQoS.
    pub audio: bool,
    pub blocked: bool,
    pub hidden: bool,
    /// Sekundy do następnego stopnia (0 = brak odliczania).
    pub next_in: u32,
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
    /// Ścieżki exe wg (pid, start) - pytamy C tylko o nowe procesy.
    paths: HashMap<(u32, u64), String>,
}

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

fn cores() -> f32 {
    let mut si = Default::default();
    unsafe { GetSystemInfo(&mut si) };
    si.dwNumberOfProcessors.max(1) as f32
}

fn base_name(exe: &str) -> String {
    exe.rsplit(['\\', '/']).next().unwrap_or(exe).to_ascii_lowercase()
}

/// Procesy, których nigdy nie dotykamy (powłoka, system, nasze okna).
fn protected(exe: &str) -> bool {
    matches!(
        base_name(exe).as_str(),
        "" | "explorer.exe" | "dwm.exe" | "csrss.exe" | "winlogon.exe" | "services.exe" | "lsass.exe"
            | "sihost.exe" | "fontdrvhost.exe" | "taskmgr.exe" | "shellexperiencehost.exe" | "audiodg.exe"
            | "searchhost.exe" | "startmenuexperiencehost.exe" | "textinputhost.exe" | "applicationframehost.exe"
            | "ctfmon.exe" | "svchost.exe" | "system" | "registry" | "smss.exe" | "wininit.exe" | "memory compression"
            | "runtimebroker.exe" | "dllhost.exe" | "taskhostw.exe" | "securityhealthsystray.exe" | "nvcontainer.exe"
            | "indeks.exe" | "indeks-panel.exe" | "indeks-nakladka.exe" | "indeks-katalog.exe" | "indeks-optymalizator.exe"
    )
}

/// Konsole i praca w tle (serwery, buildy, nagrywanie) - na nich drzewo się zatrzymuje:
/// zminimalizowany terminal czy edytor nie może zdławić `node server.js`, kompilacji, OBS.
fn work_process(exe: &str) -> bool {
    matches!(
        base_name(exe).as_str(),
        "cmd.exe" | "powershell.exe" | "pwsh.exe" | "conhost.exe" | "openconsole.exe" | "windowsterminal.exe"
            | "wsl.exe" | "wslhost.exe" | "bash.exe" | "sh.exe" | "mintty.exe" | "ssh.exe" | "git.exe"
            | "node.exe" | "python.exe" | "pythonw.exe" | "py.exe" | "cargo.exe" | "rustc.exe" | "cl.exe" | "link.exe"
            | "msbuild.exe" | "dotnet.exe" | "java.exe" | "javaw.exe" | "go.exe" | "ffmpeg.exe" | "obs64.exe"
            | "claude.exe" | "codex.exe" | "terminal64.exe" | "metatester64.exe" | "nvidia share.exe" | "nvsphelper64.exe"
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

/// Procesy mające JAKIEKOLWIEK widoczne okno najwyższego poziomu (też gry na pełnym
/// ekranie, których nie ma na liście Alt+Tab) - takiego procesu nie wliczamy do cudzego drzewa.
fn window_owners() -> HashSet<u32> {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> windows::core::BOOL {
        let out = unsafe { &mut *(lp.0 as *mut HashSet<u32>) };
        if unsafe { IsWindowVisible(h) }.as_bool() {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
            if pid != 0 {
                out.insert(pid);
            }
        }
        true.into()
    }
    let mut out = HashSet::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// Potomkowie `root` (bez niego): bez cudzych okien, chronionych, pracy w tle i `skip`.
fn descendants(root: u32, p: &Procs, owners: &HashSet<u32>, skip: &HashSet<u32>, own: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(cur) = stack.pop() {
        let cur_start = p.info.get(&cur).map(|i| i.start).unwrap_or(0);
        for &c in p.kids.get(&cur).map(|v| v.as_slice()).unwrap_or(&[]) {
            let Some(ci) = p.info.get(&c) else { continue };
            // PID rodzica użyty ponownie: "dziecko" starsze od rodzica nie jest jego dzieckiem
            if ci.start != 0 && cur_start != 0 && ci.start < cur_start {
                continue;
            }
            if c == own || owners.contains(&c) || skip.contains(&c) || protected(&ci.name) || work_process(&ci.name) {
                continue;
            }
            out.push(c);
            if out.len() >= 256 {
                return out;
            }
            stack.push(c);
        }
    }
    out
}

/// Grupa do oceny w tym przejściu.
struct Group {
    root: u32,
    visible: bool,
    hidden: bool,
    exe: String,
    hwnd: isize,
    pids: Vec<u32>,
}

/// Jedno przejście pętli: `enabled` z ustawień, `dt` = sekundy od poprzedniego.
pub fn tick(enabled: bool, dt: f32) -> Savings {
    let mut guard = ENGINE.lock().unwrap();
    let e = guard.get_or_insert_with(|| Engine {
        apps: HashMap::new(),
        cores: cores(),
        last: Savings::default(),
        worker: None,
        usage: Vec::new(),
        forced: HashSet::new(),
        paths: HashMap::new(),
    });
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
    let Some(w) = e.worker.as_mut() else { return Savings::default() };
    let Some(procs) = w.procs() else {
        e.worker = None; // potok padł - następny tick uruchomi C od nowa
        return e.last;
    };
    let fg = foreground_pid();
    let own = std::process::id();
    let own_session = procs.info.get(&own).map(|p| p.session).unwrap_or(1);

    // okna aplikacji (lista jak Alt+Tab)
    let mut shown: HashMap<u32, bool> = HashMap::new();
    let mut exe_of: HashMap<u32, String> = HashMap::new();
    let mut hwnd_of: HashMap<u32, isize> = HashMap::new();
    for win in crate::winlist::list() {
        let h = HWND(win.hwnd as *mut _);
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
        if pid == 0 || pid == own {
            continue;
        }
        *shown.entry(pid).or_insert(false) |= !win.minimized;
        exe_of.entry(pid).or_insert(win.exe);
        // do „Minimalizuj" wolimy okno widoczne (nie zminimalizowane)
        let slot = hwnd_of.entry(pid).or_insert(win.hwnd);
        if !win.minimized {
            *slot = win.hwnd;
        }
    }
    let mut owners = window_owners();
    owners.extend(shown.keys().copied());
    let audible = w.audio();
    let none = HashSet::new();
    // drzewo pierwszego planu nigdy nie jest dławione (np. gra uruchomiona z launchera)
    let fg_tree: HashSet<u32> = if fg != 0 { descendants(fg, &procs, &none, &none, own).into_iter().chain([fg]).collect() } else { HashSet::new() };

    // 1) grupy z oknem
    let mut groups: Vec<Group> = Vec::new();
    let mut claimed: HashSet<u32> = HashSet::new();
    for (&pid, &visible) in &shown {
        let exe = exe_of.get(&pid).cloned().unwrap_or_default();
        if protected(&exe) || (fg_tree.contains(&pid) && pid != fg) || !procs.info.contains_key(&pid) {
            continue;
        }
        let mut pids = vec![pid];
        if !work_process(&exe) {
            pids.extend(descendants(pid, &procs, &owners, &fg_tree, own));
        }
        claimed.extend(pids.iter().copied());
        groups.push(Group { root: pid, visible, hidden: false, exe, hwnd: hwnd_of.get(&pid).copied().unwrap_or(0), pids });
    }
    // 2) aplikacje bez okna (zasobnik): korzeń = proces sesji użytkownika, którego rodzic
    //    zniknął, jest Eksploratorem albo żyje w innej sesji (usługa)
    let mut cand: Vec<(u32, Vec<u32>)> = Vec::new();
    for (&pid, p) in &procs.info {
        if p.session != own_session || pid == own || claimed.contains(&pid) || owners.contains(&pid) || fg_tree.contains(&pid) {
            continue;
        }
        if protected(&p.name) || work_process(&p.name) {
            continue;
        }
        let root_like = match procs.info.get(&p.ppid) {
            None => true,
            Some(par) => (par.start != 0 && p.start != 0 && par.start > p.start) || par.session != own_session || par.name.eq_ignore_ascii_case("explorer.exe"),
        };
        if !root_like {
            continue;
        }
        let mut pids = vec![pid];
        pids.extend(descendants(pid, &procs, &owners, &claimed, own));
        let ws: u64 = pids.iter().filter_map(|q| procs.info.get(q)).map(|q| q.ws).sum();
        if ws >= HIDDEN_MIN_RAM {
            cand.push((pid, pids));
        }
    }
    // ścieżki tylko dla nowych procesów; nic z katalogu Windows i nic bez dostępu
    let need: Vec<u32> = cand.iter().map(|c| c.0).filter(|p| !e.paths.contains_key(&(*p, procs.info[p].start))).collect();
    if !need.is_empty() {
        for (p, path) in need.iter().zip(w.paths(&need)) {
            e.paths.insert((*p, procs.info[p].start), path);
        }
    }
    let win_dir = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()).to_ascii_lowercase();
    for (pid, pids) in cand {
        let path = e.paths.get(&(pid, procs.info[&pid].start)).cloned().unwrap_or_default();
        if path.is_empty() || path.to_ascii_lowercase().starts_with(&win_dir) || pids.iter().any(|p| claimed.contains(p)) {
            continue;
        }
        claimed.extend(pids.iter().copied());
        groups.push(Group { root: pid, visible: false, hidden: true, exe: path, hwnd: 0, pids });
    }
    let live: HashSet<(u32, u64)> = procs.info.iter().map(|(p, i)| (*p, i.start)).collect();
    e.paths.retain(|k, _| live.contains(k));

    for s in e.apps.values_mut() {
        s.seen = false;
    }
    let dt_100ns = (dt as f64 * 1e7).max(1.0);
    let cores_f = e.cores as f64;
    let now = Instant::now();
    for g in groups {
        let st = e.apps.entry(g.root).or_insert_with(|| AppState {
            exe: g.exe.clone(),
            hwnd: g.hwnd,
            active_secs: 0.0,
            idle_since: None,
            level: 0,
            ws_before: 0,
            ws_now: 0,
            cpu_now: 0.0,
            avg_cpu: 0.0,
            last_audible: None,
            blocked: false,
            hidden: g.hidden,
            visible: g.visible,
            last_trim: None,
            ws_after_trim: 0,
            members: HashMap::new(),
            seen: true,
        });
        st.seen = true;
        st.hwnd = g.hwnd;
        st.hidden = g.hidden;
        st.visible = g.visible;
        let mut ws_sum = 0u64;
        let mut d_cpu = 0u64;
        for &p in &g.pids {
            let Some(pi) = procs.info.get(&p) else { continue };
            ws_sum += pi.ws;
            let m = st.members.entry(p).or_insert(Member { last_cpu100: pi.cpu, level: 0 });
            d_cpu += pi.cpu.saturating_sub(m.last_cpu100);
            m.last_cpu100 = pi.cpu;
        }
        let alive: HashSet<u32> = g.pids.iter().copied().collect();
        st.members.retain(|p, _| alive.contains(p));
        st.ws_now = ws_sum;
        st.cpu_now = (d_cpu as f64 / dt_100ns / cores_f).clamp(0.0, 1.0) as f32;
        if st.level == 0 {
            st.avg_cpu = st.avg_cpu * 0.8 + st.cpu_now * 0.2;
        }
        if st.members.keys().any(|p| audible.contains(p)) {
            st.last_audible = Some(now);
        }
        let audio = st.last_audible.map(|t| now.duration_since(t) < AUDIO_HOLD).unwrap_or(false);

        let active = g.root == fg;
        let forced = e.forced.contains(&g.root);
        let mut target = if active {
            st.active_secs += dt;
            st.idle_since = None;
            0
        } else if forced {
            // „Ogranicz teraz": od razu i mocno, aż wróci na pierwszy plan
            2
        } else {
            let idle = st.idle_since.get_or_insert(now).elapsed().as_secs_f32();
            if g.visible {
                // widoczna, ale przykryta: tylko lekko i dopiero po dłuższej chwili
                let budget = (st.active_secs * COVERED_FRACTION).max(COVERED_MIN_S);
                (idle >= budget) as u8
            } else {
                let budget = (st.active_secs * STEP_FRACTION).max(MIN_BUDGET_S);
                ((idle / budget).floor() as u8).min(MAX_LEVEL)
            }
        };
        if audio && !forced {
            // słychać ją - bez bezczynnego priorytetu i oczyszczania RAM (dźwięk by się ciął)
            target = target.min(1);
        }
        if active {
            e.forced.remove(&g.root);
            st.blocked = false;
        }
        if target != st.level {
            set_level(w, st, target);
        } else if st.level > 0 && !st.blocked {
            // nowe procesy grupy (np. nowa karta) dostają ten sam stopień
            let lvl = st.level;
            let fresh: Vec<u32> = st.members.iter().filter(|(_, m)| m.level != lvl).map(|(&p, _)| p).collect();
            if !fresh.is_empty() {
                w.level_many(lvl.min(2), &fresh);
                for p in fresh {
                    if let Some(m) = st.members.get_mut(&p) {
                        m.level = lvl;
                    }
                }
            }
            // RAM odrasta w tle - przy najwyższym stopniu oczyść ponownie, najwyżej co 2 min
            if lvl >= MAX_LEVEL
                && st.last_trim.map(|t| t.elapsed().as_secs() >= 120).unwrap_or(true)
                && st.ws_now > st.ws_after_trim + 64 * 1024 * 1024
            {
                let all: Vec<u32> = st.members.keys().copied().collect();
                w.level_many(2, &all);
                st.last_trim = Some(now);
                st.ws_after_trim = st.ws_now / 2;
            }
        }
    }
    // zamknięte / zniknięte aplikacje: C przywraca ich procesy (te, które jeszcze żyją)
    let gone: Vec<u32> = e.apps.values().filter(|s| !s.seen && s.level > 0).flat_map(|s| s.members.keys().copied()).collect();
    if !gone.is_empty() {
        w.level_many(0, &gone);
    }
    e.apps.retain(|_, s| s.seen);
    let alive: HashSet<u32> = e.apps.keys().copied().collect();
    e.forced.retain(|p| alive.contains(p));

    let mut saved = 0u64;
    let mut count = 0u32;
    let mut would = 0.0f32;
    let mut usage: Vec<AppUsage> = Vec::new();
    for (&pid, s) in e.apps.iter() {
        let limited = s.level > 0 && !s.blocked;
        if limited {
            saved += s.ws_before.saturating_sub(s.ws_now);
            count += 1;
            would += s.avg_cpu;
        }
        let audio = s.last_audible.map(|t| now.duration_since(t) < AUDIO_HOLD).unwrap_or(false);
        // za ile sekund następny stopień (tylko zminimalizowane / bez okna)
        let next_in = match (s.idle_since, s.level < MAX_LEVEL && !s.blocked && !audio) {
            (Some(t), true) if pid != fg && !s.visible => {
                let budget = (s.active_secs * STEP_FRACTION).max(MIN_BUDGET_S);
                ((s.level as f32 + 1.0) * budget - t.elapsed().as_secs_f32()).max(0.0) as u32
            }
            _ => 0,
        };
        // pokazujemy w zakładce to, co realnie coś waży lub jest ograniczone
        if s.level > 0 || s.ws_now > 48 * 1024 * 1024 || s.cpu_now > 0.01 || s.avg_cpu > 0.01 {
            usage.push(AppUsage {
                pid,
                hwnd: s.hwnd,
                exe: s.exe.clone(),
                ws: s.ws_now,
                cpu: s.cpu_now,
                level: s.level,
                procs: s.members.len() as u32,
                audio,
                blocked: s.blocked,
                hidden: s.hidden,
                next_in,
            });
        }
    }
    usage.sort_by(|a, b| (b.ws + (b.cpu * 4e9) as u64).cmp(&(a.ws + (a.cpu * 4e9) as u64)));
    usage.truncate(60);
    e.usage = usage;
    e.last = Savings { saved_ram: saved, count, would_cpu: would };
    write_state(e);
    e.last
}

/// Ustawia stopień całej grupie i wykonuje różnicę przez osobny program C.
fn set_level(w: &mut Worker, st: &mut AppState, level: u8) {
    if level == st.level {
        return;
    }
    let pids: Vec<u32> = st.members.keys().copied().collect();
    if st.level == 0 && level > 0 {
        st.ws_before = st.ws_now;
    }
    // stopień 3 = ponowne oczyszczenie RAM (2 już ustawił priorytety)
    let (_, ok) = w.level_many(level.min(2), &pids);
    if level > 0 {
        // system odmówił wszystkim procesom (anty-cheat / podniesione uprawnienia)
        st.blocked = ok == 0;
    } else {
        st.blocked = false;
    }
    if level >= 2 {
        st.last_trim = Some(Instant::now());
        st.ws_after_trim = 0;
    }
    for m in st.members.values_mut() {
        m.level = level;
    }
    if level == 0 {
        st.ws_before = 0;
        st.last_trim = None;
    }
    st.level = level;
}

fn restore_all(e: &mut Engine) {
    if let Some(w) = e.worker.as_mut() {
        let pids: Vec<u32> = e.apps.values().filter(|s| s.level > 0).flat_map(|s| s.members.keys().copied()).collect();
        w.level_many(0, &pids);
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
            set_level(w, st, 2);
        }
    }
}

/// Plik stanu dla panelu C# (%LOCALAPPDATA%\Indeks\optymalizator.stan).
/// V<on> <saved> <count> <would_cpu_promile>
/// A<pid> <exe> <ws> <cpu_promile> <level> <procs> <audio> <blocked> <hidden> <next_in>
fn write_state(e: &Engine) {
    let dir = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Indeks");
    let file = dir.join("optymalizator.stan");
    let mut s = String::new();
    let on = e.worker.is_some();
    s.push_str(&format!("V\t{}\t{}\t{}\t{}\n", on as u8, e.last.saved_ram, e.last.count, (e.last.would_cpu * 1000.0) as u32));
    for a in &e.usage {
        let name = a.exe.rsplit(['\\', '/']).next().unwrap_or(&a.exe);
        s.push_str(&format!(
            "A\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            a.pid,
            name,
            a.ws,
            (a.cpu * 1000.0) as u32,
            a.level,
            a.procs,
            a.audio as u8,
            a.blocked as u8,
            a.hidden as u8,
            a.next_in
        ));
    }
    let tmp = file.with_extension("stan.tmp");
    if std::fs::write(&tmp, s).is_ok() {
        let _ = std::fs::rename(&tmp, &file);
    }
}
