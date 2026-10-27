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
fn a_block_naming_a_texture_no_mod_has_is_an_error() {
    let copy = Copy::new("other_mod_texture");
    copy.edit("blocks.toml", "textures = \"brick\"", "textures = \"other:brick\"");
    let err = game::load_from(&copy.0, Movement::Native).err().expect("loading fails").to_string();
    assert_eq!(err, "no texture named \"other:brick\"");
}
