//! Встроенный шрифт Montserrat (SIL OFL, см. assets/fonts/OFL.txt) для текстовых пометок.

use gpui::App;
use std::borrow::Cow;

pub const FAMILY: &str = "Montserrat";

/// Доступные начертания: (вес, файл шрифта).
pub const WEIGHTS: [(u16, &[u8]); 6] = [
    (100, include_bytes!("../assets/fonts/Montserrat-Thin.ttf")),
    (300, include_bytes!("../assets/fonts/Montserrat-Light.ttf")),
    (400, include_bytes!("../assets/fonts/Montserrat-Regular.ttf")),
    (600, include_bytes!("../assets/fonts/Montserrat-SemiBold.ttf")),
    (700, include_bytes!("../assets/fonts/Montserrat-Bold.ttf")),
    (900, include_bytes!("../assets/fonts/Montserrat-Black.ttf")),
];

pub fn register(cx: &App) {
    let _ = cx
        .text_system()
        .add_fonts(WEIGHTS.iter().map(|(_, b)| Cow::Borrowed(*b)).collect());
}
