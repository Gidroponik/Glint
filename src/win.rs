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
    make_popup_as(hwnd, x, y, w, h, activate, activate);
}

/// То же, но отдельно задаётся, активировать ли окно сразу (`activate`) и может ли оно
/// вообще получать фокус (`focusable`): окно можно показать без фокуса, а активировать позже.
pub fn make_popup_as(hwnd: HWND, x: i32, y: i32, w: i32, h: i32, activate: bool, focusable: bool) {
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_STYLE, WS_POPUP.0 as isize);
        let mut ex = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP;
        if !focusable {
            ex |= WS_EX_NOACTIVATE;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex.0 as isize);
        let mut flags = SWP_FRAMECHANGED | SWP_SHOWWINDOW;
        if !activate {
            flags |= SWP_NOACTIVATE;
        }
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, flags);
        settle(hwnd, x, y, w, h);
        if activate {
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// Делает окно активным (ввод с клавиатуры пойдёт в него).
pub fn activate(hwnd: HWND) {
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Прямоугольники видимых окон верхнего уровня (физические пиксели: left, top, right, bottom),
/// от верхнего к нижнему. Собственные окна, рабочий стол и скрытые окна пропускаются.
pub fn window_rects() -> Vec<(i32, i32, i32, i32)> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Dwm::{
        DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
    };
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId,
        IsIconic, IsWindowVisible, WS_EX_TRANSPARENT,
    };
    use windows::core::BOOL;

    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
        unsafe {
            let out = &mut *(lp.0 as *mut Vec<(i32, i32, i32, i32)>);
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                return true.into();
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == GetCurrentProcessId() {
                return true.into();
            }
            if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TRANSPARENT.0 != 0 {
                return true.into();
            }
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut c_void, 4);
            if cloaked != 0 {
                return true.into();
            }
            let mut class = [0u16; 64];
            let n = GetClassNameW(hwnd, &mut class).max(0) as usize;
            let class = String::from_utf16_lossy(&class[..n]);
            if matches!(class.as_str(), "Progman" | "WorkerW") {
                return true.into();
            }
            let mut r = RECT::default();
            let framed = DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut r as *mut _ as *mut c_void,
                size_of::<RECT>() as u32,
            );
            if framed.is_err() && GetWindowRect(hwnd, &mut r).is_err() {
                return true.into();
            }
            out.push((r.left, r.top, r.right, r.bottom));
            true.into()
        }
    }

    let mut out: Vec<(i32, i32, i32, i32)> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// Левый верхний угол окна на экране, в физических пикселях.
pub fn origin(hwnd: HWND) -> Option<(i32, i32)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r).ok()? };
    Some((r.left, r.top))
}

/// Есть ли у приложения видимые окна (оверлей, закреплённые снимки, окно настроек).
pub fn has_visible_window() -> bool {
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
    };
    use windows::core::BOOL;

    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == GetCurrentProcessId() && IsWindowVisible(hwnd).as_bool() {
                *(lp.0 as *mut bool) = true;
                return false.into();
            }
            true.into()
        }
    }
    let mut found = false;
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut found as *mut _ as isize));
    }
    found
}

/// Отдаёт системе физическую память, которой процесс сейчас не пользуется.
/// Нужные страницы подгрузятся обратно при первом обращении.
pub fn trim_memory() {
    use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

/// Прямоугольник окна на экране (left, top, right, bottom), в физических пикселях.
pub fn rect(hwnd: HWND) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r).ok()? };
    Some((r.left, r.top, r.right, r.bottom))
}

/// Положение курсора на экране, в физических пикселях.
pub fn cursor() -> (i32, i32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    (pt.x, pt.y)
}

/// Зажата ли основная кнопка мыши (с учётом настройки «поменять кнопки местами»).
pub fn primary_button_down() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};
    let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
    vk_down(if swapped { 0x02 } else { 0x01 })
}

/// Ставит окно в заданный прямоугольник (физические пиксели), не меняя порядок окон.
pub fn place(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    use windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER;
    unsafe {
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    settle(hwnd, x, y, w, h);
}

/// Добивается, чтобы окно действительно заняло заданный прямоугольник.
/// Когда окно переезжает на монитор с другим масштабом, система присылает WM_DPICHANGED
/// с «предложенным» размером, и GPUI тут же применяет его поверх нашего. После переезда
/// масштаб уже не меняется, поэтому повторная установка размера срабатывает как надо.
fn settle(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    use windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER;
    for _ in 0..3 {
        if rect(hwnd) == Some((x, y, x + w, y + h)) {
            return;
        }
        unsafe {
            let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}

/// Меняет размер окна (физические пиксели), не трогая его положение и порядок.
pub fn resize(hwnd: HWND, w: i32, h: i32) {
    use windows::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOZORDER};
    unsafe {
        let _ = SetWindowPos(hwnd, None, 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

pub fn vk_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

pub fn caps_lock_on() -> bool {
    unsafe { (GetKeyState(0x14) & 1) != 0 }
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
        settle(hwnd, x, y, w, h);
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
