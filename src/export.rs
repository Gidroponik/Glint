//! Финальный рендер снимка с аннотациями в полном разрешении, сжатие, буфер обмена, файлы.

use crate::settings::Format;
use crate::shapes::{Kind, Seg, Shape, TEXT_LINE_HEIGHT};
use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tiny_skia as sk;

pub struct Rendered {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Область кадрирования в пикселях снимка.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Вырезает область из снимка (BGRA) и рисует поверх аннотации.
/// `scale` — коэффициент перевода логических координат фигур в пиксели снимка.
pub fn render(bgra: &[u8], img_w: u32, crop: Crop, scale: f32, shapes: &[Shape]) -> Rendered {
    let (w, h) = (crop.w as usize, crop.h as usize);
    let mut rgba = vec![0u8; w * h * 4];
    let stride = img_w as usize * 4;
    for row in 0..h {
        let s = (crop.y as usize + row) * stride + crop.x as usize * 4;
        let src = &bgra[s..s + w * 4];
        let dst = &mut rgba[row * w * 4..(row + 1) * w * 4];
        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = 255;
        }
    }
    if shapes.is_empty() {
        return Rendered {
            w: crop.w,
            h: crop.h,
            rgba,
        };
    }

    let Some(mut pixmap) =
        sk::IntSize::from_wh(crop.w, crop.h).and_then(|size| sk::Pixmap::from_vec(rgba, size))
    else {
        return Rendered {
            w: 0,
            h: 0,
            rgba: vec![],
        };
    };
    let tf = sk::Transform::from_translate(-(crop.x as f32), -(crop.y as f32)).pre_scale(scale, scale);

    // Размытие меняет сам снимок, поэтому идёт первым: остальные аннотации ложатся поверх.
    for shape in shapes {
        if let Kind::Blur(a, b) = &shape.kind {
            let rect = crate::shapes::R::from_points(*a, *b);
            blur_rect(&mut pixmap, bgra, img_w, crop, scale, rect, shape.width);
        }
    }
    // Подсветка: затемняем всё, кроме отмеченных прямоугольников; фигуры рисуются поверх.
    let spots: Vec<crate::shapes::R> = shapes
        .iter()
        .filter_map(|s| match &s.kind {
            Kind::Spot(a, b) => Some(crate::shapes::R::from_points(*a, *b)),
            _ => None,
        })
        .collect();
    if !spots.is_empty() {
        let alpha = shapes
            .iter()
            .filter(|s| matches!(s.kind, Kind::Spot(..)))
            .map(|s| crate::shapes::spot_alpha(s.width))
            .fold(0.0, f32::max);
        dim_outside(&mut pixmap, crop, scale, &spots, alpha);
    }
    for shape in shapes {
        let (r, g, b) = rgb(shape.color);
        if matches!(shape.kind, Kind::Blur(..) | Kind::Spot(..)) {
            continue;
        }
        if let Kind::Text { at, text, size, weight, outline } = &shape.kind {
            let ox = at.x * scale - crop.x as f32;
            let oy = at.y * scale - crop.y as f32;
            let stroke = outline.then(|| {
                (
                    crate::shapes::outline_radius(*size) * scale,
                    rgb(crate::shapes::outline_color(shape.color)),
                )
            });
            draw_text(&mut pixmap, text, ox, oy, size * scale, *weight, (r, g, b), stroke);
            continue;
        }
        let mut paint = sk::Paint::default();
        paint.set_color_rgba8(r, g, b, (shape.alpha * 255.0).round() as u8);
        paint.anti_alias = true;
        for outline in shape.outlines() {
            let mut pb = sk::PathBuilder::new();
            for seg in &outline.segs {
                match *seg {
                    Seg::Move(q) => pb.move_to(q.x, q.y),
                    Seg::Line(q) => pb.line_to(q.x, q.y),
                    Seg::Cubic(a, b, q) => pb.cubic_to(a.x, a.y, b.x, b.y, q.x, q.y),
                    Seg::Close => pb.close(),
                }
            }
            let Some(path) = pb.finish() else { continue };
            if outline.fill {
                pixmap.fill_path(&path, &paint, sk::FillRule::Winding, tf, None);
            } else {
                let stroke = sk::Stroke {
                    width: shape.width,
                    line_cap: sk::LineCap::Round,
                    line_join: sk::LineJoin::Round,
                    ..Default::default()
                };
                pixmap.stroke_path(&path, &paint, &stroke, tf, None);
            }
        }
    }
    Rendered {
        w: crop.w,
        h: crop.h,
        rgba: pixmap.take(),
    }
}

/// Затемняет всё изображение, кроме прямоугольников `holes` (логические координаты).
fn dim_outside(pixmap: &mut sk::Pixmap, crop: Crop, scale: f32, holes: &[crate::shapes::R], alpha: f32) {
    let (w, h) = (crop.w as usize, crop.h as usize);
    let px = |v: f32, lo: u32, len: u32| {
        (((v * scale).round().max(0.0) as u32).clamp(lo, lo + len) - lo) as usize
    };
    let cuts: Vec<(usize, usize, usize, usize)> = holes
        .iter()
        .map(|r| {
            (
                px(r.x, crop.x, crop.w),
                px(r.right(), crop.x, crop.w),
                px(r.y, crop.y, crop.h),
                px(r.bottom(), crop.y, crop.h),
            )
        })
        .collect();
    let k = ((1.0 - alpha.clamp(0.0, 1.0)) * 256.0).round() as u32;
    let mut lit = vec![false; w];
    let data = pixmap.data_mut();
    for row in 0..h {
        lit.fill(false);
        for &(x0, x1, y0, y1) in &cuts {
            if row >= y0 && row < y1 && x1 > x0 {
                lit[x0..x1].fill(true);
            }
        }
        let line = &mut data[row * w * 4..(row + 1) * w * 4];
        for (px, lit) in line.chunks_exact_mut(4).zip(&lit) {
            if !lit {
                for c in &mut px[..3] {
                    *c = ((*c as u32 * k) >> 8) as u8;
                }
            }
        }
    }
}

/// Заменяет прямоугольник `rect` (логические координаты) размытым фрагментом исходного снимка.
fn blur_rect(
    pixmap: &mut sk::Pixmap,
    bgra: &[u8],
    img_w: u32,
    crop: Crop,
    scale: f32,
    rect: crate::shapes::R,
    width: f32,
) {
    let img_h = (bgra.len() / 4) as u32 / img_w.max(1);
    let px = |v: f32, lo: u32, hi: u32| ((v * scale).round().max(0.0) as u32).clamp(lo, hi);
    let (x0, x1) = (px(rect.x, crop.x, crop.x + crop.w), px(rect.right(), crop.x, crop.x + crop.w));
    let (y0, y1) = (px(rect.y, crop.y, crop.y + crop.h), px(rect.bottom(), crop.y, crop.y + crop.h));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let radius = (crate::blur::radius(width) * scale).round().max(1.0) as u32;
    let blurred = crate::blur::region(bgra, img_w, img_h, x0, y0, w as u32, h as u32, radius);
    if blurred.len() != w * h * 4 {
        return;
    }
    let stride = crop.w as usize * 4;
    let data = pixmap.data_mut();
    for row in 0..h {
        let o = ((y0 - crop.y) as usize + row) * stride + (x0 - crop.x) as usize * 4;
        let dst = &mut data[o..o + w * 4];
        let src = &blurred[row * w * 4..(row + 1) * w * 4];
        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = 255;
        }
    }
}

fn rgb(c: u32) -> (u8, u8, u8) {
    ((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

fn text_font(weight: u16) -> Option<&'static FontRef<'static>> {
    static FONTS: OnceLock<Vec<(u16, FontRef<'static>)>> = OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        crate::fonts::WEIGHTS
            .iter()
            .filter_map(|(w, bytes)| Some((*w, FontRef::try_from_slice(bytes).ok()?)))
            .collect()
    });
    fonts
        .iter()
        .min_by_key(|(w, _)| w.abs_diff(weight))
        .map(|(_, f)| f)
}

/// Рисует многострочный текст; (ox, oy) — левый верхний угол первой строки, `em` — кегль в px.
fn draw_text(
    pixmap: &mut sk::Pixmap,
    text: &str,
    ox: f32,
    oy: f32,
    em: f32,
    weight: u16,
    color: (u8, u8, u8),
    outline: Option<(f32, (u8, u8, u8))>,
) {
    let Some(font) = text_font(weight) else { return };
    let upm = font.units_per_em().unwrap_or(2048.0);
    let scale = PxScale::from(em * font.height_unscaled() / upm);
    let sf = font.as_scaled(scale);
    let line_h = em * TEXT_LINE_HEIGHT;
    let (asc, desc) = (sf.ascent(), -sf.descent());
    let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
    let data = pixmap.data_mut();

    let mut pass = |dx: f32, dy: f32, color: (u8, u8, u8)| {
        for (li, line) in text.split('\n').enumerate() {
            let baseline = oy + dy + li as f32 * line_h + (line_h - asc - desc) / 2.0 + asc;
            let mut x = ox + dx;
            let mut prev = None;
            for ch in line.chars() {
                let id = font.glyph_id(ch);
                if let Some(pv) = prev {
                    x += sf.kern(pv, id);
                }
                let glyph = id.with_scale_and_position(scale, ab_glyph::point(x, baseline));
                x += sf.h_advance(id);
                prev = Some(id);
                let Some(outlined) = font.outline_glyph(glyph) else {
                    continue;
                };
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, cov| {
                    let px = bounds.min.x as i32 + gx as i32;
                    let py = bounds.min.y as i32 + gy as i32;
                    if px < 0 || py < 0 || px >= pw || py >= ph {
                        return;
                    }
                    let a = cov.clamp(0.0, 1.0);
                    let i = ((py * pw + px) * 4) as usize;
                    data[i] = (data[i] as f32 + (color.0 as f32 - data[i] as f32) * a) as u8;
                    data[i + 1] = (data[i + 1] as f32 + (color.1 as f32 - data[i + 1] as f32) * a) as u8;
                    data[i + 2] = (data[i + 2] as f32 + (color.2 as f32 - data[i + 2] as f32) * a) as u8;
                });
            }
        }
    };
    if let Some((radius, outline_rgb)) = outline {
        for (dx, dy) in crate::shapes::outline_offsets(radius) {
            pass(dx, dy, outline_rgb);
        }
    }
    pass(0.0, 0.0, color);
}

fn to_rgb(rgba: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for p in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&p[..3]);
    }
    rgb
}

pub fn encode(img: &Rendered, format: Format, jpeg_quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    match format {
        Format::Png => {
            let rgb = to_rgb(&img.rgba);
            let mut enc = png::Encoder::new(&mut out, img.w, img.h);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::High);
            let mut w = enc.write_header().map_err(|e| e.to_string())?;
            w.write_image_data(&rgb).map_err(|e| e.to_string())?;
            w.finish().map_err(|e| e.to_string())?;
        }
        Format::Png8 => {
            let (palette, indices) = quantize(&img.rgba);
            let mut enc = png::Encoder::new(&mut out, img.w, img.h);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_palette(palette);
            enc.set_compression(png::Compression::High);
            enc.set_filter(png::Filter::NoFilter);
            let mut w = enc.write_header().map_err(|e| e.to_string())?;
            w.write_image_data(&indices).map_err(|e| e.to_string())?;
            w.finish().map_err(|e| e.to_string())?;
        }
        Format::Jpeg => {
            let rgb = to_rgb(&img.rgba);
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, jpeg_quality);
            image::ImageEncoder::write_image(enc, &rgb, img.w, img.h, image::ExtendedColorType::Rgb8)
                .map_err(|e| e.to_string())?;
        }
        Format::Webp => {
            let rgb = to_rgb(&img.rgba);
            let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
            image::ImageEncoder::write_image(enc, &rgb, img.w, img.h, image::ExtendedColorType::Rgb8)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

/// Сводит изображение к палитре ≤256 цветов. Если цветов и так мало — без потерь.
fn quantize(rgba: &[u8]) -> (Vec<u8>, Vec<u8>) {
    use std::collections::HashMap;
    let n = rgba.len() / 4;
    let mut exact: HashMap<[u8; 3], u8> = HashMap::new();
    let mut indices = Vec::with_capacity(n);
    let mut palette = Vec::new();
    let mut fits = true;
    for p in rgba.chunks_exact(4) {
        let key = [p[0], p[1], p[2]];
        let next = exact.len();
        match exact.get(&key) {
            Some(i) => indices.push(*i),
            None if next < 256 => {
                exact.insert(key, next as u8);
                palette.extend_from_slice(&key);
                indices.push(next as u8);
            }
            None => {
                fits = false;
                break;
            }
        }
    }
    if fits {
        return (palette, indices);
    }

    let sample = if n > 2_000_000 { 10 } else if n > 400_000 { 4 } else { 1 };
    let nq = color_quant::NeuQuant::new(sample, 256, rgba);
    let mut cache: HashMap<[u8; 3], u8> = HashMap::new();
    let mut indices = Vec::with_capacity(n);
    for p in rgba.chunks_exact(4) {
        let key = [p[0], p[1], p[2]];
        let i = *cache.entry(key).or_insert_with(|| nq.index_of(p) as u8);
        indices.push(i);
    }
    (nq.color_map_rgb(), indices)
}

/// Кладёт снимок в буфер обмена: растр (CF_DIB) и, если есть, файл в выбранном формате
/// (CF_HDROP). Редакторы вставят растр, а мессенджеры и Проводник — сжатый файл.
pub fn copy_to_clipboard(img: &Rendered, file: Option<&Path>) -> Result<(), String> {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    const CF_DIB: u32 = 8;
    const CF_HDROP: u32 = 15;

    let (w, h) = (img.w as usize, img.h as usize);
    // BITMAPINFOHEADER + пиксели BGRA снизу вверх
    let mut dib = Vec::with_capacity(40 + w * h * 4);
    dib.extend_from_slice(&40u32.to_le_bytes());
    dib.extend_from_slice(&(w as i32).to_le_bytes());
    dib.extend_from_slice(&(h as i32).to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&[0u8; 24]);
    for row in img.rgba.chunks_exact(w * 4).rev() {
        for p in row.chunks_exact(4) {
            dib.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
    }

    // DROPFILES: смещение списка (20), точка, fNC, fWide=1, затем путь UTF-16 с двойным нулём
    let drop = file.map(|path| {
        let mut d = Vec::new();
        d.extend_from_slice(&20u32.to_le_bytes());
        d.extend_from_slice(&[0u8; 12]);
        d.extend_from_slice(&1u32.to_le_bytes());
        for unit in path.to_string_lossy().encode_utf16().chain([0, 0]) {
            d.extend_from_slice(&unit.to_le_bytes());
        }
        d
    });

    unsafe fn put(format: u32, bytes: &[u8]) -> Result<(), String> {
        unsafe {
            let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| e.to_string())?;
            let ptr = GlobalLock(mem) as *mut u8;
            if ptr.is_null() {
                return Err("GlobalLock".into());
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            let _ = GlobalUnlock(mem);
            SetClipboardData(format, Some(HANDLE(mem.0)))
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
    }

    unsafe {
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        if !opened {
            return Err("the clipboard is busy".into());
        }
        let _ = EmptyClipboard();
        let mut result = put(CF_DIB, &dib);
        if let (Ok(()), Some(drop)) = (&result, &drop) {
            result = put(CF_HDROP, drop);
        }
        let _ = CloseClipboard();
        result
    }
}

/// Кладёт в буфер обмена текст (CF_UNICODETEXT).
pub fn copy_text(text: &str) -> Result<(), String> {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;

    let normalized = text.replace("\r\n", "\n").replace('\n', "\r\n");
    let wide: Vec<u16> = normalized.encode_utf16().chain([0]).collect();
    let bytes = wide.len() * 2;
    unsafe {
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        if !opened {
            return Err("the clipboard is busy".into());
        }
        let _ = EmptyClipboard();
        let result = (|| {
            let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| e.to_string())?;
            let ptr = GlobalLock(mem) as *mut u8;
            if ptr.is_null() {
                return Err("GlobalLock".to_string());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr, bytes);
            let _ = GlobalUnlock(mem);
            SetClipboardData(CF_UNICODETEXT, Some(HANDLE(mem.0)))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })();
        let _ = CloseClipboard();
        result
    }
}

/// Номер состояния буфера обмена — чтобы не затереть то, что пользователь скопировал позже.
pub fn clipboard_sequence() -> u32 {
    unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}

/// Временная папка для файлов, которые кладутся в буфер обмена.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("Glint")
}

/// Удаляет временные файлы старше суток.
pub fn clean_temp() {
    let Ok(entries) = std::fs::read_dir(temp_dir()) else { return };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() > 24 * 3600);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Имя файла вида `Glint 2026-11-17 14-03-22.png`.
pub fn file_name(format: Format) -> String {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "Glint {:04}-{:02}-{:02} {:02}-{:02}-{:02}.{}",
        t.wYear,
        t.wMonth,
        t.wDay,
        t.wHour,
        t.wMinute,
        t.wSecond,
        format.ext()
    )
}

/// Сохраняет в папку, не перезаписывая существующие файлы.
pub fn save_to_dir(dir: &Path, format: Format, bytes: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let name = file_name(format);
    let mut path = dir.join(&name);
    let mut n = 2;
    while path.exists() {
        let stem = name.trim_end_matches(&format!(".{}", format.ext()));
        path = dir.join(format!("{stem} ({n}).{}", format.ext()));
        n += 1;
    }
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn format_from_path(path: &Path, fallback: Format) -> Format {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => Format::Jpeg,
        Some("webp") => Format::Webp,
        Some("png") if fallback == Format::Png8 => Format::Png8,
        Some("png") => Format::Png,
        _ => fallback,
    }
}

pub fn human_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1048576.0)
    } else {
        format!("{} KB", bytes.div_ceil(1024).max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::p;

    fn sample() -> Rendered {
        let (w, h) = (320u32, 200u32);
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                bgra.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255]);
            }
        }
        let shapes = vec![
            Shape {
                kind: Kind::Arrow(p(10.0, 10.0), p(120.0, 80.0)),
                color: 0xFF3B5C,
                width: 4.0,
                alpha: 1.0,
            },
            Shape {
                kind: Kind::Text {
                    at: p(20.0, 40.0),
                    text: "Привет
Hello".into(),
                    size: 18.0,
                    weight: 700,
                    outline: true,
                },
                color: 0xFFFFFF,
                width: 2.0,
                alpha: 1.0,
            },
        ];
        let crop = Crop { x: 16, y: 8, w: 280, h: 180 };
        render(&bgra, w, crop, 2.0, &shapes)
    }

    #[test]
    fn crop_is_pixel_exact() {
        let (w, h) = (64u32, 48u32);
        let bgra: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, (i % 241) as u8, (i % 239) as u8, 255]).collect();
        let out = render(&bgra, w, Crop { x: 5, y: 7, w: 20, h: 10 }, 1.5, &[]);
        assert_eq!((out.w, out.h), (20, 10));
        let i = ((7 + 3) * w + 5 + 4) as usize * 4;
        let o = (3 * 20 + 4) * 4;
        assert_eq!(&out.rgba[o..o + 4], &[bgra[i + 2], bgra[i + 1], bgra[i], 255]);
    }

    #[test]
    fn spotlight_dims_everything_else() {
        let (w, h) = (120u32, 80u32);
        let bgra = vec![200u8; (w * h * 4) as usize];
        let crop = Crop { x: 10, y: 10, w: 100, h: 60 };
        let spot = Shape {
            kind: Kind::Spot(p(15.0, 10.0), p(40.0, 30.0)),
            color: 0,
            width: 4.0,
            alpha: 1.0,
        };
        let out = render(&bgra, w, crop, 2.0, &[spot]);
        // прямоугольник 30..80 × 20..60 в пикселях снимка → 20..70 × 10..50 в кадре
        for y in 0..60usize {
            for x in 0..100usize {
                let i = (y * 100 + x) * 4;
                let inside = (20..70).contains(&x) && (10..50).contains(&y);
                if inside {
                    assert_eq!(out.rgba[i], 200, "dimmed inside at {x},{y}");
                } else {
                    assert!((95..115).contains(&out.rgba[i]), "wrong dim at {x},{y}: {}", out.rgba[i]);
                }
                assert_eq!(out.rgba[i + 3], 255);
            }
        }
    }

    #[test]
    fn blur_stays_inside_its_rect() {
        let (w, h) = (120u32, 80u32);
        let bgra: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let v = if (i % w / 2 + i / w / 2) % 2 == 0 { 255 } else { 0 };
                [v, v, v, 255]
            })
            .collect();
        let crop = Crop { x: 10, y: 10, w: 100, h: 60 };
        let plain = render(&bgra, w, crop, 2.0, &[]);
        let blur = Shape {
            kind: Kind::Blur(p(15.0, 10.0), p(40.0, 30.0)),
            color: 0,
            width: 4.0,
            alpha: 1.0,
        };
        let out = render(&bgra, w, crop, 2.0, &[blur]);
        // прямоугольник 30..80 × 20..60 в пикселях снимка → 20..70 × 10..50 в кадре
        for y in 0..60usize {
            for x in 0..100usize {
                let i = (y * 100 + x) * 4;
                let inside = (20..70).contains(&x) && (10..50).contains(&y);
                if inside {
                    assert!((90..166).contains(&out.rgba[i]), "not blurred at {x},{y}: {}", out.rgba[i]);
                } else {
                    assert_eq!(out.rgba[i], plain.rgba[i], "changed outside at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn all_formats_roundtrip() {
        let img = sample();
        assert_eq!((img.w, img.h), (280, 180));
        // аннотации действительно попали в картинку
        assert!(img.rgba.chunks_exact(4).any(|p| p[0] == 0xFF && p[1] == 0x3B && p[2] == 0x5C));
        for format in Format::ALL {
            let bytes = encode(&img, format, 85).unwrap();
            let decoded = image::load_from_memory(&bytes).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (280, 180), "{format:?}");
            if matches!(format, Format::Png | Format::Webp) {
                assert_eq!(decoded.to_rgb8().as_raw(), &to_rgb(&img.rgba), "{format:?} must be lossless");
            }
        }
    }

    /// Проверка буфера обмена. Запускается вручную: `cargo test -- --ignored`.
    /// Сохраняет текущее содержимое (текст или картинку) и возвращает его обратно.
    #[test]
    #[ignore]
    fn clipboard_roundtrip() {
        let mut cb = arboard::Clipboard::new().unwrap();
        let text = cb.get_text().ok();
        let image = cb.get_image().ok().map(|i| i.to_owned_img());
        if text.is_none() && image.is_none() {
            eprintln!("clipboard holds something else, skipping");
            return;
        }
        let img = sample();
        let file = temp_dir().join("clipboard-test.webp");
        std::fs::create_dir_all(temp_dir()).unwrap();
        std::fs::write(&file, encode(&img, Format::Webp, 85).unwrap()).unwrap();
        copy_to_clipboard(&img, Some(&file)).unwrap();
        let mut check = arboard::Clipboard::new().unwrap();
        let got = check.get_image().unwrap();
        assert_eq!((got.width, got.height), (280, 180));
        assert_eq!(&got.bytes[..3], &img.rgba[..3]);
        let n = got.bytes.len();
        assert_eq!(&got.bytes[n - 4..n - 1], &img.rgba[n - 4..n - 1]);
        // в буфере лежит и файл в выбранном формате
        let files = check.get().file_list().unwrap();
        assert_eq!(files, vec![file.clone()]);
        drop(check);
        let mut cb = arboard::Clipboard::new().unwrap();
        match (text, image) {
            (Some(t), _) => cb.set_text(t).unwrap(),
            (None, Some(i)) => cb.set_image(i).unwrap(),
            _ => {}
        }
    }
}
