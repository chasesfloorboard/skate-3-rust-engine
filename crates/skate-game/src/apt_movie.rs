//! Original APT movie hierarchy and timeline control, independent of Bevy.
use crate::{
    apt_display::{Control, DisplayList, Placement},
    apt_vm::{Instruction, ObjectKind, Value, Vm},
};
use serde::Deserialize;
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Deserialize)]
pub struct Frame {
    pub controls: Vec<Control>,
}
#[derive(Clone, Deserialize)]
pub struct Character {
    pub id: i32,
    pub type_name: String,
    #[serde(default)]
    pub frames: Vec<Frame>,
    pub text: Option<serde_json::Value>,
    pub bounds: Option<[f32; 4]>,
}
#[derive(Clone)]
pub struct Instance {
    pub character: i32,
    pub frame: usize,
    pub playing: bool,
    pub children: BTreeMap<i32, usize>,
    pub placement: Option<Placement>,
    /// Clips made by script (createEmptyMovieClip, attachMovie, loadClip).
    /// Timeline seeks never remove them.
    pub dynamic: BTreeMap<i32, usize>,
}
pub struct Movie {
    pub characters: BTreeMap<i32, Character>,
    pub instances: BTreeMap<usize, Instance>,
    pub actions: BTreeMap<String, Vec<Instruction>>,
    pub pending: VecDeque<(usize, u32)>,
    pub root: usize,
    pub text_assets: crate::apt_text::TextAssets,
    states: BTreeMap<i32, Vec<DisplayList>>,
    /// Prototype for every clip this movie creates (MovieClip.prototype).
    pub clip_prototype: Option<usize>,
    /// Exported symbols: name -> character id.
    pub exports: BTreeMap<String, i32>,
    /// Imported characters: id -> (movie file, exported symbol).
    pub imports: BTreeMap<i32, (String, String)>,
    /// Placed imports waiting for their library movie: (clip, file, symbol,
    /// parent). The stage instantiates them before the next script runs.
    pub pending_imports: Vec<(usize, String, String, Option<usize>)>,
    /// Placed clips that became another movie's (imports): clip -> the
    /// import's character id here, and their last placement.
    imported: BTreeMap<usize, (i32, Option<Placement>)>,
    /// Imported clips this timeline dropped; the stage removes them from
    /// their library movie.
    pub removed_imports: Vec<usize>,
}
#[derive(Deserialize)]
struct Import {
    file: String,
    name: String,
    character_id: i32,
}
/// Character id of script-created empty clips.
pub const EMPTY_CLIP: i32 = -1;
impl Movie {
    pub fn load(json: &serde_json::Value) -> Result<Self, String> {
        let mut characters: Vec<Character> =
            serde_json::from_value(json["characters"].clone()).map_err(|e| e.to_string())?;
        characters.push(Character { id: EMPTY_CLIP, type_name: "empty".into(), frames: vec![], text: None, bounds: None });
        let mut states = BTreeMap::new();
        for c in &characters {
            let mut list = DisplayList::default();
            let mut frames = Vec::new();
            for frame in &c.frames {
                for control in &frame.controls {
                    list.apply(control)?;
                }
                frames.push(list.clone());
            }
            states.insert(c.id, frames);
        }
        Ok(Self {
            characters: characters.into_iter().map(|c| (c.id, c)).collect(),
            instances: BTreeMap::new(),
            actions: serde_json::from_value(json["actions"].clone()).map_err(|e| e.to_string())?,
            pending: VecDeque::new(),
            root: usize::MAX,
            text_assets: crate::apt_text::TextAssets::load(json)?,
            states,
            clip_prototype: None,
            exports: json["exports"].as_array().into_iter().flatten()
                .filter_map(|e| Some((e["name"].as_str()?.to_string(), e["character_id"].as_i64()? as i32))).collect(),
            imports: serde_json::from_value::<Vec<Import>>(json["imports"].clone()).unwrap_or_default()
                .into_iter().map(|i| (i.character_id, (i.file, i.name))).collect(),
            pending_imports: Vec::new(),
            imported: BTreeMap::new(),
            removed_imports: Vec::new(),
        })
    }
    /// An empty stand-in while a movie is temporarily moved out of a list.
    pub fn placeholder() -> Self {
        Self {
            characters: BTreeMap::new(),
            instances: BTreeMap::new(),
            actions: BTreeMap::new(),
            pending: VecDeque::new(),
            root: usize::MAX,
            text_assets: Default::default(),
            states: BTreeMap::new(),
            clip_prototype: None,
            exports: BTreeMap::new(),
            imports: BTreeMap::new(),
            pending_imports: Vec::new(),
            imported: BTreeMap::new(),
            removed_imports: Vec::new(),
        }
    }
    pub fn initialize(&mut self, vm: &mut Vm) -> Result<(), String> {
        self.root = self.create(vm, 0, None, 0)?;
        vm.set(self.root, "_root", Value::Object(self.root))?;
        // Every clip sees the same authored root, including unnamed children.
        for id in self.instances.keys() {
            vm.set(*id, "_root", Value::Object(self.root))?;
        }
        Ok(())
    }
    fn create(
        &mut self,
        vm: &mut Vm,
        character: i32,
        parent: Option<usize>,
        depth: usize,
    ) -> Result<usize, String> {
        let id = vm.object(ObjectKind::Native(format!("movie:{character}")));
        self.attach(vm, id, character, parent, depth)?;
        Ok(id)
    }
    /// Give an existing clip object this movie's character: loadClip into a
    /// target keeps the target's identity, as in Flash.
    pub fn adopt(&mut self, vm: &mut Vm, id: usize, character: i32, parent: Option<usize>) -> Result<(), String> {
        vm.objects.get_mut(id).ok_or("Invalid APT clip handle")?.kind = ObjectKind::Native(format!("movie:{character}"));
        if self.root == usize::MAX {
            self.root = id;
        }
        self.attach(vm, id, character, parent, 0)
    }
    /// Give an existing clip (another movie's import placeholder) one of this
    /// movie's characters, keeping the clip's identity.
    pub fn adopt_import(&mut self, vm: &mut Vm, id: usize, character: i32, parent: Option<usize>) -> Result<(), String> {
        vm.objects.get_mut(id).ok_or("Invalid APT clip handle")?.kind = ObjectKind::Native(format!("movie:{character}"));
        self.attach(vm, id, character, parent, 0)
    }
    /// A script-made child clip under `parent` (which may belong to another movie).
    pub fn create_child(&mut self, vm: &mut Vm, character: i32, parent: usize) -> Result<usize, String> {
        self.create(vm, character, Some(parent), 0)
    }
    fn attach(&mut self, vm: &mut Vm, id: usize, character: i32, parent: Option<usize>, depth: usize) -> Result<(), String> {
        if depth > 32 || self.instances.len() > 65536 {
            return Err("APT movie hierarchy limit".into());
        }
        // An imported symbol: an empty clip now, the library's character once
        // the stage has loaded that movie (Flash resolves imports at load).
        let character = if !self.characters.contains_key(&character) && let Some((file, symbol)) = self.imports.get(&character) {
            self.pending_imports.push((id, file.clone(), symbol.clone(), parent));
            self.imported.insert(id, (character, None));
            EMPTY_CLIP
        } else {
            character
        };
        let c = self
            .characters
            .get(&character)
            .ok_or_else(|| format!("APT unknown character {character}"))?
            .clone();
        vm.objects[id].prototype = self.clip_prototype;
        if self.root != usize::MAX {
            vm.set(id, "_root", Value::Object(self.root))?;
        }
        if let Some(parent) = parent {
            vm.set(id, "_parent", Value::Object(parent))?;
        }
        vm.set(id, "_x", Value::Number(0.0))?;
        vm.set(id, "_y", Value::Number(0.0))?;
        vm.set(id, "_visible", Value::Bool(true))?;
        vm.set(id, "_alpha", Value::Number(100.0))?;
        if let Some(text) = &c.text {
            vm.set(
                id,
                "text",
                Value::Text(text["initial_text"].as_str().unwrap_or("").into()),
            )?;
        }
        self.instances.insert(
            id,
            Instance {
                character,
                frame: 0,
                playing: !c.frames.is_empty(),
                children: BTreeMap::new(),
                placement: None,
                dynamic: BTreeMap::new(),
            },
        );
        self.text_changed(vm, id)?;
        if !c.frames.is_empty() {
            self.seek(vm, id, 0, depth + 1)?;
        }
        Ok(())
    }
    pub fn text_changed(&self, vm: &mut Vm, id: usize) -> Result<(), String> {
        let Some(instance) = self.instances.get(&id) else {
            return Ok(());
        };
        let character = &self.characters[&instance.character];
        let Some(text) = &character.text else {
            return Ok(());
        };
        if self.text_assets.fonts.is_empty() {
            return Ok(());
        }
        let value = self.text_assets.localize(&vm.get(id, "text").text());
        vm.set(id, "_displayText", Value::Text(value.clone()))?;
        let font = self
            .text_assets
            .fonts
            .get(&(text["font_id"].as_i64().ok_or("Invalid text font id")? as i32))
            .ok_or("Missing original text font")?;
        let height = text["font_height"].as_f64().ok_or("Invalid text size")? as f32;
        let width = font.width(&value, height);
        vm.set(id, "textWidth", Value::Number(width as f64))?;
        let bounds = character.bounds.ok_or("Missing text bounds")?;
        let autosize = vm.get(id, "autoSize").text();
        vm.set(
            id,
            "_width",
            Value::Number(
                if autosize == "left" || autosize == "right" || autosize == "center" {
                    width
                } else {
                    bounds[2] - bounds[0]
                } as f64,
            ),
        )?;
        Ok(())
    }
    pub fn remove(&mut self, vm: &mut Vm, id: usize) {
        if self.imported.remove(&id).is_some() {
            self.removed_imports.push(id);
        }
        if let Some(instance) = self.instances.remove(&id) {
            for child in instance.children.values().chain(instance.dynamic.values()) {
                self.remove(vm, *child);
            }
        }
        // Retired handles can remain referenced by ActionScript, but no longer
        // participate in timeline advancement or rendering.
        let _ = vm.set(id, "_visible", Value::Bool(false));
    }
    pub fn seek(
        &mut self,
        vm: &mut Vm,
        id: usize,
        frame: usize,
        nesting: usize,
    ) -> Result<(), String> {
        let instance = self
            .instances
            .get(&id)
            .ok_or("APT absent movie instance")?
            .clone();
        let character = self
            .characters
            .get(&instance.character)
            .ok_or("APT absent movie character")?;
        if frame >= character.frames.len() {
            return Err(format!(
                "APT frame {frame} outside character {}",
                character.id
            ));
        }
        let frame_count = character.frames.len();
        let actions: Vec<_> = character.frames[frame]
            .controls
            .iter()
            .filter(|c| c.type_name == "do_action" && c.actions_offset != 0)
            .map(|c| c.actions_offset)
            .collect();
        let list = self.states[&instance.character][frame].clone();
        let mut children = BTreeMap::new();
        for (depth, placement) in list.depths {
            let previous = instance.children.get(&depth).copied();
            if let Some(name) = previous
                .and_then(|old| self.instances.get(&old))
                .and_then(|i| i.placement.as_ref())
                .map(|p| p.name.clone())
            {
                if !name.is_empty() && name != placement.name {
                    vm.objects[id].fields.remove(&name);
                }
            }
            let child = if let Some(old) = previous.filter(|old| {
                self.imported.get(old).is_some_and(|i| i.0 == placement.character)
                    || self.instances.get(old).is_some_and(|i| i.character == placement.character)
            }) {
                old
            } else {
                if let Some(old) = previous {
                    self.remove(vm, old);
                }
                self.create(vm, placement.character, Some(id), nesting + 1)?
            };
            let old = match self.imported.get(&child) {
                Some((_, placed)) => placed.as_ref(),
                None => self.instances[&child].placement.as_ref(),
            };
            if old.is_none_or(|old| old.matrix != placement.matrix) {
                vm.set(child, "_x", Value::Number(placement.matrix[4] as f64))?;
                vm.set(child, "_y", Value::Number(placement.matrix[5] as f64))?;
            }
            if !placement.name.is_empty() {
                vm.set(id, &placement.name, Value::Object(child))?;
            }
            match self.imported.get_mut(&child) {
                Some(entry) => entry.1 = Some(placement),
                None => self.instances.get_mut(&child).unwrap().placement = Some(placement),
            }
            children.insert(depth, child);
        }
        for (depth, child) in &instance.children {
            if !children.contains_key(depth) {
                if let Some(name) = self
                    .instances
                    .get(child)
                    .and_then(|i| i.placement.as_ref())
                    .map(|p| p.name.clone())
                {
                    if !name.is_empty() {
                        vm.objects[id].fields.remove(&name);
                    }
                }
                self.remove(vm, *child);
            }
        }
        let current = self.instances.get_mut(&id).unwrap();
        current.frame = frame;
        current.children = children;
        vm.set(id, "_currentframe", Value::Number((frame + 1) as f64))?;
        vm.set(id, "_totalframes", Value::Number(frame_count as f64))?;
        for offset in actions {
            self.pending.push_back((id, offset));
        }
        Ok(())
    }
    pub fn advance(&mut self, vm: &mut Vm) -> Result<(), String> {
        let playing: Vec<_> = self
            .instances
            .iter()
            .filter(|(_, i)| i.playing)
            .map(|(id, i)| (*id, i.character, i.frame))
            .collect();
        for (id, character, frame) in playing {
            if !self.instances.contains_key(&id) {
                continue;
            }
            let count = self.characters[&character].frames.len();
            if count > 1 {
                self.seek(vm, id, (frame + 1) % count, 0)?;
            }
        }
        Ok(())
    }
    pub fn method(
        &mut self,
        vm: &mut Vm,
        id: usize,
        method: &str,
        args: &[Value],
    ) -> Result<bool, String> {
        if !self.instances.contains_key(&id) {
            return Ok(false);
        }
        match method {
            "stop" => self.instances.get_mut(&id).unwrap().playing = false,
            "play" => self.instances.get_mut(&id).unwrap().playing = true,
            "gotoAndPlay" | "gotoAndStop" => {
                let c = &self.characters[&self.instances[&id].character];
                let frame = match args.first().ok_or("APT goto requires frame")? {
                    Value::Text(label) => c
                        .frames
                        .iter()
                        .position(|f| {
                            f.controls.iter().any(|control| {
                                control.type_name == "frame_label"
                                    && control.label.as_deref() == Some(label)
                            })
                        })
                        .ok_or_else(|| format!("APT character {} lacks label {label}", c.id))?,
                    value => {
                        let frame = value.number();
                        if !frame.is_finite() || frame < 0.0 {
                            return Err(format!(
                                "Invalid APT frame {frame} in {} on character {}",
                                method, c.id
                            ));
                        }
                        // The shipped line timer deliberately requests zero at
                        // full capacity. Frame zero addresses the first frame.
                        (frame as usize).saturating_sub(1)
                    }
                };
                self.seek(vm, id, frame, 0)?;
                self.instances.get_mut(&id).unwrap().playing = method == "gotoAndPlay";
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
