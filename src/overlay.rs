//! Полноэкранный оверлей: «замороженный» снимок, выделение области и редактор аннотаций.

use crate::export::{self, Crop};
use crate::shapes::{self, Kind, Outline, P, R, Seg, Shape, TEXT_LINE_HEIGHT, p};
use crate::theme::{self, icon};
use crate::{Action, AppState, fonts, win};
use gpui::prelude::*;
use gpui::{
    App, BorderStyle, Bounds, ContentMask, Context, Corners, CursorStyle, DispatchPhase,
    FocusHandle, Font, FontWeight, Hsla, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Path, PathBuilder, PathStyle, Pixels, Point, RenderImage, Rgba,
    SharedString, StrokeOptions, TextRun, Window, canvas, div, fill, font, point, px, quad, rgb,
    rgba, size,
};
use lyon::tessellation::{LineCap, LineJoin};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Pen,
    Marker,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Text,
    Blur,
    /// Подсветка: затемняет всё вокруг отмеченных прямоугольников.
    Spot,
    /// Выделение распознанного текста прямо на снимке.
    Pick,
}

const TOOLS: [(Tool, &str); 11] = [
    (Tool::Select, "select"),
    (Tool::Pen, "pen"),
    (Tool::Marker, "marker"),
    (Tool::Line, "line"),
    (Tool::Arrow, "arrow"),
    (Tool::Rect, "rect"),
    (Tool::Ellipse, "ellipse"),
    (Tool::Text, "text"),
    (Tool::Blur, "blur"),
    (Tool::Spot, "spot"),
    (Tool::Pick, "ocr"),
];

const WIDTHS: [f32; 3] = [2.0, 4.0, 8.0];
const TOOLBAR_W: f32 = 1090.0;
/// Лупа: число увеличенных пикселей по стороне и размер одного из них.
const LOUPE_CELLS: i32 = 15;
const LOUPE_CELL: f32 = 8.0;
/// Во сколько раз уменьшена размытая копия снимка для предпросмотра.
const BLUR_DOWNSCALE: u32 = 4;
const TOOLBAR_H: f32 = 46.0;
/// Цвет рамки и значка найденного QR-кода.
const QR_ACCENT: u32 = 0x30D158;
const HANDLE_HIT: f32 = 9.0;

enum Drag {
    None,
    NewSel { start: P },
    MoveSel { grab: P, orig: R },
    ResizeSel { handle: usize, orig: R },
    Draw { start: P, raw: Vec<P> },
    MoveText { idx: usize, grab: P, orig: P },
    SelectText { idx: usize },
    /// Выделение распознанных слов: от слова-якоря до слова под курсором.
    PickText { start: P, anchor: Option<usize> },
}

pub struct Overlay {
    image: Arc<RenderImage>,
    img_w: u32,
    img_h: u32,
    scale: f32,
    focus: FocusHandle,
    sel: Option<R>,
    drag: Drag,
    tool: Tool,
    color: u32,
    width: f32,
    shapes: Vec<Shape>,
    redo: Vec<Shape>,
    /// Фигура, которая рисуется прямо сейчас.
    current: Option<Shape>,
    /// Индекс текстовой фигуры в режиме ввода.
    editing: Option<usize>,
    /// Выбранная текстовая фигура: её можно двигать и менять ей цвет, размер и начертание.
    selected: Option<usize>,
    text_weight: u16,
    text_size: f32,
    text_outline: bool,
    /// Каретка и якорь выделения в редактируемой надписи (байтовые позиции).
    caret: usize,
    anchor: usize,
    /// Счётчик для автоповтора «+/−»: смена значения останавливает повтор.
    hold_gen: u64,
    cursor: P,
    /// Размытые копии снимка для предпросмотра, по одной на силу размытия.
    blurs: Vec<BlurLayer>,
    /// Окна, которые были на экране в момент снимка (сверху вниз).
    windows: Vec<R>,
    /// Распознанный текст выделенной области и область, для которой он получен.
    ocr: Option<OcrLayer>,
    ocr_pending: Option<Crop>,
    ocr_failed: Option<String>,
    /// Выделенные слова: (якорь, конец) — индексы в `ocr.words`.
    pick: Option<(usize, usize)>,
    /// QR-коды, найденные на снимке.
    codes: Vec<QrHit>,
    auto_shapes: bool,
    instant: bool,
    closed: bool,
}

/// QR-код на снимке: содержимое и рамка в координатах оверлея.
#[derive(Clone)]
struct QrHit {
    text: String,
    r: R,
}

/// Распознанное слово: текст, рамка в координатах оверлея и номер строки.
#[derive(Clone)]
struct PickWord {
    text: String,
    r: R,
    line: usize,
}

struct OcrLayer {
    crop: Crop,
    words: Vec<PickWord>,
}

/// Размытая уменьшенная копия снимка и её размер в логических пикселях.
#[derive(Clone)]
struct BlurLayer {
    key: u32,
    image: Arc<RenderImage>,
    w: f32,
    h: f32,
}

/// Снимок состояния для отрисовки кадра.
struct Snap {
    image: Arc<RenderImage>,
    img_w: u32,
    img_h: u32,
    blurs: Vec<BlurLayer>,
    /// Окно под курсором, пока область не выбрана.
    hover: Option<R>,
    /// Режим выделения текста: слова, выделение и подсказка о состоянии распознавания.
    pick_mode: bool,
    words: Vec<PickWord>,
    pick: Option<(usize, usize)>,
    note: Option<&'static str>,
    /// Рамки QR-кодов внутри выделенной области.
    codes: Vec<R>,
    sel: Option<R>,
    shapes: Vec<Shape>,
    editing: Option<usize>,
    selected: Option<usize>,
    caret: usize,
    anchor: usize,
    cursor: P,
    selecting: bool,
    scale: f32,
}

fn pt(q: Point<Pixels>) -> P {
    p(f32::from(q.x), f32::from(q.y))
}

fn gp(q: P) -> Point<Pixels> {
    point(px(q.x), px(q.y))
}

fn color(c: u32, alpha: f32) -> Hsla {
    let mut c: Rgba = rgb(c);
    c.a = alpha;
    c.into()
}

impl Overlay {
    pub fn new(
        image: Arc<RenderImage>,
        img_w: u32,
        img_h: u32,
        scale: f32,
        windows: Vec<R>,
        cx: &mut Context<Self>,
    ) -> Self {
        // QR-коды ищем по всему снимку один раз, в фоне.
        let shot = image.clone();
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move {
                    match shot.as_bytes(0) {
                        Some(bytes) => crate::qr::scan(bytes, img_w, img_h),
                        None => Vec::new(),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.codes = found
                    .into_iter()
                    .map(|c| QrHit {
                        text: c.text,
                        r: R { x: c.x / scale, y: c.y / scale, w: c.w / scale, h: c.h / scale },
                    })
                    .collect();
                cx.notify();
            });
        })
        .detach();
        let s = &cx.global::<AppState>().settings;
        Self {
            image,
            img_w,
            img_h,
            scale,
            focus: cx.focus_handle(),
            sel: None,
            drag: Drag::None,
            tool: Tool::Select,
            color: s.color,
            width: s.thickness,
            shapes: Vec::new(),
            redo: Vec::new(),
            current: None,
            editing: None,
            selected: None,
            text_weight: s.text_weight,
            text_size: s.text_size,
            text_outline: s.text_outline,
            caret: 0,
            anchor: 0,
            hold_gen: 0,
            cursor: p(-100.0, -100.0),
            blurs: Vec::new(),
            windows,
            ocr: None,
            ocr_pending: None,
            ocr_failed: None,
            pick: None,
            codes: Vec::new(),
            auto_shapes: s.auto_shapes,
            instant: s.instant_copy,
            closed: false,
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn win_size(&self) -> (f32, f32) {
        (self.img_w as f32 / self.scale, self.img_h as f32 / self.scale)
    }

    fn handles(sel: &R) -> [P; 8] {
        let (x0, y0, x1, y1) = (sel.x, sel.y, sel.right(), sel.bottom());
        let (xm, ym) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        [
            p(x0, y0),
            p(xm, y0),
            p(x1, y0),
            p(x1, ym),
            p(x1, y1),
            p(xm, y1),
            p(x0, y1),
            p(x0, ym),
        ]
    }

    fn hit_handle(&self, q: P) -> Option<usize> {
        let sel = self.sel.as_ref()?;
        Self::handles(sel)
            .iter()
            .position(|h| (h.x - q.x).abs() <= HANDLE_HIT && (h.y - q.y).abs() <= HANDLE_HIT)
    }

    /// Завершает ввод текста. Пустую надпись удаляет и возвращает её индекс.
    fn commit_text(&mut self) -> Option<usize> {
        let i = self.editing.take()?;
        let empty = matches!(&self.shapes[i].kind, Kind::Text { text, .. } if text.trim().is_empty());
        if empty {
            self.shapes.remove(i);
            self.selected = None;
            return Some(i);
        }
        None
    }

    fn hit_text(&self, q: P, window: &Window) -> Option<usize> {
        self.shapes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, s)| text_bounds(s, window).is_some_and(|r| r.contains(q)))
            .map(|(i, _)| i)
    }

    /// Выбирает надпись и подтягивает её стиль в панель инструментов.
    fn select_text(&mut self, i: usize) {
        self.selected = Some(i);
        self.color = self.shapes[i].color;
        if let Kind::Text {
            size,
            weight,
            outline,
            ..
        } = &self.shapes[i].kind
        {
            self.text_size = *size;
            self.text_weight = *weight;
            self.text_outline = *outline;
        }
    }

    /// Применяет текущие размер и начертание к редактируемой или выбранной надписи.
    fn apply_text_style(&mut self) {
        if let Some(i) = self.editing.or(self.selected) {
            if let Some(Shape {
                kind:
                    Kind::Text {
                        size,
                        weight,
                        outline,
                        ..
                    },
                ..
            }) = self.shapes.get_mut(i)
            {
                *size = self.text_size;
                *weight = self.text_weight;
                *outline = self.text_outline;
            }
        }
    }

    fn edit_text(&mut self) -> Option<&mut String> {
        let i = self.editing?;
        match &mut self.shapes.get_mut(i)?.kind {
            Kind::Text { text, .. } => Some(text),
            _ => None,
        }
    }

    /// Границы выделения в редактируемом тексте (в байтах).
    fn sel_range(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    fn delete_selection(&mut self) -> bool {
        let (a, b) = self.sel_range();
        if a == b {
            return false;
        }
        if let Some(text) = self.edit_text() {
            text.replace_range(a..b, "");
        }
        self.caret = a;
        self.anchor = a;
        true
    }

    fn insert_text(&mut self, s: &str) {
        self.delete_selection();
        let c = self.caret;
        if let Some(text) = self.edit_text() {
            let c = c.min(text.len());
            text.insert_str(c, s);
            self.caret = c + s.len();
            self.anchor = self.caret;
        }
    }

    fn move_caret(&mut self, to: usize, extend: bool) {
        self.caret = to;
        if !extend {
            self.anchor = to;
        }
    }

    fn bump_size(&mut self, delta: f32) {
        self.text_size = (self.text_size + delta).clamp(8.0, 300.0);
        self.apply_text_style();
    }

    /// Нажатие на «+»/«−»: один шаг сразу, при удержании — быстрый автоповтор.
    fn start_size_hold(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.bump_size(delta);
        self.hold_gen += 1;
        let generation = self.hold_gen;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(320))
                .await;
            loop {
                let go = this
                    .update(cx, |this, cx| {
                        if this.hold_gen != generation {
                            return false;
                        }
                        this.bump_size(delta);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !go {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(28))
                    .await;
            }
        })
        .detach();
    }

    fn text_panel_visible(&self) -> bool {
        self.tool == Tool::Text || self.selected.is_some() || self.editing.is_some()
    }

    /// Готовит размытые копии снимка для всех фигур размытия, которые сейчас на экране.
    fn ensure_blurs(&mut self) {
        let keys: Vec<u32> = self
            .shapes
            .iter()
            .chain(&self.current)
            .filter(|s| matches!(s.kind, Kind::Blur(..)))
            .map(|s| s.width.to_bits())
            .collect();
        for key in keys {
            if self.blurs.iter().any(|b| b.key == key) {
                continue;
            }
            let Some(bytes) = self.image.as_bytes(0) else { return };
            let radius = crate::blur::radius(f32::from_bits(key)) * self.scale;
            let (data, w, h) =
                crate::blur::preview(bytes, self.img_w, self.img_h, BLUR_DOWNSCALE, radius);
            let Some(buffer) = image::RgbaImage::from_raw(w, h, data) else { return };
            let k = BLUR_DOWNSCALE as f32 / self.scale;
            self.blurs.push(BlurLayer {
                key,
                image: Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])),
                w: w as f32 * k,
                h: h as f32 * k,
            });
        }
    }

    /// Область выделения в пикселях снимка.
    fn crop_of(&self, sel: &R) -> Crop {
        let s = self.scale;
        let x0 = ((sel.x * s).round() as i64).clamp(0, self.img_w as i64 - 1) as u32;
        let y0 = ((sel.y * s).round() as i64).clamp(0, self.img_h as i64 - 1) as u32;
        let x1 = ((sel.right() * s).round() as i64).clamp(x0 as i64 + 1, self.img_w as i64) as u32;
        let y1 = ((sel.bottom() * s).round() as i64).clamp(y0 as i64 + 1, self.img_h as i64) as u32;
        Crop {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }

    /// В режиме выделения текста распознаёт слова в текущей области (в фоне, один раз на область).
    fn ensure_ocr(&mut self, cx: &mut Context<Self>) {
        if self.tool != Tool::Pick || !matches!(self.drag, Drag::None) {
            return;
        }
        let Some(sel) = self.sel else { return };
        let crop = self.crop_of(&sel);
        if self.ocr.as_ref().is_some_and(|o| o.crop == crop) || self.ocr_pending == Some(crop) {
            return;
        }
        let Some(bytes) = self.image.as_bytes(0) else { return };
        let img = export::render(bytes, self.img_w, crop, self.scale, &[]);
        self.ocr = None;
        self.ocr_failed = None;
        self.pick = None;
        self.ocr_pending = Some(crop);
        let scale = self.scale;
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { crate::ocr::recognize_lines(&img) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.ocr_pending != Some(crop) {
                    return;
                }
                this.ocr_pending = None;
                let mut words = Vec::new();
                match found {
                    Ok(lines) => {
                        for (line, row) in lines.into_iter().enumerate() {
                            for w in row {
                                words.push(PickWord {
                                    text: w.text,
                                    r: R {
                                        x: (crop.x as f32 + w.x) / scale,
                                        y: (crop.y as f32 + w.y) / scale,
                                        w: w.w / scale,
                                        h: w.h / scale,
                                    },
                                    line,
                                });
                            }
                        }
                    }
                    Err(e) => this.ocr_failed = Some(e),
                }
                this.ocr = Some(OcrLayer { crop, words });
                cx.notify();
            });
        })
        .detach();
    }

    /// Копирует результат: в режиме текста — выделенные слова, иначе — картинку.
    fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tool == Tool::Pick && self.sel.is_some() {
            self.copy_picked(window, cx);
        } else {
            self.finish(Action::Copy, window, cx);
        }
    }

    /// Копирует выделенные слова; без выделения — весь текст области.
    fn copy_picked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ready = self.ocr.as_ref().filter(|_| self.ocr_pending.is_none());
        let Some(layer) = ready else {
            // распознавание ещё не закончилось — распознаём область целиком и копируем всё
            self.finish(Action::Text, window, cx);
            return;
        };
        let (a, b) = match self.pick {
            Some((a, b)) => (a.min(b), a.max(b)),
            None => (0, layer.words.len().saturating_sub(1)),
        };
        let mut text = String::new();
        let mut line = None;
        for w in layer.words.iter().skip(a).take(b + 1 - a) {
            if let Some(prev) = line {
                text.push(if prev == w.line { ' ' } else { '\n' });
            }
            text.push_str(&w.text);
            line = Some(w.line);
        }
        self.close(window, cx);
        crate::deliver_plain_text(text, cx);
    }

    /// QR-коды, которые сейчас показаны: до выбора области — все найденные на экране,
    /// после — те, что попали в область. Возвращает (номер, рамка).
    fn visible_codes(&self) -> Vec<(usize, R)> {
        if matches!(self.drag, Drag::NewSel { .. }) {
            return Vec::new();
        }
        let sel = self.sel;
        self.codes
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                sel.is_none_or(|s| s.contains(p(c.r.x + c.r.w / 2.0, c.r.y + c.r.h / 2.0)))
            })
            .map(|(i, c)| (i, c.r))
            .collect()
    }

    /// Копирует содержимое QR-кода и закрывает оверлей.
    fn copy_code(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.codes.get(i).map(|c| c.text.clone()) else { return };
        self.close(window, cx);
        match export::copy_text(&text) {
            Ok(()) => crate::toast::show("QR code copied", clip(&text, 70), true, cx),
            Err(e) => crate::toast::show("Copy failed", e, false, cx),
        }
    }

    /// Верхнее окно, в которое попадает точка.
    fn window_at(&self, q: P) -> Option<R> {
        self.windows.iter().find(|w| w.contains(q)).copied()
    }

    /// Цвет пикселя снимка под точкой.
    fn color_at(&self, q: P) -> Option<u32> {
        let bytes = self.image.as_bytes(0)?;
        let (x, y) = ((q.x * self.scale) as i32, (q.y * self.scale) as i32);
        pixel(bytes, self.img_w as i32, self.img_h as i32, x, y)
    }

    fn new_shape(&self, kind: Kind) -> Shape {
        let marker = self.tool == Tool::Marker;
        Shape {
            kind,
            color: self.color,
            width: if marker { self.width * 3.0 + 8.0 } else { self.width },
            alpha: if marker { 0.4 } else { 1.0 },
        }
    }

    fn on_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        let q = pt(ev.position);
        self.cursor = q;
        let handle = self.hit_handle(q);
        let in_sel = self.sel.is_some_and(|s| s.contains(q));

        // Клик по готовой надписи: выбрать и тащить; двойной клик — снова редактировать.
        if handle.is_none() && in_sel && matches!(self.tool, Tool::Select | Tool::Text) {
            if let Some(mut i) = self.hit_text(q, window) {
                if self.editing != Some(i) {
                    if let Some(removed) = self.commit_text() {
                        if removed < i {
                            i -= 1;
                        }
                    }
                }
                self.select_text(i);
                if self.editing == Some(i) || ev.click_count >= 2 {
                    // В режиме набора клик ставит каретку, протяжка выделяет текст,
                    // двойной клик выделяет слово, тройной — всё.
                    let was_editing = self.editing == Some(i);
                    self.editing = Some(i);
                    let idx = text_index_at(&self.shapes[i], q, window);
                    self.caret = idx;
                    if !ev.modifiers.shift {
                        self.anchor = idx;
                    }
                    if let (true, Kind::Text { text, .. }) = (was_editing, &self.shapes[i].kind) {
                        if ev.click_count == 2 {
                            self.anchor = word_left(text, next_char(text, idx).min(text.len()));
                            self.caret = word_right(text, self.anchor).min(text.len());
                            self.caret = text[..self.caret].trim_end().len().max(self.anchor);
                        } else if ev.click_count >= 3 {
                            self.anchor = 0;
                            self.caret = text.len();
                        }
                    }
                    self.drag = Drag::SelectText { idx: i };
                } else if let Kind::Text { at, .. } = &self.shapes[i].kind {
                    self.drag = Drag::MoveText {
                        idx: i,
                        grab: q,
                        orig: *at,
                    };
                }
                cx.notify();
                return;
            }
        }
        self.commit_text();
        self.selected = None;

        if let Some(handle) = handle {
            self.drag = Drag::ResizeSel {
                handle,
                orig: self.sel.unwrap(),
            };
        } else if let Some(sel) = self.sel.filter(|s| s.contains(q)) {
            match self.tool {
                Tool::Select => {
                    if ev.click_count >= 2 {
                        self.copy(window, cx);
                        return;
                    }
                    self.drag = Drag::MoveSel { grab: q, orig: sel };
                }
                Tool::Pick => {
                    let words = self.ocr.as_ref().map(|o| o.words.as_slice()).unwrap_or(&[]);
                    let hit = word_hit(words, q);
                    self.pick = hit.map(|i| (i, i));
                    match (hit, ev.click_count) {
                        // двойной клик — вся строка, тройной — весь текст
                        (Some(i), 2) => {
                            let line = words[i].line;
                            let first = words.iter().position(|w| w.line == line).unwrap_or(i);
                            let last = words.iter().rposition(|w| w.line == line).unwrap_or(i);
                            self.pick = Some((first, last));
                        }
                        (Some(_), n) if n >= 3 => self.pick = Some((0, words.len() - 1)),
                        _ => self.drag = Drag::PickText { start: q, anchor: hit },
                    }
                }
                Tool::Text => {
                    self.redo.clear();
                    let size = self.text_size;
                    self.shapes.push(Shape {
                        kind: Kind::Text {
                            at: p(q.x, q.y - size * TEXT_LINE_HEIGHT / 2.0),
                            text: String::new(),
                            size,
                            weight: self.text_weight,
                            outline: self.text_outline,
                        },
                        color: self.color,
                        width: self.width,
                        alpha: 1.0,
                    });
                    self.editing = Some(self.shapes.len() - 1);
                    self.selected = self.editing;
                    self.caret = 0;
                    self.anchor = 0;
                }
                _ => {
                    self.drag = Drag::Draw {
                        start: q,
                        raw: vec![q],
                    };
                    self.current = Some(self.new_shape(Kind::Stroke(vec![q])));
                }
            }
        } else if self.sel.is_none() || self.shapes.is_empty() {
            self.sel = None;
            self.drag = Drag::NewSel { start: q };
        }
        cx.notify();
    }

    fn on_move(&mut self, ev: &MouseMoveEvent, window: &Window, cx: &mut Context<Self>) {
        let q = pt(ev.position);
        self.cursor = q;
        let (ww, wh) = self.win_size();
        let shift = ev.modifiers.shift;
        match &mut self.drag {
            Drag::None => {
                if self.sel.is_none() {
                    cx.notify();
                }
                return;
            }
            Drag::NewSel { start } => {
                let c = p(q.x.clamp(0.0, ww), q.y.clamp(0.0, wh));
                self.sel = Some(R::from_points(*start, c));
            }
            Drag::MoveSel { grab, orig } => {
                let mut r = *orig;
                r.x = (orig.x + q.x - grab.x).clamp(0.0, (ww - r.w).max(0.0));
                r.y = (orig.y + q.y - grab.y).clamp(0.0, (wh - r.h).max(0.0));
                self.sel = Some(r);
            }
            Drag::ResizeSel { handle, orig } => {
                let (mut x0, mut y0, mut x1, mut y1) = (orig.x, orig.y, orig.right(), orig.bottom());
                let (cx_, cy_) = (q.x.clamp(0.0, ww), q.y.clamp(0.0, wh));
                if matches!(handle, 0 | 6 | 7) {
                    x0 = cx_;
                }
                if matches!(handle, 2 | 3 | 4) {
                    x1 = cx_;
                }
                if matches!(handle, 0 | 1 | 2) {
                    y0 = cy_;
                }
                if matches!(handle, 4 | 5 | 6) {
                    y1 = cy_;
                }
                self.sel = Some(R::from_points(p(x0, y0), p(x1, y1)));
            }
            Drag::SelectText { idx } => {
                if let Some(shape) = self.shapes.get(*idx) {
                    self.caret = text_index_at(shape, q, window);
                }
            }
            Drag::PickText { start, anchor } => {
                if let Some(layer) = &self.ocr {
                    // протяжка с пустого места цепляется за ближайшее слово
                    if anchor.is_none() && start.dist(q) > 4.0 {
                        *anchor = word_near(&layer.words, *start);
                    }
                    if let (Some(a), Some(c)) = (*anchor, word_near(&layer.words, q)) {
                        self.pick = Some((a, c));
                    }
                }
            }
            Drag::MoveText { idx, grab, orig } => {
                let to = p(orig.x + q.x - grab.x, orig.y + q.y - grab.y);
                if let Some(Shape {
                    kind: Kind::Text { at, .. },
                    ..
                }) = self.shapes.get_mut(*idx)
                {
                    *at = to;
                }
            }
            Drag::Draw { start, raw } => {
                let a = *start;
                let kind = match self.tool {
                    Tool::Pen | Tool::Marker => {
                        if raw.last().is_none_or(|l| l.dist(q) > 0.8) {
                            raw.push(q);
                        }
                        Kind::Stroke(shapes::smooth(raw))
                    }
                    Tool::Line => Kind::Line(a, if shift { shapes::snap_angle(a, q) } else { q }),
                    Tool::Arrow => Kind::Arrow(a, if shift { shapes::snap_angle(a, q) } else { q }),
                    Tool::Rect => Kind::Rect(a, if shift { shapes::snap_square(a, q) } else { q }),
                    Tool::Ellipse => {
                        Kind::Ellipse(a, if shift { shapes::snap_square(a, q) } else { q })
                    }
                    Tool::Blur => Kind::Blur(a, if shift { shapes::snap_square(a, q) } else { q }),
                    Tool::Spot => Kind::Spot(a, if shift { shapes::snap_square(a, q) } else { q }),
                    Tool::Select | Tool::Text | Tool::Pick => return,
                };
                if let Some(cur) = &mut self.current {
                    cur.kind = kind;
                }
            }
        }
        cx.notify();
    }

    fn on_up(&mut self, _ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.hold_gen += 1;
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::None => return,
            Drag::NewSel { start } => {
                // Клик без протяжки выделяет окно под курсором.
                if !self.sel.is_some_and(|s| s.w >= 4.0 && s.h >= 4.0) {
                    self.sel = self.window_at(start);
                }
                if self.instant && self.sel.is_some() {
                    self.copy(window, cx);
                    return;
                }
            }
            Drag::MoveSel { .. } | Drag::ResizeSel { .. } => {
                if self.sel.is_some_and(|s| s.w < 4.0 || s.h < 4.0) {
                    self.sel = None;
                    self.shapes.clear();
                self.selected = None;
                    self.redo.clear();
                }
            }
            Drag::MoveText { .. } | Drag::SelectText { .. } | Drag::PickText { .. } => {}
            Drag::Draw { raw, .. } => {
                if let Some(mut shape) = self.current.take() {
                    let freehand = matches!(self.tool, Tool::Pen | Tool::Marker);
                    if freehand && self.auto_shapes {
                        match shapes::recognize(&raw) {
                            Some(kind @ Kind::Line(..)) => shape.kind = kind,
                            Some(kind) if self.tool == Tool::Pen => shape.kind = kind,
                            // не фигура — значит почерк: выравниваем линии и скругляем углы
                            _ => shape.kind = Kind::Curve(shapes::beautify(&raw)),
                        }
                    }
                    let keep = match &shape.kind {
                        Kind::Stroke(pts) | Kind::Curve(pts) => !pts.is_empty(),
                        Kind::Line(a, b) | Kind::Arrow(a, b) => a.dist(*b) > 2.0,
                        Kind::Rect(a, b) | Kind::Ellipse(a, b) | Kind::Blur(a, b) | Kind::Spot(a, b) => {
                            (a.x - b.x).abs() > 2.0 && (a.y - b.y).abs() > 2.0
                        }
                        Kind::Text { .. } => true,
                    };
                    if keep {
                        self.shapes.push(shape);
                        self.redo.clear();
                    }
                }
            }
        }
        cx.notify();
    }

    fn on_right(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            self.commit_text();
        } else if self.sel.is_some() && matches!(self.drag, Drag::None) {
            self.sel = None;
            self.shapes.clear();
                self.selected = None;
            self.redo.clear();
            self.current = None;
        } else {
            // Области ещё нет, на экране лупа: ПКМ забирает цвет пикселя под курсором.
            if !self.copy_color(pt(ev.position), window, cx) {
                self.close(window, cx);
            }
            return;
        }
        cx.notify();
    }

    /// Копирует цвет пикселя под точкой `q` как `#RRGGBB` и закрывает оверлей.
    fn copy_color(&mut self, q: P, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(c) = self.color_at(q) else { return false };
        let hex = format!("#{c:06X}");
        self.close(window, cx);
        match export::copy_text(&hex) {
            Ok(()) => crate::toast::show("Color copied", hex, true, cx),
            Err(e) => crate::toast::show("Copy failed", e, false, cx),
        }
        true
    }

    fn undo(&mut self) {
        self.commit_text();
        self.selected = None;
        if let Some(s) = self.shapes.pop() {
            self.redo.push(s);
        }
    }

    fn redo(&mut self) {
        self.commit_text();
        self.selected = None;
        if let Some(s) = self.redo.pop() {
            self.shapes.push(s);
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = ks.modifiers;
        let key = ks.key.as_str();

        if self.editing.is_some() {
            let Some(text) = self.edit_text().map(|t| t.clone()) else {
                return;
            };
            let len = text.len();
            self.caret = self.caret.min(len);
            self.anchor = self.anchor.min(len);
            let (a, b) = self.sel_range();
            let has_sel = a != b;
            let c = self.caret;
            let shift = m.shift;
            let mut done = false;
            match key {
                "escape" => done = true,
                "enter" if m.control => done = true,
                "enter" => self.insert_text("\n"),
                "space" => self.insert_text(" "),
                "backspace" => {
                    if !self.delete_selection() && c > 0 {
                        self.anchor = if m.control { word_left(&text, c) } else { prev_char(&text, c) };
                        self.delete_selection();
                    }
                }
                "delete" => {
                    if !self.delete_selection() && c < len {
                        self.anchor = if m.control { word_right(&text, c) } else { next_char(&text, c) };
                        self.delete_selection();
                    }
                }
                "left" => {
                    let to = if has_sel && !shift {
                        a
                    } else if m.control {
                        word_left(&text, c)
                    } else {
                        prev_char(&text, c)
                    };
                    self.move_caret(to, shift);
                }
                "right" => {
                    let to = if has_sel && !shift {
                        b
                    } else if m.control {
                        word_right(&text, c)
                    } else {
                        next_char(&text, c)
                    };
                    self.move_caret(to, shift);
                }
                "home" => self.move_caret(if m.control { 0 } else { line_start(&text, c) }, shift),
                "end" => self.move_caret(if m.control { len } else { line_end(&text, c) }, shift),
                "up" => self.move_caret(vertical(&text, c, true), shift),
                "down" => self.move_caret(vertical(&text, c, false), shift),
                _ if m.control && !m.alt => {
                    if win::vk_down(0x41) {
                        self.anchor = 0;
                        self.caret = len;
                    } else if win::vk_down(0x43) || win::vk_down(0x58) {
                        if has_sel {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text[a..b].to_string()));
                            if win::vk_down(0x58) {
                                self.delete_selection();
                            }
                        }
                    } else if win::vk_down(0x56) {
                        if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                            self.insert_text(&t.replace('\r', ""));
                        }
                    }
                }
                _ if !m.platform => {
                    if let Some(ch) = &ks.key_char {
                        if win::caps_lock_on() {
                            let swapped: String = ch
                                .chars()
                                .flat_map(|c| {
                                    if c.is_lowercase() {
                                        c.to_uppercase().collect::<Vec<_>>()
                                    } else {
                                        c.to_lowercase().collect::<Vec<_>>()
                                    }
                                })
                                .collect();
                            self.insert_text(&swapped);
                        } else {
                            self.insert_text(ch);
                        }
                    }
                }
                _ => {}
            }
            if done {
                self.commit_text();
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }

        if m.control {
            if win::vk_down(0x43) {
                self.copy(window, cx);
            } else if win::vk_down(0x53) {
                self.finish(Action::Save, window, cx);
            } else if win::vk_down(0x50) {
                self.finish(Action::Pin, window, cx);
            } else if win::vk_down(0x54) {
                if self.sel.is_some() {
                    self.commit_text();
                    self.tool = Tool::Pick;
                }
            } else if win::vk_down(0x41) && self.tool == Tool::Pick && self.sel.is_some() {
                // в режиме текста Ctrl+A выделяет весь распознанный текст
                let n = self.ocr.as_ref().map_or(0, |o| o.words.len());
                self.pick = (n > 0).then(|| (0, n - 1));
            } else if win::vk_down(0x5A) {
                if m.shift { self.redo() } else { self.undo() }
            } else if win::vk_down(0x59) {
                self.redo();
            } else if win::vk_down(0x41) {
                let (w, h) = self.win_size();
                self.sel = Some(R { x: 0.0, y: 0.0, w, h });
            }
            cx.notify();
            return;
        }

        // «C» до выбора области копирует цвет пикселя под курсором (пипетка).
        let picking = self.sel.is_none() || matches!(self.drag, Drag::NewSel { .. });
        if picking && win::vk_down(0x43) {
            if self.copy_color(self.cursor, window, cx) {
                return;
            }
        }

        match key {
            "escape" => {
                // Esc с выделенной областью — сразу копируем её в буфер; без выделения — просто выходим.
                if self.sel.is_some() {
                    self.copy(window, cx);
                } else {
                    self.close(window, cx);
                }
                return;
            }
            "enter" => {
                self.copy(window, cx);
                return;
            }
            "left" | "right" | "up" | "down" if self.selected.is_some() => {
                // стрелки двигают выбранную надпись (с Shift — по 10 px)
                let step = if m.shift { 10.0 } else { 1.0 };
                let (dx, dy) = match key {
                    "left" => (-step, 0.0),
                    "right" => (step, 0.0),
                    "up" => (0.0, -step),
                    _ => (0.0, step),
                };
                if let Some(Shape {
                    kind: Kind::Text { at, .. },
                    ..
                }) = self.selected.and_then(|i| self.shapes.get_mut(i))
                {
                    at.x += dx;
                    at.y += dy;
                }
            }
            "backspace" | "delete" => match self.selected.take() {
                Some(i) if i < self.shapes.len() => {
                    self.shapes.remove(i);
                }
                _ => self.undo(),
            },
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "0" if self.sel.is_some() => {
                // 1–9 — первые девять инструментов, 0 — режим текста
                self.tool = match key.parse::<usize>().unwrap() {
                    0 => Tool::Pick,
                    n => TOOLS[n - 1].0,
                };
            }
            _ => {}
        }
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closed {
            return;
        }
        self.closed = true;
        let (color, width) = (self.color, self.width);
        let (text_weight, text_size, text_outline) =
            (self.text_weight, self.text_size, self.text_outline);
        cx.update_global::<AppState, _>(|st, _| {
            st.overlay_open = false;
            let s = &mut st.settings;
            if s.color != color
                || s.thickness != width
                || s.text_weight != text_weight
                || s.text_size != text_size
                || s.text_outline != text_outline
            {
                s.color = color;
                s.thickness = width;
                s.text_weight = text_weight;
                s.text_size = text_size;
                s.text_outline = text_outline;
                s.save();
            }
        });
        let _ = window.drop_image(self.image.clone());
        for layer in self.blurs.drain(..) {
            let _ = window.drop_image(layer.image);
        }
        window.remove_window();
    }

    fn finish(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_text();
        self.current = None;
        let Some(sel) = self.sel else { return };
        let s = self.scale;
        let crop = self.crop_of(&sel);
        let Some(bytes) = self.image.as_bytes(0) else { return };
        // Текст распознаём по чистому снимку: аннотации только мешали бы.
        let shapes: &[Shape] = if action == Action::Text { &[] } else { &self.shapes };
        let rendered = export::render(bytes, self.img_w, crop, s, shapes);
        // положение области на экране в физических пикселях — туда встанет закреплённый снимок
        let (ox, oy) = win::hwnd_of(window).and_then(win::origin).unwrap_or((0, 0));
        let at = (ox + crop.x as i32, oy + crop.y as i32);
        self.close(window, cx);
        if action == Action::Pin {
            cx.defer(move |cx| crate::pin::open(rendered, at.0, at.1, cx));
        } else if action == Action::Text {
            crate::deliver_text(rendered, cx);
        } else {
            crate::deliver(rendered, action, cx);
        }
    }

    fn snapshot(&self) -> Snap {
        let mut shapes = self.shapes.clone();
        if let Some(cur) = &self.current {
            shapes.push(cur.clone());
        }
        Snap {
            image: self.image.clone(),
            img_w: self.img_w,
            img_h: self.img_h,
            blurs: self.blurs.clone(),
            pick_mode: self.tool == Tool::Pick,
            words: match &self.ocr {
                Some(layer) if self.tool == Tool::Pick => layer.words.clone(),
                _ => Vec::new(),
            },
            pick: self.pick,
            codes: self.visible_codes().into_iter().map(|(_, r)| r).collect(),
            note: if self.tool != Tool::Pick {
                None
            } else if self.ocr_pending.is_some() || self.ocr.is_none() {
                Some("Recognizing text…")
            } else if self.ocr_failed.is_some() {
                Some("Text recognition failed")
            } else if self.ocr.as_ref().is_some_and(|o| o.words.is_empty()) {
                Some("No text found")
            } else {
                None
            },
            hover: (self.sel.is_none() && matches!(self.drag, Drag::None))
                .then(|| self.window_at(self.cursor))
                .flatten(),
            sel: self.sel,
            shapes,
            editing: self.editing,
            selected: self.selected,
            caret: self.caret,
            anchor: self.anchor,
            cursor: self.cursor,
            selecting: self.sel.is_none() || matches!(self.drag, Drag::NewSel { .. }),
            scale: self.scale,
        }
    }

    fn toolbar_pos(&self, sel: &R) -> (f32, f32) {
        let (ww, wh) = self.win_size();
        let w = TOOLBAR_W.min(ww - 16.0);
        let x = (sel.right() - w).clamp(8.0, (ww - w - 8.0).max(8.0));
        let h = if self.text_panel_visible() {
            TOOLBAR_H * 2.0 + 6.0
        } else {
            TOOLBAR_H
        };
        let below = sel.bottom() + 12.0;
        let above = sel.y - 12.0 - h;
        let y = if below + h <= wh - 8.0 {
            below
        } else if above >= 8.0 {
            above
        } else {
            sel.bottom() - h - 12.0
        };
        (x, y)
    }

    fn render_toolbar(&self, sel: &R, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (x, y) = self.toolbar_pos(sel);
        let (ww, _) = self.win_size();
        let text_panel = self.text_panel_visible().then(|| self.text_panel(cx));
        div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(TOOLBAR_W.min(ww - 16.0)))
            .flex()
            .flex_col()
            .items_end()
            .gap(px(6.0))
            .child(self.toolbar_panel(cx))
            .children(text_panel)
    }

    /// Панель текста: начертания Montserrat (каждая кнопка показана своим весом) и размер.
    fn text_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        let weights: Vec<_> = fonts::WEIGHTS
            .iter()
            .map(|(w, _)| {
                let w = *w;
                let active = self.text_weight == w;
                div()
                    .w(px(46.0))
                    .h(px(32.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .cursor(CursorStyle::PointingHand)
                    .font_family(fonts::FAMILY)
                    .font_weight(FontWeight(w as f32))
                    .text_size(px(17.0))
                    .text_color(rgb(if active { theme::INK } else { theme::TEXT }))
                    .when(active, |d| d.bg(rgb(theme::WHITE)).shadow(theme::glow()))
                    .when(!active, |d| d.hover(|s| s.bg(rgb(theme::PANEL_HI))))
                    .child("Aa")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.text_weight = w;
                            this.apply_text_style();
                            cx.notify();
                        }),
                    )
            })
            .collect();
        let steps: Vec<_> = [("−", -1.0f32), ("+", 1.0)]
            .into_iter()
            .map(|(label, delta)| {
                div()
                    .size(px(32.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .cursor(CursorStyle::PointingHand)
                    .text_size(px(18.0))
                    .text_color(rgb(theme::TEXT))
                    .hover(|s| s.bg(rgb(theme::PANEL_HI)))
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| this.start_size_hold(delta, cx)),
                    )
            })
            .collect();
        let [minus, plus]: [gpui::Div; 2] = steps.try_into().ok().unwrap();
        let outline_on = self.text_outline;

        div()
            .h(px(TOOLBAR_H))
            .px(px(7.0))
            .flex()
            .items_center()
            .gap(px(2.0))
            .bg(rgb(theme::PANEL))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .rounded(px(8.0))
            .shadow_lg()
            .occlude()
            .cursor(CursorStyle::Arrow)
            .font_family(theme::UI_FONT)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px(px(8.0))
                    .text_size(px(12.0))
                    .text_color(rgb(theme::MUTED))
                    .child("Montserrat"),
            )
            .children(weights)
            .child(div().w(px(1.0)).h(px(22.0)).mx(px(6.0)).bg(rgb(theme::BORDER)))
            .child(minus)
            .child(
                div()
                    .w(px(38.0))
                    .flex()
                    .flex_none()
                    .justify_center()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT))
                    .child(format!("{}", self.text_size.round() as i32)),
            )
            .child(plus)
            .child(div().w(px(1.0)).h(px(22.0)).mx(px(6.0)).bg(rgb(theme::BORDER)))
            .child(
                div()
                    .h(px(32.0))
                    .px(px(11.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .rounded(px(5.0))
                    .cursor(CursorStyle::PointingHand)
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(if outline_on { theme::INK } else { theme::TEXT }))
                    .when(outline_on, |d| d.bg(rgb(theme::WHITE)).shadow(theme::glow()))
                    .when(!outline_on, |d| d.hover(|s| s.bg(rgb(theme::PANEL_HI))))
                    .child("Outline")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.text_outline = !this.text_outline;
                            this.apply_text_style();
                            cx.notify();
                        }),
                    ),
            )
    }

    fn toolbar_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let sep = || div().w(px(1.0)).h(px(22.0)).mx(px(6.0)).bg(rgb(theme::BORDER));
        let btn = |active: bool| {
            div()
                .size(px(32.0))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .cursor(CursorStyle::PointingHand)
                .when(active, |d| d.bg(rgb(theme::WHITE)).shadow(theme::glow()))
                .when(!active, |d| d.hover(|s| s.bg(rgb(theme::PANEL_HI))))
        };
        let glyph = |name: &str, active: bool, enabled: bool| {
            icon(name).size(px(17.0)).text_color(if active {
                rgb(theme::INK)
            } else if enabled {
                rgb(theme::TEXT)
            } else {
                rgb(0x4A4A4A)
            })
        };

        let tools = TOOLS.iter().map(|(tool, name)| {
            let tool = *tool;
            let active = self.tool == tool;
            btn(active).child(glyph(name, active, true)).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.commit_text();
                    this.tool = tool;
                    cx.notify();
                }),
            )
        });

        let colors = theme::PALETTE.iter().map(|c| {
            let c = *c;
            let active = self.color == c;
            div()
                .size(px(26.0))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor(CursorStyle::PointingHand)
                .border_2()
                .border_color(if active { rgb(0xFFFFFF) } else { rgba(0x00000000) })
                .child(
                    div()
                        .size(px(16.0))
                        .rounded_full()
                        .bg(rgb(c))
                        .border_1()
                        .border_color(rgba(0xFFFFFF30)),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.color = c;
                        if let Some(i) = this.editing.or(this.selected) {
                            this.shapes[i].color = c;
                        }
                        cx.notify();
                    }),
                )
        });

        let widths = WIDTHS.iter().enumerate().map(|(i, w)| {
            let w = *w;
            let active = (self.width - w).abs() < 0.1;
            let dot = 5.0 + i as f32 * 4.0;
            btn(active)
                .child(div().size(px(dot)).rounded_full().bg(if active {
                    rgb(theme::INK)
                } else {
                    rgb(theme::TEXT)
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.width = w;
                        cx.notify();
                    }),
                )
        });

        let can_undo = !self.shapes.is_empty();
        let can_redo = !self.redo.is_empty();

        div()
            .h(px(TOOLBAR_H))
            .px(px(7.0))
            .flex()
            .items_center()
            .gap(px(2.0))
            .bg(rgb(theme::PANEL))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .rounded(px(8.0))
            .shadow_lg()
            .occlude()
            .cursor(CursorStyle::Arrow)
            .font_family(theme::UI_FONT)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .children(tools)
            .child(sep())
            .children(colors)
            .child(sep())
            .children(widths)
            .child(sep())
            .child(btn(false).child(glyph("undo", false, can_undo)).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.undo();
                    cx.notify();
                }),
            ))
            .child(btn(false).child(glyph("redo", false, can_redo)).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.redo();
                    cx.notify();
                }),
            ))
            .child(sep())
            .child(btn(false).child(glyph("close", false, true)).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close(window, cx)),
            ))
            .child(btn(false).child(glyph("pin", false, true)).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.finish(Action::Pin, window, cx)),
            ))
            .child(btn(false).child(glyph("save", false, true)).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.finish(Action::Save, window, cx)),
            ))
            .child(
                div()
                    .ml(px(4.0))
                    .h(px(32.0))
                    .px(px(12.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(7.0))
                    .rounded(px(8.0))
                    .bg(rgb(theme::WHITE))
                    .shadow(theme::glow())
                    .hover(|s| s.bg(rgb(theme::WHITE_HOVER)))
                    .cursor(CursorStyle::PointingHand)
                    .text_color(rgb(theme::INK))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(icon("copy").size(px(15.0)).text_color(rgb(theme::INK)))
                    .child(if self.tool == Tool::Pick { "Copy text" } else { "Copy" })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.copy(window, cx)),
                    ),
            )
    }
}

#[cfg(debug_assertions)]
impl Overlay {
    /// Отладочный сценарий: прогоняет реальные обработчики мыши и клавиатуры синтетическими событиями.
    pub fn selftest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui::{Keystroke, Modifiers};
        let m = Modifiers::default();
        let drag = |this: &mut Self, pts: &[P], window: &mut Window, cx: &mut Context<Self>| {
            this.on_down(
                &MouseDownEvent {
                    button: MouseButton::Left,
                    position: gp(pts[0]),
                    modifiers: m,
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );
            for q in &pts[1..] {
                this.on_move(
                    &MouseMoveEvent {
                        position: gp(*q),
                        pressed_button: Some(MouseButton::Left),
                        modifiers: m,
                    },
                    window,
                    cx,
                );
            }
            this.on_up(
                &MouseUpEvent {
                    button: MouseButton::Left,
                    position: gp(*pts.last().unwrap()),
                    modifiers: m,
                    click_count: 1,
                },
                window,
                cx,
            );
        };
        // GLINT_HOVERTEST: только навести курсор — для снимка подсветки окна и лупы.
        if std::env::var_os("GLINT_HOVERTEST").is_some() {
            let at = MouseMoveEvent { position: gp(p(700.0, 300.0)), pressed_button: None, modifiers: m };
            self.on_move(&at, window, cx);
            cx.notify();
            return;
        }
        // GLINT_SPOTTEST: область, две подсветки, стрелка и условный QR-код — для снимка.
        if std::env::var_os("GLINT_SPOTTEST").is_some() {
            drag(self, &[p(180.0, 140.0), p(600.0, 400.0), p(1130.0, 690.0)], window, cx);
            self.tool = Tool::Spot;
            drag(self, &[p(260.0, 200.0), p(560.0, 360.0)], window, cx);
            drag(self, &[p(500.0, 320.0), p(760.0, 470.0)], window, cx);
            self.tool = Tool::Arrow;
            drag(self, &[p(900.0, 600.0), p(770.0, 480.0)], window, cx);
            self.codes.push(QrHit {
                text: "https://example.com/some/long/link?with=params&and=more-text".into(),
                r: R { x: 880.0, y: 220.0, w: 120.0, h: 120.0 },
            });
            cx.notify();
            return;
        }
        // GLINT_PICKTEST: выделить область и включить режим выделения текста.
        if std::env::var_os("GLINT_PICKTEST").is_some() {
            drag(self, &[p(180.0, 140.0), p(600.0, 400.0), p(1130.0, 690.0)], window, cx);
            self.tool = Tool::Pick;
            cx.notify();
            return;
        }
        // GLINT_OCRTEST: выделить окно под точкой кликом и распознать в нём текст.
        if std::env::var_os("GLINT_OCRTEST").is_some() {
            drag(self, &[p(700.0, 300.0)], window, cx);
            eprintln!("window selection: {:?}", self.sel);
            self.finish(Action::Text, window, cx);
            return;
        }
        let wob = |i: usize| ((i * 7919) % 13) as f32 / 13.0 * 5.0 - 2.5;
        drag(self, &[p(180.0, 140.0), p(600.0, 400.0), p(1130.0, 690.0)], window, cx);

        self.tool = Tool::Pen;
        let circle: Vec<P> = (0..=60)
            .map(|i| {
                let t = i as f32 / 60.0 * std::f32::consts::TAU + 0.3;
                p(330.0 + (75.0 + wob(i)) * t.cos(), 290.0 + (68.0 + wob(i + 3)) * t.sin())
            })
            .collect();
        drag(self, &circle, window, cx);
        let line: Vec<P> = (0..50).map(|i| p(460.0 + i as f32 * 6.0, 230.0 + wob(i))).collect();
        drag(self, &line, window, cx);
        self.color = 0x30D158;
        // «пропись»: петли с дрожанием руки
        let loops: Vec<P> = (0..230)
            .map(|i| {
                let t = i as f32 / 9.0;
                p(
                    470.0 + i as f32 * 1.25 + 20.0 * t.sin() + wob(i) * 0.7,
                    330.0 - 26.0 * t.cos() + wob(i + 1) * 0.7,
                )
            })
            .collect();
        drag(self, &loops, window, cx);
        // зигзаг с острыми углами
        let corners = [(480.0, 440.0), (512.0, 385.0), (544.0, 440.0), (576.0, 385.0), (608.0, 440.0), (660.0, 440.0)];
        let mut zig = Vec::new();
        for (k, c) in corners.windows(2).enumerate() {
            for j in 0..14 {
                let t = j as f32 / 14.0;
                zig.push(p(
                    c[0].0 + (c[1].0 - c[0].0) * t + wob(k * 14 + j) * 0.7,
                    c[0].1 + (c[1].1 - c[0].1) * t + wob(k * 14 + j + 4) * 0.7,
                ));
            }
        }
        drag(self, &zig, window, cx);

        self.color = 0x0A84FF;
        self.tool = Tool::Arrow;
        drag(self, &[p(880.0, 520.0), p(1040.0, 380.0)], window, cx);
        self.tool = Tool::Rect;
        drag(self, &[p(860.0, 200.0), p(1080.0, 330.0)], window, cx);
        self.tool = Tool::Ellipse;
        self.color = 0xFF9F0A;
        drag(self, &[p(240.0, 470.0), p(420.0, 600.0)], window, cx);
        self.tool = Tool::Marker;
        self.color = 0xFFD60A;
        let hl: Vec<P> = (0..40).map(|i| p(700.0 + i as f32 * 4.0, 470.0 + wob(i))).collect();
        drag(self, &hl, window, cx);

        self.tool = Tool::Blur;
        drag(self, &[p(185.0, 350.0), p(420.0, 440.0)], window, cx);
        drag(self, &[p(940.0, 560.0), p(1300.0, 800.0)], window, cx);

        self.tool = Tool::Text;
        self.color = 0xFF3B5C;
        drag(self, &[p(480.0, 560.0), p(480.0, 560.0)], window, cx);
        for ch in "Привет, Glint! Hello".chars() {
            let key = if ch == ' ' { "space".to_string() } else { ch.to_string() };
            self.on_key(
                &KeyDownEvent {
                    keystroke: Keystroke {
                        modifiers: m,
                        key,
                        key_char: (ch != ' ').then(|| ch.to_string()),
                    },
                    is_held: false,
                },
                window,
                cx,
            );
        }
        // правка текста кареткой: Home, Shift+→ ×8, Backspace, End, «!»
        let key = |this: &mut Self, name: &str, shift: bool, ch: Option<&str>, window: &mut Window, cx: &mut Context<Self>| {
            let mut mods = Modifiers::default();
            mods.shift = shift;
            this.on_key(
                &KeyDownEvent {
                    keystroke: Keystroke {
                        modifiers: mods,
                        key: name.to_string(),
                        key_char: ch.map(str::to_string),
                    },
                    is_held: false,
                },
                window,
                cx,
            );
        };
        key(self, "home", false, None, window, cx);
        for _ in 0..8 {
            key(self, "right", true, None, window, cx);
        }
        key(self, "backspace", false, None, window, cx);
        key(self, "end", false, None, window, cx);
        key(self, "!", false, Some("!"), window, cx);
        if let Some(text) = self.edit_text() {
            eprintln!("edited text: {text:?}");
        }
        // готовую надпись: выбрать кликом, сменить начертание/размер/цвет и перетащить
        self.commit_text();
        self.tool = Tool::Select;
        drag(self, &[p(500.0, 562.0), p(470.0, 600.0)], window, cx);
        self.text_weight = 900;
        self.text_size = 34.0;
        self.text_outline = true;
        self.apply_text_style();
        if let Some(i) = self.selected {
            self.shapes[i].color = 0xFFFFFF;
            self.color = 0xFFFFFF;
        }
        // снова в режим набора с выделенным первым словом — для снимка
        self.editing = self.selected;
        self.anchor = 0;
        self.caret = "Glint!".len();
        cx.notify();
    }

    /// Первый Esc завершает ввод текста, второй — копирует выделение и закрывает оверлей.
    pub fn selftest_finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // GLINT_PINTEST: закрепить выделенную область поверх окон.
        if std::env::var_os("GLINT_PINTEST").is_some() {
            self.finish(Action::Pin, window, cx);
            return;
        }
        // GLINT_PICKTEST: протянуть мышью от третьего слова до пятнадцатого и оставить оверлей открытым.
        if std::env::var_os("GLINT_PICKTEST").is_some() {
            let centers: Vec<P> = self
                .ocr
                .iter()
                .flat_map(|o| &o.words)
                .map(|w| p(w.r.x + w.r.w / 2.0, w.r.y + w.r.h / 2.0))
                .collect();
            eprintln!("words: {}", centers.len());
            if centers.len() > 3 {
                let m = gpui::Modifiers::default();
                let (a, b) = (centers[2], centers[15.min(centers.len() - 1)]);
                let down = MouseDownEvent {
                    button: MouseButton::Left,
                    position: gp(a),
                    modifiers: m,
                    click_count: 1,
                    first_mouse: false,
                };
                self.on_down(&down, window, cx);
                let mv = MouseMoveEvent { position: gp(b), pressed_button: Some(MouseButton::Left), modifiers: m };
                self.on_move(&mv, window, cx);
                let up = MouseUpEvent { button: MouseButton::Left, position: gp(b), modifiers: m, click_count: 1 };
                self.on_up(&up, window, cx);
                eprintln!("pick: {:?}", self.pick);
                if std::env::var_os("GLINT_PICKCOPY").is_some() {
                    self.copy(window, cx);
                }
            }
            return;
        }
        for _ in 0..2 {
            if self.closed {
                break;
            }
            let esc = KeyDownEvent {
                keystroke: gpui::Keystroke {
                    modifiers: gpui::Modifiers::default(),
                    key: "escape".into(),
                    key_char: None,
                },
                is_held: false,
            };
            self.on_key(&esc, window, cx);
        }
    }
}

impl Render for Overlay {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        self.ensure_blurs();
        self.ensure_ocr(cx);
        let snap = self.snapshot();
        let pointer = if self.tool == Tool::Pick && self.sel.is_some() {
            CursorStyle::IBeam
        } else {
            CursorStyle::Crosshair
        };
        let show_toolbar = self.sel.is_some() && !matches!(self.drag, Drag::NewSel { .. });
        let toolbar = self
            .sel
            .filter(|_| show_toolbar)
            .map(|sel| self.render_toolbar(&sel, cx));
        let (ww, wh) = self.win_size();
        let chips: Vec<_> = self
            .visible_codes()
            .into_iter()
            .map(|(i, r)| {
                let text = clip(&self.codes[i].text, 44);
                let y = if r.bottom() + 40.0 <= wh { r.bottom() + 6.0 } else { r.y - 36.0 };
                div()
                    .absolute()
                    .left(px(r.x.clamp(4.0, (ww - 120.0).max(4.0))))
                    .top(px(y.max(4.0)))
                    .h(px(30.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .rounded(px(7.0))
                    .bg(rgb(theme::PANEL))
                    .border_1()
                    .border_color(rgb(QR_ACCENT))
                    .shadow_lg()
                    .occlude()
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(rgb(theme::PANEL_HI)))
                    .font_family(theme::UI_FONT)
                    .text_size(px(12.0))
                    .text_color(rgb(theme::TEXT))
                    .child(icon("qr").size(px(18.0)).text_color(rgb(QR_ACCENT)))
                    .child(text)
                    .child(icon("copy").size(px(14.0)).text_color(rgb(theme::TEXT)))
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.copy_code(i, window, cx);
                        }),
                    )
            })
            .collect();
        let hint = self.sel.is_none().then(|| {
            div()
                .absolute()
                .top(px(28.0))
                .left_0()
                .w_full()
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(16.0))
                        .py(px(9.0))
                        .rounded(px(7.0))
                        .bg(rgba(0x000000F0))
                        .border_1()
                        .border_color(rgb(theme::BORDER))
                        .shadow_lg()
                        .font_family(theme::UI_FONT)
                        .text_size(px(13.0))
                        .text_color(rgb(theme::TEXT))
                        .child("Drag to select   ·   Ctrl+A — full screen   ·   Esc — exit"),
                )
        });

        div()
            .size_full()
            .relative()
            .track_focus(&self.focus)
            .cursor(pointer)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let e = entity.clone();
                        window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble {
                                e.update(cx, |this, cx| this.on_move(ev, window, cx));
                            }
                        });
                        let e = entity.clone();
                        window.on_mouse_event(move |ev: &MouseUpEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble && ev.button == MouseButton::Left {
                                e.update(cx, |this, cx| this.on_up(ev, window, cx));
                            }
                        });
                        paint(&snap, bounds, window, cx);
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(chips)
            .children(toolbar)
            .children(hint)
    }
}

fn rect(r: &R) -> Bounds<Pixels> {
    Bounds {
        origin: point(px(r.x), px(r.y)),
        size: size(px(r.w), px(r.h)),
    }
}

fn build_path(outline: &Outline, width: f32) -> Option<Path<Pixels>> {
    let mut b = if outline.fill {
        PathBuilder::fill()
    } else {
        PathBuilder::stroke(px(width)).with_style(PathStyle::Stroke(
            StrokeOptions::default()
                .with_line_width(width)
                .with_line_cap(LineCap::Round)
                .with_line_join(LineJoin::Round),
        ))
    };
    for seg in &outline.segs {
        match *seg {
            Seg::Move(q) => b.move_to(gp(q)),
            Seg::Line(q) => b.line_to(gp(q)),
            Seg::Cubic(a, c, q) => b.cubic_bezier_to(gp(q), gp(a), gp(c)),
            Seg::Close => b.close(),
        }
    }
    b.build().ok()
}

fn prev_char(text: &str, i: usize) -> usize {
    text[..i].char_indices().next_back().map(|(j, _)| j).unwrap_or(0)
}

fn next_char(text: &str, i: usize) -> usize {
    i + text[i..].chars().next().map(char::len_utf8).unwrap_or(0)
}

fn word_left(text: &str, i: usize) -> usize {
    let mut j = i;
    let mut seen_word = false;
    for (k, ch) in text[..i].char_indices().rev() {
        if ch.is_whitespace() {
            if seen_word {
                break;
            }
        } else {
            seen_word = true;
        }
        j = k;
    }
    j
}

fn word_right(text: &str, i: usize) -> usize {
    let mut seen_space = false;
    for (k, ch) in text[i..].char_indices() {
        if ch.is_whitespace() {
            seen_space = true;
        } else if seen_space {
            return i + k;
        }
    }
    text.len()
}

fn line_start(text: &str, i: usize) -> usize {
    text[..i].rfind('\n').map(|j| j + 1).unwrap_or(0)
}

fn line_end(text: &str, i: usize) -> usize {
    text[i..].find('\n').map(|j| i + j).unwrap_or(text.len())
}

/// Каретка на строку выше/ниже с сохранением номера символа в строке.
fn vertical(text: &str, i: usize, up: bool) -> usize {
    let ls = line_start(text, i);
    let col = text[ls..i].chars().count();
    let target = if up {
        if ls == 0 {
            return 0;
        }
        line_start(text, ls - 1)
    } else {
        let le = line_end(text, i);
        if le == text.len() {
            return text.len();
        }
        le + 1
    };
    let end = line_end(text, target);
    text[target..end]
        .char_indices()
        .nth(col)
        .map(|(k, _)| target + k)
        .unwrap_or(end)
}

fn shape_note(line: &str, size: f32, weight: u16, c: Hsla, window: &Window) -> gpui::ShapedLine {
    window.text_system().shape_line(
        SharedString::from(line.to_string()),
        px(size),
        &[TextRun {
            len: line.len(),
            font: note_font(weight),
            color: c,
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    )
}

/// Позиция в тексте (в байтах), ближайшая к точке `q`.
fn text_index_at(shape: &Shape, q: P, window: &Window) -> usize {
    let Kind::Text {
        at,
        text,
        size,
        weight,
        ..
    } = &shape.kind
    else {
        return 0;
    };
    let line_h = size * TEXT_LINE_HEIGHT;
    let count = text.split('\n').count();
    let row = (((q.y - at.y) / line_h).floor().max(0.0) as usize).min(count - 1);
    let mut offset = 0;
    for (k, line) in text.split('\n').enumerate() {
        if k == row {
            let shaped = shape_note(line, *size, *weight, color(0xFFFFFF, 1.0), window);
            return offset + shaped.closest_index_for_x(px(q.x - at.x));
        }
        offset += line.len() + 1;
    }
    text.len()
}

/// Шрифт текстовых пометок: Montserrat нужного веса.
fn note_font(weight: u16) -> Font {
    Font {
        weight: FontWeight(weight as f32),
        ..font(fonts::FAMILY)
    }
}

/// Рамка надписи (с небольшим полем) — для попадания мышью и подсветки выбора.
fn text_bounds(shape: &Shape, window: &Window) -> Option<R> {
    let Kind::Text {
        at,
        text,
        size,
        weight,
        ..
    } = &shape.kind
    else {
        return None;
    };
    let mut w = size * 0.6;
    let mut lines = 0;
    for line in text.split('\n') {
        lines += 1;
        if line.is_empty() {
            continue;
        }
        let shaped = window.text_system().shape_line(
            SharedString::from(line.to_string()),
            px(*size),
            &[TextRun {
                len: line.len(),
                font: note_font(*weight),
                color: color(0xFFFFFF, 1.0),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        w = w.max(f32::from(shaped.width));
    }
    Some(R {
        x: at.x - 6.0,
        y: at.y - 3.0,
        w: w + 12.0,
        h: lines as f32 * size * TEXT_LINE_HEIGHT + 6.0,
    })
}

fn text_font(weight: FontWeight) -> Font {
    Font {
        weight,
        ..font(theme::UI_FONT)
    }
}

fn paint(snap: &Snap, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let _ = window.paint_image(bounds, Corners::default(), snap.image.clone(), 0, false);
    let (ww, wh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));

    // Затемнение вне выделения.
    let dim = color(0x000000, if snap.sel.is_some() { 0.5 } else { 0.45 });
    match snap.sel.as_ref().or(snap.hover.as_ref()) {
        None => window.paint_quad(fill(bounds, color(0x000000, 0.35))),
        Some(s) => {
            let parts = [
                R { x: 0.0, y: 0.0, w: ww, h: s.y },
                R { x: 0.0, y: s.bottom(), w: ww, h: wh - s.bottom() },
                R { x: 0.0, y: s.y, w: s.x, h: s.h },
                R { x: s.right(), y: s.y, w: ww - s.right(), h: s.h },
            ];
            for part in parts.iter().filter(|r| r.w > 0.0 && r.h > 0.0) {
                window.paint_quad(fill(rect(part), dim));
            }
        }
    }

    // Аннотации — только внутри выделенной области.
    if let Some(sel) = &snap.sel {
        window.with_content_mask(Some(ContentMask { bounds: rect(sel) }), |window| {
            // Размытие — под остальными аннотациями, как и в готовом снимке.
            for shape in &snap.shapes {
                if let Kind::Blur(a, b) = &shape.kind {
                    let key = shape.width.to_bits();
                    if let Some(layer) = snap.blurs.iter().find(|l| l.key == key) {
                        let area = rect(&R::from_points(*a, *b));
                        let full = Bounds {
                            origin: bounds.origin,
                            size: size(px(layer.w), px(layer.h)),
                        };
                        window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
                            let _ = window.paint_image(
                                full,
                                Corners::default(),
                                layer.image.clone(),
                                0,
                                false,
                            );
                        });
                    }
                }
            }
            // Подсветка: затемняем область вокруг отмеченных прямоугольников.
            let spots: Vec<&Shape> =
                snap.shapes.iter().filter(|s| matches!(s.kind, Kind::Spot(..))).collect();
            if !spots.is_empty() {
                let holes: Vec<R> = spots
                    .iter()
                    .filter_map(|s| match &s.kind {
                        Kind::Spot(a, b) => Some(R::from_points(*a, *b)),
                        _ => None,
                    })
                    .collect();
                let alpha = spots.iter().map(|s| shapes::spot_alpha(s.width)).fold(0.0, f32::max);
                for part in shapes::spot_dim(*sel, &holes) {
                    window.paint_quad(fill(rect(&part), color(0x000000, alpha)));
                }
            }
            for (i, shape) in snap.shapes.iter().enumerate() {
                let c = color(shape.color, shape.alpha);
                if matches!(shape.kind, Kind::Blur(..) | Kind::Spot(..)) {
                    continue;
                }
                if let Kind::Text {
                    at,
                    text,
                    size,
                    weight,
                    outline,
                } = &shape.kind
                {
                    let line_h = size * TEXT_LINE_HEIGHT;
                    let mut caret = p(at.x, at.y);
                    let editing = snap.editing == Some(i);
                    let (sa, sb) = (snap.caret.min(snap.anchor), snap.caret.max(snap.anchor));
                    let mut offset = 0usize;
                    for (li, line) in text.split('\n').enumerate() {
                        let y = at.y + li as f32 * line_h;
                        let shaped = window.text_system().shape_line(
                            SharedString::from(line.to_string()),
                            px(*size),
                            &[TextRun {
                                len: line.len(),
                                font: note_font(*weight),
                                color: c,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            }],
                            None,
                        );
                        let (ls, le) = (offset, offset + line.len());
                        offset = le + 1;
                        if editing {
                            // подсветка выделенного фрагмента
                            if sa < sb && sa <= le && sb >= ls {
                                let x0 = f32::from(shaped.x_for_index(sa.max(ls) - ls));
                                let mut x1 = f32::from(shaped.x_for_index(sb.min(le) - ls));
                                if sb > le {
                                    x1 += size * 0.3;
                                }
                                window.paint_quad(fill(
                                    rect(&R {
                                        x: at.x + x0,
                                        y,
                                        w: (x1 - x0).max(0.0),
                                        h: line_h,
                                    }),
                                    color(0x2F7BFF, 0.6),
                                ));
                            }
                            if snap.caret >= ls && snap.caret <= le {
                                caret = p(at.x + f32::from(shaped.x_for_index(snap.caret - ls)), y);
                            }
                        }
                        if *outline {
                            let stroke = window.text_system().shape_line(
                                SharedString::from(line.to_string()),
                                px(*size),
                                &[TextRun {
                                    len: line.len(),
                                    font: note_font(*weight),
                                    color: color(shapes::outline_color(shape.color), 1.0),
                                    background_color: None,
                                    underline: None,
                                    strikethrough: None,
                                }],
                                None,
                            );
                            for (dx, dy) in shapes::outline_offsets(shapes::outline_radius(*size)) {
                                let _ = stroke.paint(
                                    point(px(at.x + dx), px(y + dy)),
                                    px(line_h),
                                    window,
                                    cx,
                                );
                            }
                        }
                        let _ = shaped.paint(point(px(at.x), px(y)), px(line_h), window, cx);
                    }
                    if snap.editing == Some(i) {
                        window.paint_quad(fill(
                            rect(&R {
                                x: caret.x - 1.0,
                                y: caret.y + line_h * 0.12,
                                w: 2.0,
                                h: line_h * 0.76,
                            }),
                            c,
                        ));
                    }
                    if snap.editing == Some(i) || snap.selected == Some(i) {
                        if let Some(b) = text_bounds(shape, window) {
                            window.paint_quad(quad(
                                rect(&b),
                                px(4.0),
                                color(0, 0.0),
                                px(1.0),
                                color(0xFFFFFF, 0.75),
                                BorderStyle::Dashed,
                            ));
                        }
                    }
                    continue;
                }
                for outline in shape.outlines() {
                    if let Some(path) = build_path(&outline, shape.width) {
                        window.paint_path(path, c);
                    }
                }
            }
            // Режим текста: распознанные слова подсвечены, выделенные — залиты по строкам.
            if snap.pick_mode {
                for w in &snap.words {
                    window.paint_quad(quad(
                        rect(&R { x: w.r.x - 1.5, y: w.r.y - 1.5, w: w.r.w + 3.0, h: w.r.h + 3.0 }),
                        px(3.0),
                        color(0xFFFFFF, 0.16),
                        px(0.0),
                        color(0, 0.0),
                        BorderStyle::default(),
                    ));
                }
                if let Some((a, b)) = snap.pick {
                    let (a, b) = (a.min(b), a.max(b).min(snap.words.len().saturating_sub(1)));
                    let mut i = a;
                    while i <= b && i < snap.words.len() {
                        let line = snap.words[i].line;
                        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                        while i <= b && snap.words[i].line == line {
                            let r = &snap.words[i].r;
                            x0 = x0.min(r.x);
                            y0 = y0.min(r.y);
                            x1 = x1.max(r.right());
                            y1 = y1.max(r.bottom());
                            i += 1;
                        }
                        window.paint_quad(quad(
                            rect(&R { x: x0 - 2.0, y: y0 - 2.0, w: x1 - x0 + 4.0, h: y1 - y0 + 4.0 }),
                            px(3.0),
                            color(0x2F7BFF, 0.45),
                            px(0.0),
                            color(0, 0.0),
                            BorderStyle::default(),
                        ));
                    }
                }
            }
        });
        if let Some(note) = snap.note {
            paint_label(note, p(sel.x + 8.0, sel.y + 8.0), window, cx);
        }
    }
    // Найденные QR-коды обведены рамкой — и до выбора области, и внутри неё.
    for r in &snap.codes {
        window.paint_quad(quad(
            rect(&R { x: r.x - 4.0, y: r.y - 4.0, w: r.w + 8.0, h: r.h + 8.0 }),
            px(5.0),
            color(0, 0.0),
            px(2.0),
            color(QR_ACCENT, 1.0),
            BorderStyle::default(),
        ));
    }

    let accent = color(theme::WHITE, 1.0);
    match &snap.sel {
        Some(sel) => {
            // Рамка выделения.
            window.paint_quad(quad(
                rect(&R {
                    x: sel.x - 1.0,
                    y: sel.y - 1.0,
                    w: sel.w + 2.0,
                    h: sel.h + 2.0,
                }),
                px(0.0),
                color(0, 0.0),
                px(1.5),
                accent,
                BorderStyle::default(),
            ));
            if !snap.selecting {
                for h in Overlay::handles(sel) {
                    window.paint_quad(quad(
                        rect(&R {
                            x: h.x - 4.5,
                            y: h.y - 4.5,
                            w: 9.0,
                            h: 9.0,
                        }),
                        px(4.5),
                        color(0xFFFFFF, 1.0),
                        px(1.5),
                        color(0x000000, 1.0),
                        BorderStyle::default(),
                    ));
                }
            }
            // Размер области в пикселях снимка.
            let label = format!(
                "{} × {}",
                (sel.w * snap.scale).round() as u32,
                (sel.h * snap.scale).round() as u32
            );
            let ly = if sel.y >= 34.0 { sel.y - 30.0 } else { sel.y + 8.0 };
            let lx = if sel.y >= 34.0 { sel.x } else { sel.x + 8.0 };
            paint_label(&label, p(lx, ly), window, cx);
        }
        None => {
            // Окно под курсором: клик выделит его целиком.
            if let Some(h) = &snap.hover {
                // лёгкая вуаль, чтобы и на полноэкранном окне было видно, что экран «заморожен»
                window.paint_quad(fill(rect(h), color(0x000000, 0.12)));
                window.paint_quad(quad(
                    rect(h),
                    px(0.0),
                    color(0, 0.0),
                    px(2.0),
                    accent,
                    BorderStyle::default(),
                ));
            }
            // Перекрестие до начала выделения.
            let line = color(0xFFFFFF, 0.35);
            let c = snap.cursor;
            if c.x >= 0.0 && c.y >= 0.0 {
                window.paint_quad(fill(rect(&R { x: 0.0, y: c.y, w: ww, h: 1.0 }), line));
                window.paint_quad(fill(rect(&R { x: c.x, y: 0.0, w: 1.0, h: wh }), line));
            }
        }
    }
    if snap.selecting {
        paint_loupe(snap, ww, wh, window, cx);
    }
}

/// Слово, в рамку которого (с небольшим запасом) попадает точка.
fn word_hit(words: &[PickWord], q: P) -> Option<usize> {
    words.iter().position(|w| {
        q.x >= w.r.x - 3.0 && q.x <= w.r.right() + 3.0 && q.y >= w.r.y - 3.0 && q.y <= w.r.bottom() + 3.0
    })
}

/// Ближайшее к точке слово: сначала выбирается строка по вертикали, затем слово в ней.
fn word_near(words: &[PickWord], q: P) -> Option<usize> {
    let gap = |lo: f32, hi: f32, v: f32| (lo - v).max(v - hi).max(0.0);
    (0..words.len()).min_by(|&i, &j| {
        let score = |k: usize| {
            let r = &words[k].r;
            gap(r.y, r.bottom(), q.y) * 6.0 + gap(r.x, r.right(), q.x)
        };
        score(i).total_cmp(&score(j))
    })
}

/// Цвет пикселя снимка (BGRA) в виде 0xRRGGBB.
fn pixel(bytes: &[u8], w: i32, h: i32, x: i32, y: i32) -> Option<u32> {
    if x < 0 || y < 0 || x >= w || y >= h {
        return None;
    }
    let i = (y as usize * w as usize + x as usize) * 4;
    let px = bytes.get(i..i + 3)?;
    Some((px[2] as u32) << 16 | (px[1] as u32) << 8 | px[0] as u32)
}

/// Лупа у курсора: увеличенные пиксели снимка, центральный обведён, под ними — его цвет.
fn paint_loupe(snap: &Snap, ww: f32, wh: f32, window: &mut Window, cx: &mut App) {
    let c = snap.cursor;
    if c.x < 0.0 || c.y < 0.0 || c.x > ww || c.y > wh {
        return;
    }
    let Some(bytes) = snap.image.as_bytes(0) else { return };
    let (iw, ih) = (snap.img_w as i32, snap.img_h as i32);
    let cx_px = ((c.x * snap.scale) as i32).clamp(0, iw - 1);
    let cy_px = ((c.y * snap.scale) as i32).clamp(0, ih - 1);
    let half = LOUPE_CELLS / 2;
    let side = LOUPE_CELLS as f32 * LOUPE_CELL;
    let total_h = side + 28.0;

    let mut x = c.x + 24.0;
    if x + side + 8.0 > ww {
        x = c.x - 24.0 - side;
    }
    let mut y = c.y + 24.0;
    if y + total_h + 8.0 > wh {
        y = c.y - 24.0 - total_h;
    }
    // по границам физических пикселей, чтобы между клетками не было щелей
    let snap_px = |v: f32| (v * snap.scale).round() / snap.scale;
    let (x, y) = (snap_px(x), snap_px(y));

    window.paint_quad(quad(
        rect(&R { x: x - 3.0, y: y - 3.0, w: side + 6.0, h: total_h + 6.0 }),
        px(6.0),
        color(theme::PANEL, 1.0),
        px(1.0),
        color(0xFFFFFF, 0.85),
        BorderStyle::default(),
    ));
    for j in 0..LOUPE_CELLS {
        for i in 0..LOUPE_CELLS {
            let Some(col) = pixel(bytes, iw, ih, cx_px + i - half, cy_px + j - half) else {
                continue;
            };
            window.paint_quad(fill(
                rect(&R {
                    x: x + i as f32 * LOUPE_CELL,
                    y: y + j as f32 * LOUPE_CELL,
                    w: LOUPE_CELL,
                    h: LOUPE_CELL,
                }),
                color(col, 1.0),
            ));
        }
    }
    let (mx, my) = (x + half as f32 * LOUPE_CELL, y + half as f32 * LOUPE_CELL);
    for (grow, col) in [(2.0, 0x000000), (1.0, 0xFFFFFF)] {
        window.paint_quad(quad(
            rect(&R {
                x: mx - grow,
                y: my - grow,
                w: LOUPE_CELL + grow * 2.0,
                h: LOUPE_CELL + grow * 2.0,
            }),
            px(0.0),
            color(0, 0.0),
            px(1.0),
            color(col, 1.0),
            BorderStyle::default(),
        ));
    }

    let col = pixel(bytes, iw, ih, cx_px, cy_px).unwrap_or(0);
    window.paint_quad(quad(
        rect(&R { x: x + 4.0, y: y + side + 7.0, w: 14.0, h: 14.0 }),
        px(3.0),
        color(col, 1.0),
        px(1.0),
        color(0xFFFFFF, 0.5),
        BorderStyle::default(),
    ));
    let label = format!("#{col:06X}");
    let shaped = window.text_system().shape_line(
        SharedString::from(label.clone()),
        px(12.0),
        &[TextRun {
            len: label.len(),
            font: text_font(FontWeight::MEDIUM),
            color: color(0xFFFFFF, 1.0),
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    );
    let _ = shaped.paint(point(px(x + 25.0), px(y + side + 5.0)), px(18.0), window, cx);
}

/// Обрезает текст до `max` символов, заменяя переводы строк пробелами.
fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(max).collect();
    if flat.chars().count() > max {
        out.push('…');
    }
    out
}

fn paint_label(text: &str, at: P, window: &mut Window, cx: &mut App) {
    let shaped = window.text_system().shape_line(
        SharedString::from(text.to_string()),
        px(12.0),
        &[TextRun {
            len: text.len(),
            font: text_font(FontWeight::MEDIUM),
            color: color(0xFFFFFF, 1.0),
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    );
    let w = f32::from(shaped.width) + 16.0;
    window.paint_quad(quad(
        rect(&R { x: at.x, y: at.y, w, h: 22.0 }),
        px(6.0),
        color(theme::PANEL, 0.94),
        px(0.0),
        color(0, 0.0),
        BorderStyle::default(),
    ));
    let _ = shaped.paint(point(px(at.x + 8.0), px(at.y + 2.0)), px(18.0), window, cx);
}
