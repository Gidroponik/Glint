#![windows_subsystem = "windows"]

mod autostart;
mod capture;
mod export;
mod fonts;
mod home;
mod hotkeys;
mod icon;
mod overlay;
mod settings;
mod shapes;
mod theme;
mod toast;
mod win;

use capture::MonitorRect;
use export::Rendered;
use gpui::prelude::*;
use gpui::{
    App, Application, Bounds, DisplayId, Global, RenderImage, TitlebarOptions, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, point, px, size,
};
use settings::Settings;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Copy,
    Save,
}

pub struct AppState {
    pub settings: Settings,
    /// Удалось ли зарегистрировать хоткеи: (область, весь экран).
    pub hotkeys_ok: (bool, bool),
    pub main: Option<WindowHandle<home::Home>>,
    pub overlay_open: bool,
}

impl Global for AppState {}

fn main() {
    win::enable_dpi_awareness();
    let testing = cfg!(debug_assertions)
        && ["GLINT_SELFTEST", "GLINT_TOASTTEST", "GLINT_OFFSCREEN"]
            .iter()
            .any(|v| std::env::var_os(v).is_some());
    if !testing && hotkeys::already_running() {
        return;
    }
    let settings = Settings::load();
    std::thread::spawn(export::clean_temp);
    autostart::migrate();
    let hidden = std::env::args().any(|a| a == "--hidden");
    let (tx, rx) = async_channel::unbounded();
    hotkeys::start(tx, settings.hotkey_region, settings.hotkey_full);

    Application::new().with_assets(theme::Assets).run(move |cx: &mut App| {
        fonts::register(cx);
        cx.set_global(AppState {
            settings,
            hotkeys_ok: (true, true),
            main: None,
            overlay_open: false,
        });
        home::open(!hidden, cx);
        #[cfg(debug_assertions)]
        if std::env::var_os("GLINT_TOASTTEST").is_some() {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let _ = cx.update(|cx| toast::show("Copied to clipboard", "1425 × 825", true, cx));
                cx.background_executor().timer(Duration::from_millis(4000)).await;
                let _ = cx.update(quit);
            })
            .detach();
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("GLINT_SELFTEST").is_some() {
            selftest(cx);
        }
        cx.spawn(async move |cx| {
            while let Ok(ev) = rx.recv().await {
                let _ = cx.update(|cx| handle_event(ev, cx));
            }
        })
        .detach();
    });
}

fn handle_event(ev: hotkeys::Event, cx: &mut App) {
    match ev {
        hotkeys::Event::Region => start_region(cx),
        hotkeys::Event::Full => capture_full(cx),
        hotkeys::Event::ShowMain => home::show(cx),
        hotkeys::Event::Quit => quit(cx),
        hotkeys::Event::Refresh => cx.refresh_windows(),
        hotkeys::Event::Status(region, full) => {
            cx.global_mut::<AppState>().hotkeys_ok = (region, full);
            cx.refresh_windows();
        }
    }
}

pub fn quit(cx: &mut App) {
    hotkeys::shutdown();
    cx.quit();
}

/// Находит дисплей GPUI, соответствующий монитору Win32.
pub fn display_for(mon: &MonitorRect, cx: &App) -> Option<DisplayId> {
    let (lx, ly) = (mon.x as f32 / mon.scale, mon.y as f32 / mon.scale);
    cx.displays()
        .into_iter()
        .find(|d| {
            let o = d.bounds().origin;
            (f32::from(o.x) - lx).abs() < 2.0 && (f32::from(o.y) - ly).abs() < 2.0
        })
        .map(|d| d.id())
}

/// Выполняет Win32-настройку окна сразу после его создания, но вне обновления GPUI.
/// Менять размер окна прямо в конструкторе нельзя: GPUI не успевает принять событие
/// изменения размера и остаётся с неверной областью отрисовки.
pub fn after_open<V: 'static>(
    handle: WindowHandle<V>,
    cx: &mut App,
    setup: impl FnOnce(windows::Win32::Foundation::HWND) + 'static,
) {
    cx.spawn(async move |cx| {
        let hwnd = handle
            .update(cx, |_, window, _| win::hwnd_of(window).map(|h| h.0 as isize))
            .ok()
            .flatten();
        if let Some(raw) = hwnd {
            setup(windows::Win32::Foundation::HWND(raw as *mut _));
        }
        let _ = handle.update(cx, |_, window, _| window.refresh());
    })
    .detach();
}

/// «Замораживает» экран и открывает оверлей выбора области.
pub fn start_region(cx: &mut App) {
    open_overlay(cx);
}

fn open_overlay(cx: &mut App) -> Option<WindowHandle<overlay::Overlay>> {
    if cx.global::<AppState>().overlay_open {
        return None;
    }
    let t0 = std::time::Instant::now();
    let mon = capture::monitor_under_cursor();
    let Some(shot) = capture::capture_monitor(mon) else {
        toast::show("Capture failed", "The screen is not available", false, cx);
        return None;
    };
    let t_capture = t0.elapsed();
    let (w, h, scale) = (shot.w, shot.h, shot.scale);
    let buffer = image::RgbaImage::from_raw(w, h, shot.bgra)?;
    // RenderImage хранит пиксели в BGRA — ровно то, что отдаёт GDI, без конвертации.
    let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));

    cx.global_mut::<AppState>().overlay_open = true;
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(mon.x as f32 / scale), px(mon.y as f32 / scale)),
                size: size(px(w as f32 / scale), px(h as f32 / scale)),
            })),
            titlebar: Some(TitlebarOptions {
                title: None,
                appears_transparent: false,
                traffic_light_position: None,
            }),
            focus: true,
            show: false,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: display_for(&mon, cx),
            ..Default::default()
        },
        move |window, cx| {
            let view = cx.new(|cx| overlay::Overlay::new(image, w, h, scale, cx));
            window.focus(&view.read(cx).focus_handle());
            view
        },
    );
    match &opened {
        Ok(handle) => after_open(*handle, cx, move |hwnd| {
            // Отладка: оверлей за пределами экрана и без активации (для снимка через PrintWindow).
            if cfg!(debug_assertions) && std::env::var_os("GLINT_OFFSCREEN").is_some() {
                win::make_popup(hwnd, -30000, mon.y, w as i32, h as i32, false);
            } else {
                win::make_popup(hwnd, mon.x, mon.y, w as i32, h as i32, true);
            }
        }),
        Err(_) => cx.global_mut::<AppState>().overlay_open = false,
    }
    if cfg!(debug_assertions) {
        eprintln!("capture {t_capture:?}, overlay open {:?}", t0.elapsed());
    }
    opened.ok()
}

/// Отладочный прогон: GLINT_SELFTEST=<папка> открывает оверлей, рисует набор фигур
/// и сохраняет результат в файл, не трогая буфер обмена.
#[cfg(debug_assertions)]
fn selftest(cx: &mut App) {
    cx.spawn(async move |cx| {
        let bg = cx.background_executor().clone();
        let wait = move |ms| bg.timer(Duration::from_millis(ms));
        wait(600).await;
        let Ok(Some(handle)) = cx.update(open_overlay) else { return };
        wait(400).await;
        let _ = handle.update(cx, |ov, window, cx| ov.selftest(window, cx));
        wait(2500).await;
        let _ = handle.update(cx, |ov, window, cx| ov.selftest_finish(window, cx));
        wait(3500).await;
        let _ = cx.update(quit);
    })
    .detach();
}

/// Мгновенный снимок всего монитора под курсором.
pub fn capture_full(cx: &mut App) {
    if cx.global::<AppState>().overlay_open {
        return;
    }
    let mon = capture::monitor_under_cursor();
    let Some(shot) = capture::capture_monitor(mon) else {
        toast::show("Capture failed", "The screen is not available", false, cx);
        return;
    };
    let crop = export::Crop {
        x: 0,
        y: 0,
        w: shot.w,
        h: shot.h,
    };
    let rendered = export::render(&shot.bgra, shot.w, crop, 1.0, &[]);
    deliver(rendered, Action::Copy, cx);
}

struct Outcome {
    title: String,
    detail: String,
    ok: bool,
}

fn process(img: &Rendered, st: &Settings, target: Option<std::path::PathBuf>, copy: bool) -> Outcome {
    let dims = format!("{} × {}", img.w, img.h);
    let fail = |title: &str, detail: String| Outcome {
        title: title.into(),
        detail,
        ok: false,
    };

    // 1. Растр — в буфер сразу, чтобы вставка работала мгновенно, пока идёт сжатие.
    let mut sequence = 0;
    if copy {
        if let Err(e) = export::copy_to_clipboard(img, None) {
            return fail("Copy failed", e);
        }
        sequence = export::clipboard_sequence();
    }

    // 2. Сжимаем в выбранный формат и пишем файл: по выбранному пути, в папку автосохранения
    //    или во временную папку (только ради буфера обмена).
    let format = match &target {
        Some(path) => export::format_from_path(path, st.format),
        None => st.format,
    };
    let bytes = match export::encode(img, format, st.jpeg_quality) {
        Ok(bytes) => bytes,
        Err(e) => return fail("Encoding failed", e),
    };
    let kept = target.is_some() || st.auto_save;
    let written = match target {
        Some(path) => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            std::fs::write(&path, &bytes).map(|_| path).map_err(|e| e.to_string())
        }
        None if st.auto_save => export::save_to_dir(&st.save_dir, format, &bytes),
        None => export::save_to_dir(&export::temp_dir(), format, &bytes),
    };
    let path = match written {
        Ok(path) => path,
        Err(e) if kept => return fail("Save failed", e),
        // временный файл не записался — растр в буфере всё равно есть
        Err(_) => {
            return Outcome {
                title: "Copied to clipboard".into(),
                detail: dims,
                ok: true,
            };
        }
    };

    // 3. Дополняем буфер файлом в выбранном формате (если пользователь не скопировал что-то ещё).
    if copy && export::clipboard_sequence() == sequence {
        let _ = export::copy_to_clipboard(img, Some(&path));
    }

    let size = format!("{} {}", format.title(), export::human_size(bytes.len()));
    let title = match (copy, kept) {
        (true, true) => "Saved and copied",
        (true, false) => "Copied to clipboard",
        _ => "Saved",
    };
    let detail = if kept {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        format!("{dims}  ·  {size}  ·  {name}")
    } else {
        format!("{dims}  ·  {size}")
    };
    Outcome {
        title: title.into(),
        detail,
        ok: true,
    }
}

/// Доставляет готовый снимок: буфер обмена и/или файл, затем показывает уведомление.
pub fn deliver(img: Rendered, action: Action, cx: &mut App) {
    if img.w == 0 || img.h == 0 {
        return;
    }
    let st = cx.global::<AppState>().settings.clone();
    let prompt = (action == Action::Save).then(|| {
        let _ = std::fs::create_dir_all(&st.save_dir);
        cx.prompt_for_new_path(&st.save_dir, Some(&export::file_name(st.format)))
    });
    cx.spawn(async move |cx| {
        let mut target = None;
        if let Some(rx) = prompt {
            match rx.await {
                Ok(Ok(Some(mut path))) => {
                    if path.extension().is_none() {
                        path.set_extension(st.format.ext());
                    }
                    target = Some(path);
                }
                // Диалог отменён — снимок всё равно остаётся в буфере обмена.
                _ => {}
            }
        }
        #[allow(unused_mut)]
        let mut copy = true;
        #[cfg(debug_assertions)]
        if let Some(dir) = std::env::var_os("GLINT_SELFTEST") {
            let name = format!("selftest.{}", st.format.ext());
            target = Some(std::path::PathBuf::from(dir).join(name));
            copy = false;
        }
        let outcome = cx
            .background_executor()
            .spawn(async move { process(&img, &st, target, copy) })
            .await;
        let _ = cx.update(|cx| toast::show(outcome.title, outcome.detail, outcome.ok, cx));
    })
    .detach();
}

/// Запускает выбор области с небольшой задержкой (чтобы окно приложения успело скрыться).
pub fn start_region_delayed(cx: &mut App, full: bool) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_millis(220)).await;
        let _ = cx.update(|cx| if full { capture_full(cx) } else { start_region(cx) });
    })
    .detach();
}
