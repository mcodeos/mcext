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
use std::collections::HashMap;
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

/// The project root an entry belongs to — the directory that holds the
/// manifest. An entry that is a folder is the root itself when it carries a
/// manifest; a file finds its nearest ancestor that has one.
pub fn project_root_for(entry: &str) -> Option<PathBuf> {
    let path = Path::new(entry);
    if path.is_dir() {
        return PROJECT_MANIFEST_NAMES
            .iter()
            .any(|name| path.join(name).is_file())
            .then(|| path.to_path_buf());
    }
    find_project_root_from_file(path)
}

/// Project configuration loaded from a project manifest
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectConfig {
    pub project: ProjectSection,
    #[serde(default)]
    /// Dependency name → version requirement. An order-preserving map on
    /// purpose (see the `preserve_order` note in Cargo.toml): lib.load
    /// resolves `use` targets against the already-loaded set, so the
    /// manifest's authoring order (system libs first) is load order.
    pub dependencies: toml::map::Map<String, toml::Value>,
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

/// The lock face's version per library: `mcode.lock` names one section per
/// library, and a section without a `version` (the `rev` shape) states no
/// version.
fn read_lock_versions(root: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(content) = std::fs::read_to_string(root.join("mcode.lock")) else {
        return out;
    };
    let Ok(parsed) = content.parse::<toml::Value>() else {
        return out;
    };
    let Some(table) = parsed.as_table() else {
        return out;
    };
    for (name, section) in table {
        let Some(version) = section.get("version").and_then(|v| v.as_str()) else {
            continue;
        };
        out.insert(name.clone(), version.to_string());
    }
    out
}

/// The selection face's version per library: `mcode.selection.toml` keeps a
/// `[selection]` table of `name = "version"`.
fn read_selection_versions(root: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(content) = std::fs::read_to_string(root.join("mcode.selection.toml")) else {
        return out;
    };
    let Ok(parsed) = content.parse::<toml::Value>() else {
        return out;
    };
    let Some(table) = parsed.get("selection").and_then(|s| s.as_table()) else {
        return out;
    };
    for (name, version) in table {
        if let Some(version) = version.as_str() {
            out.insert(name.clone(), version.to_string());
        }
    }
    out
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

    /// The dependency names as `lib.load` wants them: the name plus the
    /// version the project stands on.
    ///
    /// `lib.load` refuses a bare name when the store holds several copies of
    /// that library ("installed as multiple versions ... no explicit version
    /// was given"), so a bare name silently loads *nothing* on a machine
    /// with a populated store — and the editor's world then misses every
    /// component class the board names. The project's own lock face
    /// (`mcode.lock`) is the build's only authority; the selection face
    /// (`mcode.selection.toml`) is the world's copy list. Both name one
    /// version per library; the lock wins where the two differ, and a
    /// library neither face pins loads by its bare name (the store holds one
    /// copy, or the load fails loudly, which is the honest outcome).
    pub fn dependency_load_names(&self, root: &Path) -> Vec<String> {
        let locked = read_lock_versions(root);
        let selected = read_selection_versions(root);
        self.dependencies
            .keys()
            .map(|name| {
                let version = locked.get(name).or_else(|| selected.get(name));
                match version {
                    Some(v) => format!("{name}@{v}"),
                    None => name.clone(),
                }
            })
            .collect()
    }

    /// The declared dependencies with their version requirements.
    ///
    /// A requirement is written either as a bare string (`mcfam = "0.141"`)
    /// or as a table (`mcfam = { version = "0.141", local = true }`); both
    /// name the same version requirement, so the readiness check reads one
    /// shape. A declaration without a version states no requirement (`*`).
    pub fn dependency_requirements(&self) -> Vec<(String, String)> {
        self.dependencies
            .iter()
            .map(|(name, value)| {
                let req = match value {
                    toml::Value::String(s) => s.clone(),
                    toml::Value::Table(t) => t
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("*")
                        .to_string(),
                    _ => "*".to_string(),
                };
                (name.clone(), req)
            })
            .collect()
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
    fn test_dependency_load_names_pins_versions() {
        let dir = std::env::temp_dir().join(format!("mcext_loadnames_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("project.toml"),
            "[project]\nname = \"p\"\nversion = \"0.1.0\"\nentry = \"src/main.mc\"\n\n[dependencies]\nmcode = \"1.40\"\nsmbj5 = \">= 0.11\"\nloose = \"*\"\n",
        )
        .unwrap();
        // The lock face names one version per library.
        std::fs::write(
            dir.join("mcode.lock"),
            "[mcode]\nversion = \"1.40\"\nrev = \"0\"\n\n[smbj5]\nversion = \"0.16\"\n",
        )
        .unwrap();
        // The selection face covers what the lock leaves out.
        std::fs::write(
            dir.join("mcode.selection.toml"),
            "format = 1\n\n[selection]\nmcode = \"1.40\"\nloose = \"0.7\"\n",
        )
        .unwrap();

        let config = ProjectConfig::load_from(&dir).expect("manifest should load");
        let names = config.dependency_load_names(&dir);
        assert_eq!(names, vec!["mcode@1.40", "smbj5@0.16", "loose@0.7"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_dependency_load_names_without_faces_stays_bare() {
        let dir = std::env::temp_dir().join(format!("mcext_loadnames_bare_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("project.toml"),
            "[project]\nname = \"p\"\nversion = \"0.1.0\"\nentry = \"src/main.mc\"\n\n[dependencies]\nmcode = \"1.40\"\n",
        )
        .unwrap();
        let config = ProjectConfig::load_from(&dir).expect("manifest should load");
        assert_eq!(config.dependency_load_names(&dir), vec!["mcode"]);
        std::fs::remove_dir_all(&dir).ok();
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
