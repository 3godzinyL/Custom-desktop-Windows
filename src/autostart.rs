// Autostart przez HKCU\Software\Microsoft\Windows\CurrentVersion\Run (tylko
// biezacy uzytkownik, bez uprawnien administratora, w pelni odwracalne).
use windows::core::w;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteKeyValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_SZ,
};

use crate::util::{self, pcwstr, wide};

const RUN: windows::core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: windows::core::PCWSTR = w!("Indeks");

pub fn enabled() -> bool {
    let mut size = 0u32;
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, NAME, RRF_RT_REG_SZ, None, None, Some(&mut size)) }.is_ok()
}

pub fn set(on: bool) {
    unsafe {
        if on {
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
            let value = wide(&format!("\"{}\"", exe));
            let mut key = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, RUN, None, KEY_SET_VALUE, &mut key).is_ok() {
                let bytes = std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * 2);
                let _ = RegSetValueExW(key, NAME, None, REG_SZ, Some(bytes));
                let _ = RegCloseKey(key);
                util::log(&format!("autostart: wlaczony ({})", exe));
            }
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME);
            util::log("autostart: wylaczony");
        }
    }
    let _ = pcwstr;
}
