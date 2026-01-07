use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use dwg_core::blueprint::{blueprint_paths, BlueprintConfig, EdgeKind};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let mut dir = std::env::temp_dir();
        let unique = format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        dir.push(unique);
        fs::create_dir_all(&dir).expect("create temp dir");
        Self { path: dir }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, contents).expect("write file");
}

#[test]
fn blueprint_resolves_workspace_crate_imports_and_keeps_paths_consistent() {
    let tmp = TempDir::new("dwg-blueprint");

    write_file(
        &tmp.path.join("Cargo.toml"),
        r#"[workspace]
members = ["core", "cli"]
"#,
    );

    write_file(
        &tmp.path.join("core/Cargo.toml"),
        r#"[package]
name = "dwg-core"
version = "0.0.0"
edition = "2021"

[lib]
name = "dwg_core"
path = "src/lib.rs"
"#,
    );

    write_file(
        &tmp.path.join("cli/Cargo.toml"),
        r#"[package]
name = "dwg-cli"
version = "0.0.0"
edition = "2021"
"#,
    );

    write_file(
        &tmp.path.join("core/src/lib.rs"),
        r#"pub mod cfg;

pub use cfg::Thing;
"#,
    );
    write_file(&tmp.path.join("core/src/cfg.rs"), "pub struct Thing;\n");
    write_file(
        &tmp.path.join("cli/src/main.rs"),
        r#"use dwg_core::cfg::Thing;

fn main() {
    let _ = std::mem::size_of::<Thing>();
}
"#,
    );

    let config = BlueprintConfig {
        ignore_globs: Vec::new(),
        base_dir: Some(tmp.path.clone()),
    };

    let report = blueprint_paths(&[tmp.path.clone()], &config).expect("blueprint succeeds");

    let node_paths: HashSet<String> = report.nodes.iter().map(|n| n.path.clone()).collect();
    assert!(node_paths.contains("core/src/cfg.rs"));
    assert!(node_paths.contains("cli/src/main.rs"));

    for node in &report.nodes {
        assert!(
            !node.path.starts_with('/'),
            "expected display paths to be relative, got: {}",
            node.path
        );
    }

    for edge in &report.edges {
        if edge.resolved {
            let to = edge.to.as_deref().expect("resolved edges must have `to`");
            assert!(
                node_paths.contains(to),
                "resolved edge points to missing node: {} -> {}",
                edge.from,
                to
            );
        }
    }

    assert!(
        report.edges.iter().any(|e| {
            e.kind == EdgeKind::Use
                && e.resolved
                && e.from == "cli/src/main.rs"
                && e.to.as_deref() == Some("core/src/cfg.rs")
        }),
        "expected dwg_core::cfg::* to resolve to core/src/cfg.rs"
    );
}

