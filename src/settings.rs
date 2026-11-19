//! Настройки приложения: хранятся в %APPDATA%\Glint\settings.json.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub vk: u32,
}

impl Hotkey {
    pub const fn new(ctrl: bool, alt: bool, shift: bool, vk: u32) -> Self {
        Self {
            ctrl,
            alt,
            shift,
            win: false,
            vk,
        }
    }

    /// Годится ли сочетание как глобальное. Клавиши, которыми печатают и правят текст,
    /// без Ctrl, Alt или Win назначать нельзя: иначе ими стало бы невозможно пользоваться.
    pub fn usable(&self) -> bool {
        if self.ctrl || self.alt || self.win {
            return true;
        }
        !matches!(
            self.vk,
            0x08 | 0x09 | 0x0D | 0x1B | 0x20 | 0x25..=0x28 | 0x2E | 0x30..=0x39 | 0x41..=0x5A
                | 0x60..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDF | 0xE2
        )
    }

    pub fn parts(&self) -> Vec<String> {
        let mut v = Vec::new();
        if self.ctrl {
            v.push("Ctrl".to_string());
        }
        if self.alt {
            v.push("Alt".to_string());
        }
        if self.shift {
            v.push("Shift".to_string());
        }
        if self.win {
            v.push("Win".to_string());
        }
        v.push(vk_name(self.vk));
        v
    }
}

pub fn vk_name(vk: u32) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x60..=0x69 => format!("Num {}", vk - 0x60),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x2C => "PrtSc".into(),
        0x20 => "Space".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x2D => "Insert".into(),
        0x2E => "Delete".into(),
        0x24 => "Home".into(),
        0x23 => "End".into(),
        0x21 => "PgUp".into(),
        0x22 => "PgDn".into(),
        0x25 => "←".into(),
        0x26 => "↑".into(),
        0x27 => "→".into(),
        0x28 => "↓".into(),
        0x13 => "Pause".into(),
        0x08 => "Backspace".into(),
        0x1B => "Esc".into(),
        0x5D => "Menu".into(),
        0x90 => "NumLk".into(),
        0x6A => "Num *".into(),
        0x6B => "Num +".into(),
        0x6D => "Num -".into(),
        0x6E => "Num .".into(),
        0x6F => "Num /".into(),
        0xAD => "Mute".into(),
        0xAE => "Vol-".into(),
        0xAF => "Vol+".into(),
        0xB0 => "Next".into(),
        0xB1 => "Prev".into(),
        0xB2 => "Stop".into(),
        0xB3 => "Play".into(),
        0x91 => "ScrLk".into(),
        0xC0 => "`".into(),
        0xBD => "-".into(),
        0xBB => "=".into(),
        0xDB => "[".into(),
        0xDD => "]".into(),
        0xDC => "\\".into(),
        0xBA => ";".into(),
        0xDE => "'".into(),
        0xBC => ",".into(),
        0xBE => ".".into(),
        0xBF => "/".into(),
        _ => format!("0x{vk:02X}"),
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    /// PNG без потерь, максимальное сжатие.
    Png,
    /// PNG с палитрой 256 цветов — в разы легче, почти без видимых потерь на интерфейсах.
    Png8,
    /// JPEG с настраиваемым качеством.
    Jpeg,
    /// WebP без потерь.
    Webp,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Png, Format::Png8, Format::Jpeg, Format::Webp];

    pub fn ext(&self) -> &'static str {
        match self {
            Format::Png | Format::Png8 => "png",
            Format::Jpeg => "jpg",
            Format::Webp => "webp",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Format::Png => "PNG",
            Format::Png8 => "PNG-8",
            Format::Jpeg => "JPEG",
            Format::Webp => "WebP",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    pub hotkey_region: Hotkey,
    pub hotkey_full: Hotkey,
    pub format: Format,
    pub jpeg_quality: u8,
    pub auto_save: bool,
    pub save_dir: PathBuf,
    /// Копировать сразу после выделения области, без редактора.
    pub instant_copy: bool,
    /// Выпрямлять нарисованное от руки: линии, прямоугольники, эллипсы.
    pub auto_shapes: bool,
    pub color: u32,
    pub thickness: f32,
    pub text_weight: u16,
    pub text_size: f32,
    pub text_outline: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let save_dir = dirs::picture_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Glint");
        Self {
            hotkey_region: Hotkey::new(true, false, true, 0x53), // Ctrl+Shift+S
            hotkey_full: Hotkey::new(true, false, true, 0x46),   // Ctrl+Shift+F
            format: Format::Png,
            jpeg_quality: 85,
            auto_save: false,
            save_dir,
            instant_copy: false,
            auto_shapes: true,
            color: 0xFF3B5C,
            thickness: 4.0,
            text_weight: 600,
            text_size: 26.0,
            text_outline: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_typing_keys_are_not_usable() {
        let bare = |vk| Hotkey { ctrl: false, alt: false, shift: false, win: false, vk };
        // Print Screen, Pause, F8, Insert — можно и без модификаторов
        for vk in [0x2C, 0x13, 0x77, 0x2D] {
            assert!(bare(vk).usable(), "{vk:#x}");
        }
        // буква, цифра, пробел, Enter, стрелка — нельзя, даже с Shift
        for vk in [0x53, 0x31, 0x20, 0x0D, 0x25] {
            assert!(!bare(vk).usable(), "{vk:#x}");
            assert!(!Hotkey { shift: true, ..bare(vk) }.usable(), "{vk:#x}");
            assert!(Hotkey { win: true, shift: true, ..bare(vk) }.usable(), "{vk:#x}");
        }
        assert_eq!(bare(0x2C).parts(), ["PrtSc"]);
    }
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Glint").join("settings.json"))
}

/// Настройки, оставшиеся от прежнего названия приложения (Kadr).
fn legacy_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Kadr").join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .or_else(|| legacy_path().and_then(|p| std::fs::read_to_string(p).ok()))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, s);
            }
        }
    }
}
