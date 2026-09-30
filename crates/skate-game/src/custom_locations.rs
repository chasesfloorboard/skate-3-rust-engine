//! Imported locations (tools/import_skate2.py): one directory per location in
//! assets/private/custom-locations/<key>/ holding location.json, the .skate map
//! and its photos. They join the maps list and the challenge map's travel list.
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub(crate) const DIRECTORY: &str = "private/custom-locations";

#[derive(Deserialize)]
pub(crate) struct Spot {
    pub id: String,
    pub name: String,
    pub matrix: [[f32; 4]; 4],
    #[serde(default)]
    pub description: Option<String>,
    /// Photo file in the location directory.
    #[serde(default)]
    pub image: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct Location {
    version: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Map file name in the location directory.
    pub map: String,
    #[serde(default)]
    pub image: Option<String>,
    /// The first spot is where the location starts.
    pub destinations: Vec<Spot>,
    /// Source disc ("skate2"/"skate3"); locations without one are Freeskate only.
    #[serde(default)]
    game: Option<crate::editions::Game>,
}

pub(crate) struct Loaded {
    pub key: String,
    pub location: Location,
    pub map_path: PathBuf,
}

impl Loaded {
    /// Travel row for the location itself (its first spot).
    pub fn start_id(&self) -> String { format!("custom:{}", self.key) }
    pub fn spot_id(&self, spot: &Spot) -> String { format!("custom:{}:{}", self.key, spot.id) }
    /// Map id used by travel rows: the .skate file stem.
    pub fn map_name(&self) -> String {
        self.map_path.file_stem().unwrap_or_default().to_string_lossy().into_owned()
    }
    /// Source disc. Imports made before the field existed are Skate 2's (S2 keys).
    pub fn game(&self) -> Option<crate::editions::Game> {
        self.location.game.or_else(|| self.key.starts_with("S2").then_some(crate::editions::Game::Skate2))
    }
    /// Asset path of a photo in this location's directory.
    pub fn asset(&self, file: &str) -> String { format!("{DIRECTORY}/{}/{file}", self.key) }
}

fn plain(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', ':']) && !matches!(name, "." | "..")
}

/// Locations belonging to the running edition.
pub(crate) fn all(assets: &Path) -> Vec<Loaded> {
    let edition = crate::editions::current();
    scan(assets).into_iter().filter(|l| edition.shows(l.game())).collect()
}

/// Every installed location, whatever the edition.
pub(crate) fn scan(assets: &Path) -> Vec<Loaded> {
    let mut result = Vec::new();
    let Ok(entries) = std::fs::read_dir(assets.join(DIRECTORY)) else { return result };
    for entry in entries.flatten() {
        let dir = entry.path();
        let key = entry.file_name().to_string_lossy().into_owned();
        let Ok(bytes) = std::fs::read(dir.join("location.json")) else { continue };
        let location: Location = match serde_json::from_slice(&bytes) {
            Ok(l) => l,
            Err(e) => { bevy::log::warn!("Custom location {key}: {e}"); continue }
        };
        let images_ok = location.image.iter().chain(location.destinations.iter().filter_map(|d| d.image.as_ref())).all(|i| plain(i));
        if location.version != 1 || !plain(&key) || !plain(&location.map) || !images_ok
            || location.destinations.is_empty() || location.title.is_empty() {
            bevy::log::warn!("Custom location {key}: invalid location.json");
            continue;
        }
        let map_path = dir.join(&location.map);
        if !map_path.is_file() { bevy::log::warn!("Custom location {key}: missing {}", location.map); continue; }
        result.push(Loaded { key, location, map_path });
    }
    result.sort_by(|a, b| a.location.title.to_lowercase().cmp(&b.location.title.to_lowercase()));
    result
}
