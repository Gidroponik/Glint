//! Распознавание текста на снимке встроенным в Windows движком OCR (Windows.Media.Ocr).
//! Языки берутся из языковых пакетов, установленных в системе.

use crate::export::Rendered;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;

/// Мелкий текст распознаётся заметно лучше после увеличения; больше этого числа пикселей
/// исходник не увеличиваем, чтобы не тратить время и память.
const UPSCALE_LIMIT: u64 = 6_000_000;

/// Распознанное слово и его рамка в пикселях исходной картинки.
#[derive(Clone, Debug)]
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Возвращает распознанный текст, строки разделены `\n`. Пустая строка — текста не нашлось.
pub fn recognize(img: &Rendered) -> Result<String, String> {
    let lines = recognize_lines(img)?;
    let text: Vec<String> = lines
        .iter()
        .map(|line| line.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "))
        .collect();
    Ok(text.join("\n"))
}

/// Распознаёт текст построчно: каждая строка — список слов с рамками.
pub fn recognize_lines(img: &Rendered) -> Result<Vec<Vec<Word>>, String> {
    let err = |e: windows::core::Error| e.message().to_string();
    if img.w == 0 || img.h == 0 {
        return Ok(Vec::new());
    }
    let max = OcrEngine::MaxImageDimension().map_err(err)?;
    if img.w > max || img.h > max {
        return Err("The area is too large".into());
    }

    let (mut w, mut h) = (img.w, img.h);
    let mut zoom = 1.0f32;
    let mut pixels = img.rgba.clone();
    if w * 2 <= max && h * 2 <= max && (w as u64) * (h as u64) <= UPSCALE_LIMIT {
        if let Some(src) = image::RgbaImage::from_raw(w, h, pixels.clone()) {
            let big = image::imageops::resize(&src, w * 2, h * 2, image::imageops::FilterType::CatmullRom);
            (w, h) = (w * 2, h * 2);
            zoom = 2.0;
            pixels = big.into_raw();
        }
    }
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }

    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|_| "No OCR language is installed in Windows".to_string())?;
    let writer = DataWriter::new().map_err(err)?;
    writer.WriteBytes(&pixels).map_err(err)?;
    let buffer = writer.DetachBuffer().map_err(err)?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, w as i32, h as i32)
        .map_err(err)?;
    let result = engine.RecognizeAsync(&bitmap).map_err(err)?.get().map_err(err)?;

    let mut lines = Vec::new();
    for line in result.Lines().map_err(err)? {
        let mut words = Vec::new();
        for word in line.Words().map_err(err)? {
            let r = word.BoundingRect().map_err(err)?;
            words.push(Word {
                text: word.Text().map_err(err)?.to_string(),
                x: r.X / zoom,
                y: r.Y / zoom,
                w: r.Width / zoom,
                h: r.Height / zoom,
            });
        }
        if !words.is_empty() {
            lines.push(words);
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::{Kind, Shape, p};

    /// Нужен языковой пакет OCR в системе. Запуск: `cargo test -- --ignored ocr`.
    #[test]
    #[ignore]
    fn reads_rendered_text() {
        let (w, h) = (640u32, 160u32);
        let bgra = vec![255u8; (w * h * 4) as usize];
        let label = Shape {
            kind: Kind::Text {
                at: p(30.0, 50.0),
                text: "Glint 2026 screenshot".into(),
                size: 34.0,
                weight: 600,
                outline: false,
            },
            color: 0x000000,
            width: 2.0,
            alpha: 1.0,
        };
        let crop = crate::export::Crop { x: 0, y: 0, w, h };
        let img = crate::export::render(&bgra, w, crop, 1.0, &[label]);
        let text = recognize(&img).unwrap();
        assert!(text.contains("Glint") && text.contains("2026"), "got {text:?}");
        // рамки слов лежат там, где нарисован текст
        let lines = recognize_lines(&img).unwrap();
        let first = &lines[0][0];
        assert!(first.x > 20.0 && first.x < 60.0 && first.y > 40.0 && first.y < 90.0, "{first:?}");
    }
}
