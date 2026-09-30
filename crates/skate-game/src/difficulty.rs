//! Host preference selects the native physics_mode collection, never a multiplier.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const NATIVE_MODES: [&str; 5] = ["easy", "normal", "hardcore", "motorized", "test"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u32)]
pub(crate) enum Difficulty {
    #[default]
    Easy = 0,
    Normal = 1,
    Hardcore = 2,
}
impl Difficulty {
    pub const ALL: [Self; 3] = [Self::Easy, Self::Normal, Self::Hardcore];
    pub fn key(self) -> &'static str { NATIVE_MODES[self as usize] }
    pub fn label(self) -> &'static str {
        match self { Self::Easy => "Easy", Self::Normal => "Normal", Self::Hardcore => "Hardcore" }
    }
    pub fn parse(value: &str) -> Result<Self, String> {
        Self::ALL.into_iter().find(|d| d.key().eq_ignore_ascii_case(value))
            .ok_or_else(|| format!("Unknown difficulty {value:?}; expected easy, normal or hardcore"))
    }
    pub fn path(root: &Path) -> PathBuf {
        root.parent().unwrap_or(root).join("settings/gameplay.json")
    }
    pub fn load(root: &Path) -> Result<Self, String> {
        match std::fs::read(Self::path(root)) {
            Ok(bytes) => serde_json::from_slice::<Saved>(&bytes).map(|s| s.difficulty)
                .map_err(|e| format!("Invalid saved gameplay settings: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Cannot read gameplay settings: {e}")),
        }
    }
    pub fn save(self, root: &Path) -> Result<(), String> {
        write(root, Saved { difficulty: self, ..saved(root) })
    }
}
#[derive(Default, Serialize, Deserialize)]
struct Saved { difficulty: Difficulty, #[serde(default)] physics: Feel }

fn saved(root: &Path) -> Saved {
    std::fs::read(Difficulty::path(root)).ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}
fn write(root: &Path, saved: Saved) -> Result<(), String> {
    let path = Difficulty::path(root);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(path, serde_json::to_vec_pretty(&saved).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Which game's physics tuning loads: Skate 3's vault, or Skate 2's
/// (tools/skate2/physics.py). The Skate 2 and Skate 3 editions fix it;
/// Freeskate keeps the player's choice in gameplay.json.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Feel {
    #[default]
    Skate3,
    Skate2,
}
static FEEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
impl Feel {
    pub const SKATE2_FILE: &str = "skater-collections-skate2.json";
    pub fn current() -> Self {
        if FEEL.load(std::sync::atomic::Ordering::Relaxed) { Self::Skate2 } else { Self::Skate3 }
    }
    pub fn set(self) { FEEL.store(self == Self::Skate2, std::sync::atomic::Ordering::Relaxed); }
    pub fn label(self) -> &'static str { match self { Self::Skate3 => "Skate 3", Self::Skate2 => "Skate 2" } }
    pub fn toggled(self) -> Self { match self { Self::Skate3 => Self::Skate2, Self::Skate2 => Self::Skate3 } }
    /// The edition's fixed feel, or the saved Freeskate choice.
    pub fn for_edition(root: &Path) -> Self {
        match crate::editions::current() {
            crate::editions::Edition::Skate2 => Self::Skate2,
            crate::editions::Edition::Skate3 => Self::Skate3,
            crate::editions::Edition::Freeskate => saved(root).physics,
        }
    }
    pub fn save(self, root: &Path) -> Result<(), String> {
        write(root, Saved { physics: self, ..saved(root) })
    }
    /// Skate 3's vault stands in when Skate 2's was never converted.
    pub fn collections_file(self, root: &Path) -> &'static str {
        if self == Self::Skate2 && root.join("private/stock").join(Self::SKATE2_FILE).is_file() {
            Self::SKATE2_FILE
        } else {
            "skater-collections.json"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retail_indices_and_saved_names_agree() {
        for (index, d) in Difficulty::ALL.into_iter().enumerate() {
            assert_eq!(d as usize, index);
            assert_eq!(Difficulty::parse(d.label()).unwrap(), d);
            let bytes = serde_json::to_vec(&Saved { difficulty: d }).unwrap();
            assert_eq!(serde_json::from_slice::<Saved>(&bytes).unwrap().difficulty, d);
        }
        assert!(Difficulty::parse("motorized").is_err());
        assert!(serde_json::from_str::<Saved>(r#"{"difficulty":"made-up"}"#).is_err());
    }
}
