//! Закреплённый снимок: небольшое окно без рамки поверх всех окон.
//! Его можно таскать, менять размер за уголок или колесом мыши, скопировать и закрыть двойным кликом.

use crate::export::{self, Rendered};
use crate::theme::{self, icon};
use crate::win;
use gpui::prelude::*;
use gpui::{
    App, BorderStyle, Bounds, Context, Corners, CursorStyle, FocusHandle, KeyDownEvent,
    MouseButton, MouseDownEvent, RenderImage, ScrollDelta, ScrollWheelEvent, TitlebarOptions,
    Window, WindowBounds, WindowKind, WindowOptions, canvas, div, point, px, quad, rgb, rgba, size,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Два нажатия быстрее этого интервала считаются двойным кликом.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Наименьшая сторона окна при уменьшении, в пикселях.
const MIN_SIDE: f32 = 48.0;
const MAX_ZOOM: f32 = 4.0;
/// Размер зоны в углу окна, за которую тянут, чтобы изменить размер.
const CORNER: f32 = 18.0;

pub struct Pin {
    image: Arc<RenderImage>,
    shot: Arc<Rendered>,
    focus: FocusHandle,
    last_down: Option<Instant>,
    zoom: f32,
    /// Идёт изменение размера за уголок.
    resizing: bool,
}

/// Открывает закреплённый снимок в точке `(x, y)` экрана (физические пиксели).
pub fn open(img: Rendered, x: i32, y: i32, cx: &mut App) {
    let (w, h) = (img.w, img.h);
    if w == 0 || h == 0 {
        return;
    }
    // RenderImage хранит пиксели в BGRA.
    let mut bgra = img.rgba.clone();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let Some(buffer) = image::RgbaImage::from_raw(w, h, bgra) else { return };
    let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
    let shot = Arc::new(img);
    // Окно создаём сразу на том мониторе, где оно окажется: иначе при переносе между
    // мониторами с разным масштабом система изменит его размер.
    let mon = crate::capture::monitor_at(x + w as i32 / 2, y + h as i32 / 2);
    let scale = mon.scale;
    let display_id = crate::display_for(&mon, cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(x as f32 / scale), px(y as f32 / scale)),
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
            is_movable: true,
            is_resizable: false,
            is_minimizable: false,
            display_id,
            ..Default::default()
        },
        move |window, cx| {
            let view = cx.new(|cx| Pin {
                image,
                shot,
                focus: cx.focus_handle(),
                last_down: None,
                zoom: 1.0,
                resizing: false,
            });
            window.focus(&view.read(cx).focus);
            view
        },
    );
    if let Ok(handle) = opened {
        crate::after_open(handle, cx, move |hwnd| {
            win::make_popup(hwnd, x, y, w as i32, h as i32, true);
        });
    }
}

impl Pin {
    fn close(&mut self, window: &mut Window) {
        let _ = window.drop_image(self.image.clone());
        window.remove_window();
    }

    fn on_down(&mut self, _ev: &MouseDownEvent, window: &mut Window, _cx: &mut Context<Self>) {
        window.focus(&self.focus);
        let now = Instant::now();
        if self.last_down.is_some_and(|t| now.duration_since(t) < DOUBLE_CLICK) {
            self.close(window);
            return;
        }
        self.last_down = Some(now);
        if let Some(hwnd) = win::hwnd_of(window) {
            win::begin_drag(hwnd);
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key == "escape" {
            self.close(window);
        } else if ev.keystroke.modifiers.control && win::vk_down(0x43) {
            match export::copy_to_clipboard(&self.shot, None) {
                Ok(()) => {
                    let dims = format!("{} × {}", self.shot.w, self.shot.h);
                    crate::toast::show("Copied to clipboard", dims, true, cx);
                }
                Err(e) => crate::toast::show("Copy failed", e, false, cx),
            }
        }
    }

    /// Изменение размера за угол: противоположный угол стоит на месте, пропорции сохраняются.
    /// `right`/`bottom` — какой угол тянут.
    fn start_resize(&mut self, right: bool, bottom: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hwnd) = win::hwnd_of(window) else { return };
        let Some((l, t, r, b)) = win::rect(hwnd) else { return };
        if self.resizing {
            return;
        }
        self.resizing = true;
        self.last_down = None;
        let raw = hwnd.0 as isize;
        let (w0, h0) = (self.shot.w as f32, self.shot.h as f32);
        let min_zoom = (MIN_SIDE / w0.min(h0)).min(1.0);
        // неподвижный угол и смещение курсора от того угла, за который взялись
        let (ax, ay) = (if right { l } else { r }, if bottom { t } else { b });
        let (cx0, cy0) = win::cursor();
        let (dx, dy) = (cx0 - if right { r } else { l }, cy0 - if bottom { b } else { t });
        cx.spawn(async move |this, cx| {
            let hwnd = windows::Win32::Foundation::HWND(raw as *mut _);
            let mut last = (0, 0);
            let mut zoom = 1.0;
            while win::primary_button_down() {
                let (mx, my) = win::cursor();
                let (w, h) = (((mx - dx) - ax).abs() as f32, ((my - dy) - ay).abs() as f32);
                // берём ту сторону, по которой окно получается больше
                zoom = (w / w0).max(h / h0).clamp(min_zoom, MAX_ZOOM);
                let size = ((w0 * zoom).round() as i32, (h0 * zoom).round() as i32);
                if size != last {
                    last = size;
                    let x = if right { ax } else { ax - size.0 };
                    let y = if bottom { ay } else { ay - size.1 };
                    win::place(hwnd, x, y, size.0, size.1);
                }
                cx.background_executor().timer(Duration::from_millis(8)).await;
            }
            let _ = this.update(cx, |this, cx| {
                this.resizing = false;
                if last != (0, 0) {
                    this.zoom = zoom;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Колесо мыши меняет размер окна с шагом 10 %.
    fn on_wheel(&mut self, ev: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let dy = match ev.delta {
            ScrollDelta::Lines(d) => d.y,
            ScrollDelta::Pixels(d) => f32::from(d.y),
        };
        if dy == 0.0 {
            return;
        }
        let (w, h) = (self.shot.w as f32, self.shot.h as f32);
        let min_zoom = (MIN_SIDE / w.min(h)).min(1.0);
        let zoom = (self.zoom * if dy > 0.0 { 1.1 } else { 1.0 / 1.1 }).clamp(min_zoom, MAX_ZOOM);
        // возле исходного размера — ровно 100 %, чтобы картинка снова стала резкой
        let zoom = if (zoom - 1.0).abs() < 0.04 { 1.0 } else { zoom };
        if zoom == self.zoom {
            return;
        }
        self.zoom = zoom;
        let Some(raw) = win::hwnd_of(window).map(|h| h.0 as isize) else { return };
        let (nw, nh) = ((w * zoom).round() as i32, (h * zoom).round() as i32);
        // Размер меняем вне обновления GPUI, иначе событие изменения размера теряется.
        cx.spawn(async move |this, cx| {
            win::resize(windows::Win32::Foundation::HWND(raw as *mut _), nw, nh);
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }
}

impl Render for Pin {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let image = self.image.clone();
        // Уголки: за них окно тянут, чтобы изменить размер; при наведении видны «скобки».
        let corners = [(false, false), (true, false), (false, true), (true, true)].map(|(right, bottom)| {
            let mark = div()
                .size(px(9.0))
                .border_color(rgba(0xFFFFFFE6))
                .when(right, |d| d.border_r_2())
                .when(!right, |d| d.border_l_2())
                .when(bottom, |d| d.border_b_2())
                .when(!bottom, |d| d.border_t_2())
                .invisible()
                .group_hover("pin", |s| s.visible());
            div()
                .absolute()
                .size(px(CORNER))
                .p(px(3.0))
                .flex()
                .when(right, |d| d.right_0().justify_end())
                .when(!right, |d| d.left_0())
                .when(bottom, |d| d.bottom_0().items_end())
                .when(!bottom, |d| d.top_0())
                .cursor(if right == bottom {
                    CursorStyle::ResizeUpLeftDownRight
                } else {
                    CursorStyle::ResizeUpRightDownLeft
                })
                .child(mark)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.start_resize(right, bottom, window, cx);
                    }),
                )
        });
        div()
            .group("pin")
            .size_full()
            .relative()
            .track_focus(&self.focus)
            .cursor(CursorStyle::OpenHand)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, window, _| this.close(window)),
            )
            .on_scroll_wheel(cx.listener(Self::on_wheel))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let _ = window.paint_image(bounds, Corners::default(), image.clone(), 0, false);
                        // тонкая рамка, чтобы снимок не сливался с тем, что под ним
                        window.paint_quad(quad(
                            bounds,
                            px(0.0),
                            rgba(0x00000000),
                            px(1.0),
                            rgba(0xFFFFFF59),
                            BorderStyle::default(),
                        ));
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                // крестик появляется при наведении на снимок
                div()
                    .absolute()
                    .top(px(CORNER + 2.0))
                    .right(px(CORNER + 2.0))
                    .size(px(22.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(rgba(0x000000C0))
                    .border_1()
                    .border_color(rgba(0xFFFFFF40))
                    .cursor(CursorStyle::PointingHand)
                    .invisible()
                    .group_hover("pin", |s| s.visible())
                    .hover(|s| s.bg(rgb(theme::PANEL_HI)))
                    .child(icon("close").size(px(12.0)).text_color(rgb(0xFFFFFF)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.close(window);
                        }),
                    ),
            )
            .children(corners)
    }
}
