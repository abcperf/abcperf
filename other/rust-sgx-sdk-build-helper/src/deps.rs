use std::path::Path;

use cargo_toml::Manifest;

pub(super) fn get(crate_path: &Path) -> Vec<String> {
    let manifest = Manifest::from_path(crate_path.join("Cargo.toml")).unwrap();

    manifest
        .dependencies
        .into_iter()
        .filter_map(|(_, d)| d.detail().and_then(|d| d.path.clone()))
        .collect()
}
