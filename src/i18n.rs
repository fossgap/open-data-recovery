//! Embedded UI translations. Backend identifiers, stored records and CLI output stay stable.
use gtk::prelude::*;
use std::{collections::BTreeMap, path::PathBuf, sync::OnceLock};

pub mod smoke;
mod widgets;
pub use widgets::{builder, language_action};

pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("fr", "Français"),
    ("es", "Español"),
    ("zh_CN", "简体中文"),
    ("hi", "हिन्दी"),
    ("ar", "العربية"),
    ("pt", "Português"),
];

const CATALOGS: &[(&str, &str)] = &[
    ("fr", include_str!("i18n/fr.json")),
    ("es", include_str!("i18n/es.json")),
    ("zh_CN", include_str!("i18n/zh_CN.json")),
    ("hi", include_str!("i18n/hi.json")),
    ("ar", include_str!("i18n/ar.json")),
    ("pt", include_str!("i18n/pt.json")),
];

struct Locale {
    code: &'static str,
    messages: BTreeMap<String, String>,
}

static LOCALE: OnceLock<Locale> = OnceLock::new();

fn resolve(value: &str) -> Option<&'static str> {
    let normalized = value.trim().replace('-', "_").to_ascii_lowercase();
    let language = normalized.split(['.', '@']).next().unwrap_or("");
    if matches!(language, "c" | "posix") {
        return Some("en");
    }
    if language.starts_with("zh") {
        return match language {
            "zh" | "zh_cn" | "zh_sg" | "zh_hans" | "zh_hans_cn" | "zh_hans_sg" => Some("zh_CN"),
            _ => None,
        };
    }
    let base = language.split('_').next().unwrap_or("");
    LANGUAGES
        .iter()
        .find(|(code, _)| *code == base)
        .map(|(code, _)| *code)
}

fn system_language(language: &str, locale: &str) -> &'static str {
    // LANGUAGE is ignored in the C/POSIX locale, as with gettext.
    if matches!(locale, "C" | "POSIX") {
        return "en";
    }
    language
        .split(':')
        .chain(std::iter::once(locale))
        .find_map(resolve)
        .unwrap_or("en")
}

fn config_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(".config"))
        })
        .map(|p| p.join("open-data-recovery/language"))
}

fn preference() -> String {
    read_preference(config_path().as_deref())
}

fn read_preference(path: Option<&std::path::Path>) -> String {
    path.and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| LANGUAGES.iter().any(|(code, _)| code == s))
        .unwrap_or_else(|| "system".into())
}

fn save_preference(code: &str) -> std::io::Result<()> {
    let path = config_path().ok_or_else(|| std::io::Error::other("No configuration directory"))?;
    write_preference(&path, code)
}

fn write_preference(path: &std::path::Path, code: &str) -> std::io::Result<()> {
    if code != "system" && !LANGUAGES.iter().any(|(language, _)| *language == code) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Unknown language",
        ));
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    // A private, newly created temporary file prevents following an existing symlink.
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| {
        writeln!(file, "{code}")?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

pub fn init() {
    LOCALE.get_or_init(|| {
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .find_map(|key| std::env::var(key).ok().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "C".into());
        let code = std::env::var("ODR_LANGUAGE")
            .ok()
            .and_then(|s| resolve(&s))
            .or_else(|| resolve(&preference()))
            .unwrap_or_else(|| {
                system_language(&std::env::var("LANGUAGE").unwrap_or_default(), &locale)
            });
        let messages = CATALOGS
            .iter()
            .find(|(language, _)| *language == code)
            .map(|(_, json)| serde_json::from_str(json).expect("Validated translation catalog"))
            .unwrap_or_default();
        Locale { code, messages }
    });
}

pub fn language() -> &'static str {
    LOCALE.get().map_or("en", |locale| locale.code)
}

pub fn direction() {
    gtk::Widget::set_default_direction(if language() == "ar" {
        gtk::TextDirection::Rtl
    } else {
        gtk::TextDirection::Ltr
    });
}

pub fn tr(message: &str) -> &str {
    LOCALE
        .get()
        .and_then(|locale| locale.messages.get(message))
        .map_or(message, String::as_str)
}

/// Substitute named fields once, without interpreting braces inside filenames or diagnostics.
pub fn interpolate(template: &str, values: &[(&str, String)]) -> String {
    let mut result = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}').map(|n| n + start) else {
            break;
        };
        result.push_str(&rest[..start]);
        let key = &rest[start + 1..end];
        if let Some((_, value)) = values.iter().find(|(name, _)| *name == key) {
            result.push_str(value);
        } else {
            result.push_str(&rest[start..=end]);
        }
        rest = &rest[end + 1..];
    }
    result.push_str(rest);
    result
}

pub fn state(value: &str) -> &str {
    tr(match value {
        "preparing" => "Preparing",
        "scanning" => "Scanning",
        "complete" => "Complete",
        "cancelled" => "Cancelled",
        "failed" => "Failed",
        "recovering" => "Recovering",
        "recovered" => "Recovered",
        "consolidating" => "Consolidating",
        "imaging" => "Imaging",
        "imaged" => "Imaged",
        "partial" => "Partial",
        "inspection" => "Inspection",
        _ => value,
    })
}

#[macro_export]
macro_rules! trf {
    ($message:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::i18n::interpolate($crate::i18n::tr($message), &[$((stringify!($name), ($value).to_string())),*])
    };
}

#[cfg(test)]
mod tests;
