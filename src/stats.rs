// Dane dolnego paska: zajetosc dyskow, CPU, RAM i pamiec samego Indeksu.
// Odswiezane tylko gdy pulpit jest widoczny (patrz win.rs), dyski rzadziej.
use std::time::{Duration, Instant};

use windows::Win32::Foundation::FILETIME;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes, GetSystemTimes};

use crate::util::{pcwstr, wide};

const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOVABLE: u32 = 2;

#[derive(Clone)]
pub struct DriveStat {
    pub root: String,
    pub total: u64,
    pub free: u64,
}

pub struct Stats {
    pub drives: Vec<DriveStat>,
    pub cpu: f32,
    /// Obciazenie GPU (silniki 3D, jak w Menedzerze zadan), 0..1.
    pub gpu: f32,
    pub ram_used: u64,
    pub ram_total: u64,
    pub own: u64,
    /// CPU samego Indeksu (czesc wszystkich rdzeni, jak w Menedzerze zadan), 0..1.
    pub own_cpu: f32,
    prev: Option<(u64, u64, u64)>,
    disks_at: Option<Instant>,
    gpu_q: Option<(isize, isize)>,
}

fn ft(f: &FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

impl Stats {
    pub fn new() -> Stats {
        let mut s = Stats { drives: Vec::new(), cpu: 0.0, gpu: 0.0, ram_used: 0, ram_total: 0, own: 0, own_cpu: 0.0, prev: None, disks_at: None, gpu_q: gpu_query() };
        s.update();
        s
    }

    pub fn update(&mut self) {
        unsafe {
            // CPU: roznica czasow systemu od ostatniego odczytu
            let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
            if GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).is_ok() {
                let total = ft(&kernel) + ft(&user);
                let idle = ft(&idle);
                // czas procesora zuzyty przez Indeks
                let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
                let own = if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u).is_ok() { ft(&k) + ft(&u) } else { 0 };
                if let Some((pt, pi, po)) = self.prev {
                    let dt = total.saturating_sub(pt);
                    if dt > 0 {
                        self.cpu = (1.0 - idle.saturating_sub(pi) as f32 / dt as f32).clamp(0.0, 1.0);
                        self.own_cpu = (own.saturating_sub(po) as f32 / dt as f32).clamp(0.0, 1.0);
                    }
                }
                self.prev = Some((total, idle, own));
            }
            let mut ms = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
            if GlobalMemoryStatusEx(&mut ms).is_ok() {
                self.ram_total = ms.ullTotalPhys;
                self.ram_used = ms.ullTotalPhys - ms.ullAvailPhys;
            }
            let mut pmc = PROCESS_MEMORY_COUNTERS_EX { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
            if GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut _ as *mut PROCESS_MEMORY_COUNTERS, pmc.cb).is_ok() {
                self.own = pmc.PrivateUsage as u64;
            }
        }
        if let Some((q, c)) = self.gpu_q {
            self.gpu = gpu_read(q, c).unwrap_or(self.gpu);
        }
        // dyski co 30 s (to wolniejsze wywolania)
        if self.disks_at.map(|t| t.elapsed() > Duration::from_secs(30)).unwrap_or(true) {
            self.disks_at = Some(Instant::now());
            self.drives = read_drives();
        }
    }
}

fn read_drives() -> Vec<DriveStat> {
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        let w = wide(&root);
        let kind = unsafe { GetDriveTypeW(pcwstr(&w)) };
        if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE {
            continue;
        }
        let (mut free, mut total) = (0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(pcwstr(&w), Some(&mut free), Some(&mut total), None) }.is_ok() && total > 0 {
            out.push(DriveStat { root, total, free });
        }
    }
    out
}

/// "412 GB" - krotko, po polsku (przecinek dziesietny).
pub fn gb(bytes: u64) -> String {
    let g = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    if g >= 100.0 {
        format!("{:.0} GB", g)
    } else {
        format!("{:.1} GB", g).replace('.', ",")
    }
}

pub fn mb(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
}

// ---------- GPU (licznik PDH "GPU Engine", silniki 3D) ----------

/// Kazde okno Indeksu ma wlasne Stats - zapytanie PDH zamykamy razem z oknem.
impl Drop for Stats {
    fn drop(&mut self) {
        if let Some((q, _)) = self.gpu_q.take() {
            unsafe {
                let _ = windows::Win32::System::Performance::PdhCloseQuery(windows::Win32::System::Performance::PDH_HQUERY(q as *mut _));
            }
        }
    }
}

fn gpu_query() -> Option<(isize, isize)> {
    use windows::Win32::System::Performance::{PdhAddEnglishCounterW, PdhCollectQueryData, PdhOpenQueryW};
    unsafe {
        let mut q = Default::default();
        if PdhOpenQueryW(windows::core::PCWSTR::null(), 0, &mut q) != 0 {
            return None;
        }
        let mut c = Default::default();
        if PdhAddEnglishCounterW(q, windows::core::w!("\\GPU Engine(*engtype_3D)\\Utilization Percentage"), 0, &mut c) != 0 {
            return None;
        }
        let _ = PdhCollectQueryData(q);
        Some((q.0 as isize, c.0 as isize))
    }
}

fn gpu_read(q: isize, c: isize) -> Option<f32> {
    use windows::Win32::System::Performance::{
        PdhCollectQueryData, PdhGetFormattedCounterArrayW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
    };
    unsafe {
        let q = PDH_HQUERY(q as *mut _);
        let c = PDH_HCOUNTER(c as *mut _);
        if PdhCollectQueryData(q) != 0 {
            return None;
        }
        let (mut size, mut count) = (0u32, 0u32);
        let _ = PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u64; (size as usize).div_ceil(8)]; // wyrownanie do f64
        let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        if PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
            return None;
        }
        let sum: f64 = std::slice::from_raw_parts(items, count as usize).iter().map(|i| i.FmtValue.Anonymous.doubleValue).sum();
        Some((sum / 100.0).clamp(0.0, 1.0) as f32)
    }
}