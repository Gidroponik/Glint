//! Уведомления о результате: нативные уведомления Windows от значка в трее.
//! (Отдельное окно GPUI здесь не годится: чтобы оно отрисовалось, его пришлось бы
//! активировать, а это отнимало бы фокус у приложения, в котором работает пользователь.)

use crate::hotkeys;
use gpui::App;

pub fn show(title: impl Into<String>, detail: impl Into<String>, ok: bool, _cx: &mut App) {
    hotkeys::notify(title.into(), detail.into(), ok);
}
