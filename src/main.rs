#![windows_subsystem = "windows"]

mod autostart;
mod blur;
mod capture;
mod export;
mod fonts;
mod home;
mod hotkeys;
mod icon;
mod ocr;
mod overlay;
mod pin;
mod qr;
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
    /// Распознать текст в выделенной области и скопировать его.
    Text,
    /// Закрепить снимок поверх окон.
    Pin,
}

pub struct AppState {
    pub settings: Settings,
    /// Удалось ли зарегистрировать хоткеи: (область, весь экран).
    pub hotkeys_ok: (bool, bool),
    pub main: Option<WindowHandle<home::Home>>,
    pub overlay_open: bool,
    /// Открытые оверлеи — по одному на монитор.
    pub overlays: Vec<WindowHandle<overlay::Overlay>>,
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
            overlays: Vec::new(),
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
        // В простое (нет ни одного видимого окна) возвращаем системе неиспользуемую память:
        // приложение почти всё время сидит в трее и не должно держать её за собой.
        cx.spawn(async move |cx| {
            let mut idle = 0u32;
            loop {
                cx.background_executor().timer(Duration::from_secs(4)).await;
                if win::has_visible_window() {
                    idle = 0;
                } else {
                    idle += 1;
                    // один раз после того, как всё закрылось и фоновые задачи улеглись
                    if idle == 2 {
                        win::trim_memory();
                    }
                }
            }
        })
        .detach();
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
    #[allow(unused_mut)]
    let mut main = capture::monitor_under_cursor();
    // Отладка: GLINT_MONPT=x,y — взять монитор по точке экрана, а не по курсору.
    #[cfg(debug_assertions)]
    if let Some(pt) = std::env::var("GLINT_MONPT").ok().and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
    }) {
        main = capture::monitor_at(pt.0, pt.1);
        eprintln!("monitor: {main:?}, display: {:?}", display_for(&main, cx));
    }
    // Оверлей открывается на всех мониторах сразу: область можно выбрать на любом.
    // Первым идёт монитор под курсором — он получает фокус. В отладочных прогонах
    // (окно за пределами экрана) хватает одного монитора.
    let offscreen = cfg!(debug_assertions) && std::env::var_os("GLINT_OFFSCREEN").is_some();
    let mut monitors = vec![main];
    // GLINT_ALLMON — и в отладочном прогоне открыть оверлеи всех мониторов (каждый за экраном)
    if !offscreen || std::env::var_os("GLINT_ALLMON").is_some() {
        monitors.extend(
            capture::all_monitors().into_iter().filter(|m| (m.x, m.y) != (main.x, main.y)),
        );
    }
    let found = win::window_rects();
    // Сначала снимаем все мониторы и только потом открываем окна, чтобы оверлей
    // одного монитора не попал на снимок другого.
    let shots: Vec<(MonitorRect, capture::Shot)> = monitors
        .into_iter()
        .filter_map(|m| capture::capture_monitor(m).map(|s| (m, s)))
        .collect();
    if shots.first().is_none_or(|(m, _)| (m.x, m.y) != (main.x, main.y)) {
        toast::show("Capture failed", "The screen is not available", false, cx);
        return None;
    }
    let t_capture = t0.elapsed();

    cx.global_mut::<AppState>().overlay_open = true;
    let mut handles = Vec::new();
    for (index, (mon, shot)) in shots.into_iter().enumerate() {
        #[allow(unused_mut)]
        let mut shot = shot;
        // Отладка: вместо экрана — цветная сетка, чтобы проверять отрисовку на тёмном экране.
        #[cfg(debug_assertions)]
        if std::env::var_os("GLINT_FAKESHOT").is_some() {
            let w = shot.w as usize;
            for (i, px) in shot.bgra.chunks_exact_mut(4).enumerate() {
                let (x, y) = (i % w, i / w);
                let grid = x % 60 < 2 || y % 60 < 2;
                px.copy_from_slice(&[(x / 8) as u8, (y / 6) as u8, if grid { 40 } else { 230 }, 255]);
            }
            // GLINT_FAKEQR=<png> — картинка (например, QR-код), вставленная в этот «экран»
            if let Some(pic) = std::env::var_os("GLINT_FAKEQR").and_then(|p| image::open(p).ok()) {
                let pic = pic.to_rgba8();
                for (x, y, c) in pic.enumerate_pixels() {
                    let o = ((450 + y as usize) * w + 900 + x as usize) * 4;
                    shot.bgra[o..o + 4].copy_from_slice(&[c[2], c[1], c[0], 255]);
                }
            }
        }
        let (w, h, scale) = (shot.w, shot.h, shot.scale);
        let Some(buffer) = image::RgbaImage::from_raw(w, h, shot.bgra) else { continue };
        // RenderImage хранит пиксели в BGRA — ровно то, что отдаёт GDI, без конвертации.
        let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));

        // Окна на этом мониторе — в логических координатах оверлея, для выделения одним кликом.
        let windows: Vec<shapes::R> = found
            .iter()
            .filter_map(|&(l, t, r, b)| {
                let (x0, y0) = ((l - mon.x).max(0), (t - mon.y).max(0));
                let (x1, y1) = ((r - mon.x).min(w as i32), (b - mon.y).min(h as i32));
                (x1 - x0 >= 40 && y1 - y0 >= 40).then(|| shapes::R {
                    x: x0 as f32 / scale,
                    y: y0 as f32 / scale,
                    w: (x1 - x0) as f32 / scale,
                    h: (y1 - y0) as f32 / scale,
                })
            })
            .collect();

        let first = index == 0;
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
                focus: first,
                show: false,
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                display_id: display_for(&mon, cx),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| overlay::Overlay::new(image, w, h, scale, windows, cx));
                // при смене активного окна перерисовываем: лупа и перекрестие видны только в активном
                view.update(cx, |_, cx| {
                    cx.observe_window_activation(window, |_, _, cx| cx.notify()).detach();
                });
                if first {
                    window.focus(&view.read(cx).focus_handle());
                }
                view
            },
        );
        let Ok(handle) = opened else { continue };
        after_open(handle, cx, move |hwnd| {
            if offscreen {
                // Отладка: оверлей за пределами экрана и без активации (для снимка через PrintWindow).
                win::make_popup(hwnd, -30000 - index as i32 * 6000, mon.y, w as i32, h as i32, false);
            } else {
                // остальные мониторы показываются без фокуса и получают его, когда на них придёт мышь
                win::make_popup_as(hwnd, mon.x, mon.y, w as i32, h as i32, first, true);
            }
        });
        handles.push(handle);
    }
    if cfg!(debug_assertions) {
        eprintln!("capture {t_capture:?}, {} overlay(s) open {:?}", handles.len(), t0.elapsed());
    }
    let first = handles.first().copied();
    let st = cx.global_mut::<AppState>();
    st.overlay_open = first.is_some();
    st.overlays = handles;
    first
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

/// Распознаёт текст на снимке и кладёт его в буфер обмена.
pub fn deliver_text(img: Rendered, cx: &mut App) {
    cx.spawn(async move |cx| {
        let outcome = cx
            .background_executor()
            .spawn(async move {
                let text = match ocr::recognize(&img) {
                    Ok(text) => text,
                    Err(e) => return ("Text recognition failed".to_string(), e, false),
                };
                put_text(&text)
            })
            .await;
        let _ = cx.update(|cx| toast::show(outcome.0, outcome.1, outcome.2, cx));
    })
    .detach();
}

/// Кладёт уже распознанный текст в буфер обмена и показывает уведомление.
pub fn deliver_plain_text(text: String, cx: &mut App) {
    let outcome = put_text(&text);
    toast::show(outcome.0, outcome.1, outcome.2, cx);
}

/// Копирует текст; возвращает заголовок, пояснение и признак успеха для уведомления.
fn put_text(text: &str) -> (String, String, bool) {
    if text.trim().is_empty() {
        return ("No text found".to_string(), "Nothing was copied".to_string(), false);
    }
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("GLINT_SELFTEST") {
        let _ = std::fs::write(std::path::PathBuf::from(dir).join("ocr.txt"), text);
        return ("Text copied".to_string(), String::new(), true);
    }
    if let Err(e) = export::copy_text(text) {
        return ("Copy failed".to_string(), e, false);
    }
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview: String = flat.chars().take(70).collect();
    if flat.chars().count() > 70 {
        preview.push('…');
    }
    ("Text copied".to_string(), preview, true)
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
