//! The base mod's data half, loaded the way the game loads it: edits to its files take effect
//! on the next start, with no rebuild, and mistakes in them come back as readable errors.

use std::fs;
use std::path::{Path, PathBuf};

use game::Movement;

/// A copy of `mods/base` in the temp directory, removed when dropped.
struct Copy(PathBuf);

impl Copy {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("boxel_game_data_{}_{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        copy_dir(Path::new(base::DATA_DIR), &dir);
        Copy(dir)
    }

    fn edit(&self, file: &str, from: &str, to: &str) {
        let path = self.0.join(file);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(from), "{file} should contain {from:?}");
        fs::write(path, text.replacen(from, to, 1)).unwrap();
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() { copy_dir(&path, &target) } else { fs::copy(&path, &target).map(drop).unwrap() }
    }
}

#[test]
fn editing_the_data_files_changes_the_game() {
    let copy = Copy::new("edit");
    copy.edit("blocks.toml", "friction = 0.1", "friction = 0.5");
    let sim = game::load_from(&copy.0, Movement::Native).expect("the edited mod loads");
    let ice = sim.blocks().get(sim.blocks().id(base::ICE).unwrap()).unwrap();
    assert_eq!(ice.friction, 0.5);
}

#[test]
fn a_broken_data_file_is_a_readable_error() {
    let copy = Copy::new("broken");
    copy.edit("blocks.toml", "friction = 0.1", "fricton = 0.1");
    let err = game::load_from(&copy.0, Movement::Native).err().expect("loading fails").to_string();
    let text = fs::read_to_string(copy.0.join("blocks.toml")).unwrap();
    let line = text.lines().position(|l| l.contains("fricton")).unwrap() + 1;
    assert!(err.starts_with(&format!("{}:{line}: ", copy.0.join("blocks.toml").display())), "{err}");
    assert!(err.contains("fricton"), "{err}");
}

#[test]
fn a_block_naming_a_look_no_mod_has_is_an_error() {
    let copy = Copy::new("other_mod_look");
    copy.edit("blocks.toml", "look = \"brick\"", "look = \"other:brick\"");
    let err = game::load_from(&copy.0, Movement::Native).err().expect("loading fails").to_string();
    assert_eq!(err, "no look named \"other:brick\"");
}

#[test]
fn editing_a_recipe_changes_the_look() {
    let before = game::load_from(Path::new(base::DATA_DIR), Movement::Native).expect("the base mod loads");
    let copy = Copy::new("recipe");
    copy.edit("blocks.toml", "scale = 2, seed = 1", "scale = 2, seed = 99");
    let after = game::load_from(&copy.0, Movement::Native).expect("the edited mod loads");
    let stone = |sim: &sim::Simulation| sim.looks().get(base::STONE).unwrap().rgba.clone();
    assert_ne!(stone(&before), stone(&after), "a new seed gives new speckle");
    assert_eq!(before.looks().get("base:brick"), after.looks().get("base:brick"), "other looks stay the same");
}

#[test]
fn a_broken_recipe_names_its_line() {
    let copy = Copy::new("broken_recipe");
    copy.edit("blocks.toml", "length = 8, height = 4, colors = [[150", "length = 5, height = 4, colors = [[150");
    let err = game::load_from(&copy.0, Movement::Native).err().expect("loading fails").to_string();
    let text = fs::read_to_string(copy.0.join("blocks.toml")).unwrap();
    let line = text.lines().position(|l| l == "[looks.brick]").unwrap() + 1;
    assert!(err.starts_with(&format!("{}:{line}: ", copy.0.join("blocks.toml").display())), "{err}");
    assert!(err.contains("look \"brick\"") && err.contains("divide"), "{err}");
}

#[test]
fn the_cow_is_data_and_its_numbers_can_be_edited() {
    let sim = game::load(Movement::Native).unwrap();
    let cow = sim.target_defs().iter().find(|d| d.name == base::COW).expect("base defines the cow");
    let model = sim.models().iter().find(|m| m.name == cow.model).expect("with its model");
    assert_eq!((model.texture.width, model.texture.height), (128, 128));
    assert!(model.indices.len() > 300, "the whole cow, {} indices", model.indices.len());
    let skin = model.skin.as_ref().expect("with its skeleton");
    for clip in ["stand", "eat", "walk", "run", "punch", "die"] {
        assert!(skin.clip(clip).is_some(), "and its {clip} clip");
    }
    assert!(cow.hit_sound.as_ref().is_some_and(|s| sim.sounds().iter().any(|d| &d.name == s)), "and its moo");
    assert_eq!(cow.brain.as_deref(), Some(base::cow::COW_BRAIN), "and the base crate's brain");
    assert_eq!(cow.step_height, 0.6);

    let copy = Copy::new("cow");
    copy.edit("targets.toml", "health = 130", "health = 55");
    let sim = game::load_from(&copy.0, Movement::Native).unwrap();
    assert_eq!(sim.target_defs().iter().find(|d| d.name == base::COW).unwrap().health, 55.0);

    let copy = Copy::new("cow_broken");
    copy.edit("targets.toml", "mesh = \"models/cow.glb\"", "mesh = \"models/bull.glb\"");
    let error = game::load_from(&copy.0, Movement::Native).err().unwrap().to_string();
    assert!(error.contains("targets.toml:") && error.contains("bull.glb"), "{error}");
}

#[test]
fn a_cow_naming_a_brain_no_mod_has_is_an_error() {
    let copy = Copy::new("cow_brainless");
    copy.edit("targets.toml", "brain = \"cow\"", "brain = \"sheep\"");
    let error = game::load_from(&copy.0, Movement::Native).err().unwrap().to_string();
    assert!(error.contains("base:sheep"), "{error}");
}
