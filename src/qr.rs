//! Поиск и чтение QR-кодов на снимке.

/// Найденный код: содержимое и рамка в пикселях снимка.
#[derive(Clone, Debug)]
pub struct Code {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Ищет QR-коды на четырёхканальной картинке (порядок цветовых каналов не важен).
pub fn scan(pixels: &[u8], w: u32, h: u32) -> Vec<Code> {
    let (w, h) = (w as usize, h as usize);
    if w < 21 || h < 21 || pixels.len() < w * h * 4 {
        return Vec::new();
    }
    let mut found: Vec<Code> = Vec::new();
    // Второй проход — по негативу: так читаются светлые коды на тёмном фоне.
    for invert in [false, true] {
        scan_pass(pixels, w, h, invert, &mut found);
    }
    found
}

fn scan_pass(pixels: &[u8], w: usize, h: usize, invert: bool, found: &mut Vec<Code>) {
    let luma: Vec<u8> = pixels[..w * h * 4]
        .chunks_exact(4)
        .map(|px| {
            let v = ((px[0] as u32 + px[1] as u32 * 2 + px[2] as u32) / 4) as u8;
            if invert { 255 - v } else { v }
        })
        .collect();
    let mut hints = rxing::DecodeHints::default();
    hints.PossibleFormats = Some(std::collections::HashSet::from([rxing::BarcodeFormat::QR_CODE]));
    let Ok(results) =
        rxing::helpers::detect_multiple_in_luma_with_hints(luma, w as u32, h as u32, &mut hints)
    else {
        return;
    };
    for res in results {
        let text = res.getText().to_string();
        let pts = res.getPoints();
        if text.is_empty() || pts.is_empty() {
            continue;
        }
        let (x0, x1) = pts.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
        let (y0, y1) = pts.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
        if found.iter().any(|c| (c.x - x0).abs() < 8.0 && (c.y - y0).abs() < 8.0) {
            continue;
        }
        found.push(Code {
            text,
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Рисует QR-код с модулем `module` пикселей в точке `(ox, oy)` белого холста.
    fn canvas_with_code(text: &str, w: u32, h: u32, ox: u32, oy: u32, module: u32) -> (Vec<u8>, u32) {
        let code = qrcode::QrCode::new(text.as_bytes()).unwrap();
        let n = code.width() as u32;
        let cells = code.to_colors();
        let mut px = vec![255u8; (w * h * 4) as usize];
        for cy in 0..n {
            for cx in 0..n {
                if cells[(cy * n + cx) as usize] != qrcode::Color::Dark {
                    continue;
                }
                for y in 0..module {
                    for x in 0..module {
                        let o = (((oy + cy * module + y) * w + ox + cx * module + x) * 4) as usize;
                        px[o..o + 3].fill(0);
                    }
                }
            }
        }
        (px, n * module)
    }

    #[test]
    fn reads_code_and_its_place() {
        let (px, side) = canvas_with_code("https://example.com/glint?x=1", 640, 480, 200, 120, 5);
        let found = scan(&px, 640, 480);
        assert_eq!(found.len(), 1, "{found:?}");
        let c = &found[0];
        assert_eq!(c.text, "https://example.com/glint?x=1");
        assert!((c.x - 200.0).abs() < 6.0 && (c.y - 120.0).abs() < 6.0, "{c:?}");
        assert!((c.w - side as f32).abs() < 10.0 && (c.h - side as f32).abs() < 10.0, "{c:?}");
    }

    #[test]
    fn reads_light_code_on_dark_background() {
        let (mut px, _) = canvas_with_code("dark theme", 400, 400, 100, 100, 6);
        for v in px.chunks_exact_mut(4) {
            for c in &mut v[..3] {
                *c = 255 - *c;
            }
        }
        let found = scan(&px, 400, 400);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].text, "dark theme");
    }

    #[test]
    fn empty_picture_has_no_codes() {
        let px = vec![200u8; 300 * 200 * 4];
        assert!(scan(&px, 300, 200).is_empty());
    }
}

#[cfg(test)]
mod big {
    use super::*;

    /// Код на большом пёстром «экране», с нецелым масштабом и сглаженными краями.
    #[test]
    fn finds_code_on_busy_4k_screen() {
        let (w, h) = (3840u32, 2160u32);
        for module in [2.0f32, 3.3, 4.7, 9.5] {
            let mut px: Vec<u8> = (0..w * h)
                .flat_map(|i| {
                    let (x, y) = (i % w, i / w);
                    let v = ((x * 7 + y * 13) % 251) as u8;
                    let t = if (x / 9 + y / 17) % 2 == 0 { 30 } else { 220 };
                    [v, t, v / 2, 255]
                })
                .collect();
            let code = qrcode::QrCode::new(b"https://github.com/Gidroponik/Glint/releases/tag/v0.2.0").unwrap();
            let n = code.width();
            let cells = code.to_colors();
            let side = ((n as f32 + 8.0) * module).ceil() as u32;
            let (ox, oy) = (1700u32, 900u32);
            for y in 0..side {
                for x in 0..side {
                    // 3×3 выборки — как сглаживание при масштабировании картинки в браузере
                    let mut dark = 0;
                    for sy in 0..3 {
                        for sx in 0..3 {
                            let mx = ((x as f32 + sx as f32 / 3.0) / module - 4.0).floor();
                            let my = ((y as f32 + sy as f32 / 3.0) / module - 4.0).floor();
                            if mx >= 0.0 && my >= 0.0 && (mx as usize) < n && (my as usize) < n
                                && cells[my as usize * n + mx as usize] == qrcode::Color::Dark
                            {
                                dark += 1;
                            }
                        }
                    }
                    let v = 255 - (dark * 255 / 9) as u8;
                    let o = (((oy + y) * w + ox + x) * 4) as usize;
                    px[o..o + 3].fill(v);
                }
            }
            let t = std::time::Instant::now();
            let found = scan(&px, w, h);
            eprintln!("module {module}: {} found in {:?}", found.len(), t.elapsed());
            assert_eq!(found.len(), 1, "module {module}");
        }
    }
}
