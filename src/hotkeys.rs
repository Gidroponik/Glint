//! Глобальные горячие клавиши и значок в трее.
//! Живёт в отдельном потоке со своим циклом сообщений Win32 — не зависит от окон GPUI.
//!
//! Обычное сочетание регистрируется через `RegisterHotKey`. Если оно занято системой
//! (Print Screen, Win+Shift+S) или другой программой, клавиши перехватывает низкоуровневый
//! хук: пока сочетание назначено в Glint, до остальных оно не доходит. Хук ставится только
//! когда нужен и ничего не меняет в настройках Windows — стоит сменить сочетание или
//! закрыть Glint, и системное поведение возвращается само.

use crate::settings::Hotkey;
use async_channel::Sender;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, MOD_ALT,
    MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, SendInput, UnregisterHotKey,
    VIRTUAL_KEY,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_LARGE_ICON, NIIF_NOSOUND, NIIF_USER, NIIF_WARNING,
    NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallNextHookEx, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyMenu, DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW,
    GetWindowThreadProcessId, HHOOK, HICON, KBDLLHOOKSTRUCT, MF_SEPARATOR, MF_STRING, MSG,
    PostThreadMessageW, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow,
    SetWindowsHookExW, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_HOTKEY,
    WM_KEYDOWN, WM_LBUTTONUP, WM_RBUTTONUP, WM_SYSKEYDOWN, WNDCLASSW,
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
    Status(Bound, Bound),
    /// Во время записи нового сочетания нажали клавишу.
    Recorded(Hotkey),
    /// Запись сочетания отменена (Esc или окно потеряло фокус).
    RecordCancel,
}

/// Как назначено сочетание.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    /// Обычный системный хоткей.
    Plain,
    /// Сочетание занято системой или другой программой — Glint перехватывает его первым.
    Override,
    /// Назначить не удалось.
    Failed,
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
/// Идёт запись нового сочетания: хук отдаёт приложению все нажатия.
static RECORDING: AtomicBool = AtomicBool::new(false);
static HOOK: Mutex<Hook> = Mutex::new(Hook { handle: 0, recording: false, binds: [None, None], held: 0 });

/// Состояние перехвата клавиатуры.
struct Hook {
    /// Установленный хук (0 — не стоит).
    handle: isize,
    recording: bool,
    /// Сочетания, которые ловит хук: область и весь экран.
    binds: [Option<Hotkey>; 2],
    /// Проглоченная клавиша: её повторы и отпускание тоже не пропускаем дальше.
    held: u32,
}

const VK_PRINT_SCREEN: u32 = 0x2C;
/// Ничем не занятая клавиша: её «нажатие» не даёт открыться меню «Пуск» или меню окна,
/// когда основную клавишу сочетания с Win или Alt мы проглотили.
const VK_MASK: u16 = 0xE8;
/// Метка собственных искусственных нажатий, чтобы хук их не разбирал.
const OWN_INPUT: usize = 0x476C_696E;
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
    // Отладка: GLINT_HOOKTEST=<код клавиши в hex> — снимок области по этой клавише через хук.
    #[cfg(debug_assertions)]
    let region = std::env::var("GLINT_HOOKTEST")
        .ok()
        .and_then(|v| u32::from_str_radix(&v, 16).ok())
        .map_or(region, |vk| Hotkey { ctrl: false, alt: false, shift: false, win: false, vk });
    *PENDING.lock().unwrap() = (Some(region), Some(full));
    std::thread::Builder::new()
        .name("glint-hotkeys".into())
        .spawn(run)
        .expect("hotkey thread");
}

/// Начинает запись нового сочетания: хоткеи отключаются, каждое нажатие приходит
/// событием `Recorded` и дальше в систему не идёт. Заканчивается вызовом `rebind`.
pub fn record() {
    RECORDING.store(true, Ordering::SeqCst);
    if let Some(tid) = THREAD_ID.get() {
        unsafe {
            let _ = PostThreadMessageW(*tid, WM_REBIND, WPARAM(0), LPARAM(0));
        }
    }
}

/// Назначает хоткеи заново и завершает запись сочетания, если она шла.
pub fn rebind(region: Hotkey, full: Hotkey) {
    RECORDING.store(false, Ordering::SeqCst);
    *PENDING.lock().unwrap() = (Some(region), Some(full));
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
        // Print Screen система обрабатывает сама (буфер обмена, «Ножницы») — только перехват.
        if hk.vk == VK_PRINT_SCREEN || force_hook() {
            return false;
        }
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

/// Отладка: все сочетания идут через хук.
fn force_hook() -> bool {
    cfg!(debug_assertions) && std::env::var_os("GLINT_HOOKTEST").is_some()
}

fn apply_bindings() {
    let recording = RECORDING.load(Ordering::SeqCst);
    let (r, f) = if recording { (None, None) } else { *PENDING.lock().unwrap() };
    // Что не удалось зарегистрировать обычным способом — перехватываем хуком.
    let hooked = [(ID_REGION, r), (ID_FULL, f)].map(|(id, hk)| if register(id, hk) { None } else { hk });
    let need = recording || hooked.iter().any(Option::is_some);
    let installed = {
        let mut h = HOOK.lock().unwrap();
        h.recording = recording;
        h.binds = hooked;
        h.held = 0;
        unsafe {
            if need && h.handle == 0 {
                let hinst = GetModuleHandleW(None).unwrap_or_default();
                if let Ok(hook) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), Some(hinst.into()), 0) {
                    h.handle = hook.0 as isize;
                }
            } else if !need && h.handle != 0 {
                let _ = UnhookWindowsHookEx(HHOOK(h.handle as *mut _));
                h.handle = 0;
            }
        }
        h.handle != 0
    };
    if recording {
        return;
    }
    let state = |hk: Option<Hotkey>| match hk {
        None => Bound::Plain,
        Some(_) if installed => Bound::Override,
        Some(_) => Bound::Failed,
    };
    send(Event::Status(state(hooked[0]), state(hooked[1])));
}

fn remove_hook() {
    let mut h = HOOK.lock().unwrap();
    if h.handle != 0 {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(h.handle as *mut _));
        }
        h.handle = 0;
    }
}

unsafe extern "system" fn key_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code == 0 {
            let k = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            if k.dwExtraInfo != OWN_INPUT && on_hook_key(k.vkCode, down) {
                return LRESULT(1);
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

/// Разбирает нажатие, пришедшее в хук. Возвращает true, если клавишу нужно проглотить.
fn on_hook_key(vk: u32, down: bool) -> bool {
    let mut h = HOOK.lock().unwrap();
    if !down {
        if h.held == vk {
            h.held = 0;
            return true;
        }
        return false;
    }
    // модификаторы и Caps Lock сами по себе сочетанием не считаются
    if matches!(vk, 0x10..=0x12 | 0x14 | 0x5B | 0x5C | 0xA0..=0xA5) {
        return false;
    }
    if h.held == vk {
        return true; // автоповтор
    }
    let pressed = |vk: i32| crate::win::vk_down(vk);
    let hk = Hotkey {
        ctrl: pressed(0x11),
        alt: pressed(0x12),
        shift: pressed(0x10),
        win: pressed(0x5B) || pressed(0x5C),
        vk,
    };
    let ev = if h.recording {
        if !own_window_active() {
            // окно записи потеряло фокус — чужой ввод не трогаем
            drop(h);
            send(Event::RecordCancel);
            return false;
        }
        if vk == 0x1B && !(hk.ctrl || hk.alt || hk.shift || hk.win) {
            Event::RecordCancel
        } else {
            Event::Recorded(hk)
        }
    } else if h.binds[0] == Some(hk) {
        Event::Region
    } else if h.binds[1] == Some(hk) {
        Event::Full
    } else {
        return false;
    };
    h.held = vk;
    drop(h);
    #[cfg(debug_assertions)]
    eprintln!("hook: {ev:?}");
    if hk.win || hk.alt {
        mask_modifier();
    }
    send(ev);
    true
}

/// Активное окно принадлежит нашему процессу.
fn own_window_active() -> bool {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        pid == windows::Win32::System::Threading::GetCurrentProcessId()
    }
}

fn mask_modifier() {
    let key = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(VK_MASK),
                dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() },
                dwExtraInfo: OWN_INPUT,
                ..Default::default()
            },
        },
    };
    unsafe {
        SendInput(&[key(false), key(true)], size_of::<INPUT>() as i32);
    }
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
        remove_hook();
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
