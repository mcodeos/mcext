//! The dependency readiness face — the build's precondition.
//!
//! A build loads the libraries the project declares; a library that is not
//! on disk cannot be loaded, and the build then reports a missing library
//! where the real fact is "not installed yet". This face answers that fact
//! first: it asks the daemon's loader whether each declaration is in place,
//! and when one is not, it asks the daemon to solve and install it — the
//! same authority the CLI's `mcc lib resolve` is, so the lock the build
//! obeys and the packs the build loads are the same ones.
//!
//! The probe is the loader's own answer (`lib.load`): the daemon resolves
//! the copy against the declaration's version requirement and says whether
//! it found one. That is the same question the build asks when it loads
//! the library, so the answer cannot disagree with the build.

use crate::mccsrv::MccServer;
use std::path::Path;

/// What the readiness face found.
#[derive(Debug, Clone, Default)]
pub struct DepReadiness {
    /// How many dependencies the project declares.
    pub declared: usize,
    /// The packs this run installed (`name@version`).
    pub installed: Vec<String>,
    /// The declarations that are still unsatisfied.
    pub missing: Vec<String>,
    /// What the reader should know — a failed solve, an unsatisfiable
    /// declaration, a missing registry.
    pub notes: Vec<String>,
}

impl DepReadiness {
    /// Whether every declared dependency is in place.
    pub fn ready(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Check the project's dependencies and install what is missing.
///
/// `declared` are the manifest's `(name, version requirement)` pairs. The
/// daemon owns both answers: its loader face says what is in place, and
/// `lib.resolve` solves and installs what is not.
pub async fn ensure_dependencies(
    server: &MccServer,
    project_root: &Path,
    declared: &[(String, String)],
) -> DepReadiness {
    let mut readiness = DepReadiness {
        declared: declared.len(),
        ..Default::default()
    };
    if declared.is_empty() {
        return readiness;
    }

    let root = project_root.to_string_lossy().to_string();
    let mut missing = unsatisfied(server, declared).await;
    if missing.is_empty() {
        readiness.missing = missing;
        return readiness;
    }

    // Something is not in place: let the daemon solve it against the
    // registry and download the missing packs.
    match server.lib_resolve(&root).await {
        Ok(resp) => {
            readiness.installed = resp.installed.clone();
            for d in &resp.diagnostics {
                readiness
                    .notes
                    .push(format!("dependency `{}`: {}", d.key, d.message));
            }
            if resp.lock_written {
                readiness
                    .notes
                    .push("wrote the dependency lock (mcode.lock)".to_string());
            }
            missing = unsatisfied(server, declared).await;
        }
        Err(e) => {
            readiness.notes.push(format!(
                "cannot install the missing dependencies ({}): {e}",
                missing.join(", ")
            ));
        }
    }
    readiness.missing = missing;
    readiness
}

/// The declarations the daemon cannot load.
async fn unsatisfied(server: &MccServer, declared: &[(String, String)]) -> Vec<String> {
    let mut missing = Vec::new();
    for (name, req) in declared {
        if !in_place(server, name, req).await {
            missing.push(if req.trim().is_empty() || req.trim() == "*" {
                name.clone()
            } else {
                format!("{name} ({req})")
            });
        }
    }
    missing
}

/// Whether the daemon can load the declared library.
///
/// The requirement decides how the library is named: an exact pin is
/// asked for by name and version, so a copy of another version cannot
/// answer for it; a requirement without a version asks for the library
/// itself, and the daemon's own multi-version refusal still counts as
/// "this library is installed" — the copy is there, only the naming is
/// ambiguous.
async fn in_place(server: &MccServer, name: &str, req: &str) -> bool {
    let req = req.trim();
    if exact_pin(req).is_some() {
        // The exact copy the declaration names: `name@version`.
        let spec = format!("{name}@{req}");
        return server.lib_load(&spec).await.is_ok();
    }
    // A wildcard (or an open range) asks for the library itself. The
    // daemon refuses a multi-version install loudly — that refusal names
    // an installed library, so it is not a missing one.
    match server.lib_load(name).await {
        Ok(()) => true,
        Err(e) => !e.to_string().contains("not found"),
    }
}

/// The version a requirement pins exactly, or `None` when it is a range.
///
/// A bare version string is an exact pin (the mcc version law: `0.141`
/// names `0.141`, not "any 0.141.x"); every operator marks a range.
fn exact_pin(req: &str) -> Option<(u32, u32, u32)> {
    if req.contains(['^', '~', '<', '>', '=', '*', ',']) {
        return None;
    }
    if req.is_empty() {
        return None;
    }
    Some(version_key(req))
}

/// A version as a comparable triple — the missing segments count as zero
/// (`0.141` and `0.141.0` are the same version).
fn version_key(v: &str) -> (u32, u32, u32) {
    let mut parts = [0u32; 3];
    for (i, seg) in v.trim().split('.').take(3).enumerate() {
        parts[i] = seg.parse::<u32>().unwrap_or(0);
    }
    (parts[0], parts[1], parts[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_version_is_an_exact_pin() {
        assert_eq!(exact_pin("0.141"), Some((0, 141, 0)));
        assert_eq!(exact_pin("0.141.2"), Some((0, 141, 2)));
        // The missing segments count as zero.
        assert_eq!(version_key("0.141"), (0, 141, 0));
    }

    #[test]
    fn an_operator_marks_a_range() {
        assert_eq!(exact_pin("^0.141"), None);
        assert_eq!(exact_pin("~0.141"), None);
        assert_eq!(exact_pin(">=0.141"), None);
        assert_eq!(exact_pin("*"), None);
        assert_eq!(exact_pin(""), None);
    }
}
