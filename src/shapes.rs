//! Модель аннотаций и геометрия: сглаживание рукописных штрихов и распознавание фигур.
//! Все координаты — логические пиксели окна оверлея.

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct P {
    pub x: f32,
    pub y: f32,
}

pub fn p(x: f32, y: f32) -> P {
    P { x, y }
}

impl P {
    pub fn dist(self, o: P) -> f32 {
        (self.x - o.x).hypot(self.y - o.y)
    }
    fn lerp(self, o: P, t: f32) -> P {
        p(self.x + (o.x - self.x) * t, self.y + (o.y - self.y) * t)
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub fn from_points(a: P, b: P) -> R {
        R {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
            w: (a.x - b.x).abs(),
            h: (a.y - b.y).abs(),
        }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, q: P) -> bool {
        q.x >= self.x && q.x <= self.right() && q.y >= self.y && q.y <= self.bottom()
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Kind {
    /// Свободный штрих (точки уже сглажены), рисуется сплайном.
    Stroke(Vec<P>),
    /// «Причёсанный» штрих: ломаная из ровных отрезков со скруглёнными углами.
    Curve(Vec<P>),
    Line(P, P),
    Arrow(P, P),
    Rect(P, P),
    Ellipse(P, P),
    /// Размытый прямоугольник: скрывает то, что под ним на снимке.
    Blur(P, P),
    /// Подсветка: всё вокруг этого прямоугольника затемняется.
    Spot(P, P),
    Text { at: P, text: String, size: f32, weight: u16, outline: bool },
}

#[derive(Clone, PartialEq, Debug)]
pub struct Shape {
    pub kind: Kind,
    pub color: u32,
    pub width: f32,
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug)]
pub enum Seg {
    Move(P),
    Line(P),
    /// (control_a, control_b, to)
    Cubic(P, P, P),
    Close,
}

/// Контур для отрисовки: либо обводка заданной толщины, либо заливка.
pub struct Outline {
    pub segs: Vec<Seg>,
    pub fill: bool,
}

pub const TEXT_LINE_HEIGHT: f32 = 1.3;

/// Сила затемнения вокруг подсвеченных областей для выбранной толщины инструмента.
pub fn spot_alpha(width: f32) -> f32 {
    (0.3 + width * 0.045).min(0.75)
}

/// Прямоугольники, покрывающие `area` за вычетом всех `holes` (для затемнения вокруг подсветки).
pub fn spot_dim(area: R, holes: &[R]) -> Vec<R> {
    let mut ys = vec![area.y, area.bottom()];
    for h in holes {
        ys.push(h.y.clamp(area.y, area.bottom()));
        ys.push(h.bottom().clamp(area.y, area.bottom()));
    }
    ys.sort_by(f32::total_cmp);
    ys.dedup();
    let mut out = Vec::new();
    for band in ys.windows(2) {
        let (y0, y1) = (band[0], band[1]);
        let mid = (y0 + y1) / 2.0;
        // отрезки по X, закрытые подсветкой в этой полосе
        let mut cuts: Vec<(f32, f32)> = holes
            .iter()
            .filter(|h| h.y <= mid && h.bottom() >= mid)
            .map(|h| (h.x.max(area.x), h.right().min(area.right())))
            .filter(|(a, b)| b > a)
            .collect();
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut x = area.x;
        for (a, b) in cuts {
            if a > x {
                out.push(R { x, y: y0, w: a - x, h: y1 - y0 });
            }
            x = x.max(b);
        }
        if area.right() > x {
            out.push(R { x, y: y0, w: area.right() - x, h: y1 - y0 });
        }
    }
    out
}

/// Число смещённых копий текста, из которых складывается обводка.
pub const OUTLINE_STEPS: usize = 20;

/// Толщина обводки текста для данного кегля.
pub fn outline_radius(size: f32) -> f32 {
    (size * 0.075).clamp(1.5, 7.0)
}

/// Контрастный цвет обводки: чёрный для светлого текста, белый для тёмного.
pub fn outline_color(c: u32) -> u32 {
    let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    let luma = (r * 299 + g * 587 + b * 114) / 1000;
    if luma > 110 { 0x000000 } else { 0xFFFFFF }
}

/// Смещения копий текста для обводки радиуса `r`.
pub fn outline_offsets(r: f32) -> impl Iterator<Item = (f32, f32)> {
    (0..OUTLINE_STEPS).map(move |k| {
        let a = k as f32 / OUTLINE_STEPS as f32 * std::f32::consts::TAU;
        (a.cos() * r, a.sin() * r)
    })
}

impl Shape {
    pub fn outlines(&self) -> Vec<Outline> {
        match &self.kind {
            Kind::Stroke(pts) => vec![Outline {
                segs: spline(pts),
                fill: false,
            }],
            Kind::Curve(pts) => vec![Outline {
                segs: rounded(pts),
                fill: false,
            }],
            Kind::Line(a, b) => vec![Outline {
                segs: vec![Seg::Move(*a), Seg::Line(*b)],
                fill: false,
            }],
            Kind::Arrow(a, b) => arrow(*a, *b, self.width),
            Kind::Rect(a, b) => {
                let r = R::from_points(*a, *b);
                vec![Outline {
                    segs: vec![
                        Seg::Move(p(r.x, r.y)),
                        Seg::Line(p(r.right(), r.y)),
                        Seg::Line(p(r.right(), r.bottom())),
                        Seg::Line(p(r.x, r.bottom())),
                        Seg::Close,
                    ],
                    fill: false,
                }]
            }
            Kind::Ellipse(a, b) => {
                let r = R::from_points(*a, *b);
                let (cx, cy, rx, ry) = (r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, r.h / 2.0);
                const K: f32 = 0.552_284_8;
                let (kx, ky) = (rx * K, ry * K);
                vec![Outline {
                    segs: vec![
                        Seg::Move(p(cx + rx, cy)),
                        Seg::Cubic(p(cx + rx, cy + ky), p(cx + kx, cy + ry), p(cx, cy + ry)),
                        Seg::Cubic(p(cx - kx, cy + ry), p(cx - rx, cy + ky), p(cx - rx, cy)),
                        Seg::Cubic(p(cx - rx, cy - ky), p(cx - kx, cy - ry), p(cx, cy - ry)),
                        Seg::Cubic(p(cx + kx, cy - ry), p(cx + rx, cy - ky), p(cx + rx, cy)),
                        Seg::Close,
                    ],
                    fill: false,
                }]
            }
            Kind::Blur(..) | Kind::Spot(..) | Kind::Text { .. } => vec![],
        }
    }
}

fn arrow(a: P, b: P, width: f32) -> Vec<Outline> {
    let len = a.dist(b);
    if len < 0.5 {
        return vec![];
    }
    let (ux, uy) = ((b.x - a.x) / len, (b.y - a.y) / len);
    let head = (width * 4.5).max(12.0).min(len * 0.6);
    let half = head * 0.42;
    let base = p(b.x - ux * head, b.y - uy * head);
    let shaft_end = p(b.x - ux * head * 0.8, b.y - uy * head * 0.8);
    let l = p(base.x - uy * half, base.y + ux * half);
    let r = p(base.x + uy * half, base.y - ux * half);
    vec![
        Outline {
            segs: vec![Seg::Move(a), Seg::Line(shaft_end)],
            fill: false,
        },
        Outline {
            segs: vec![Seg::Move(b), Seg::Line(l), Seg::Line(r), Seg::Close],
            fill: true,
        },
    ]
}

/// Сплайн Катмулла—Рома через точки, в виде кубических кривых Безье.
fn spline(pts: &[P]) -> Vec<Seg> {
    match pts.len() {
        0 => vec![],
        1 => vec![Seg::Move(pts[0]), Seg::Line(p(pts[0].x + 0.01, pts[0].y))],
        2 => vec![Seg::Move(pts[0]), Seg::Line(pts[1])],
        n => {
            let mut segs = Vec::with_capacity(n);
            segs.push(Seg::Move(pts[0]));
            for i in 0..n - 1 {
                let p0 = pts[i.saturating_sub(1)];
                let p1 = pts[i];
                let p2 = pts[i + 1];
                let p3 = pts[(i + 2).min(n - 1)];
                let c1 = p(p1.x + (p2.x - p0.x) / 6.0, p1.y + (p2.y - p0.y) / 6.0);
                let c2 = p(p2.x - (p3.x - p1.x) / 6.0, p2.y - (p3.y - p1.y) / 6.0);
                segs.push(Seg::Cubic(c1, c2, p2));
            }
            segs
        }
    }
}

/// Ломаная со скруглёнными углами. Плавные повороты сглаживаются целиком (как B-сплайн),
/// резкие углы получают аккуратное небольшое скругление, длинные отрезки остаются прямыми.
fn rounded(pts: &[P]) -> Vec<Seg> {
    let n = pts.len();
    if n < 3 {
        return spline(pts);
    }
    let mut segs = vec![Seg::Move(pts[0])];
    for i in 1..n - 1 {
        let (a, v, b) = (pts[i - 1], pts[i], pts[i + 1]);
        let (la, lb) = (a.dist(v), v.dist(b));
        if la < 1e-3 || lb < 1e-3 {
            continue;
        }
        let cos = ((v.x - a.x) * (b.x - v.x) + (v.y - a.y) * (b.y - v.y)) / (la * lb);
        let turn = cos.clamp(-1.0, 1.0).acos().to_degrees();
        let cap = if turn < 75.0 { f32::MAX } else { 18.0 };
        let da = (la * 0.5).min(cap);
        let db = (lb * 0.5).min(cap);
        let p_in = v.lerp(a, da / la);
        let p_out = v.lerp(b, db / lb);
        segs.push(Seg::Line(p_in));
        // квадратичная кривая с вершиной в углу, записанная кубической
        segs.push(Seg::Cubic(p_in.lerp(v, 2.0 / 3.0), p_out.lerp(v, 2.0 / 3.0), p_out));
    }
    segs.push(Seg::Line(pts[n - 1]));
    segs
}

/// «Причёсывает» рукописный штрих: убирает дрожание и раскладывает его на ровные отрезки,
/// которые затем рисуются со скруглёнными углами.
pub fn beautify(raw: &[P]) -> Vec<P> {
    if raw.len() < 3 {
        return raw.to_vec();
    }
    let mut pts = resample(raw, 2.0);
    let n = pts.len();
    if n >= 5 {
        for _ in 0..9 {
            let src = pts.clone();
            for i in 1..n - 1 {
                let a = src[i.saturating_sub(2)];
                let b = src[i - 1];
                let c = src[i];
                let d = src[i + 1];
                let e = src[(i + 2).min(n - 1)];
                pts[i] = p(
                    (a.x + 4.0 * b.x + 6.0 * c.x + 4.0 * d.x + e.x) / 16.0,
                    (a.y + 4.0 * b.y + 6.0 * c.y + 4.0 * d.y + e.y) / 16.0,
                );
            }
        }
    }
    let mut out = rdp(&pts, 3.4);
    // замкнутый штрих (буква «о», петля) смыкаем точно
    if out.len() > 3 && path_len(&out) > 50.0 && out[0].dist(out[out.len() - 1]) < 7.0 {
        let last = out.len() - 1;
        out[last] = out[0];
    }
    out
}

fn path_len(pts: &[P]) -> f32 {
    pts.windows(2).map(|w| w[0].dist(w[1])).sum()
}

/// Равномерная передискретизация ломаной с шагом `step`.
fn resample(pts: &[P], step: f32) -> Vec<P> {
    if pts.len() < 2 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    let mut carry = 0.0;
    for w in pts.windows(2) {
        let d = w[0].dist(w[1]);
        if d <= f32::EPSILON {
            continue;
        }
        let mut t = step - carry;
        while t <= d {
            out.push(w[0].lerp(w[1], t / d));
            t += step;
        }
        carry = d - (t - step);
    }
    let last = *pts.last().unwrap();
    if out.last().unwrap().dist(last) > 0.3 {
        out.push(last);
    }
    out
}

/// Упрощение Рамера—Дугласа—Пекера.
fn rdp(pts: &[P], eps: f32) -> Vec<P> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (mut worst, mut idx) = (0.0f32, 0usize);
        for i in a + 1..b {
            let d = seg_dist(pts[i], pts[a], pts[b]);
            if d > worst {
                worst = d;
                idx = i;
            }
        }
        if worst > eps {
            keep[idx] = true;
            stack.push((a, idx));
            stack.push((idx, b));
        }
    }
    pts.iter()
        .zip(keep)
        .filter_map(|(q, k)| k.then_some(*q))
        .collect()
}

fn seg_dist(q: P, a: P, b: P) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l2 = dx * dx + dy * dy;
    if l2 < 1e-6 {
        return q.dist(a);
    }
    let t = (((q.x - a.x) * dx + (q.y - a.y) * dy) / l2).clamp(0.0, 1.0);
    q.dist(p(a.x + dx * t, a.y + dy * t))
}

/// Сглаживает рукописный штрих: убирает дрожание мыши, сохраняя форму букв.
pub fn smooth(raw: &[P]) -> Vec<P> {
    if raw.len() < 3 {
        return raw.to_vec();
    }
    let mut pts = resample(raw, 2.5);
    let n = pts.len();
    if n >= 5 {
        for _ in 0..9 {
            let src = pts.clone();
            for i in 1..n - 1 {
                let a = src[i.saturating_sub(2)];
                let b = src[i - 1];
                let c = src[i];
                let d = src[i + 1];
                let e = src[(i + 2).min(n - 1)];
                pts[i] = p(
                    (a.x + 4.0 * b.x + 6.0 * c.x + 4.0 * d.x + e.x) / 16.0,
                    (a.y + 4.0 * b.y + 6.0 * c.y + 4.0 * d.y + e.y) / 16.0,
                );
            }
        }
    }
    rdp(&pts, 0.35)
}

/// Привязка направления к 0/45/90° (с зажатым Shift).
pub fn snap_angle(a: P, b: P) -> P {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx.hypot(dy);
    let step = std::f32::consts::FRAC_PI_4;
    let ang = (dy.atan2(dx) / step).round() * step;
    p(a.x + ang.cos() * len, a.y + ang.sin() * len)
}

/// Делает прямоугольник квадратным (с зажатым Shift).
pub fn snap_square(a: P, b: P) -> P {
    let s = (b.x - a.x).abs().max((b.y - a.y).abs());
    p(
        a.x + s * (b.x - a.x).signum(),
        a.y + s * (b.y - a.y).signum(),
    )
}

/// Пытается распознать в штрихе ровную фигуру: отрезок, прямоугольник или эллипс.
pub fn recognize(raw: &[P]) -> Option<Kind> {
    if raw.len() < 4 {
        return None;
    }
    let pts = resample(raw, 3.0);
    let len = path_len(&pts);
    let first = pts[0];
    let last = *pts.last().unwrap();
    let chord = first.dist(last);
    if len < 24.0 {
        return None;
    }

    // Отрезок: все точки близко к хорде, и путь почти не длиннее хорды.
    if chord > 24.0 && len < chord * 1.12 {
        let dev = pts
            .iter()
            .map(|q| seg_dist(*q, first, last))
            .fold(0.0f32, f32::max);
        if dev < (chord * 0.055).max(5.0) {
            let (mut a, mut b) = (first, last);
            // почти горизонтальные/вертикальные линии выравниваем точно
            let ang = (b.y - a.y).atan2(b.x - a.x).to_degrees();
            let near = |t: f32| (ang - t).abs() < 4.0;
            if near(0.0) || near(180.0) || near(-180.0) {
                let y = (a.y + b.y) / 2.0;
                a.y = y;
                b.y = y;
            } else if near(90.0) || near(-90.0) {
                let x = (a.x + b.x) / 2.0;
                a.x = x;
                b.x = x;
            }
            return Some(Kind::Line(a, b));
        }
    }

    // Замкнутые фигуры.
    if len > 70.0 && chord < len * 0.18 {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for q in &pts {
            x0 = x0.min(q.x);
            y0 = y0.min(q.y);
            x1 = x1.max(q.x);
            y1 = y1.max(q.y);
        }
        let (w, h) = (x1 - x0, y1 - y0);
        if w < 14.0 || h < 14.0 {
            return None;
        }
        let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, w / 2.0, h / 2.0);
        let n = pts.len() as f32;
        let ell_err = pts
            .iter()
            .map(|q| ((((q.x - cx) / rx).powi(2) + ((q.y - cy) / ry).powi(2)).sqrt() - 1.0).abs())
            .sum::<f32>()
            / n;
        let rect_err = pts
            .iter()
            .map(|q| {
                (q.x - x0)
                    .abs()
                    .min((q.x - x1).abs())
                    .min((q.y - y0).abs())
                    .min((q.y - y1).abs())
            })
            .sum::<f32>()
            / n
            / w.min(h);
        let ell_score = ell_err / 0.11;
        let rect_score = rect_err / 0.042;
        if ell_score < 1.0 && ell_score <= rect_score {
            let (mut rx, mut ry) = (rx, ry);
            if (rx - ry).abs() / rx.max(ry) < 0.14 {
                let r = (rx + ry) / 2.0;
                rx = r;
                ry = r;
            }
            return Some(Kind::Ellipse(p(cx - rx, cy - ry), p(cx + rx, cy + ry)));
        }
        if rect_score < 1.0 {
            let (mut w2, mut h2) = (w, h);
            if (w - h).abs() / w.max(h) < 0.1 {
                let s = (w + h) / 2.0;
                w2 = s;
                h2 = s;
            }
            return Some(Kind::Rect(
                p(cx - w2 / 2.0, cy - h2 / 2.0),
                p(cx + w2 / 2.0, cy + h2 / 2.0),
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wobble(i: usize) -> f32 {
        ((i * 7919) % 13) as f32 / 13.0 * 3.0 - 1.5
    }

    #[test]
    fn spot_dim_covers_everything_but_holes() {
        let area = R { x: 0.0, y: 0.0, w: 100.0, h: 60.0 };
        let holes = [R { x: 10.0, y: 10.0, w: 30.0, h: 20.0 }, R { x: 30.0, y: 20.0, w: 40.0, h: 30.0 }];
        let parts = spot_dim(area, &holes);
        let inside = |rs: &[R], q: P| rs.iter().filter(|r| q.x > r.x && q.x < r.right() && q.y > r.y && q.y < r.bottom()).count();
        for yi in 0..60 {
            for xi in 0..100 {
                let q = p(xi as f32 + 0.5, yi as f32 + 0.5);
                let lit = inside(&holes, q) > 0;
                assert_eq!(inside(&parts, q), if lit { 0 } else { 1 }, "{q:?}");
            }
        }
        assert_eq!(spot_dim(area, &[]), vec![area]);
    }

    #[test]
    fn line_is_straightened() {
        let raw: Vec<P> = (0..60)
            .map(|i| p(10.0 + i as f32 * 4.0, 50.0 + wobble(i)))
            .collect();
        match recognize(&raw) {
            Some(Kind::Line(a, b)) => assert!((a.y - b.y).abs() < 0.01),
            other => panic!("expected line, got {other:?}"),
        }
    }

    #[test]
    fn circle_is_recognized() {
        let raw: Vec<P> = (0..=80)
            .map(|i| {
                let t = i as f32 / 80.0 * std::f32::consts::TAU;
                p(
                    200.0 + (80.0 + wobble(i)) * t.cos(),
                    200.0 + (76.0 + wobble(i)) * t.sin(),
                )
            })
            .collect();
        match recognize(&raw) {
            Some(Kind::Ellipse(a, b)) => assert!(((b.x - a.x) - (b.y - a.y)).abs() < 0.01),
            other => panic!("expected ellipse, got {other:?}"),
        }
    }

    #[test]
    fn rect_is_recognized() {
        let mut raw = Vec::new();
        let c = [(100.0, 100.0), (300.0, 104.0), (297.0, 220.0), (98.0, 216.0), (100.0, 100.0)];
        for s in c.windows(2) {
            for i in 0..30 {
                let t = i as f32 / 30.0;
                raw.push(p(
                    s[0].0 + (s[1].0 - s[0].0) * t + wobble(i),
                    s[0].1 + (s[1].1 - s[0].1) * t + wobble(i + 5),
                ));
            }
        }
        assert!(matches!(recognize(&raw), Some(Kind::Rect(..))), "{:?}", recognize(&raw));
    }

    #[test]
    fn scribble_stays_freehand() {
        let raw: Vec<P> = (0..120)
            .map(|i| {
                let t = i as f32 / 10.0;
                p(50.0 + i as f32 * 2.0, 100.0 + 30.0 * t.sin())
            })
            .collect();
        assert!(recognize(&raw).is_none());
        let s = smooth(&raw);
        assert!(s.len() > 4 && s.len() < raw.len() * 2);
        let b = beautify(&raw);
        assert!(b.len() >= 4 && b.len() < raw.len() / 2, "{}", b.len());
        assert!(b[0].dist(raw[0]) < 3.0 && b[b.len() - 1].dist(raw[raw.len() - 1]) < 3.0);
        let shape = Shape { kind: Kind::Curve(b), color: 0, width: 2.0, alpha: 1.0 };
        assert!(shape.outlines()[0].segs.len() > 4);
    }
}
