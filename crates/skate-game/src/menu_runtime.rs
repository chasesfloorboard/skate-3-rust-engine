//! The retail front end (fedata.big) on the APT VM: main.apt's ActionScript
//! framework, the screens its ScreenManager loads and the controls they
//! attach, converted by tools/menus/prepare_menus.py. The game engine drives
//! it like the original: ScreenManager.OpenScreen / UpdateInput calls in, and
//! the scripts call back into engine natives (Game, Mission, Audio...).
//! Natives not provided yet are logged by name, never guessed.
use crate::{
    apt_movie::{Movie, EMPTY_CLIP},
    apt_vm::{Host, ObjectKind, Value, Vm},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// Engine objects the scripts reach through _global (tools/menus/natives.py).
pub const NATIVES: [&str; 10] =
    ["Game", "Mission", "HUDComponents", "Audio", "Tricks", "LetterBox", "tos", "ReplayEditor", "Profile", "Online"];

pub struct Stage {
    dir: PathBuf,
    pub movies: Vec<Movie>,
    /// Level number -> root clip (_level0 is main.apt).
    pub levels: BTreeMap<u32, usize>,
    /// Object.registerClass(symbol, "source/controls/x.swf").
    classes: BTreeMap<String, String>,
    /// Object.registerClass(symbol, Class): AS2 linkage of exported symbols.
    linkage: BTreeMap<String, usize>,
    /// Control movies loaded as import libraries: file -> movie index.
    libraries: BTreeMap<String, usize>,
    /// Native class prototypes by name ("MovieClip", "Array", ...).
    prototypes: BTreeMap<&'static str, usize>,
    pub log: Vec<String>,
    /// Set when ScreenManager reports itself constructed (Game.ScreenManagerReady):
    /// from then on the engine may RegisterTemplate/OpenScreen.
    pub screen_manager_ready: bool,
    /// Distinct missing features, for the probe summary.
    pub missing: BTreeSet<String>,
    random: u32,
    /// Flash loads movies between frames, never inside the calling script.
    loads: std::collections::VecDeque<Load>,
}

struct Load {
    url: String,
    target: Target,
    /// MovieClipLoader listeners to notify (onLoadStart ... onLoadInit).
    listeners: Vec<Value>,
}
enum Target {
    Level(u32),
    Clip(usize),
}

pub struct Frontend {
    pub vm: Vm,
    pub stage: Stage,
}

fn class_of(vm: &Vm, mut object: usize) -> Option<String> {
    for _ in 0..32 {
        let o = vm.objects.get(object)?;
        if let ObjectKind::Native(kind) = &o.kind {
            if let Some(name) = kind.strip_prefix("proto:").or_else(|| kind.strip_prefix("native:")) {
                return Some(name.to_string());
            }
            if kind.starts_with("movie:") {
                return Some("MovieClip".into());
            }
        }
        object = o.prototype?;
    }
    None
}

fn array_values(vm: &Vm, id: usize) -> Vec<Value> {
    let n = vm.get(id, "length").number();
    let n = if n.is_finite() && n > 0.0 { (n as usize).min(1 << 16) } else { 0 };
    (0..n).map(|i| vm.get(id, &i.to_string())).collect()
}
fn set_array(vm: &mut Vm, id: usize, values: Vec<Value>) -> Result<(), String> {
    let old = vm.get(id, "length").number();
    if old.is_finite() {
        for i in values.len()..(old.max(0.0) as usize).min(1 << 16) {
            vm.objects[id].fields.remove(&i.to_string());
        }
    }
    let n = values.len();
    for (i, v) in values.into_iter().enumerate() {
        vm.set(id, i.to_string(), v)?;
    }
    vm.set(id, "length", Value::Number(n as f64))
}

impl Frontend {
    /// Load main.apt into _level0 and run it to its first frame, like the
    /// engine's boot. `dir` is the prepare_menus.py output.
    pub fn boot(dir: &Path) -> Result<Self, String> {
        let mut vm = Vm::new();
        vm.timeline_functions = true;
        let mut stage = Stage {
            dir: dir.to_path_buf(),
            movies: Vec::new(),
            levels: BTreeMap::new(),
            classes: BTreeMap::new(),
            linkage: BTreeMap::new(),
            libraries: BTreeMap::new(),
            prototypes: BTreeMap::new(),
            log: Vec::new(),
            screen_manager_ready: false,
            missing: BTreeSet::new(),
            random: 0x2545_F491,
            loads: Default::default(),
        };
        stage.builtins(&mut vm)?;
        vm.begin_update();
        let root = vm.object(ObjectKind::Plain);
        stage.load_into(&mut vm, "source/screens/main.swf", root, None)?;
        stage.levels.insert(0, root);
        vm.set(vm.global, "_level0", Value::Object(root))?;
        let mut frontend = Self { vm, stage };
        frontend.drain()?;
        Ok(frontend)
    }
    /// Run queued frame scripts, then any movies they asked to load (whose
    /// first frames queue more scripts), until everything settles.
    pub fn drain(&mut self) -> Result<(), String> {
        for _ in 0..256 {
            self.run_scripts()?;
            let Some(load) = self.stage.loads.pop_front() else {
                return Ok(());
            };
            self.stage.perform(&mut self.vm, load)?;
        }
        Err("Menu load limit".into())
    }
    fn run_scripts(&mut self) -> Result<(), String> {
        for _ in 0..16384 {
            self.stage.resolve_imports(&mut self.vm)?;
            let Some((movie, object, offset)) = self
                .stage
                .movies
                .iter_mut()
                .enumerate()
                .find_map(|(i, m)| m.pending.pop_front().map(|(o, off)| (i, o, off)))
            else {
                return Ok(());
            };
            let Some(code) = self.stage.movies[movie].actions.get(&offset.to_string()).cloned() else {
                self.stage.log.push(format!("missing action block {offset:x} in movie {movie}"));
                continue;
            };
            if let Err(error) = self.vm.run_on(object, &code, &mut self.stage) {
                self.stage.log.push(format!("script error in movie {movie} at {offset:x}: {error}"));
            }
        }
        Err("Menu frame script limit".into())
    }
    /// One display frame: advance every playing timeline, then run scripts.
    pub fn advance(&mut self) -> Result<(), String> {
        self.vm.begin_update();
        for movie in &mut self.stage.movies {
            movie.advance(&mut self.vm)?;
        }
        self.drain()
    }
    /// Open a front-end screen the way the engine does: register its template
    /// (one depth per template) and load its movie into a new Screen_<depth>.
    pub fn open_screen(&mut self, movie: &str, layer: &str, template: &str) -> Result<(), String> {
        self.screen_manager("RegisterTemplate", vec![Value::Text(template.into())])?;
        self.screen_manager("OpenScreen", vec![Value::Text(movie.into()), Value::Text(layer.into()),
            Value::Text("screen".into()), Value::Text(template.into())])?;
        Ok(())
    }
    /// Call a script function on _global.ScreenManager (OpenScreen, UpdateInput...).
    pub fn screen_manager(&mut self, method: &str, args: Vec<Value>) -> Result<Value, String> {
        self.vm.begin_update();
        let Value::Object(manager) = self.vm.get(self.vm.global, "ScreenManager") else {
            return Err("ScreenManager was not constructed".into());
        };
        let value = self.vm.call_method(manager, method, args, &mut self.stage)?;
        self.drain()?;
        Ok(value)
    }
}

impl Stage {
    /// Instantiate placed imports from their library movies (loaded once, init
    /// actions only, as Flash runs an imported movie's class definitions but
    /// not its root timeline), then apply AS2 linkage.
    fn resolve_imports(&mut self, vm: &mut Vm) -> Result<(), String> {
        let removed: Vec<usize> = self.movies.iter_mut().flat_map(|m| m.removed_imports.drain(..)).collect();
        for clip in removed {
            if let Some(index) = self.movie_of(clip) {
                self.movies[index].remove(vm, clip);
            }
        }
        for _ in 0..64 {
            let pending: Vec<_> = self.movies.iter_mut().flat_map(|m| m.pending_imports.drain(..)).collect();
            if pending.is_empty() {
                return Ok(());
            }
            for (clip, file, symbol, parent) in pending {
                let library = match self.library(vm, &file) {
                    Ok(index) => index,
                    Err(error) => {
                        self.log.push(format!("import {symbol} from {file}: {error}"));
                        continue;
                    }
                };
                let Some(character) = self.movies[library].exports.get(&symbol).copied() else {
                    self.missing.insert(format!("export {symbol} in {file}"));
                    continue;
                };
                if let Some(old) = self.movie_of(clip) {
                    self.movies[old].instances.remove(&clip);
                }
                let mut movie = std::mem::replace(&mut self.movies[library], Movie::placeholder());
                let result = movie.adopt_import(vm, clip, character, parent);
                self.movies[library] = movie;
                result.map_err(|e| format!("{file} {symbol}: {e}"))?;
                self.link(vm, clip, &symbol)?;
            }
        }
        Err("Menu import limit".into())
    }
    fn library(&mut self, vm: &mut Vm, file: &str) -> Result<usize, String> {
        if let Some(&index) = self.libraries.get(file) {
            return Ok(index);
        }
        let path = self.dir.join(format!("{}.json", file.trim_start_matches('/')));
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut movie = Movie::load(&json)?;
        movie.clip_prototype = Some(self.prototypes["MovieClip"]);
        let init: Vec<u32> = movie.characters.values().flat_map(|c| &c.frames).flat_map(|f| &f.controls)
            .filter(|c| c.type_name == "do_init_action" && c.actions_offset != 0).map(|c| c.actions_offset).collect();
        let index = self.movies.len();
        self.movies.push(movie);
        self.libraries.insert(file.to_string(), index);
        let scope = vm.object(ObjectKind::Plain);
        for offset in init {
            let code = self.movies[index].actions.get(&offset.to_string()).cloned().unwrap_or_default();
            if let Err(error) = vm.run_on(scope, &code, self) {
                self.log.push(format!("init action error in {file} at {offset:x}: {error}"));
            }
        }
        self.log.push(format!("library {file}"));
        Ok(index)
    }
    /// AS2 linkage: a symbol registered to a class gets its prototype and constructor.
    fn link(&mut self, vm: &mut Vm, clip: usize, symbol: &str) -> Result<(), String> {
        let class = self.linkage.get(symbol).copied()
            .or_else(|| match vm.get(vm.global, symbol) { Value::Object(c) => Some(c), _ => None });
        if let Some(class) = class {
            if let Value::Object(proto) = vm.get(class, "prototype") {
                vm.objects[clip].prototype = Some(proto);
            }
            if let ObjectKind::Function(_) = vm.objects[class].kind {
                vm.call_function(class, clip, vec![], self)?;
            }
        }
        Ok(())
    }
    fn builtins(&mut self, vm: &mut Vm) -> Result<(), String> {
        for name in ["Object", "Array", "MovieClip", "MovieClipLoader", "TextField", "String", "Number", "Boolean", "Function", "Sound", "Color", "Key", "Stage", "Date"] {
            let class = vm.object(ObjectKind::Native(format!("native:{name}")));
            let proto = vm.object(ObjectKind::Native(format!("proto:{name}")));
            vm.set(class, "prototype", Value::Object(proto))?;
            vm.set(proto, "constructor", Value::Object(class))?;
            vm.set(vm.global, name, Value::Object(class))?;
            self.prototypes.insert(name, proto);
        }
        // Everything inherits Object.prototype, as in AS2.
        let object_proto = self.prototypes["Object"];
        for (name, proto) in &self.prototypes {
            if *name != "Object" {
                vm.objects[*proto].prototype = Some(object_proto);
            }
        }
        let math = vm.object(ObjectKind::Native("native:Math".into()));
        vm.set(vm.global, "Math", Value::Object(math))?;
        vm.set(math, "PI", Value::Number(std::f64::consts::PI))?;
        vm.set(vm.global, "_global", Value::Object(vm.global))?;
        for name in NATIVES {
            let object = vm.object(ObjectKind::Native(format!("native:{name}")));
            vm.set(vm.global, name, Value::Object(object))?;
        }
        Ok(())
    }

    fn perform(&mut self, vm: &mut Vm, load: Load) -> Result<(), String> {
        match load.target {
            Target::Level(level) => {
                let root = vm.object(ObjectKind::Plain);
                match self.load_into(vm, &load.url, root, None) {
                    Ok(_) => {
                        self.levels.insert(level, root);
                        vm.set(vm.global, format!("_level{level}"), Value::Object(root))?;
                    }
                    Err(error) => self.log.push(format!("load {} -> _level{level}: {error}", load.url)),
                }
            }
            Target::Clip(target) => {
                let parent = match vm.get(target, "_parent") { Value::Object(p) => Some(p), _ => None };
                self.notify(vm, &load.listeners, "onLoadStart", target)?;
                match self.load_into(vm, &load.url, target, parent) {
                    Ok(_) => {
                        for event in ["onLoadProgress", "onLoadComplete", "onLoadInit"] {
                            self.notify(vm, &load.listeners, event, target)?;
                        }
                    }
                    Err(error) => {
                        self.log.push(format!("load {} into clip: {error}", load.url));
                        self.notify(vm, &load.listeners, "onLoadError", target)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn movie_of(&self, clip: usize) -> Option<usize> {
        self.movies.iter().rposition(|m| m.instances.contains_key(&clip))
    }

    /// Load a converted movie ("source/x/y.swf") into an existing clip.
    fn load_into(&mut self, vm: &mut Vm, url: &str, target: usize, parent: Option<usize>) -> Result<usize, String> {
        let relative = url.trim_start_matches('/').trim_end_matches(".swf");
        let path = self.dir.join(format!("{relative}.json"));
        let json: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut movie = Movie::load(&json).map_err(|e| format!("{url}: {e}"))?;
        movie.clip_prototype = Some(self.prototypes["MovieClip"]);
        // Init actions (AS2 class definitions) run once, before frame 1.
        let init: Vec<u32> = movie
            .characters
            .values()
            .flat_map(|c| &c.frames)
            .flat_map(|f| &f.controls)
            .filter(|c| c.type_name == "do_init_action" && c.actions_offset != 0)
            .map(|c| c.actions_offset)
            .collect();
        if let Some(old) = self.movie_of(target) {
            self.movies[old].instances.remove(&target);
        }
        let index = self.movies.len();
        self.movies.push(movie);
        for offset in init {
            let code = self.movies[index].actions.get(&offset.to_string()).cloned().unwrap_or_default();
            if let Err(error) = vm.run_on(target, &code, self) {
                self.log.push(format!("init action error in {url} at {offset:x}: {error}"));
            }
        }
        let mut movie = std::mem::replace(&mut self.movies[index], Movie::placeholder());
        let result = movie.adopt(vm, target, 0, parent);
        self.movies[index] = movie;
        result.map_err(|e| format!("{url}: {e}"))?;
        self.log.push(format!("loaded {url}"));
        Ok(index)
    }

    fn clip_method(&mut self, vm: &mut Vm, clip: usize, method: &str, args: &[Value]) -> Result<Option<Value>, String> {
        let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
        let Some(index) = self.movie_of(clip) else {
            return Ok(None);
        };
        Ok(Some(match method {
            "createEmptyMovieClip" => {
                let name = arg(0).text();
                let depth = arg(1).number() as i32;
                let child = self.movies[index].create_child(vm, EMPTY_CLIP, clip)?;
                vm.set(child, "_name", Value::Text(name.clone()))?;
                vm.set(clip, &name, Value::Object(child))?;
                if let Some(instance) = self.movies[index].instances.get_mut(&clip) {
                    instance.dynamic.insert(depth, child);
                }
                Value::Object(child)
            }
            "attachMovie" => {
                let symbol = arg(0).text();
                let name = arg(1).text();
                let depth = arg(2).number() as i32;
                let child = self.movies[index].create_child(vm, EMPTY_CLIP, clip)?;
                vm.set(child, "_name", Value::Text(name.clone()))?;
                if let Value::Object(init) = arg(3) {
                    let fields: Vec<_> = vm.objects[init].fields.clone().into_iter().collect();
                    for (k, v) in fields {
                        vm.set(child, k, v)?;
                    }
                }
                vm.set(clip, &name, Value::Object(child))?;
                if let Some(instance) = self.movies[index].instances.get_mut(&clip) {
                    instance.dynamic.insert(depth, child);
                }
                if let Some(url) = self.classes.get(&symbol).cloned() {
                    self.load_into(vm, &url, child, Some(clip))?;
                } else {
                    self.missing.insert(format!("attachMovie symbol {symbol}"));
                }
                // AS2 linkage: the class of the same name owns the clip.
                if let Value::Object(class) = vm.get(vm.global, &symbol) {
                    if let Value::Object(proto) = vm.get(class, "prototype") {
                        vm.objects[child].prototype = Some(proto);
                    }
                    if let ObjectKind::Function(_) = vm.objects[class].kind {
                        vm.call_function(class, child, vec![], self)?;
                    }
                }
                Value::Object(child)
            }
            "removeMovieClip" | "unloadMovie" => {
                self.movies[index].remove(vm, clip);
                Value::Undefined
            }
            "getNextHighestDepth" => {
                let instance = &self.movies[index].instances[&clip];
                let top = instance.children.keys().chain(instance.dynamic.keys()).max().copied().unwrap_or(-1);
                Value::Number((top + 1).max(0) as f64)
            }
            "loadMovie" => {
                self.loads.push_back(Load { url: arg(0).text(), target: Target::Clip(clip), listeners: vec![] });
                Value::Undefined
            }
            "getDepth" => Value::Number(0.0),
            "swapDepths" | "setMask" | "startDrag" | "stopDrag" => Value::Undefined,
            "hitTest" => Value::Bool(false),
            "getBytesLoaded" | "getBytesTotal" => Value::Number(1.0),
            _ => {
                if self.movies[index].method(vm, clip, method, args)? {
                    Value::Undefined
                } else {
                    return Ok(None);
                }
            }
        }))
    }

    fn native(&mut self, vm: &mut Vm, class: &str, object: usize, method: &str, args: Vec<Value>) -> Result<Option<Value>, String> {
        let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
        let num = |i: usize| args.get(i).map(Value::number).unwrap_or(f64::NAN);
        Ok(Some(match (class, method) {
            ("Object", "registerClass") => {
                match arg(1) {
                    Value::Object(class) => { self.linkage.insert(arg(0).text(), class); }
                    other => { self.classes.insert(arg(0).text(), other.text()); }
                }
                Value::Bool(true)
            }
            (_, "ASSetPropFlags") => Value::Undefined,
            ("Game", "ScreenManagerReady") => {
                self.screen_manager_ready = true;
                Value::Undefined
            }
            // Timeline calls on the plain _level0 root (it has no timeline).
            ("Object", "stop" | "play") => Value::Undefined,
            (_, "hasOwnProperty") => {
                Value::Bool(vm.objects[object].fields.contains_key(&arg(0).text()))
            }
            ("Math", "floor") => Value::Number(num(0).floor()),
            ("Math", "ceil") => Value::Number(num(0).ceil()),
            ("Math", "round") => Value::Number((num(0) + 0.5).floor()),
            ("Math", "abs") => Value::Number(num(0).abs()),
            ("Math", "sqrt") => Value::Number(num(0).sqrt()),
            ("Math", "sin") => Value::Number(num(0).sin()),
            ("Math", "cos") => Value::Number(num(0).cos()),
            ("Math", "atan2") => Value::Number(num(0).atan2(num(1))),
            ("Math", "pow") => Value::Number(num(0).powf(num(1))),
            ("Math", "min") => Value::Number(args.iter().map(Value::number).fold(f64::INFINITY, f64::min)),
            ("Math", "max") => Value::Number(args.iter().map(Value::number).fold(f64::NEG_INFINITY, f64::max)),
            ("Math", "random") => Value::Number(self.random(1 << 24) as f64 / (1u32 << 24) as f64),
            ("Array", _) => return self.array_method(vm, object, method, args),
            ("MovieClipLoader", "addListener") => {
                let listeners = match vm.get(object, "__listeners") {
                    Value::Object(id) => id,
                    _ => {
                        let id = vm.array(vec![])?;
                        vm.set(object, "__listeners", Value::Object(id))?;
                        id
                    }
                };
                let mut values = array_values(vm, listeners);
                values.push(arg(0));
                set_array(vm, listeners, values)?;
                Value::Bool(true)
            }
            ("MovieClipLoader", "loadClip") => {
                let url = arg(0).text();
                let Value::Object(target) = arg(1) else {
                    return Err(format!("loadClip {url} without a target clip"));
                };
                let listeners = match vm.get(object, "__listeners") {
                    Value::Object(id) => array_values(vm, id),
                    _ => vec![],
                };
                self.loads.push_back(Load { url, target: Target::Clip(target), listeners });
                Value::Bool(true)
            }
            ("MovieClipLoader", "unloadClip") => {
                if let Value::Object(target) = arg(0) {
                    if let Some(index) = self.movie_of(target) {
                        self.movies[index].remove(vm, target);
                    }
                }
                Value::Bool(true)
            }
            _ => return Ok(None),
        }))
    }

    fn notify(&mut self, vm: &mut Vm, listeners: &[Value], event: &str, target: usize) -> Result<(), String> {
        for listener in listeners {
            if let Value::Object(id) = listener {
                if let Value::Object(_) = vm.get(*id, event) {
                    vm.call_method(*id, event, vec![Value::Object(target)], self)?;
                }
            }
        }
        Ok(())
    }

    fn array_method(&mut self, vm: &mut Vm, id: usize, method: &str, args: Vec<Value>) -> Result<Option<Value>, String> {
        let mut values = array_values(vm, id);
        let index = |v: &Value, len: usize| {
            let n = v.number();
            let n = if n.is_finite() { n as i64 } else { 0 };
            if n < 0 { (len as i64 + n).max(0) as usize } else { (n as usize).min(len) }
        };
        let result = match method {
            "push" => {
                values.extend(args);
                Value::Number(values.len() as f64)
            }
            "pop" => values.pop().unwrap_or_default(),
            "shift" => if values.is_empty() { Value::Undefined } else { values.remove(0) },
            "unshift" => {
                for (i, v) in args.into_iter().enumerate() {
                    values.insert(i, v);
                }
                Value::Number(values.len() as f64)
            }
            "reverse" => {
                values.reverse();
                Value::Object(id)
            }
            "join" => {
                let sep = args.first().map_or(",".into(), Value::text);
                return Ok(Some(Value::Text(values.iter().map(Value::text).collect::<Vec<_>>().join(&sep))));
            }
            "slice" => {
                let len = values.len();
                let start = args.first().map_or(0, |v| index(v, len));
                let end = args.get(1).map_or(len, |v| index(v, len));
                let part = values.get(start..end.max(start)).unwrap_or_default().to_vec();
                return Ok(Some(Value::Object(vm.array(part)?)));
            }
            "concat" => {
                let mut all = values.clone();
                for a in args {
                    match a {
                        Value::Object(o) if class_of(vm, o).as_deref() == Some("Array") => all.extend(array_values(vm, o)),
                        other => all.push(other),
                    }
                }
                return Ok(Some(Value::Object(vm.array(all)?)));
            }
            "splice" => {
                let len = values.len();
                let start = args.first().map_or(0, |v| index(v, len));
                let count = args.get(1).map_or(len - start, |v| (v.number().max(0.0) as usize).min(len - start));
                let removed: Vec<_> = values.splice(start..start + count, args.into_iter().skip(2)).collect();
                set_array(vm, id, values)?;
                return Ok(Some(Value::Object(vm.array(removed)?)));
            }
            "sort" => {
                values.sort_by(|a, b| a.text().cmp(&b.text()));
                Value::Object(id)
            }
            "toString" => return Ok(Some(Value::Text(values.iter().map(Value::text).collect::<Vec<_>>().join(",")))),
            _ => return Ok(None),
        };
        set_array(vm, id, values)?;
        Ok(Some(result))
    }

    fn random(&mut self, limit: u32) -> u32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 17;
        self.random ^= self.random << 5;
        if limit == 0 { 0 } else { self.random % limit }
    }
}

impl Host for Stage {
    fn call(&mut self, vm: &mut Vm, object: usize, method: &str, args: Vec<Value>) -> Result<Value, String> {
        if let Some(value) = self.clip_method(vm, object, method, &args)? {
            return Ok(value);
        }
        let class = class_of(vm, object).unwrap_or_else(|| "Object".into());
        if let Some(value) = self.native(vm, &class, object, method, args.clone())? {
            return Ok(value);
        }
        if NATIVES.contains(&class.as_str()) {
            self.missing.insert(format!("{class}.{method}"));
        } else {
            self.missing.insert(format!("{class}.{method} (builtin)"));
        }
        Ok(Value::Undefined)
    }
    fn trace(&mut self, message: &str) {
        self.log.push(format!("trace: {message}"));
    }
    fn random(&mut self, limit: u32) -> u32 {
        Stage::random(self, limit)
    }
    fn missing_call(&mut self, method: &str) {
        self.missing.insert(format!("undefined.{method}()"));
    }
    fn primitive_call(&mut self, _vm: &mut Vm, value: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
        let Value::Text(text) = value else {
            return Ok(match method {
                "toString" => Value::Text(value.text()),
                _ => Value::Undefined,
            });
        };
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len() as i64;
        let int = |i: usize, default: i64| args.get(i).map(|v| v.number()).filter(|n| n.is_finite()).map_or(default, |n| n as i64);
        let clamp = |n: i64| n.clamp(0, len) as usize;
        let slice = |a: usize, b: usize| chars[a.min(b)..b.max(a)].iter().collect::<String>();
        Ok(match method {
            "substr" => {
                let mut start = int(0, 0);
                if start < 0 { start += len; }
                let start = clamp(start);
                let count = int(1, len).max(0) as usize;
                Value::Text(slice(start, (start + count).min(chars.len())))
            }
            "substring" => Value::Text(slice(clamp(int(0, 0)), clamp(int(1, len)))),
            "slice" => {
                let fix = |n: i64| clamp(if n < 0 { n + len } else { n });
                Value::Text(slice(fix(int(0, 0)), fix(int(1, len)).max(fix(int(0, 0)))))
            }
            "charAt" => Value::Text(chars.get(int(0, 0).max(0) as usize).map(|c| c.to_string()).unwrap_or_default()),
            "charCodeAt" => Value::Number(chars.get(int(0, 0).max(0) as usize).map_or(f64::NAN, |c| *c as u32 as f64)),
            "indexOf" => Value::Number(text.find(&args.first().map(Value::text).unwrap_or_default())
                .map_or(-1.0, |b| text[..b].chars().count() as f64)),
            "lastIndexOf" => Value::Number(text.rfind(&args.first().map(Value::text).unwrap_or_default())
                .map_or(-1.0, |b| text[..b].chars().count() as f64)),
            "toUpperCase" => Value::Text(text.to_uppercase()),
            "toLowerCase" => Value::Text(text.to_lowercase()),
            "toString" | "valueOf" => Value::Text(text.clone()),
            "split" => {
                let sep = args.first().map(Value::text).unwrap_or_default();
                let parts: Vec<Value> = if sep.is_empty() {
                    chars.iter().map(|c| Value::Text(c.to_string())).collect()
                } else {
                    text.split(sep.as_str()).map(|p| Value::Text(p.into())).collect()
                };
                Value::Object(_vm.array(parts)?)
            }
            _ => {
                self.missing.insert(format!("String.{method}"));
                Value::Undefined
            }
        })
    }
    fn get_url(&mut self, _vm: &mut Vm, url: &str, target: &str) -> Result<(), String> {
        let Some(level) = target.strip_prefix("_level").and_then(|n| n.parse::<u32>().ok()) else {
            self.missing.insert(format!("getURL target {target}"));
            return Ok(());
        };
        self.loads.push_back(Load { url: url.into(), target: Target::Level(level), listeners: vec![] });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// Headless boot of the retail front end, listing what's still missing:
    /// SKATE_MENUS=<prepare_menus.py output> cargo test -p skate-game menu_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn menu_probe() {
        let dir = std::env::var("SKATE_MENUS").expect("SKATE_MENUS");
        let mut frontend = super::Frontend::boot(std::path::Path::new(&dir)).unwrap();
        for _ in 0..3 {
            frontend.advance().unwrap();
        }
        println!("ScreenManagerReady: {}", frontend.stage.screen_manager_ready);
        // SKATE_MENU_OPEN=movie[,template]: open a screen like the engine.
        let open = std::env::var("SKATE_MENU_OPEN").unwrap_or("source/screens/main/core_menu.swf,core_menu".into());
        let mut parts = open.split(',');
        let movie = parts.next().unwrap().to_string();
        let template = parts.next().map(str::to_string)
            .unwrap_or_else(|| movie.rsplit('/').next().unwrap().trim_end_matches(".swf").to_string());
        if let Err(error) = frontend.open_screen(&movie, "screen", &template) {
            println!("open {movie}: {error}");
        }
        for _ in 0..30 {
            if let Err(error) = frontend.advance() { println!("advance: {error}"); break; }
        }
        for line in &frontend.stage.log {
            println!("{line}");
        }
        for (i, movie) in frontend.stage.movies.iter().enumerate() {
            if movie.instances.len() > 100 { println!("movie {i}: {} instances", movie.instances.len()); }
        }
        println!("--- missing ({})", frontend.stage.missing.len());
        for line in &frontend.stage.missing {
            println!("{line}");
        }
    }
}
