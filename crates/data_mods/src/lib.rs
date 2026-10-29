//! Data mods: content defined in files rather than code. A mod folder holds
//!
//! - `mod.toml`: the mod's `name` (its namespace), `version` and an optional `description`.
//! - `blocks.toml`: `[[block]]` entries.
//! - `textures/*.png`: each registered as `<namespace>:<file stem>`.
//!
//! [`load`] reads a folder into a [`DataMod`], an ordinary [`Mod`] that registers its content
//! through the same API as code. Mistakes are reported with the file and line, and unknown
//! fields are errors, so a typo is never silently ignored.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use api::block::is_valid_name;
use api::{ApiError, BlockDef, FaceTextures, Mod, Registrar, TextureDef};
use serde::Deserialize;
use toml::Spanned;

/// Content read from a mod folder.
#[derive(Clone, Debug)]
pub struct DataMod {
    name: String,
    version: String,
    description: String,
    textures: Vec<TextureDef>,
    blocks: Vec<BlockDef>,
}

impl DataMod {
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    /// Textures, in file name order.
    pub fn textures(&self) -> &[TextureDef] {
        &self.textures
    }

    /// Blocks, in file order.
    pub fn blocks(&self) -> &[BlockDef] {
        &self.blocks
    }
}

impl Mod for DataMod {
    fn name(&self) -> &str {
        &self.name
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        for texture in &self.textures {
            r.register_texture(texture.clone())?;
        }
        for block in &self.blocks {
            r.register_block(block.clone())?;
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
    textures: Option<Spanned<Textures>>,
}

fn yes() -> bool {
    true
}

fn one() -> Spanned<f32> {
    Spanned::new(0..0, 1.0)
}

#[derive(Deserialize)]
#[serde(untagged, expecting = "a texture name, or a table with top, bottom and side")]
enum Textures {
    All(String),
    Faces(Faces),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Faces {
    top: String,
    bottom: String,
    side: String,
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

    let textures = load_textures(&dir.join("textures"), &namespace)?;

    let blocks_path = dir.join("blocks.toml");
    let blocks = if blocks_path.exists() {
        let file = Source::read(blocks_path)?;
        let entries: BlocksFile = file.parse()?;
        entries
            .block
            .into_iter()
            .map(|entry| block_def(&file, &namespace, &textures, entry))
            .collect::<Result<_, _>>()?
    } else {
        Vec::new()
    };

    Ok(DataMod { name: namespace, version: manifest.version, description: manifest.description, textures, blocks })
}

/// Reads every PNG in `dir`, in file name order. A missing folder means no textures.
fn load_textures(dir: &Path, namespace: &str) -> Result<Vec<TextureDef>, LoadError> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let error = |path: &Path, message: String| LoadError { path: path.to_owned(), line: None, message };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| error(dir, e.to_string()))?.path();
        if path.extension().is_some_and(|e| e == "png") {
            paths.push(path);
        }
    }
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
            let name = format!("{namespace}:{stem}");
            if !is_valid_name(&name) {
                let message = format!("invalid texture name {stem:?}: use lowercase letters, digits and underscores");
                return Err(error(&path, message));
            }
            let bytes = fs::read(&path).map_err(|e| error(&path, e.to_string()))?;
            let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
                .map_err(|e| error(&path, format!("not a readable PNG: {e}")))?
                .to_rgba8();
            Ok(TextureDef { name, width: image.width(), height: image.height(), rgba: image.into_raw() })
        })
        .collect()
}

/// Turns one `[[block]]` entry into a definition. Names are short in the file and get the
/// mod's namespace; texture names may also name another mod's texture in full.
fn block_def(
    file: &Source,
    namespace: &str,
    textures: &[TextureDef],
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

    let faces = match &entry.textures {
        None => None,
        Some(spanned) => {
            let qualify = |t: &str| -> Result<String, LoadError> {
                let full = if t.contains(':') { t.to_owned() } else { format!("{namespace}:{t}") };
                // Only this mod's own textures can be checked now; others once every mod has loaded.
                let own = full.strip_prefix(namespace).is_some_and(|rest| rest.starts_with(':'));
                if !is_valid_name(&full) || (own && !textures.iter().any(|d| d.name == full)) {
                    let message = format!("block {short:?} names texture {t:?}, but there is no textures/{t}.png");
                    return Err(file.error_at(spanned, message));
                }
                Ok(full)
            };
            Some(match spanned.get_ref() {
                Textures::All(t) => FaceTextures::all(qualify(t)?),
                Textures::Faces(f) => {
                    FaceTextures { top: qualify(&f.top)?, bottom: qualify(&f.bottom)?, side: qualify(&f.side)? }
                }
            })
        }
    };

    let color = match (entry.color, &faces) {
        (Some(color), _) => color,
        (None, Some(faces)) => textures.iter().find(|t| t.name == faces.side).map_or([128; 3], average_color),
        (None, None) => {
            return Err(file.error_at(&entry.name, format!("block {short:?} needs a color or textures")));
        }
    };

    let mut def = BlockDef::new(name, entry.solid, color).with_friction(friction).with_placeable(entry.placeable);
    def.textures = faces;
    Ok(def)
}

/// The average color of a texture's visible pixels, for wherever a block is drawn as one color.
fn average_color(t: &TextureDef) -> [u8; 3] {
    let (mut sum, mut count) = ([0u64; 3], 0u64);
    for px in t.rgba.as_chunks::<4>().0.iter().filter(|px| px[3] > 0) {
        for (s, &c) in sum.iter_mut().zip(px) {
            *s += c as u64;
        }
        count += 1;
    }
    match count {
        0 => [128; 3],
        n => sum.map(|s| (s / n) as u8),
    }
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
            fs::create_dir_all(dir.join("textures")).unwrap();
            Folder(dir)
        }

        fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> &Self {
            fs::write(self.0.join(name), contents).unwrap();
            self
        }

        /// A 2×2 PNG, all one color.
        fn png(&self, name: &str, rgba: [u8; 4]) -> &Self {
            let image = image::RgbaImage::from_pixel(2, 2, image::Rgba(rgba));
            image.save(self.0.join("textures").join(format!("{name}.png"))).unwrap();
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

    #[test]
    fn reads_blocks_and_textures() {
        let folder = Folder::new("good");
        folder.write("mod.toml", MANIFEST).png("stone", [100, 100, 100, 255]).png("grass", [0, 200, 0, 255]).write(
            "blocks.toml",
            r#"
                [[block]]
                name = "stone"
                textures = "stone"

                [[block]]
                name = "grass"
                textures = { top = "grass", bottom = "stone", side = "grass" }

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
        let names: Vec<_> = m.textures().iter().map(|t| (t.name.as_str(), t.width, t.height)).collect();
        assert_eq!(names, [("test:grass", 2, 2), ("test:stone", 2, 2)]);
        assert_eq!(m.textures()[1].rgba, [100, 100, 100, 255].repeat(4));

        let [stone, grass, ice] = m.blocks() else { panic!("three blocks") };
        assert_eq!(stone.name, "test:stone");
        assert_eq!(stone.textures, Some(FaceTextures::all("test:stone")));
        assert_eq!(stone.color, [100, 100, 100], "color defaults to the side texture's average");
        assert_eq!(
            grass.textures,
            Some(FaceTextures { top: "test:grass".into(), bottom: "test:stone".into(), side: "test:grass".into() })
        );
        assert_eq!(*ice, BlockDef::new("test:ice", false, [170, 210, 240]).with_friction(0.1).with_placeable(false));
    }

    #[test]
    fn another_mods_texture_is_left_for_the_engine_to_check() {
        let folder = Folder::new("other_mod_texture");
        folder.write("mod.toml", MANIFEST).write("blocks.toml", "[[block]]\nname = \"a\"\ntextures = \"base:stone\"\n");
        let m = folder.load().unwrap();
        assert_eq!(m.blocks()[0].textures, Some(FaceTextures::all("base:stone")));
    }

    /// Loads a mod whose `blocks.toml` is `blocks` and returns the error, checking it names
    /// the file and `line`.
    fn blocks_error(test: &str, blocks: &str, line: usize) -> String {
        let folder = Folder::new(test);
        folder.write("mod.toml", MANIFEST).png("stone", [1, 2, 3, 255]).write("blocks.toml", blocks);
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
    fn a_missing_texture_is_an_error() {
        let err = blocks_error(
            "missing_texture",
            "[[block]]\nname = \"a\"\ntextures = \"stone\"\n\n[[block]]\nname = \"b\"\ntextures = { top = \"stone\", bottom = \"stone\", side = \"wood\" }\n",
            7,
        );
        assert!(err.contains("textures/wood.png"), "{err}");
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
        blocks_error("bad_textures", "[[block]]\nname = \"a\"\ntextures = { top = \"stone\" }\n", 3);
        blocks_error("bad_toml", "[[block]\nname = \"a\"\n", 1);
    }

    #[test]
    fn a_broken_png_is_an_error() {
        let folder = Folder::new("broken_png");
        folder.write("mod.toml", MANIFEST).write("textures/stone.png", b"not a png");
        let err = folder.load().unwrap_err();
        assert_eq!(err.path, folder.0.join("textures/stone.png"));
        assert!(err.message.contains("PNG"), "{err}");
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
}
