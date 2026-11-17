//! Автозапуск вместе с Windows: запись в HKCU\...\Run, приложение стартует сразу в трей.

use windows_registry::CURRENT_USER;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "Glint";

fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" --hidden", exe.display()))
}

/// Переносит автозапуск, включённый под прежним названием (Kadr), на новое имя.
pub fn migrate() {
    const OLD: &str = "Kadr";
    let Ok(key) = CURRENT_USER.create(RUN_KEY) else { return };
    if key.get_string(OLD).is_ok() {
        let _ = key.remove_value(OLD);
        set(true);
    }
}

pub fn enabled() -> bool {
    enabled_as(VALUE)
}

/// Включает или выключает автозапуск. Возвращает true при успехе.
pub fn set(on: bool) -> bool {
    set_as(VALUE, on)
}

fn enabled_as(name: &str) -> bool {
    let current = CURRENT_USER
        .open(RUN_KEY)
        .and_then(|key| key.get_string(name))
        .ok();
    current.is_some() && current == command()
}

fn set_as(name: &str, on: bool) -> bool {
    let Ok(key) = CURRENT_USER.create(RUN_KEY) else {
        return false;
    };
    if on {
        command().is_some_and(|cmd| key.set_string(name, &cmd).is_ok())
    } else {
        key.remove_value(name).is_ok() || !enabled_as(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Пишет и удаляет отдельное тестовое значение, настоящий автозапуск не трогает.
    #[test]
    fn roundtrip_on_test_value() {
        let name = "GlintSelfTest";
        assert!(set_as(name, true));
        assert!(enabled_as(name));
        assert!(set_as(name, false));
        assert!(!enabled_as(name));
    }
}
