//! Game editions. Skate 2 and Skate 3 each show only their own disc's content;
//! Freeskate mixes everything from both. The start-up picker
//! (edition_picker.rs) chooses one and relaunches with --edition.
use std::{path::Path, sync::OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edition { Skate2, Skate3, Freeskate }

/// The disc a piece of content came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Game { Skate2, Skate3 }

impl Edition {
    pub const ALL: [Edition; 3] = [Edition::Skate2, Edition::Skate3, Edition::Freeskate];
    pub fn key(self) -> &'static str {
        match self { Edition::Skate2 => "skate2", Edition::Skate3 => "skate3", Edition::Freeskate => "freeskate" }
    }
    pub fn parse(value: &str) -> Result<Self, String> {
        Self::ALL.into_iter().find(|e| e.key() == value)
            .ok_or_else(|| format!("Unknown edition {value:?}: use skate2, skate3 or freeskate"))
    }
    pub fn title(self) -> &'static str {
        match self { Edition::Skate2 => "skate 2", Edition::Skate3 => "skate 3", Edition::Freeskate => "freeskate" }
    }
    pub fn description(self) -> &'static str {
        match self {
            Edition::Skate2 => "New San Vanelona and everything from the Skate 2 disc.",
            Edition::Skate3 => "Port Carverton and everything from the Skate 3 disc.",
            Edition::Freeskate => "Every map, spot and skater from both games together.",
        }
    }
    pub fn window_title(self) -> &'static str {
        match self { Edition::Skate2 => "Skate 2 Rust", Edition::Skate3 => "Skate 3 Rust", Edition::Freeskate => "Skate Rust: Freeskate" }
    }
    /// Whether content from `game` belongs to this edition.
    pub fn shows(self, game: Option<Game>) -> bool {
        match self {
            Edition::Freeskate => true,
            Edition::Skate2 => game == Some(Game::Skate2),
            Edition::Skate3 => game == Some(Game::Skate3),
        }
    }
}

static CURRENT: OnceLock<Edition> = OnceLock::new();

/// The running edition. Development and verification runs that never chose
/// one see everything, as before editions existed.
pub(crate) fn current() -> Edition { CURRENT.get().copied().unwrap_or(Edition::Freeskate) }
pub(crate) fn set(edition: Edition) { let _ = CURRENT.set(edition); }

/// Editions this installation can offer: a game's edition needs its disc's
/// content, and Freeskate needs either.
pub(crate) fn installed(assets: &Path) -> Vec<Edition> {
    let skate3 = assets.join("private/game.json").is_file();
    let skate2 = crate::custom_locations::scan(assets).iter().any(|l| l.game() == Some(Game::Skate2));
    let mut editions = Vec::new();
    if skate2 { editions.push(Edition::Skate2); }
    if skate3 { editions.push(Edition::Skate3); }
    if skate2 || skate3 { editions.push(Edition::Freeskate); }
    editions
}

fn last_path(assets: &Path) -> std::path::PathBuf {
    assets.parent().unwrap_or(assets).join("settings/edition.json")
}
/// The edition chosen last time, highlighted when the picker opens.
pub(crate) fn last(assets: &Path) -> Option<Edition> {
    let bytes = std::fs::read(last_path(assets)).ok()?;
    Edition::parse(&serde_json::from_slice::<String>(&bytes).ok()?).ok()
}
pub(crate) fn remember(assets: &Path, edition: Edition) {
    let path = last_path(assets);
    if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
    let _ = std::fs::write(path, serde_json::to_vec(edition.key()).unwrap_or_default());
}

/// Per-edition default map pointer. Freeskate keeps the original file so
/// existing installations start where they left off.
pub(crate) fn default_map_file(edition: Edition) -> String {
    match edition {
        Edition::Freeskate => "default-map.json".into(),
        other => format!("default-map-{}.json", other.key()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editions_parse_and_filter_by_game() {
        for edition in Edition::ALL { assert_eq!(Edition::parse(edition.key()).unwrap(), edition); }
        assert!(Edition::parse("skate1").is_err());
        assert!(Edition::Skate2.shows(Some(Game::Skate2)) && !Edition::Skate2.shows(Some(Game::Skate3)));
        assert!(!Edition::Skate3.shows(None) && Edition::Freeskate.shows(None));
    }
}
