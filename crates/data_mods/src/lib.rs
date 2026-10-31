//! Data mods: content defined in files rather than code. A mod folder holds
//!
//! - `mod.toml`: the mod's `name` (its namespace), `version` and an optional `description`.
//! - `blocks.toml`: `[[block]]` entries, a `[shapes]` table of named shapes they can share, and
//!   `[looks.<name>]` recipes for how blocks look, generated into solid volumes (see [`looks`]).
//! - `targets.toml`: `[models.<name>]` and `[sounds.<name>]` read from files in the folder, and
//!   `[[target]]` entries drawn and heard through them (see [`targets`]).
//!
//! [`load`] reads a folder into a [`DataMod`], an ordinary [`Mod`] that registers its content
//! through the same API as code. Mistakes are reported with the file and line, and unknown
//! fields are errors, so a typo is never silently ignored.

mod looks;
mod targets;

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use api::block::is_valid_name;
use api::{ApiError, BlockBox, BlockDef, LookDef, Mod, ModelDef, Registrar, SoundDef, TargetDef, Turning};
use serde::Deserialize;
use toml::Spanned;

/// Content read from a mod folder.
#[derive(Clone, Debug)]
pub struct DataMod {
    name: String,
    version: String,
    description: String,
    looks: Vec<LookDef>,
    blocks: Vec<BlockDef>,
    models: Vec<ModelDef>,
    sounds: Vec<SoundDef>,
    targets: Vec<TargetDef>,
}

impl DataMod {
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    /// Looks, in name order.
    pub fn looks(&self) -> &[LookDef] {
        &self.looks
    }

    /// Blocks, in file order.
    pub fn blocks(&self) -> &[BlockDef] {
        &self.blocks
    }

    /// Models, in name order.
    pub fn models(&self) -> &[ModelDef] {
        &self.models
    }

    /// Sounds, in name order.
    pub fn sounds(&self) -> &[SoundDef] {
        &self.sounds
    }

    /// Targets, in file order.
    pub fn targets(&self) -> &[TargetDef] {
        &self.targets
    }
}

impl Mod for DataMod {
    fn name(&self) -> &str {
        &self.name
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        for look in &self.looks {
            r.register_look(look.clone())?;
        }
        for block in &self.blocks {
            r.register_block(block.clone())?;
        }
        for model in &self.models {
            r.register_model(model.clone())?;
        }
        for sound in &self.sounds {
            r.register_sound(sound.clone())?;
        }
        for target in &self.targets {
            r.register_target(target.clone())?;
        }
        Ok(())
    }
}

/// Why a mod folder couldn't be loaded: which file, which line if known, and what's wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadError {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{line}: {}", self.path.display(), self.message),
            None => write!(f, "{}: {}", self.path.display(), self.message),
        }
    }
}

impl std::error::Error for LoadError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    name: Spanned<String>,
    version: String,
    #[serde(default)]
    description: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlocksFile {
    #[serde(default)]
    shapes: HashMap<String, Spanned<Vec<[u8; 6]>>>,
    #[serde(default)]
    looks: BTreeMap<String, Spanned<looks::Recipe>>,
    #[serde(default)]
    block: Vec<BlockEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockEntry {
    name: Spanned<String>,
    #[serde(default = "yes")]
    solid: bool,
    #[serde(default = "one")]
    friction: Spanned<f32>,
    #[serde(default = "yes")]
    placeable: bool,
    color: Option<[u8; 3]>,
    look: Option<Spanned<String>>,
    shape: Option<Spanned<Shape>>,
    #[serde(default)]
    turning: TurningName,
    material: Option<Spanned<String>>,
}

/// A shape: the name of one in `[shapes]`, or boxes given in place.
#[derive(Deserialize)]
#[serde(untagged, expecting = "a shape name, or a list of boxes like [x0, y0, z0, x1, y1, z1]")]
enum Shape {
    Named(String),
    Boxes(Vec<[u8; 6]>),
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum TurningName {
    #[default]
    None,
    Axis,
    Any,
}

impl From<TurningName> for Turning {
    fn from(t: TurningName) -> Self {
        match t {
            TurningName::None => Turning::None,
            TurningName::Axis => Turning::Axis,
            TurningName::Any => Turning::Any,
        }
    }
}

fn yes() -> bool {
    true
}

fn one() -> Spanned<f32> {
    Spanned::new(0..0, 1.0)
}

/// One file being read, for errors that point at a line in it.
struct Source {
    path: PathBuf,
    text: String,
}

impl Source {
    fn read(path: PathBuf) -> Result<Self, LoadError> {
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Source { path, text }),
            Err(e) => Err(LoadError { path, line: None, message: e.to_string() }),
        }
    }

    fn parse<T: for<'de> Deserialize<'de>>(&self) -> Result<T, LoadError> {
        toml::from_str(&self.text).map_err(|e| {
            let at = e.span().map(|s| s.start);
            self.error(at, e.message().to_owned())
        })
    }

    /// An error at byte `at` of the file, if known.
    fn error(&self, at: Option<usize>, message: String) -> LoadError {
        let line = at.map(|at| self.text[..at.min(self.text.len())].matches('\n').count() + 1);
        LoadError { path: self.path.clone(), line, message }
    }

    fn error_at<T>(&self, spanned: &Spanned<T>, message: String) -> LoadError {
        // Defaults have an empty span at 0: no line to point at.
        let at = (spanned.span() != (0..0)).then_some(spanned.span().start);
        self.error(at, message)
    }
}

/// Reads the mod folder at `dir`.
pub fn load(dir: &Path) -> Result<DataMod, LoadError> {
    let manifest_file = Source::read(dir.join("mod.toml"))?;
    let manifest: Manifest = manifest_file.parse()?;
    let namespace = manifest.name.get_ref().clone();
    if !is_valid_name(&format!("{namespace}:x")) {
        let message = format!("invalid mod name {namespace:?}: use lowercase letters, digits and underscores");
        return Err(manifest_file.error_at(&manifest.name, message));
    }

    let blocks_path = dir.join("blocks.toml");
    let (looks, blocks) = if blocks_path.exists() {
        let file = Source::read(blocks_path)?;
        let entries: BlocksFile = file.parse()?;
        let mut shapes = HashMap::new();
        for (name, boxes) in &entries.shapes {
            shapes.insert(name.as_str(), to_boxes(&file, boxes, &format!("shape {name:?}"))?);
        }
        let mut looks = Vec::new();
        for (short, recipe) in &entries.looks {
            let name = format!("{namespace}:{short}");
            if !is_valid_name(&name) {
                let message = format!("invalid look name {short:?}: use lowercase letters, digits and underscores");
                return Err(file.error_at(recipe, message));
            }
            let rgba = recipe.get_ref().generate().map_err(|e| file.error_at(recipe, format!("look {short:?}: {e}")))?;
            looks.push(LookDef { name, turns: recipe.get_ref().turns, rgba });
        }
        let blocks = entries
            .block
            .into_iter()
            .map(|entry| block_def(&file, &namespace, &looks, &shapes, entry))
            .collect::<Result<_, _>>()?;
        (looks, blocks)
    } else {
        (Vec::new(), Vec::new())
    };

    let (models, sounds, targets) = targets::load(dir, &namespace)?;
    Ok(DataMod {
        name: namespace,
        version: manifest.version,
        description: manifest.description,
        looks,
        blocks,
        models,
        sounds,
        targets,
    })
}

/// Turns one `[[block]]` entry into a definition. Names are short in the file and get the
/// mod's namespace; a look may also name another mod's look in full.
fn block_def(
    file: &Source,
    namespace: &str,
    looks: &[LookDef],
    shapes: &HashMap<&str, Vec<BlockBox>>,
    entry: BlockEntry,
) -> Result<BlockDef, LoadError> {
    let short = entry.name.get_ref();
    let name = format!("{namespace}:{short}");
    if short.contains(':') || !is_valid_name(&name) {
        let message = format!(
            "invalid block name {short:?}: use lowercase letters, digits and underscores; the namespace comes from mod.toml"
        );
        return Err(file.error_at(&entry.name, message));
    }
    let friction = *entry.friction.get_ref();
    if !(friction.is_finite() && friction >= 0.0) {
        return Err(file.error_at(&entry.friction, format!("friction must be 0 or more, not {friction}")));
    }

    let look = match &entry.look {
        None => None,
        Some(spanned) => {
            let l = spanned.get_ref();
            let full = if l.contains(':') { l.clone() } else { format!("{namespace}:{l}") };
            // Only this mod's own looks can be checked now; others once every mod has loaded.
            let own = full.strip_prefix(namespace).is_some_and(|rest| rest.starts_with(':'));
            if !is_valid_name(&full) || (own && !looks.iter().any(|d| d.name == full)) {
                let message = format!("block {short:?} names look {l:?}, but there is no [looks.{l}]");
                return Err(file.error_at(spanned, message));
            }
            Some(full)
        }
    };

    let color = match (entry.color, &look) {
        (Some(color), _) => color,
        (None, Some(look)) => looks.iter().find(|d| d.name == *look).map_or([128; 3], LookDef::average),
        (None, None) => {
            return Err(file.error_at(&entry.name, format!("block {short:?} needs a color or a look")));
        }
    };

    let shape = match &entry.shape {
        None => vec![BlockBox::FULL],
        Some(spanned) => match spanned.get_ref() {
            Shape::Named(shape) => shapes.get(shape.as_str()).cloned().ok_or_else(|| {
                file.error_at(spanned, format!("block {short:?} names shape {shape:?}, but [shapes] has no {shape:?}"))
            })?,
            Shape::Boxes(boxes) => {
                to_boxes(file, &Spanned::new(spanned.span(), boxes.clone()), &format!("block {short:?}"))?
            }
        },
    };

    // Every block drops itself unless it names a material: a short name is this mod's own.
    let material = match &entry.material {
        None => name.clone(),
        Some(spanned) => {
            let m = spanned.get_ref();
            let full = if m.contains(':') { m.clone() } else { format!("{namespace}:{m}") };
            if !is_valid_name(&full) {
                let message = format!("block {short:?} has material {m:?}: use lowercase letters, digits and underscores");
                return Err(file.error_at(spanned, message));
            }
            full
        }
    };

    let mut def = BlockDef::new(name, entry.solid, color)
        .with_friction(friction)
        .with_placeable(entry.placeable)
        .with_shape(shape)
        .with_turning(entry.turning.into())
        .with_material(material);
    def.look = look;
    Ok(def)
}

/// Turns `[x0, y0, z0, x1, y1, z1]` lists into boxes, checking each is inside the boxel (0 to
/// 16) with some size. `what` names the shape or block, for errors.
fn to_boxes(file: &Source, boxes: &Spanned<Vec<[u8; 6]>>, what: &str) -> Result<Vec<BlockBox>, LoadError> {
    if boxes.get_ref().is_empty() {
        return Err(file.error_at(boxes, format!("{what} has no boxes")));
    }
    boxes
        .get_ref()
        .iter()
        .map(|&[x0, y0, z0, x1, y1, z1]| {
            let b = BlockBox::new([x0, y0, z0], [x1, y1, z1]);
            if b.is_valid() {
                Ok(b)
            } else {
                let message = format!(
                    "{what} has box {:?}: each box needs 0 <= start < end <= 16 on every axis",
                    [x0, y0, z0, x1, y1, z1]
                );
                Err(file.error_at(boxes, message))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway mod folder in the temp directory, removed when dropped.
    struct Folder(PathBuf);

    impl Folder {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("boxel_data_mods_{}_{test}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Folder(dir)
        }

        fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> &Self {
            fs::write(self.0.join(name), contents).unwrap();
            self
        }

        fn load(&self) -> Result<DataMod, LoadError> {
            load(&self.0)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const MANIFEST: &str = "name = \"test\"\nversion = \"0.1.0\"\n";

    /// A one-color look named `stone`, for blocks to name.
    const STONE_LOOK: &str = "[looks.stone]\nnoise = { colors = [[100, 100, 100]] }\n\n";

    #[test]
    fn reads_blocks_and_looks() {
        let folder = Folder::new("good");
        folder.write("mod.toml", MANIFEST).write(
            "blocks.toml",
            r#"
                [looks.stone]
                noise = { colors = [[100, 100, 100]] }

                [looks.grass]
                noise = { colors = [[120, 80, 50]] }
                top = { colors = [[60, 140, 40]], depth = 2 }

                [[block]]
                name = "stone"
                look = "stone"

                [[block]]
                name = "grass"
                look = "grass"
                color = [0, 200, 0]

                [[block]]
                name = "ice"
                color = [170, 210, 240]
                friction = 0.1
                placeable = false
                solid = false
            "#,
        );
        let m = folder.load().unwrap();
        assert_eq!(m.name(), "test");
        assert_eq!(m.version(), "0.1.0");
        let names: Vec<_> = m.looks().iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["test:grass", "test:stone"]);
        assert_eq!(m.looks()[1].rgba, [100, 100, 100, 255].repeat(LookDef::VOLUME));
        assert_eq!(m.looks()[0].texel(0, 0, LookDef::SIZE - 1), [60, 140, 40, 255], "grass on top");
        assert!(m.looks().iter().all(|l| !l.turns), "looks stay lined up with the world by default");

        let [stone, grass, ice] = m.blocks() else { panic!("three blocks") };
        assert_eq!(stone.name, "test:stone");
        assert_eq!(stone.look.as_deref(), Some("test:stone"));
        assert_eq!(stone.color, [100, 100, 100], "color defaults to the look's average");
        assert_eq!((grass.look.as_deref(), grass.color), (Some("test:grass"), [0, 200, 0]));
        let plain_ice = BlockDef::new("test:ice", false, [170, 210, 240]).with_friction(0.1).with_placeable(false);
        assert_eq!(*ice, plain_ice.with_material("test:ice"), "a block is its own material by default");
    }

    #[test]
    fn reads_shapes_and_turning() {
        let folder = Folder::new("shapes");
        folder.write("mod.toml", MANIFEST).write(
            "blocks.toml",
            r#"
                [shapes]
                slab = [[0, 0, 0, 16, 16, 8]]

                [[block]]
                name = "slab"
                color = [1, 2, 3]
                shape = "slab"
                turning = "any"

                [[block]]
                name = "stairs"
                color = [1, 2, 3]
                shape = [[0, 0, 0, 16, 16, 8], [0, 8, 8, 16, 16, 16]]
                turning = "any"

                [[block]]
                name = "log"
                color = [1, 2, 3]
                turning = "axis"
            "#,
        );
        let m = folder.load().unwrap();
        let [slab, stairs, log] = m.blocks() else { panic!("three blocks") };
        assert_eq!(slab.shape, [BlockBox::new([0, 0, 0], [16, 16, 8])]);
        assert_eq!(slab.turning, Turning::Any);
        assert_eq!(stairs.shape.len(), 2);
        assert_eq!(stairs.turning, Turning::Any);
        assert!(log.is_full());
        assert_eq!(log.turning, Turning::Axis);
    }

    #[test]
    fn reads_materials() {
        let folder = Folder::new("materials");
        folder.write("mod.toml", MANIFEST).write(
            "blocks.toml",
            r#"
                [[block]]
                name = "stone_slab"
                color = [1, 2, 3]
                material = "stone"

                [[block]]
                name = "borrowed"
                color = [1, 2, 3]
                material = "other:clay"
            "#,
        );
        let m = folder.load().unwrap();
        let [slab, borrowed] = m.blocks() else { panic!("two blocks") };
        assert_eq!(slab.material.as_deref(), Some("test:stone"));
        assert_eq!(borrowed.material.as_deref(), Some("other:clay"));

        let err = blocks_error("bad_material", "[[block]]\nname = \"a\"\ncolor = [1, 2, 3]\nmaterial = \"Stone\"\n", 4);
        assert!(err.contains("material"), "{err}");
    }

    #[test]
    fn bad_shapes_are_errors() {
        let err = blocks_error("unknown_shape", "[[block]]\nname = \"a\"\ncolor = [1, 2, 3]\nshape = \"slab\"\n", 4);
        assert!(err.contains("[shapes] has no \"slab\""), "{err}");
        let err = blocks_error("big_box", "[shapes]\nbig = [[0, 0, 0, 16, 16, 17]]\n", 2);
        assert!(err.contains("shape \"big\""), "{err}");
        blocks_error("flat_box", "[[block]]\nname = \"a\"\ncolor = [1, 2, 3]\nshape = [[0, 0, 4, 16, 16, 4]]\n", 4);
        blocks_error("no_boxes", "[[block]]\nname = \"a\"\ncolor = [1, 2, 3]\nshape = []\n", 4);
        let err = blocks_error("bad_turning", "[[block]]\nname = \"a\"\ncolor = [1, 2, 3]\nturning = \"sideways\"\n", 4);
        assert!(err.contains("sideways"), "{err}");
    }

    #[test]
    fn another_mods_look_is_left_for_the_engine_to_check() {
        let folder = Folder::new("other_mod_look");
        folder.write("mod.toml", MANIFEST).write("blocks.toml", "[[block]]\nname = \"a\"\nlook = \"base:stone\"\n");
        let m = folder.load().unwrap();
        assert_eq!(m.blocks()[0].look.as_deref(), Some("base:stone"));
    }

    /// Loads a mod whose `blocks.toml` is `blocks` and returns the error, checking it names
    /// the file and `line`.
    fn blocks_error(test: &str, blocks: &str, line: usize) -> String {
        let folder = Folder::new(test);
        folder.write("mod.toml", MANIFEST).write("blocks.toml", blocks);
        let err = folder.load().unwrap_err();
        assert_eq!(err.path, folder.0.join("blocks.toml"));
        assert_eq!(err.line, Some(line), "{err}");
        err.to_string()
    }

    #[test]
    fn an_unknown_field_is_an_error() {
        let err = blocks_error("unknown_field", "[[block]]\nname = \"stone\"\nfricton = 0.5\ncolor = [1, 2, 3]\n", 3);
        assert!(err.contains("fricton"), "{err}");
    }

    #[test]
    fn a_missing_look_is_an_error() {
        let err = blocks_error("missing_look", &format!("{STONE_LOOK}[[block]]\nname = \"a\"\nlook = \"wood\"\n"), 6);
        assert!(err.contains("no [looks.wood]"), "{err}");
    }

    /// Errors in a recipe point at its `[looks.<name>]` line.
    #[test]
    fn a_broken_recipe_is_an_error() {
        let err = blocks_error("empty_colors", "[looks.stone]\nnoise = { colors = [] }\n", 1);
        assert!(err.contains("look \"stone\"") && err.contains("at least one color"), "{err}");
        let err = blocks_error("two_bases", &format!("{STONE_LOOK}[looks.b]\nnoise = {{ colors = [[1, 1, 1]] }}\nrings = {{ colors = [[1, 1, 1]], spacing = 2, bark = [[0, 0, 0]] }}\n"), 4);
        assert!(err.contains("exactly one"), "{err}");
        let err = blocks_error("unknown_recipe_field", "[looks.stone]\nnoise = { colors = [[1, 1, 1]], scael = 2 }\n", 2);
        assert!(err.contains("scael"), "{err}");
        blocks_error("bad_look_name", "[looks.Stone]\nnoise = { colors = [[1, 1, 1]] }\n", 1);
    }

    #[test]
    fn a_bad_name_is_an_error() {
        let err = blocks_error("bad_name", "[[block]]\nname = \"Stone\"\ncolor = [1, 2, 3]\n", 2);
        assert!(err.contains("\"Stone\""), "{err}");
        let err = blocks_error("namespaced_name", "[[block]]\nname = \"base:stone\"\ncolor = [1, 2, 3]\n", 2);
        assert!(err.contains("namespace comes from mod.toml"), "{err}");
    }

    #[test]
    fn other_mistakes_are_errors() {
        blocks_error("no_look", "[[block]]\nname = \"a\"\n", 2);
        blocks_error("bad_friction", "[[block]]\nname = \"a\"\ncolor = [1, 2, 3]\nfriction = -1.0\n", 4);
        blocks_error("bad_toml", "[[block]\nname = \"a\"\n", 1);
    }

    #[test]
    fn a_bad_manifest_is_an_error() {
        let folder = Folder::new("bad_manifest");
        folder.write("mod.toml", "name = \"Base\"\nversion = \"1\"\n");
        let err = folder.load().unwrap_err();
        assert_eq!((err.path.file_name().unwrap().to_str(), err.line), (Some("mod.toml"), Some(1)));

        let missing = Folder::new("no_manifest");
        fs::remove_file(missing.0.join("mod.toml")).ok();
        assert_eq!(missing.load().unwrap_err().path, missing.0.join("mod.toml"));
    }

    /// A 2×1 PNG: an opaque red texel, then a see-through one.
    fn tiny_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&[255, 0, 0, 255, 0, 0, 0, 0]).unwrap();
        bytes
    }

    const TRIANGLE: &str = "v 0 0 0\nv 1 0 0\nv 0 0 1\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n";

    const TARGETS: &str = r#"
        [models.box]
        mesh = "box.obj"
        texture = "box.png"

        [sounds.thud]
        file = "thud.ogg"

        [[target]]
        name = "dummy"
        model = "box"
        hit_sound = "thud"
        width = 0.8
        height = 1.2
        health = 10
        weight = 2
        gravity = 25
        drag = 5
        air_drag = 2
        bounce = 0.2
        respawn_time = 2
        slam_speed = 4
        slam_damage = 1

        [[target.hit_box]]
        name = "top"
        min = [-0.4, -0.4, 0.8]
        max = [0.4, 0.4, 1.2]
        damage = 2
    "#;

    fn target_folder(test: &str, targets: &str) -> Folder {
        let folder = Folder::new(test);
        folder.write("mod.toml", MANIFEST).write("box.obj", TRIANGLE).write("box.png", tiny_png()).write("thud.ogg", b"OggS");
        folder.write("targets.toml", targets);
        folder
    }

    #[test]
    fn reads_models_sounds_and_targets() {
        let m = target_folder("targets", TARGETS).load().unwrap();
        let [model] = m.models() else { panic!("one model") };
        assert_eq!(model.name, "test:box");
        assert_eq!((model.positions.len(), model.indices.len()), (3, 3));
        assert_eq!((model.texture.width, model.texture.height), (2, 1));
        assert_eq!(model.texture.rgba, [255, 0, 0, 255, 0, 0, 0, 0]);
        assert_eq!(m.sounds()[0].name, "test:thud");
        assert_eq!(m.sounds()[0].ogg, b"OggS");
        let [target] = m.targets() else { panic!("one target") };
        assert_eq!((target.name.as_str(), target.model.as_str()), ("test:dummy", "test:box"));
        assert_eq!(target.hit_sound.as_deref(), Some("test:thud"));
        assert_eq!((target.health, target.weight, target.air_drag, target.slam_speed), (10.0, 2.0, 2.0, 4.0));
        let [top] = &target.hit_boxes[..] else { panic!("one hit box") };
        assert_eq!((top.name.as_str(), top.min, top.max, top.damage), ("top", [-0.4, -0.4, 0.8], [0.4, 0.4, 1.2], 2.0));
    }

    #[test]
    fn target_mistakes_name_the_file_and_line() {
        let error = |test: &str, from: &str, to: &str| {
            let e = target_folder(test, &TARGETS.replacen(from, to, 1)).load().unwrap_err();
            assert!(e.path.ends_with("targets.toml"), "{e}");
            (e.line, e.message)
        };
        let (line, message) = error("t_model", "model = \"box\"", "model = \"crate\"");
        assert_eq!(line, Some(11));
        assert!(message.contains("no model \"crate\""), "{message}");
        let (line, message) = error("t_mesh", "mesh = \"box.obj\"", "mesh = \"gone.obj\"");
        assert_eq!(line, Some(2));
        assert!(message.contains("gone.obj"), "{message}");
        let (_, message) = error("t_health", "health = 10", "health = 0");
        assert!(message.contains("health must be positive"), "{message}");
        let (line, message) = error("t_field", "drag = 5", "drag = 5\n        speed = 3");
        assert_eq!(line, Some(19));
        assert!(message.contains("unknown field"), "{message}");
        let (_, message) = error("t_missing", "        weight = 2\n", "");
        assert!(message.contains("weight"), "{message}");
        let (_, message) = error("t_box", "max = [0.4, 0.4, 1.2]", "max = [0.4, 0.4, 0.5]");
        assert!(message.contains("hit box \"top\""), "{message}");
    }

    #[test]
    fn another_mods_model_is_left_for_the_engine_to_check() {
        let m = target_folder("t_other", &TARGETS.replacen("model = \"box\"", "model = \"other:box\"", 1)).load().unwrap();
        assert_eq!(m.targets()[0].model, "other:box");
    }
}
