use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let target = out_dir.join("auth_ui_component.wasm");
    let source = find_auth_ui_component().expect("find auth_ui_component.wasm");
    fs::copy(&source, &target).expect("copy auth_ui_component.wasm");
    println!("cargo:rerun-if-changed={}", source.display());
}

fn find_auth_ui_component() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(home) = env::var("HOME") {
        roots.push(PathBuf::from(home).join(".cargo/git/checkouts"));
    }
    if let Ok(cargo_home) = env::var("CARGO_HOME") {
        roots.push(PathBuf::from(cargo_home).join("git/checkouts"));
    }
    for root in roots {
        if let Some(found) = walk_for_auth_ui(&root) {
            return Some(found);
        }
    }
    None
}

fn walk_for_auth_ui(root: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("wasm") {
                let candidate = path.join("auth_ui_component.wasm");
                if candidate.exists() {
                    return Some(candidate);
                }
            }
            if let Some(found) = walk_for_auth_ui(&path) {
                return Some(found);
            }
        }
    }
    None
}
