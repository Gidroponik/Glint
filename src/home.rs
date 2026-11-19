//! Главное окно: компактная безрамочная карточка необычной формы —
//! кнопки снимков, горячие клавиши и настройки.

use crate::settings::{Format, Hotkey, Settings};
use crate::theme::{self, icon};
use crate::{AppState, autostart, capture, hotkeys, win};
use gpui::prelude::*;
use gpui::{
    App, BoxShadow, Context, CursorStyle, Div, FocusHandle, FontWeight, KeyDownEvent,
    MouseButton, PathPromptOptions, SharedString, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, div, point, px, rgb, rgba, size,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SW_HIDE, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
};

/// Размер карточки и прозрачное поле вокруг неё (под свечение).
const CARD_W: f32 = 348.0;
const CARD_H: f32 = 556.0;
const MARGIN: f32 = 22.0;
/// «Лист»: два противоположных угла сильно скруглены, два — почти острые.
const R_BIG: f32 = 44.0;
const R_SMALL: f32 = 10.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Region,
    Full,
}

pub struct Home {
    focus: FocusHandle,
    recording: Option<Which>,
    record_error: bool,
}

/// Скругление «листом» в заданном масштабе.
fn leaf(d: Div, big: f32, small: f32) -> Div {
    d.rounded_tl(px(big))
        .rounded_br(px(big))
        .rounded_tr(px(small))
        .rounded_bl(px(small))
}

pub fn open(visible: bool, cx: &mut App) {
    let mon = capture::monitor_under_cursor();
    let s = mon.scale;
    let (w, h) = (CARD_W + MARGIN * 2.0, CARD_H + MARGIN * 2.0);
    let (pw, ph) = ((w * s).round() as i32, (h * s).round() as i32);
    let x = mon.x + (mon.w as i32 - pw) / 2;
    let y = mon.y + (mon.h as i32 - ph) / 2;
    // Отладка: окно за пределами экрана и без активации — для снимка через PrintWindow.
    let offscreen = cfg!(debug_assertions) && std::env::var_os("GLINT_OFFSCREEN").is_some();
    let x = if offscreen { -30000 } else { x };
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(gpui::Bounds {
                    origin: point(px(x as f32 / s), px(y as f32 / s)),
                    size: size(px(w), px(h)),
                })),
                titlebar: Some(TitlebarOptions {
                    title: Some("Glint".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                show: false,
                focus: visible && !offscreen,
                is_resizable: false,
                display_id: crate::display_for(&mon, cx),
                window_background: WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            move |window, cx| {
                // Закрытие окна прячет его в трей, приложение продолжает работать.
                window.on_window_should_close(cx, |window, _| {
                    hide(window);
                    false
                });
                cx.new(|cx| {
                    // Запись сочетания перехватывает всю клавиатуру — вне окна она не нужна.
                    cx.observe_window_activation(window, |this: &mut Home, window, cx| {
                        if !window.is_window_active() {
                            this.stop_recording(cx);
                        }
                    })
                    .detach();
                    Home {
                        focus: cx.focus_handle(),
                        recording: None,
                        record_error: false,
                    }
                })
            },
        )
        .expect("main window");
    cx.global_mut::<AppState>().main = Some(handle);
    crate::after_open(handle, cx, move |hwnd| {
        win::make_frameless(hwnd, x, y, pw, ph, visible, !offscreen);
    });
}

fn hide(window: &Window) {
    if let Some(hwnd) = win::hwnd_of(window) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

pub fn show(cx: &mut App) {
    if let Some(main) = cx.global::<AppState>().main {
        let _ = main.update(cx, |_, window, _| {
            if let Some(hwnd) = win::hwnd_of(window) {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                    let _ = SetForegroundWindow(hwnd);
                }
            }
            window.refresh();
        });
    }
}

fn update_settings(cx: &mut App, f: impl FnOnce(&mut Settings)) {
    let st = cx.global_mut::<AppState>();
    f(&mut st.settings);
    st.settings.save();
}

impl Home {
    fn capture(&mut self, full: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_recording(cx);
        hide(window);
        crate::start_region_delayed(cx, full);
    }

    fn start_recording(&mut self, which: Which, window: &mut Window, cx: &mut Context<Self>) {
        self.recording = Some(which);
        self.record_error = false;
        window.focus(&self.focus);
        hotkeys::record();
        cx.notify();
    }

    fn stop_recording(&mut self, cx: &mut Context<Self>) {
        if self.recording.take().is_some() {
            let s = &cx.global::<AppState>().settings;
            hotkeys::rebind(s.hotkey_region, s.hotkey_full);
            cx.notify();
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, _cx: &mut Context<Self>) {
        // Esc прячет окно в трей. Во время записи сочетания клавиши сюда не доходят:
        // их целиком забирает перехват в `hotkeys`.
        if self.recording.is_none() && ev.keystroke.key == "escape" {
            hide(window);
        }
    }

    /// Нажатие во время записи сочетания (`None` — запись отменена).
    pub fn recorded(&mut self, hk: Option<Hotkey>, cx: &mut Context<Self>) {
        let Some(which) = self.recording else { return };
        let Some(hk) = hk else {
            self.stop_recording(cx);
            return;
        };
        let other = {
            let s = &cx.global::<AppState>().settings;
            if which == Which::Region { s.hotkey_full } else { s.hotkey_region }
        };
        if !hk.usable() || hk == other {
            self.record_error = true;
            cx.notify();
            return;
        }
        update_settings(cx, |s| match which {
            Which::Region => s.hotkey_region = hk,
            Which::Full => s.hotkey_full = hk,
        });
        self.record_error = false;
        self.stop_recording(cx);
    }

    fn pick_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(dir) = paths.into_iter().next() {
                    let _ = this.update(cx, |_, cx| {
                        update_settings(cx, |s| s.save_dir = dir);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Строка-действие: клик делает снимок, клик по клавишам — переназначение хоткея.
    fn action_row(
        &self,
        which: Which,
        title: &'static str,
        icon_name: &'static str,
        cx: &mut Context<Self>,
    ) -> Div {
        let st = cx.global::<AppState>();
        let (hk, bound) = match which {
            Which::Region => (st.settings.hotkey_region, st.hotkeys.0),
            Which::Full => (st.settings.hotkey_full, st.hotkeys.1),
        };
        let recording = self.recording == Some(which);
        let full = which == Which::Full;

        // Подпись появляется только когда есть что сообщить.
        let status: Option<(&'static str, u32)> = if recording && self.record_error {
            Some(("Add Ctrl, Alt or Win to this key", theme::WARN))
        } else if recording {
            Some(("Press any keys · Esc cancels", theme::TEXT))
        } else {
            match bound {
                hotkeys::Bound::Plain => None,
                hotkeys::Bound::Override => Some(("Takes over the system shortcut", theme::MUTED)),
                hotkeys::Bound::Failed => Some(("Shortcut is in use", theme::WARN)),
            }
        };

        let keys = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(3.0))
            .p(px(4.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(if recording { theme::WHITE } else { 0x000000 }))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.border_color(rgb(0x5A5A5A)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    if this.recording == Some(which) {
                        this.stop_recording(cx);
                    } else {
                        this.stop_recording(cx);
                        this.start_recording(which, window, cx);
                    }
                }),
            )
            .children(hk.parts().into_iter().map(|k| {
                div()
                    .px(px(6.0))
                    .h(px(21.0))
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .bg(rgb(theme::BG))
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT))
                    .child(SharedString::from(k))
            }));

        leaf(div(), 20.0, 6.0)
            .flex()
            .items_center()
            .gap(px(11.0))
            .pl(px(10.0))
            .pr(px(6.0))
            .h(px(56.0))
            .bg(rgb(theme::PANEL))
            .border_1()
            .border_color(rgb(if recording { theme::WHITE } else { theme::BORDER }))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(theme::PANEL_HI)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| this.capture(full, window, cx)),
            )
            .child(
                leaf(div(), 13.0, 4.0)
                    .size(px(36.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .bg(rgb(theme::WHITE))
                    .shadow(theme::glow())
                    .child(icon(icon_name).size(px(18.0)).text_color(rgb(theme::INK))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .children(status.map(|(text, color)| {
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(color))
                            .truncate()
                            .child(text)
                    })),
            )
            .child(keys)
    }

    fn toggle_row(
        &self,
        title: &'static str,
        hint: Option<SharedString>,
        on: bool,
        cx: &mut Context<Self>,
        toggle: impl Fn(&mut App) + 'static,
    ) -> Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.0))
            .px(px(13.0))
            .h(px(if hint.is_some() { 46.0 } else { 38.0 }))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(theme::PANEL_HI)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| {
                    toggle(cx);
                    cx.notify();
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().text_size(px(12.5)).child(title))
                    .children(hint.map(|hint| {
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(theme::MUTED))
                            .truncate()
                            .child(hint)
                    })),
            )
            .child(
                div()
                    .w(px(34.0))
                    .h(px(20.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(px(3.0))
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(if on { theme::WHITE } else { 0x3A3A3A }))
                    .bg(rgb(if on { theme::WHITE } else { theme::BG }))
                    .when(on, |d| d.justify_end().shadow(theme::glow()))
                    .child(
                        div()
                            .size(px(12.0))
                            .rounded_full()
                            .bg(rgb(if on { theme::INK } else { 0x6E6E6E })),
                    ),
            )
    }
}

fn chip(label: SharedString, active: bool) -> Div {
    div()
        .flex_1()
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .cursor(CursorStyle::PointingHand)
        .when(active, |d| {
            d.bg(rgb(theme::WHITE))
                .text_color(rgb(theme::INK))
                .shadow(theme::glow())
        })
        .when(!active, |d| {
            d.text_color(rgb(theme::MUTED))
                .hover(|s| s.bg(rgb(theme::PANEL_HI)).text_color(rgb(theme::TEXT)))
        })
        .child(label)
}

fn segmented() -> Div {
    div()
        .flex()
        .gap(px(3.0))
        .p(px(3.0))
        .rounded(px(9.0))
        .bg(rgb(theme::PANEL))
        .border_1()
        .border_color(rgb(theme::BORDER))
}

fn label(text: &'static str) -> Div {
    div()
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(0x6A6A6A))
        .child(text)
}

impl Render for Home {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = cx.global::<AppState>().settings.clone();
        let autostart_on = autostart::enabled();

        let region = self.action_row(Which::Region, "Region", "crop", cx);
        let full = self.action_row(Which::Full, "Full screen", "monitor", cx);

        let toggles = [
            self.toggle_row(
                "Save every capture",
                Some(s.save_dir.to_string_lossy().to_string().into()),
                s.auto_save,
                cx,
                move |cx| update_settings(cx, |s| s.auto_save = !s.auto_save),
            ),
            self.toggle_row(
                "Copy right after selecting",
                None,
                s.instant_copy,
                cx,
                move |cx| update_settings(cx, |s| s.instant_copy = !s.instant_copy),
            ),
            self.toggle_row(
                "Smooth freehand drawing",
                None,
                s.auto_shapes,
                cx,
                move |cx| update_settings(cx, |s| s.auto_shapes = !s.auto_shapes),
            ),
            self.toggle_row(
                "Start with Windows",
                None,
                autostart_on,
                cx,
                move |_| {
                    autostart::set(!autostart_on);
                },
            ),
        ];
        let rows = toggles.into_iter().enumerate().flat_map(|(i, row)| {
            let line = (i > 0).then(|| div().h(px(1.0)).bg(rgb(theme::BORDER)));
            line.into_iter().chain(Some(row))
        });

        let formats: Vec<Div> = Format::ALL
            .iter()
            .map(|f| {
                let f = *f;
                chip(f.title().into(), s.format == f).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_, _, _, cx| {
                        update_settings(cx, |s| s.format = f);
                        cx.notify();
                    }),
                )
            })
            .collect();
        let qualities: Vec<Div> = [60u8, 75, 85, 95]
            .into_iter()
            .map(|q| {
                chip(format!("{q}%").into(), s.jpeg_quality == q).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_, _, _, cx| {
                        update_settings(cx, |s| s.jpeg_quality = q);
                        cx.notify();
                    }),
                )
            })
            .collect();

        let on_key = cx.listener(Self::on_key);
        let pick_folder =
            cx.listener(|this: &mut Self, _: &gpui::MouseDownEvent, _, cx| this.pick_folder(cx));
        let close = cx.listener(|_: &mut Self, _: &gpui::MouseDownEvent, window, cx| {
            cx.stop_propagation();
            hide(window);
        });

        let header = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .pl(px(22.0))
            .pr(px(14.0))
            .pt(px(18.0))
            .pb(px(12.0))
            // за шапку окно можно перетаскивать
            .on_mouse_down(MouseButton::Left, |_, window, _| {
                if let Some(hwnd) = win::hwnd_of(window) {
                    win::begin_drag(hwnd);
                }
            })
            .child(
                leaf(div(), 15.0, 4.0)
                    .size(px(40.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .bg(rgb(theme::WHITE))
                    .shadow(theme::glow())
                    .child(icon("crop").size(px(20.0)).text_color(rgb(theme::INK))),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(18.0))
                            .font_weight(FontWeight::BOLD)
                            .child("Glint"),
                    ),
            )
            .child(
                div()
                    .size(px(28.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(rgb(0x2A2A2A)))
                    .on_mouse_down(MouseButton::Left, close)
                    .child(icon("close").size(px(13.0)).text_color(rgb(theme::TEXT))),
            );

        let body = div()
            .px(px(14.0))
            .pb(px(16.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(region)
            .child(full)
            .child(div().pt(px(6.0)).pl(px(4.0)).child(label("FORMAT")))
            .child(segmented().children(formats))
            // качество относится только к JPEG — для остальных форматов строка приглушена
            .child(
                segmented()
                    .children(qualities)
                    .when(s.format != Format::Jpeg, |d| d.opacity(0.3)),
            )
            .child(div().pt(px(6.0)).pl(px(4.0)).child(label("OPTIONS")))
            .child(
                leaf(div(), 20.0, 6.0)
                    .flex()
                    .flex_col()
                    .bg(rgb(theme::PANEL))
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .overflow_hidden()
                    .children(rows),
            )
            .child(
                div()
                    .pl(px(4.0))
                    .pt(px(2.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(11.5))
                    .text_color(rgb(0xB4B4B4))
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.text_color(rgb(theme::WHITE)))
                    .child(icon("folder").size(px(13.0)).text_color(rgb(0xB4B4B4)))
                    .child("Change folder")
                    .on_mouse_down(MouseButton::Left, pick_folder),
            );

        // Прозрачное окно; сама карточка — «лист» с мягким белым свечением по контуру.
        div()
            .size_full()
            .p(px(MARGIN))
            .track_focus(&self.focus)
            .on_key_down(on_key)
            .font_family(theme::UI_FONT)
            .text_color(rgb(theme::TEXT))
            .child(
                leaf(div(), R_BIG, R_SMALL)
                    .size_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .bg(rgb(theme::BG))
                    .border_1()
                    .border_color(rgba(0xFFFFFF38))
                    .shadow(vec![BoxShadow {
                        color: rgba(0xFFFFFF30).into(),
                        offset: point(px(0.0), px(0.0)),
                        blur_radius: px(18.0),
                        spread_radius: px(0.0),
                    }])
                    .child(header)
                    .child(body),
            )
    }
}
