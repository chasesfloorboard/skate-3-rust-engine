//! Installed map discovery and the default pointer, shared by startup and the menu.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub label: String,
    pub path: Option<PathBuf>,
}

pub(crate) fn discover(assets: &Path) -> Vec<Entry> {
    let mut maps = Vec::new();
    let root = assets.parent().unwrap_or(assets).join("maps");
    // Maps converted from the setup disc are Skate 3's.
    let retail = crate::editions::current().shows(Some(crate::editions::Game::Skate3));
    for directory in [&root, &root.join("private")].into_iter().filter(|_| retail) {
        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("skate")) {
                    maps.push(Entry {
                        label: path.file_stem().unwrap().to_string_lossy().replace('_', " "),
                        path: Some(path.canonicalize().unwrap_or(path)),
                    });
                }
            }
        }
    }
    for loc in crate::custom_locations::all(assets) {
        maps.push(Entry { label: loc.location.title.clone(), path: Some(loc.map_path.canonicalize().unwrap_or(loc.map_path)) });
    }
    maps.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    maps.insert(0, Entry { label: "Test world".into(), path: None });
    maps
}

pub(crate) fn default_map(assets: &Path) -> Result<Option<PathBuf>, String> {
    // Only a completed release installation creates this pointer. Existing
    // development checkouts continue to boot the test world.
    let settings = assets.parent().unwrap_or(assets).join("settings");
    let edition = crate::editions::current();
    let pointer = settings.join(crate::editions::default_map_file(edition));
    let bytes = match std::fs::read(&pointer) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && edition != crate::editions::Edition::Freeskate
            && settings.join("default-map.json").is_file() => return Ok(first_map(assets)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", pointer.display())),
    };
    let relative: Option<String> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let Some(relative) = relative else { return Ok(None); };
    let relative = Path::new(&relative);
    if relative.is_absolute() || relative.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return Err("Invalid installed default map path".into());
    }
    Ok(Some(assets.parent().unwrap_or(assets).join(relative)))
}

/// An edition's first launch: its home map (the Skate 2 city, Skate 3's
/// University) or else its first map.
fn first_map(assets: &Path) -> Option<PathBuf> {
    let maps: Vec<PathBuf> = discover(assets).into_iter().filter_map(|e| e.path).collect();
    ["S2SanVanelona", "University"].iter()
        .find_map(|home| maps.iter().find(|p| p.file_stem().is_some_and(|s| s == *home)))
        .or_else(|| maps.first()).cloned()
}

/// Save only a successfully activated map. Null explicitly selects the test world.
pub(crate) fn save_default(assets: &Path, map: Option<&Path>) -> Result<(), String> {
    let root = assets.parent().unwrap_or(assets);
    let relative = map.map(|path| path.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .map_err(|_| "Map is outside this installation; default map unchanged".to_string())).transpose()?;
    let directory = root.join("settings");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let file = crate::editions::default_map_file(crate::editions::current());
    let temporary = directory.join(format!("{file}.tmp"));
    std::fs::write(&temporary, serde_json::to_vec_pretty(&relative).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    std::fs::rename(temporary, directory.join(file)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_pointer_round_trips_map_and_explicit_test_world() {
        let root = std::env::temp_dir().join(format!("skate-map-pointer-{}", std::process::id()));
        let assets = root.join("assets");
        let map = root.join("maps/example.skate");
        save_default(&assets, Some(&map)).unwrap();
        assert_eq!(default_map(&assets).unwrap(), Some(map));
        save_default(&assets, None).unwrap();
        assert_eq!(default_map(&assets).unwrap(), None);
        std::fs::write(root.join("settings/default-map.json"), br#""../outside.skate""#).unwrap();
        assert!(default_map(&assets).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
