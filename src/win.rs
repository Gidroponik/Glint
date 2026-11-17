//! Мелкие Win32-помощники: стиль окон оверлея и состояние клавиш.

use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyState};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
    WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

pub fn enable_dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn hwnd_of(window: &Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}

/// Превращает окно в безрамочное поверх всех окон в заданном прямоугольнике (физические пиксели).
pub fn make_popup(hwnd: HWND, x: i32, y: i32, w: i32, h: i32, activate: bool) {
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_STYLE, WS_POPUP.0 as isize);
        let mut ex = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP;
        if !activate {
            ex |= WS_EX_NOACTIVATE;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex.0 as isize);
        let mut flags = SWP_FRAMECHANGED | SWP_SHOWWINDOW;
        if !activate {
            flags |= SWP_NOACTIVATE;
        }
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, flags);
        if activate {
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

pub fn vk_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

pub fn caps_lock_on() -> bool {
    unsafe { (GetKeyState(0x14) & 1) != 0 }
}

/// Первая нажатая клавиша, не являющаяся модификатором (для записи хоткея).
pub fn pressed_key() -> Option<u32> {
    (0x08u32..=0xDE).find(|&vk| {
        !matches!(vk, 0x10..=0x12 | 0x14 | 0x5B | 0x5C | 0xA0..=0xA5) && vk_down(vk as i32)
    })
}

/// Делает главное окно безрамочным (без системного заголовка), сохраняя его в панели задач.
pub fn make_frameless(hwnd: HWND, x: i32, y: i32, w: i32, h: i32, visible: bool, activate: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{SWP_NOZORDER, WS_MINIMIZEBOX, WS_SYSMENU};
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_STYLE, (WS_POPUP | WS_MINIMIZEBOX | WS_SYSMENU).0 as isize);
        let mut flags = SWP_FRAMECHANGED | SWP_NOZORDER;
        if visible {
            flags |= SWP_SHOWWINDOW;
        }
        if !activate {
            flags |= SWP_NOACTIVATE;
        }
        let _ = SetWindowPos(hwnd, None, x, y, w, h, flags);
        if visible && activate {
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// Начинает системное перетаскивание окна (как за заголовок).
pub fn begin_drag(hwnd: HWND) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows::Win32::UI::WindowsAndMessaging::{HTCAPTION, PostMessageW, WM_NCLBUTTONDOWN};
    unsafe {
        let _ = ReleaseCapture();
        let _ = PostMessageW(Some(hwnd), WM_NCLBUTTONDOWN, WPARAM(HTCAPTION as usize), LPARAM(0));
    }
}
