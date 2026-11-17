//! Глобальные горячие клавиши и значок в трее.
//! Живёт в отдельном потоке со своим циклом сообщений Win32 — не зависит от окон GPUI.

use crate::settings::Hotkey;
use async_channel::Sender;
use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_LARGE_ICON, NIIF_NOSOUND, NIIF_USER, NIIF_WARNING,
    NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DispatchMessageW, GetCursorPos, GetMessageW, HICON, MF_SEPARATOR, MF_STRING, MSG,
    PostThreadMessageW, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_HOTKEY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

#[derive(Debug, Clone, Copy)]
pub enum Event {
    Region,
    Full,
    ShowMain,
    Quit,
    /// Что-то изменилось снаружи (например, автозапуск из меню трея) — перерисовать окна.
    Refresh,
    /// Результат регистрации хоткеев: (область, весь экран).
    Status(bool, bool),
}

const WM_TRAY: u32 = WM_APP + 1;
const WM_REBIND: u32 = WM_APP + 2;
const WM_SHUTDOWN: u32 = WM_APP + 3;
const WM_NOTIFY_USER: u32 = WM_APP + 4;
const WM_SHOW_MAIN: u32 = WM_APP + 5;
const TRAY_CLASS: windows::core::PCWSTR = w!("GlintTrayWindow");
const ID_REGION: i32 = 1;
const ID_FULL: i32 = 2;

static SENDER: OnceLock<Sender<Event>> = OnceLock::new();
static PENDING: Mutex<(Option<Hotkey>, Option<Hotkey>)> = Mutex::new((None, None));
static THREAD_ID: OnceLock<u32> = OnceLock::new();
static NOTICE: Mutex<Option<(String, String, bool)>> = Mutex::new(None);
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

fn send(ev: Event) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.try_send(ev);
    }
}

/// Если приложение уже запущено — просит тот экземпляр показать окно и возвращает true.
pub fn already_running() -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
    unsafe {
        // Мьютекс намеренно не закрываем: он живёт до конца процесса.
        let _ = CreateMutexW(None, false, w!("GlintSingleInstanceMutex"));
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return false;
        }
        if let Ok(hwnd) = FindWindowW(TRAY_CLASS, None) {
            let _ = PostMessageW(Some(hwnd), WM_SHOW_MAIN, WPARAM(0), LPARAM(0));
        }
        true
    }
}

/// Запускает поток хоткеев/трея.
pub fn start(tx: Sender<Event>, region: Hotkey, full: Hotkey) {
    let _ = SENDER.set(tx);
    *PENDING.lock().unwrap() = (Some(region), Some(full));
    std::thread::Builder::new()
        .name("glint-hotkeys".into())
        .spawn(run)
        .expect("hotkey thread");
}

/// Перерегистрирует хоткеи (None — временно отключить).
pub fn rebind(region: Option<Hotkey>, full: Option<Hotkey>) {
    *PENDING.lock().unwrap() = (region, full);
    if let Some(tid) = THREAD_ID.get() {
        unsafe {
            let _ = PostThreadMessageW(*tid, WM_REBIND, WPARAM(0), LPARAM(0));
        }
    }
}

/// Показывает системное уведомление от значка в трее.
pub fn notify(title: String, text: String, ok: bool) {
    *NOTICE.lock().unwrap() = Some((title, text, ok));
    if let Some(tid) = THREAD_ID.get() {
        unsafe {
            let _ = PostThreadMessageW(*tid, WM_NOTIFY_USER, WPARAM(0), LPARAM(0));
        }
    }
}

fn show_notice(hwnd: HWND) {
    let Some((title, text, ok)) = NOTICE.lock().unwrap().take() else {
        return;
    };
    let mut nid = tray_data(hwnd);
    nid.uFlags = NIF_INFO;
    if ok {
        nid.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON | NIIF_NOSOUND;
        if let Some(icon) = ICON.get() {
            nid.hBalloonIcon = HICON(*icon as *mut _);
        }
    } else {
        nid.dwInfoFlags = NIIF_WARNING | NIIF_NOSOUND;
    }
    copy_wide(&mut nid.szInfoTitle, &title);
    copy_wide(&mut nid.szInfo, &text);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

fn copy_wide(dst: &mut [u16], src: &str) {
    let wide: Vec<u16> = src.encode_utf16().take(dst.len() - 1).collect();
    dst[..wide.len()].copy_from_slice(&wide);
    dst[wide.len()] = 0;
}

/// Убирает значок из трея перед выходом.
pub fn shutdown() {
    if let Some(tid) = THREAD_ID.get() {
        unsafe {
            let _ = PostThreadMessageW(*tid, WM_SHUTDOWN, WPARAM(0), LPARAM(0));
        }
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
}

fn register(id: i32, hk: Option<Hotkey>) -> bool {
    unsafe {
        let _ = UnregisterHotKey(None, id);
        let Some(hk) = hk else { return false };
        let mut m = MOD_NOREPEAT;
        if hk.ctrl {
            m |= MOD_CONTROL;
        }
        if hk.alt {
            m |= MOD_ALT;
        }
        if hk.shift {
            m |= MOD_SHIFT;
        }
        if hk.win {
            m |= MOD_WIN;
        }
        RegisterHotKey(None, id, HOT_KEY_MODIFIERS(m.0), hk.vk).is_ok()
    }
}

fn apply_bindings() {
    let (r, f) = *PENDING.lock().unwrap();
    let ok_r = register(ID_REGION, r);
    let ok_f = register(ID_FULL, f);
    send(Event::Status(ok_r, ok_f));
}

fn run() {
    unsafe {
        let _ = THREAD_ID.set(GetCurrentThreadId());
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let class = TRAY_CLASS;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let _ = TASKBAR_CREATED.set(RegisterWindowMessageW(w!("TaskbarCreated")));
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Glint"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .unwrap_or_default();

        let icon = make_icon();
        add_tray(hwnd, icon);
        apply_bindings();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            match msg.message {
                WM_HOTKEY if msg.hwnd.0.is_null() => match msg.wParam.0 as i32 {
                    ID_REGION => send(Event::Region),
                    ID_FULL => send(Event::Full),
                    _ => {}
                },
                WM_REBIND if msg.hwnd.0.is_null() => apply_bindings(),
                WM_NOTIFY_USER if msg.hwnd.0.is_null() => show_notice(hwnd),
                WM_SHUTDOWN if msg.hwnd.0.is_null() => break,
                _ => {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        remove_tray(hwnd);
    }
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    }
}

static ICON: OnceLock<isize> = OnceLock::new();

fn add_tray(hwnd: HWND, icon: HICON) {
    let _ = ICON.set(icon.0 as isize);
    let mut nid = tray_data(hwnd);
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = icon;
    let tip: Vec<u16> = "Glint".encode_utf16().collect();
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

fn remove_tray(hwnd: HWND) {
    let nid = tray_data(hwnd);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn make_icon() -> HICON {
    let n = 32usize;
    let rgba = crate::icon::icon_rgba(n);
    let mut bgra = rgba.clone();
    for p in bgra.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    let and_mask = vec![0u8; n * n / 8];
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        CreateIcon(
            Some(hinst.into()),
            n as i32,
            n as i32,
            1,
            32,
            and_mask.as_ptr(),
            bgra.as_ptr(),
        )
        .unwrap_or_default()
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_TRAY {
            match lparam.0 as u32 {
                WM_LBUTTONUP => send(Event::ShowMain),
                WM_RBUTTONUP => show_menu(hwnd),
                _ => {}
            }
            return LRESULT(0);
        }
        if msg == WM_SHOW_MAIN {
            send(Event::ShowMain);
            return LRESULT(0);
        }
        if Some(&msg) == TASKBAR_CREATED.get() {
            if let Some(icon) = ICON.get() {
                add_tray(hwnd, HICON(*icon as *mut _));
            }
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

unsafe fn show_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let item = |id: usize, text: &str| {
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(wide.as_ptr()));
        };
        item(1, "Capture region");
        item(2, "Capture full screen");
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        item(3, "Open Glint");
        let autostart = crate::autostart::enabled();
        {
            use windows::Win32::UI::WindowsAndMessaging::{MF_CHECKED, MF_UNCHECKED};
            let wide: Vec<u16> = "Start with Windows".encode_utf16().chain(Some(0)).collect();
            let flag = if autostart { MF_CHECKED } else { MF_UNCHECKED };
            let _ = AppendMenuW(menu, MF_STRING | flag, 5, PCWSTR(wide.as_ptr()));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        item(4, "Exit");
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
        match cmd.0 {
            1 => send(Event::Region),
            2 => send(Event::Full),
            3 => send(Event::ShowMain),
            4 => send(Event::Quit),
            5 => {
                crate::autostart::set(!autostart);
                send(Event::Refresh);
            }
            _ => {}
        }
    }
}
