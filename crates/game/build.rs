//! Compiles the WebAssembly mods to components in `OUT_DIR`: the base game's movement
//! controller and cow brain, embedded in the game, and test fixtures that misbehave, used only
//! by tests.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use wit_component::ComponentEncoder;

/// Crates compiled to WebAssembly, by package name.
const GUESTS: &[&str] = &["base_movement", "base_cow", "panicky_movement", "hostile_movement", "stuck_movement", "hostile_brain"];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.join("../..").canonicalize().unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    for path in ["crates/api", "crates/base_movement", "crates/base_cow", "crates/test_mods", "Cargo.toml", "Cargo.lock"] {
        println!("cargo::rerun-if-changed={}", root.join(path).display());
    }

    // A separate target directory, so this doesn't wait on the lock the outer build holds.
    let target_dir = root.join("target/wasm");
    let mut cargo = Command::new(env::var("CARGO").unwrap());
    cargo
        .current_dir(&root)
        .args(["build", "--release", "--target", "wasm32-unknown-unknown", "--target-dir"])
        .arg(&target_dir);
    for guest in GUESTS {
        cargo.args(["-p", guest]);
    }
    // Settings meant for the outer build, such as clippy's wrapper, don't apply here.
    for var in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_TARGET_DIR"] {
        cargo.env_remove(var);
    }
    let status = cargo.status().expect("running cargo for the WebAssembly mods");
    assert!(status.success(), "building the WebAssembly mods failed");

    for guest in GUESTS {
        let module_path = target_dir.join(format!("wasm32-unknown-unknown/release/{guest}.wasm"));
        let module = fs::read(&module_path).unwrap_or_else(|e| panic!("reading {}: {e}", module_path.display()));
        let component = ComponentEncoder::default()
            .module(&module)
            .and_then(|encoder| encoder.validate(true).encode())
            .unwrap_or_else(|e| panic!("making a component of {guest}: {e:?}"));
        fs::write(out_dir.join(format!("{guest}.wasm")), component).unwrap();
    }
}
