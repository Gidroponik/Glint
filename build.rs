//! Генерирует значок приложения (.ico) тем же кодом, что рисует значок в трее,
//! и встраивает его в exe как ресурс №1 — GPUI берёт оттуда иконку окна.

use std::io::Write;
use std::path::PathBuf;

include!("src/icon.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let sizes = [16usize, 24, 32, 48, 64, 128];
    let images: Vec<Vec<u8>> = sizes.iter().map(|&n| dib(n)).collect();

    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len();
    for (n, img) in sizes.iter().zip(&images) {
        ico.extend_from_slice(&[*n as u8, *n as u8, 0, 0, 1, 0, 32, 0]);
        ico.extend_from_slice(&(img.len() as u32).to_le_bytes());
        ico.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += img.len();
    }
    for img in &images {
        ico.extend_from_slice(img);
    }
    let ico_path = out.join("glint.ico");
    std::fs::write(&ico_path, ico).unwrap();

    let rc_path = out.join("glint.rc");
    let mut rc = std::fs::File::create(&rc_path).unwrap();
    writeln!(rc, "1 ICON \"{}\"", ico_path.display().to_string().replace('\\', "/")).unwrap();
    drop(rc);
    let _ = embed_resource::compile(&rc_path, embed_resource::NONE);
}

/// Изображение в формате значка: BITMAPINFOHEADER + BGRA снизу вверх + AND-маска.
fn dib(n: usize) -> Vec<u8> {
    let rgba = icon_rgba(n);
    let mut v = Vec::new();
    v.extend_from_slice(&40u32.to_le_bytes());
    v.extend_from_slice(&(n as i32).to_le_bytes());
    v.extend_from_slice(&(2 * n as i32).to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&32u16.to_le_bytes());
    v.extend_from_slice(&[0u8; 24]);
    for y in (0..n).rev() {
        for x in 0..n {
            let i = (y * n + x) * 4;
            v.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    let mask_row = n.div_ceil(32) * 4;
    v.extend(std::iter::repeat_n(0u8, mask_row * n));
    v
}
