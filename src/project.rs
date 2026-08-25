//! Project configuration parsing from the project manifest
//!
//! Handles:
//! - Detecting the project manifest in workspace root, trying
//!   project.toml / manifest.toml / mcc.toml in priority order (same as the
//!   mcc CLI)
//! - Parsing [project] section (name, version, entry, top_module)
//! - Parsing [dependencies] section
//! - Auto-loading dependencies when opening a workspace folder

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Project manifest file names, in priority order. Kept in sync with the mcc
/// CLI (`mcc::cli::datadir::PROJECT_MANIFEST_NAMES`); the mcc server parses
/// any of them with the same TOML schema ([project] + [dependencies]).
pub const PROJECT_MANIFEST_NAMES: [&str; 3] = ["project.toml", "manifest.toml", "mcc.toml"];

/// Find the nearest ancestor directory (walking up from `path`'s parent) that
/// contains a project manifest, in [`PROJECT_MANIFEST_NAMES`] priority order.
/// Used by `mcode.viz` to resolve the project from the active *file* rather
/// than a single configured project_root, and mirrored by the TS client
/// (`projectIdFor`) so client and server agree on "same project".
pub fn find_project_root_from_file(path: &Path) -> Option<PathBuf> {
    let mut dir = path.parent()?;
    loop {
        for name in PROJECT_MANIFEST_NAMES {
            if dir.join(name).is_file() {
                return Some(dir.to_path_buf());
            }
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return None, // reached filesystem root
        }
    }
}

/// Project configuration loaded from a project manifest
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectConfig {
    pub project: ProjectSection,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectSection {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    /// Entry .mc file (relative to project root)
    pub entry: String,
    /// Default top-level module name
    #[serde(default)]
    pub top_module: Option<String>,
}

fn default_version() -> String {
    "0.1.0".into()
}

impl ProjectConfig {
    /// Load the project manifest from a directory. Tries the manifest names
    /// in [`PROJECT_MANIFEST_NAMES`] priority order (same as the mcc CLI).
    pub fn load_from(root: &Path) -> Option<Self> {
        let mut toml_path: Option<PathBuf> = None;
        for name in PROJECT_MANIFEST_NAMES {
            let p = root.join(name);
            if p.exists() {
                if name != "project.toml" {
                    tracing::warn!(
                        "deprecated project manifest name '{}'; rename it to project.toml",
                        p.display()
                    );
                }
                toml_path = Some(p);
                break;
            }
        }
        let toml_path = match toml_path {
            Some(p) => p,
            None => {
                debug!("no project manifest found in {}", root.display());
                return None;
            }
        };

        let content = match std::fs::read_to_string(&toml_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Failed to read project manifest {}: {}",
                    toml_path.display(),
                    e
                );
                return None;
            }
        };

        match toml::from_str::<ProjectConfig>(&content) {
            Ok(config) => {
                tracing::info!(
                    "Loaded project config: {} v{} (entry: {})",
                    config.project.name,
                    config.project.version,
                    config.project.entry
                );
                Some(config)
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to parse project manifest {}: {}",
                    toml_path.display(),
                    e
                );
                None
            }
        }
    }

    /// Get absolute path to entry file
    pub fn entry_path(&self, root: &Path) -> String {
        let abs = root.join(&self.project.entry);
        abs.to_string_lossy().to_string()
    }

    /// Get list of dependency names
    pub fn dependency_names(&self) -> Vec<&String> {
        self.dependencies.keys().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hbl_project() {
        let toml = r#"
[project]
name = "hbl"
version = "0.1.0"
entry = "src/hbl.mc"
top_module = "main"

[dependencies]
mcode = "*"
"#;
        let config: ProjectConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.project.name, "hbl");
        assert_eq!(config.project.entry, "src/hbl.mc");
        assert!(config.dependencies.contains_key("mcode"));
    }

    #[test]
    fn test_load_manifest_names_priority() {
        let dir =
            std::env::temp_dir().join(format!("mcext_manifest_priority_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.toml"),
            "[project]\nname = \"mt\"\nversion = \"0.1.0\"\nentry = \"src/main.mc\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("project.toml"),
            "[project]\nname = \"pt\"\nversion = \"0.1.0\"\nentry = \"src/main.mc\"\n",
        )
        .unwrap();
        // project.toml wins over manifest.toml, matching the mcc CLI order.
        let config = ProjectConfig::load_from(&dir).expect("project.toml should be found");
        assert_eq!(config.project.name, "pt");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_find_project_root_from_file_walks_up() {
        let base = std::env::temp_dir().join(format!("mcext_projroot_{}", std::process::id()));
        let inner = base.join("a/b/c");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(
            base.join("project.toml"),
            "[project]\nname = \"p\"\nversion = \"0.1.0\"\nentry = \"src/main.mc\"\n",
        )
        .unwrap();

        // File deep under the project → nearest manifest dir is `base`.
        let f = inner.join("deep.mc");
        std::fs::write(&f, "").unwrap();
        assert_eq!(find_project_root_from_file(&f), Some(base.clone()));

        // Nested manifest wins (nearest-first).
        let sub = base.join("a");
        std::fs::write(
            sub.join("manifest.toml"),
            "[project]\nname = \"sub\"\nentry = \"src/main.mc\"\n",
        )
        .unwrap();
        assert_eq!(find_project_root_from_file(&f), Some(sub));

        // Standalone file (no manifest anywhere above) → None.
        let orphan = std::env::temp_dir().join(format!("mcext_orphan_{}", std::process::id()));
        std::fs::create_dir_all(&orphan).unwrap();
        let of = orphan.join("standalone.mc");
        std::fs::write(&of, "").unwrap();
        assert_eq!(find_project_root_from_file(&of), None);

        std::fs::remove_dir_all(&base).ok();
        std::fs::remove_dir_all(&orphan).ok();
    }

    #[test]
    fn test_load_mcc_toml_fallback() {
        let dir = std::env::temp_dir().join(format!("mcext_manifest_mcc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("mcc.toml"),
            "[project]\nname = \"mcc\"\nversion = \"0.1.0\"\nentry = \"src/main.mc\"\n",
        )
        .unwrap();
        let config = ProjectConfig::load_from(&dir).expect("mcc.toml should be found");
        assert_eq!(config.project.name, "mcc");
        std::fs::remove_dir_all(&dir).ok();
    }
}
