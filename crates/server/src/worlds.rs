//! Where saved worlds live: one folder per world, `$XDG_DATA_HOME/boxel/worlds/<name>/`
//! (usually `~/.local/share/boxel/worlds/<name>/`), holding `world.bin` (see [`sim::save`]).
//! Copy a world's folder under another name to keep a copy to come back to.

use std::path::{Path, PathBuf};

use sim::SaveData;

/// The world played when none is named.
pub const DEFAULT_WORLD: &str = "default";

/// One saved world's file, whether or not it's been written yet.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldFile {
    pub name: String,
    folder: PathBuf,
}

impl WorldFile {
    /// The world `name` in the usual place. Names are letters, digits, `-` and `_`.
    pub fn named(name: &str) -> Result<WorldFile, String> {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local").join("share")))
            .ok_or("no XDG_DATA_HOME or HOME to keep worlds in")?;
        WorldFile::in_folder(&data.join("boxel").join("worlds"), name)
    }

    /// The world `name` in `worlds`, a folder of worlds.
    pub fn in_folder(worlds: &Path, name: &str) -> Result<WorldFile, String> {
        let ok = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !ok {
            return Err(format!("\"{name}\" isn't a world name: use letters, digits, - and _"));
        }
        Ok(WorldFile { name: name.to_owned(), folder: worlds.join(name) })
    }

    /// The world `name` in the same folder of worlds as this one.
    pub fn sibling(&self, name: &str) -> Result<WorldFile, String> {
        let worlds = self.folder.parent().ok_or("a world outside any folder")?;
        WorldFile::in_folder(worlds, name)
    }

    /// The names of every saved world in the same folder as this one, last saved first.
    pub fn saved_worlds(&self) -> Vec<String> {
        let Some(Ok(entries)) = self.folder.parent().map(std::fs::read_dir) else { return Vec::new() };
        let mut worlds: Vec<(std::time::SystemTime, String)> = entries
            .filter_map(|e| {
                let e = e.ok()?;
                let name = e.file_name().into_string().ok()?;
                let file = self.sibling(&name).ok()?;
                let modified = std::fs::metadata(file.path()).ok()?.modified().ok()?;
                Some((modified, name))
            })
            .collect();
        worlds.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        worlds.into_iter().map(|(_, name)| name).collect()
    }

    /// The save file itself.
    pub fn path(&self) -> PathBuf {
        self.folder.join("world.bin")
    }

    /// The saved world, or `None` if it hasn't been saved yet.
    pub fn read(&self) -> Result<Option<SaveData>, String> {
        let path = self.path();
        match std::fs::read(&path) {
            Ok(bytes) => SaveData::from_bytes(&bytes).map(Some).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Writes `save`, replacing what was there only once the new file is whole, so a crash
    /// halfway leaves the old save.
    pub fn write(&self, save: &SaveData) -> Result<(), String> {
        let at = |e: std::io::Error| format!("{}: {e}", self.folder.display());
        std::fs::create_dir_all(&self.folder).map_err(at)?;
        let part = self.folder.join("world.bin.part");
        std::fs::write(&part, save.to_bytes()).map_err(at)?;
        std::fs::rename(&part, self.path()).map_err(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("boxel-worlds-test-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_world_is_written_and_read_back() {
        let dir = scratch("roundtrip");
        let file = WorldFile::in_folder(&dir, "door-test_2").unwrap();
        assert_eq!(file.read(), Ok(None), "not saved yet");
        let save = SaveData { tick: 77, inventory: vec![("base:wood".into(), 3)], ..SaveData::default() };
        file.write(&save).unwrap();
        assert_eq!(file.read(), Ok(Some(save.clone())));
        // Writing again replaces it.
        let newer = SaveData { tick: 78, ..save };
        file.write(&newer).unwrap();
        assert_eq!(file.read(), Ok(Some(newer)));
        assert!(!dir.join("door-test_2").join("world.bin.part").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saved_worlds_are_listed_last_saved_first() {
        let dir = scratch("list");
        let a = WorldFile::in_folder(&dir, "a").unwrap();
        assert_eq!(a.saved_worlds(), Vec::<String>::new(), "no folder yet");
        a.write(&SaveData::default()).unwrap();
        let b = a.sibling("b").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        b.write(&SaveData::default()).unwrap();
        // A folder without a save isn't a world.
        std::fs::create_dir_all(dir.join("empty")).unwrap();
        assert_eq!(a.saved_worlds(), ["b", "a"]);
        assert!(a.sibling("../x").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_stay_inside_the_worlds_folder() {
        let dir = scratch("names");
        for bad in ["", "..", "a/b", "../x", "with space", "é"] {
            assert!(WorldFile::in_folder(&dir, bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_damaged_save_is_an_error_not_a_new_world() {
        let dir = scratch("damaged");
        let file = WorldFile::in_folder(&dir, "w").unwrap();
        std::fs::create_dir_all(dir.join("w")).unwrap();
        std::fs::write(file.path(), b"BOXELSAV garbage").unwrap();
        assert!(file.read().is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
