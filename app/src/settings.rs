//! User preferences kept between sessions: colour scheme and text size.
//! They are stored in `$XDG_CONFIG_HOME/logibuild/settings`, one
//! `key = value` per line; unknown keys and values are ignored.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scheme {
    /// Light or dark as the desktop is.
    #[default]
    System,
    Light,
    Dark,
}

impl Scheme {
    pub const ALL: [Scheme; 3] = [Scheme::System, Scheme::Light, Scheme::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Scheme::System => "Follow system",
            Scheme::Light => "Light",
            Scheme::Dark => "Dark",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Scheme::System => "system",
            Scheme::Light => "light",
            Scheme::Dark => "dark",
        }
    }
}

/// Text sizes accepted, in percent of the default.
pub const TEXT_SIZE_RANGE: std::ops::RangeInclusive<u32> = 50..=200;
/// Text sizes offered in the settings menu, in percent.
pub const TEXT_SIZES: [u32; 6] = [90, 100, 110, 125, 150, 175];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub scheme: Scheme,
    /// Percent of the default text size, in [`TEXT_SIZE_RANGE`].
    pub text_size: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { scheme: Scheme::System, text_size: 100 }
    }
}

impl Settings {
    /// The file of the current user, when a configuration directory is known.
    pub fn user_path() -> Option<PathBuf> {
        let dir = match std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
            Some(d) => PathBuf::from(d),
            None => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
        };
        Some(dir.join("logibuild").join("settings"))
    }

    /// Reads `path`; a missing or unreadable file gives the defaults.
    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path).map(|t| Settings::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_text())
    }

    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "scheme" => {
                    if let Some(&scheme) = Scheme::ALL.iter().find(|s| s.key() == value) {
                        s.scheme = scheme;
                    }
                }
                "text_size" => {
                    if let Some(n) = value.parse().ok().filter(|n| TEXT_SIZE_RANGE.contains(n)) {
                        s.text_size = n;
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn to_text(self) -> String {
        format!("scheme = {}\ntext_size = {}\n", self.scheme.key(), self.text_size)
    }
}
