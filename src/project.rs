//! Project configuration parsing from project.toml
//!
//! Handles:
//! - Detecting project.toml in workspace root
//! - Parsing [project] section (name, version, entry, top_module)
//! - Parsing [dependencies] section
//! - Auto-loading dependencies when opening a workspace folder

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Project configuration loaded from project.toml
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
    /// Try to find system root by looking for mclibs directory
    /// Searches upward from project_root until finding a directory containing mclibs/
    pub fn find_system_root(project_root: &Path) -> Option<PathBuf> {
        let mut current = project_root.to_path_buf();
        loop {
            if current.join("mclibs").exists() {
                return Some(current.clone());
            }
            if !current.pop() {
                break;
            }
        }
        // Fallback: check sibling directory (e.g., project_root/../mclibs)
        if let Some(parent) = project_root.parent() {
            let mut current = parent.to_path_buf();
            loop {
                if current.join("mclibs").exists() {
                    return Some(current.clone());
                }
                if !current.pop() {
                    break;
                }
            }
        }
        None
    }

    /// Load project.toml from a directory
    pub fn load_from(root: &Path) -> Option<Self> {
        let toml_path = root.join("project.toml");
        if !toml_path.exists() {
            debug!("project.toml not found in {}", root.display());
            return None;
        }

        let content = match std::fs::read_to_string(&toml_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to read project.toml: {}", e);
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
                tracing::warn!("Failed to parse project.toml: {}", e);
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
}
