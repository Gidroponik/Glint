//! Захват экрана через GDI: пиксель-в-пиксель, без масштабирования и потерь.

use std::ffi::c_void;
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW, HMONITOR,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// Снимок прямоугольника экрана. Пиксели в BGRA (альфа = 255), сверху вниз.
pub struct Shot {
    pub w: u32,
    pub h: u32,
    pub scale: f32,
    pub bgra: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f32,
}

/// Монитор, на котором сейчас находится курсор.
pub fn monitor_under_cursor() -> MonitorRect {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    monitor_at(pt.x, pt.y)
}

/// Монитор, на который приходится точка экрана (физические пиксели).
pub fn monitor_at(x: i32, y: i32) -> MonitorRect {
    let mon = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    monitor_rect(mon)
}

/// Все мониторы рабочего стола.
pub fn all_monitors() -> Vec<MonitorRect> {
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC};
    use windows::core::BOOL;

    unsafe extern "system" fn each(mon: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
        unsafe {
            (*(lp.0 as *mut Vec<MonitorRect>)).push(monitor_rect(mon));
        }
        true.into()
    }
    let mut out: Vec<MonitorRect> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

fn monitor_rect(mon: HMONITOR) -> MonitorRect {
    unsafe {
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(mon, &mut info);
        let r: RECT = info.rcMonitor;
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        MonitorRect {
            x: r.left,
            y: r.top,
            w: (r.right - r.left).max(1) as u32,
            h: (r.bottom - r.top).max(1) as u32,
            scale: dx as f32 / 96.0,
        }
    }
}

pub fn capture_monitor(m: MonitorRect) -> Option<Shot> {
    let bgra = capture_rect(m.x, m.y, m.w, m.h)?;
    Some(Shot {
        w: m.w,
        h: m.h,
        scale: m.scale,
        bgra,
    })
}

fn capture_rect(x: i32, y: i32, w: u32, h: u32) -> Option<Vec<u8>> {
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let mem = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let Ok(bmp) = CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            return None;
        };
        let old = SelectObject(mem, bmp.into());
        let ok = BitBlt(mem, 0, 0, w as i32, h as i32, Some(screen), x, y, SRCCOPY | CAPTUREBLT);

        let mut out = None;
        if ok.is_ok() && !bits.is_null() {
            let len = (w as usize) * (h as usize) * 4;
            let src = std::slice::from_raw_parts(bits as *const u8, len);
            let mut buf = src.to_vec();
            for px in buf.chunks_exact_mut(4) {
                px[3] = 255;
            }
            out = Some(buf);
        }
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        out
    }
}
