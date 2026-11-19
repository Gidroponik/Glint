//! Палитра интерфейса и встроенные SVG-иконки.

use gpui::{AssetSource, BoxShadow, SharedString, Styled, Svg, point, px, rgb, svg};
use std::borrow::Cow;

/// Строгая монохромная тема: чёрный фон, белые элементы управления с мягким свечением.
pub const BG: u32 = 0x000000;
pub const PANEL: u32 = 0x0A0A0A;
pub const PANEL_HI: u32 = 0x1A1A1A;
pub const BORDER: u32 = 0x262626;
pub const TEXT: u32 = 0xFFFFFF;
pub const MUTED: u32 = 0x8C8C8C;
pub const WHITE: u32 = 0xFFFFFF;
pub const WHITE_HOVER: u32 = 0xDCDCDC;
pub const INK: u32 = 0x000000;
pub const WARN: u32 = 0xFF9F0A;

/// Мягкое белое свечение вокруг белых кнопок.
pub fn glow() -> Vec<BoxShadow> {
    let mut c = rgb(WHITE);
    c.a = 0.38;
    vec![BoxShadow {
        color: c.into(),
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(14.0),
        spread_radius: px(0.0),
    }]
}

pub const UI_FONT: &str = "Segoe UI";

pub const PALETTE: [u32; 8] = [
    0xFF3B5C, 0xFF9F0A, 0xFFD60A, 0x30D158, 0x0A84FF, 0x7C5CFF, 0xFFFFFF, 0x15161A,
];

fn icon_body(name: &str) -> Option<&'static str> {
    Some(match name {
        "select" => r#"<path d="M5 3.5l13.5 7.2-5.6 1.7-2.2 5.6z"/>"#,
        "pen" => {
            r#"<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>"#
        }
        "marker" => {
            r#"<path d="m9 11-6 6v3h9l3-3"/><path d="m22 12-4.6 4.6a2 2 0 0 1-2.8 0l-5.2-5.2a2 2 0 0 1 0-2.8L14 4"/>"#
        }
        "line" => r#"<path d="M5 19 19 5"/>"#,
        "arrow" => r#"<path d="M8 6h10v10"/><path d="M6 18 18 6"/>"#,
        "rect" => r#"<rect x="3" y="5" width="18" height="14" rx="2"/>"#,
        "ellipse" => r#"<circle cx="12" cy="12" r="9"/>"#,
        "text" => r#"<path d="M4 7V4h16v3"/><path d="M9 20h6"/><path d="M12 4v16"/>"#,
        // мозаика: так обычно обозначают скрытие/пикселизацию
        "blur" => {
            r#"<rect x="1.5" y="1.5" width="7" height="7" fill="black" stroke="none"/><rect x="8.5" y="1.5" width="7" height="7" fill="black" stroke="none" opacity=".4"/><rect x="15.5" y="1.5" width="7" height="7" fill="black" stroke="none"/><rect x="1.5" y="8.5" width="7" height="7" fill="black" stroke="none" opacity=".4"/><rect x="8.5" y="8.5" width="7" height="7" fill="black" stroke="none"/><rect x="15.5" y="8.5" width="7" height="7" fill="black" stroke="none" opacity=".4"/><rect x="1.5" y="15.5" width="7" height="7" fill="black" stroke="none"/><rect x="8.5" y="15.5" width="7" height="7" fill="black" stroke="none" opacity=".4"/><rect x="15.5" y="15.5" width="7" height="7" fill="black" stroke="none"/>"#
        }
        // подсветка: яркий прямоугольник внутри приглушённого кадра
        "spot" => {
            r#"<rect x="2" y="4" width="20" height="16" rx="2" opacity=".45"/><rect x="7" y="8" width="10" height="8" rx="1" fill="black"/>"#
        }
        "pin" => {
            r#"<path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>"#
        }
        // три «глазка» с точкой внутри и россыпь модулей в четвёртом углу
        "qr" => {
            r#"<rect x="2.5" y="2.5" width="8" height="8" rx="2"/><rect x="13.5" y="2.5" width="8" height="8" rx="2"/><rect x="2.5" y="13.5" width="8" height="8" rx="2"/><rect x="5.4" y="5.4" width="2.2" height="2.2" rx=".5" fill="black" stroke="none"/><rect x="16.4" y="5.4" width="2.2" height="2.2" rx=".5" fill="black" stroke="none"/><rect x="5.4" y="16.4" width="2.2" height="2.2" rx=".5" fill="black" stroke="none"/><rect x="13" y="13" width="3.6" height="3.6" rx=".9" fill="black" stroke="none"/><rect x="18.6" y="13" width="3.4" height="3.4" rx=".9" fill="black" stroke="none" opacity=".55"/><rect x="13" y="18.6" width="3.4" height="3.4" rx=".9" fill="black" stroke="none" opacity=".55"/><rect x="18.4" y="18.4" width="3.6" height="3.6" rx=".9" fill="black" stroke="none"/>"#
        }
        "ocr" => {
            r#"<path d="M3 7V5a2 2 0 0 1 2-2h2"/><path d="M17 3h2a2 2 0 0 1 2 2v2"/><path d="M21 17v2a2 2 0 0 1-2 2h-2"/><path d="M7 21H5a2 2 0 0 1-2-2v-2"/><path d="M7 8h8"/><path d="M7 12h10"/><path d="M7 16h6"/>"#
        }
        "undo" => {
            r#"<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"/>"#
        }
        "redo" => {
            r#"<path d="m15 14 5-5-5-5"/><path d="M20 9H9.5A5.5 5.5 0 0 0 4 14.5A5.5 5.5 0 0 0 9.5 20H13"/>"#
        }
        "save" => r#"<path d="M12 3v12"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/>"#,
        "copy" => {
            r#"<rect x="8" y="8" width="13" height="13" rx="2"/><path d="M4 16V5a1 1 0 0 1 1-1h11"/>"#
        }
        "close" => r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#,
        "crop" => r#"<path d="M6 2v14a2 2 0 0 0 2 2h14"/><path d="M18 22V8a2 2 0 0 0-2-2H2"/>"#,
        "monitor" => {
            r#"<rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/>"#
        }
        "check" => r#"<path d="M20 6 9 17l-5-5"/>"#,
        "folder" => {
            r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#
        }
        "alert" => r#"<circle cx="12" cy="12" r="9"/><path d="M12 8v5"/><path d="M12 16.5v.01"/>"#,
        _ => return None,
    })
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let name = path
            .strip_prefix("icons/")
            .and_then(|p| p.strip_suffix(".svg"))
            .unwrap_or("");
        Ok(icon_body(name).map(|body| {
            Cow::Owned(
                format!(
                    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##
                )
                .into_bytes(),
            )
        }))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

pub fn icon(name: &str) -> Svg {
    svg().path(format!("icons/{name}.svg")).flex_none()
}
