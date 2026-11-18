//! Размытие участков снимка: три прохода box-blur дают результат, близкий к гауссову.
//! Работает с четырёхканальными пикселями в любом порядке каналов.

/// Число проходов box-blur.
const PASSES: usize = 3;

/// Радиус размытия (в логических пикселях) для выбранной толщины инструмента.
pub fn radius(width: f32) -> f32 {
    width * 2.5 + 5.0
}

/// Размывает прямоугольник `(x, y, w, h)` снимка и возвращает его пиксели (`w * h * 4`).
/// Соседние пиксели за границей прямоугольника берутся из самого снимка.
pub fn region(src: &[u8], img_w: u32, img_h: u32, x: u32, y: u32, w: u32, h: u32, radius: u32) -> Vec<u8> {
    let (iw, ih) = (img_w as usize, img_h as usize);
    let (x, y) = ((x as usize).min(iw), (y as usize).min(ih));
    let (w, h) = ((w as usize).min(iw - x), (h as usize).min(ih - y));
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let r = radius as usize;
    let pad = r * PASSES;
    let (px0, py0) = (x.saturating_sub(pad), y.saturating_sub(pad));
    let (px1, py1) = ((x + w + pad).min(iw), (y + h + pad).min(ih));
    let (pw, ph) = (px1 - px0, py1 - py0);

    let mut a = Vec::with_capacity(pw * ph * 4);
    for row in py0..py1 {
        let s = (row * iw + px0) * 4;
        a.extend_from_slice(&src[s..s + pw * 4]);
    }
    if r > 0 {
        let mut b = vec![0u8; a.len()];
        for _ in 0..PASSES {
            horizontal(&a, &mut b, pw, ph, r);
            vertical(&b, &mut a, pw, ph, r);
        }
    }

    let mut out = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        let s = ((row + y - py0) * pw + (x - px0)) * 4;
        out.extend_from_slice(&a[s..s + w * 4]);
    }
    out
}

/// Уменьшенная в `k` раз размытая копия всего снимка — для предпросмотра на экране.
/// Возвращает пиксели и размеры уменьшенной картинки; альфа-канал непрозрачный.
pub fn preview(src: &[u8], img_w: u32, img_h: u32, k: u32, radius: f32) -> (Vec<u8>, u32, u32) {
    let (iw, ih, k) = (img_w as usize, img_h as usize, k.max(1) as usize);
    let (pw, ph) = (iw.div_ceil(k), ih.div_ceil(k));
    let mut small = vec![0u8; pw * ph * 4];
    let mut sums = vec![0u32; pw * 4];
    for by in 0..ph {
        sums.fill(0);
        let rows = (by * k)..((by + 1) * k).min(ih);
        let n_rows = rows.len() as u32;
        for row in rows {
            let line = &src[row * iw * 4..(row + 1) * iw * 4];
            for (i, px) in line.chunks_exact(4).enumerate() {
                let o = i / k * 4;
                sums[o] += px[0] as u32;
                sums[o + 1] += px[1] as u32;
                sums[o + 2] += px[2] as u32;
            }
        }
        for bx in 0..pw {
            let cols = ((bx + 1) * k).min(iw) - bx * k;
            let n = cols as u32 * n_rows;
            let o = (by * pw + bx) * 4;
            small[o] = (sums[bx * 4] / n) as u8;
            small[o + 1] = (sums[bx * 4 + 1] / n) as u8;
            small[o + 2] = (sums[bx * 4 + 2] / n) as u8;
            small[o + 3] = 255;
        }
    }
    let r = (radius / k as f32).round().max(1.0) as u32;
    let out = region(&small, pw as u32, ph as u32, 0, 0, pw as u32, ph as u32, r);
    (out, pw as u32, ph as u32)
}

fn horizontal(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    let n = (2 * r + 1) as u32;
    let last = w - 1;
    for row in 0..h {
        let line = &src[row * w * 4..(row + 1) * w * 4];
        let out = &mut dst[row * w * 4..(row + 1) * w * 4];
        let mut sum = [0u32; 4];
        for i in 0..=2 * r {
            let x = i.saturating_sub(r).min(last);
            for c in 0..4 {
                sum[c] += line[x * 4 + c] as u32;
            }
        }
        for x in 0..w {
            for c in 0..4 {
                out[x * 4 + c] = ((sum[c] + n / 2) / n) as u8;
            }
            let add = (x + r + 1).min(last);
            let sub = x.saturating_sub(r);
            for c in 0..4 {
                sum[c] = sum[c] + line[add * 4 + c] as u32 - line[sub * 4 + c] as u32;
            }
        }
    }
}

fn vertical(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    let n = (2 * r + 1) as u32;
    let last = h - 1;
    let stride = w * 4;
    let line = |y: usize| &src[y * stride..(y + 1) * stride];
    let mut sums = vec![0u32; stride];
    for i in 0..=2 * r {
        let y = i.saturating_sub(r).min(last);
        for (s, v) in sums.iter_mut().zip(line(y)) {
            *s += *v as u32;
        }
    }
    for y in 0..h {
        for (d, s) in dst[y * stride..(y + 1) * stride].iter_mut().zip(&sums) {
            *d = ((*s + n / 2) / n) as u8;
        }
        let add = line((y + r + 1).min(last));
        let sub = line(y.saturating_sub(r));
        for ((s, a), b) in sums.iter_mut().zip(add).zip(sub) {
            *s = *s + *a as u32 - *b as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(w: u32, h: u32) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| {
                let v = if (i % w / 4 + i / w / 4) % 2 == 0 { 255 } else { 0 };
                [v, v, v, 255]
            })
            .collect()
    }

    #[test]
    fn flat_color_is_unchanged() {
        let src: Vec<u8> = (0..40 * 30).flat_map(|_| [10, 120, 250, 255]).collect();
        let out = region(&src, 40, 30, 5, 4, 20, 15, 6);
        assert_eq!(out.len(), 20 * 15 * 4);
        assert!(out.chunks_exact(4).all(|p| p == [10, 120, 250, 255]));
    }

    #[test]
    fn detail_is_wiped_out() {
        let src = checker(96, 96);
        let out = region(&src, 96, 96, 24, 24, 48, 48, 10);
        let (lo, hi) = out.chunks_exact(4).fold((255u8, 0u8), |(lo, hi), p| (lo.min(p[0]), hi.max(p[0])));
        assert!(hi - lo < 24, "contrast left: {lo}..{hi}");
    }

    #[test]
    fn preview_has_block_size() {
        let src = checker(101, 50);
        let (out, w, h) = preview(&src, 101, 50, 4, 12.0);
        assert_eq!((w, h), (26, 13));
        assert_eq!(out.len(), 26 * 13 * 4);
        assert!(out.chunks_exact(4).all(|p| p[3] == 255));
    }
}
