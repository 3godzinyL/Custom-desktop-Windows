// Nakladka nad oknem na pelnym ekranie (np. gra): CPU / GPU / RAM i dyski,
// polprzezroczyste. Okno warstwowe, zawsze na wierzchu, przepuszcza mysz
// (WS_EX_TRANSPARENT) - klik w grze w to miejsce trafia do gry, nie do pulpitu.
use windows::core::{w, Result};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Factory, ID2D1RenderTarget, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteTextFormat, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL, DWRITE_TEXT_METRICS,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetMonitorInfoW, MonitorFromWindow, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::Config;
use crate::page::StatusLine;
use crate::stats;

pub struct Overlay {
    hwnd: HWND,
    shown: bool,
    mono: Option<(IDWriteTextFormat, IDWriteTextFormat)>,
}

fn col(c: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: ((c >> 16) & 0xFF) as f32 / 255.0, g: ((c >> 8) & 0xFF) as f32 / 255.0, b: (c & 0xFF) as f32 / 255.0, a }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }
}

unsafe extern "system" fn proc_(h: HWND, m: u32, w: windows::Win32::Foundation::WPARAM, l: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    // przezroczysta dla myszy takze na poziomie trafienia
    if m == WM_NCHITTEST {
        return windows::Win32::Foundation::LRESULT(-1); // HTTRANSPARENT
    }
    DefWindowProcW(h, m, w, l)
}

impl Overlay {
    pub fn new() -> Option<Overlay> {
        unsafe {
            let hinst = GetModuleHandleW(None).ok()?;
            let wc = WNDCLASSW { lpfnWndProc: Some(proc_), hInstance: hinst.into(), lpszClassName: w!("IndeksNakladka"), ..Default::default() };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("IndeksNakladka"),
                w!(""),
                WS_POPUP,
                0, 0, 1, 1,
                None,
                None,
                Some(hinst.into()),
                None,
            )
            .ok()?;
            Some(Overlay { hwnd, shown: false, mono: None })
        }
    }

    pub fn hide(&mut self) {
        if self.shown {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.shown = false;
        }
    }

    /// Rysuje nakladke w lewym dolnym rogu monitora okna `over`.
    pub fn show(&mut self, over: HWND, work: bool, st: &StatusLine, cfg: &Config, d2d: &ID2D1Factory, dw: &IDWriteFactory) -> Result<()> {
        unsafe {
            let mon = MonitorFromWindow(over, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let s = dx as f32 / 96.0;
            if self.mono.is_none() {
                let mk = |wt| dw.CreateTextFormat(w!("Courier New"), None, wt, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 14.0, w!("pl-PL"));
                self.mono = Some((mk(DWRITE_FONT_WEIGHT_NORMAL)?, mk(DWRITE_FONT_WEIGHT_BOLD)?));
            }
            let (mono, bold) = self.mono.clone().unwrap();
            let a1 = cfg.overlay_alpha as f32 / 100.0;
            let a2 = cfg.disks_alpha as f32 / 100.0;
            let pct = |f: f32| format!("{:>3}%", (f * 100.0).round() as u32);
            let ram = st.ram_used as f32 / st.ram_total.max(1) as f32;
            // (etykieta, udzial, opis, widocznosc)
            let mut items: Vec<(String, f32, String, f32)> = vec![
                ("CPU".into(), st.cpu, pct(st.cpu), a1),
                ("GPU".into(), st.gpu, pct(st.gpu), a1),
                ("RAM".into(), ram, format!("{} {}/{}", pct(ram), stats::gb(st.ram_used), stats::gb(st.ram_total)), a1),
            ];
            if cfg.overlay_disks && a2 > 0.0 {
                for (root, total, free) in &st.drives {
                    let used = 1.0 - *free as f32 / (*total).max(1) as f32;
                    items.push((root.trim_end_matches('\\').to_string(), used, format!("{} {} wolne", pct(used), stats::gb(*free)), a2));
                }
            }
            items.retain(|i| i.3 > 0.0);
            if items.is_empty() {
                self.hide();
                return Ok(());
            }
            // pomiar
            let measure = |t: &str, f: &IDWriteTextFormat| -> f32 {
                let w: Vec<u16> = t.encode_utf16().collect();
                dw.CreateTextLayout(&w, f, 4000.0, 100.0)
                    .ok()
                    .and_then(|l| {
                        let mut m = DWRITE_TEXT_METRICS::default();
                        l.GetMetrics(&mut m).ok().map(|_| m.widthIncludingTrailingWhitespace)
                    })
                    .unwrap_or(t.len() as f32 * 8.0)
            };
            let seg_w = 10.0 * 8.0;
            let widths: Vec<f32> = items.iter().map(|i| measure(&i.0, &bold) + 6.0 + seg_w + 6.0 + measure(&i.2, &mono) + 18.0).collect();
            let wd = widths.iter().sum::<f32>() + 12.0;
            let hd = 30.0;
            let (wp, hp) = ((wd * s).ceil() as i32, (hd * s).ceil() as i32);
            // bitmapa 32-bit z kanalem alfa
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: wp,
                    biHeight: -hp,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mem = CreateCompatibleDC(None);
            let mut bits = std::ptr::null_mut();
            let dib = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let old = SelectObject(mem, dib.into());
            let result = (|| -> Result<()> {
                let props = D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                    dpiX: dx as f32,
                    dpiY: dy as f32,
                    ..Default::default()
                };
                let rt = d2d.CreateDCRenderTarget(&props)?;
                rt.BindDC(mem, &RECT { left: 0, top: 0, right: wp, bottom: hp })?;
                let rtb: &ID2D1RenderTarget = &rt;
                rtb.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                rtb.BeginDraw();
                rtb.Clear(Some(&col(0, 0.0)));
                let mut x = 6.0;
                for (item, iw) in items.iter().zip(&widths) {
                    let (label, frac, info, a) = item;
                    // ciemna podkladka pod kazda grupa (czytelnosc na jasnej grze)
                    let plate = rtb.CreateSolidColorBrush(&col(0x000000, a * 0.55), None)?;
                    rtb.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rect(x - 4.0, 2.0, iw - 6.0, hd - 4.0), radiusX: 3.0, radiusY: 3.0 }, &plate);
                    let ty = (hd - 16.0) / 2.0;
                    let lb = rtb.CreateSolidColorBrush(&col(0x55FFFF, *a), None)?;
                    let t: Vec<u16> = label.encode_utf16().collect();
                    rtb.DrawText(&t, &bold, &rect(x, ty, 60.0, 18.0), &lb, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
                    x += measure(label, &bold) + 6.0;
                    let on = if *frac < 0.75 { 0x55FF55 } else if *frac < 0.9 { 0xFFFF55 } else { 0xFF5555 };
                    let on_b = rtb.CreateSolidColorBrush(&col(on, *a), None)?;
                    let off_b = rtb.CreateSolidColorBrush(&col(0x404040, *a), None)?;
                    let filled = (frac.clamp(0.0, 1.0) * 10.0).round() as usize;
                    for k in 0..10 {
                        rtb.FillRectangle(&rect(x + k as f32 * 8.0, (hd - 13.0) / 2.0, 6.0, 13.0), if k < filled { &on_b } else { &off_b });
                    }
                    x += seg_w + 6.0;
                    let tb = rtb.CreateSolidColorBrush(&col(0xFFFFFF, *a), None)?;
                    let t: Vec<u16> = info.encode_utf16().collect();
                    rtb.DrawText(&t, &mono, &rect(x, ty, 400.0, 18.0), &tb, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
                    x += measure(info, &mono) + 18.0;
                }
                rtb.EndDraw(None, None)?;
                let m = if work { mi.rcWork } else { mi.rcMonitor };
                let pos = POINT { x: m.left + (12.0 * s) as i32, y: m.bottom - hp - (12.0 * s) as i32 };
                let size = SIZE { cx: wp, cy: hp };
                let src = POINT { x: 0, y: 0 };
                let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
                UpdateLayeredWindow(self.hwnd, None, Some(&pos), Some(&size), Some(mem), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA)?;
                Ok(())
            })();
            SelectObject(mem, old);
            let _ = DeleteObject(dib.into());
            let _ = DeleteDC(mem);
            result?;
            if !self.shown {
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
                self.shown = true;
            }
            Ok(())
        }
    }
}
