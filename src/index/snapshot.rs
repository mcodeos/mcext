//! Global symbol snapshot
//!
//! `ProjectIndex` is a **read-only snapshot**, rebuilt periodically by worker from mcc global table.
//! LSP handler gets latest version via `watch::Receiver`.

use std::collections::HashMap;
use std::path::PathBuf;
use tower_lsp::lsp_types::Url;

/// Single index entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub uri: Url,
    pub span: (usize, usize), // byte offset range
    pub name: String,
}

/// Symbol kind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndexKind {
    Component,
    Interface,
    Enum,
    Module,
}

/// Global symbol snapshot (read-only)
#[derive(Debug, Clone, Default)]
pub struct ProjectIndex {
    /// Project root directory
    pub project_root: Option<PathBuf>,

    /// Set of registered file URIs
    pub files: Vec<Url>,

    /// Entry list indexed by (kind, name)
    by_name: HashMap<(IndexKind, String), Vec<IndexEntry>>,

    /// Enum value rows indexed by (class_name, value_name). Stored separately
    /// because the key is a tuple, not a single string.
    enum_value_by_name: HashMap<(String, String), IndexEntry>,

    /// Files indexed by URI
    by_uri: HashMap<Url, ()>,
}

impl ProjectIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Add an entry
    pub fn add(&mut self, kind: IndexKind, entry: IndexEntry) {
        self.by_name
            .entry((kind, entry.name.clone()))
            .or_default()
            .push(entry);
    }

    /// Add an enum value row, keyed by (class_name, value_name).
    pub fn add_enum_value(
        &mut self,
        class_name: impl Into<String>,
        value_name: impl Into<String>,
        entry: IndexEntry,
    ) {
        self.enum_value_by_name
            .insert((class_name.into(), value_name.into()), entry);
    }

    /// Mark file as existing
    pub fn add_file(&mut self, uri: Url) {
        if self.by_uri.insert(uri.clone(), ()).is_none() {
            self.files.push(uri);
        }
    }

    /// Remove all entries for a file
    pub fn remove_file(&mut self, uri: &Url) {
        self.by_uri.remove(uri);
        self.files.retain(|u| u != uri);
        for entries in self.by_name.values_mut() {
            entries.retain(|e| &e.uri != uri);
        }
        self.by_name.retain(|_, v| !v.is_empty());
        self.enum_value_by_name.retain(|_, e| &e.uri != uri);
    }

    /// Lookup all entries for (kind, name). Exact case match.
    pub fn lookup(&self, kind: IndexKind, name: &str) -> &[IndexEntry] {
        self.by_name
            .get(&(kind, name.to_string()))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Lookup an enum value row by (class, value). Returns the entry whose
    /// span is the body row of the value inside the class — for F12 on
    /// `PKG.SOP8`, jump to that span.
    pub fn lookup_enum_value(&self, class_name: &str, value_name: &str) -> Option<&IndexEntry> {
        self.enum_value_by_name
            .get(&(class_name.to_string(), value_name.to_string()))
    }

    /// Lookup all entries for a file (across kinds)
    pub fn lookup_file(&self, uri: &Url) -> Vec<(IndexKind, &IndexEntry)> {
        let mut out = Vec::new();
        for ((kind, _name), entries) in &self.by_name {
            for e in entries {
                if &e.uri == uri {
                    out.push((*kind, e));
                }
            }
        }
        out
    }

    /// Count number of entries (for testing)
    pub fn len(&self) -> usize {
        self.by_name.values().map(|v| v.len()).sum::<usize>() + self.enum_value_by_name.len()
    }

    /// Number of enum-value entries in the project index.
    pub fn enum_value_len(&self) -> usize {
        self.enum_value_by_name.len()
    }
}

/// Build ProjectIndex from mcc iterators
pub fn build_from_mcb_iter(
    project_root: Option<PathBuf>,
    components: Vec<(String, String)>,
    interfaces: Vec<(String, String)>,
    enums: Vec<(String, String, [usize; 2])>,
    modules: Vec<(String, String)>,
    enum_values: Vec<crate::rpc::EnumValueEntry>,
) -> ProjectIndex {
    let mut idx = ProjectIndex::new();
    idx.project_root = project_root;

    for (name, uri_str) in components {
        if let Some(uri) = url_from_path(&uri_str) {
            idx.add(
                IndexKind::Component,
                IndexEntry {
                    uri: uri.clone(),
                    span: (0, 0),
                    name,
                },
            );
            idx.add_file(uri);
        }
    }
    for (name, uri_str) in interfaces {
        if let Some(uri) = url_from_path(&uri_str) {
            idx.add(
                IndexKind::Interface,
                IndexEntry {
                    uri: uri.clone(),
                    span: (0, 0),
                    name,
                },
            );
            idx.add_file(uri);
        }
    }
    for (name, uri_str, span) in enums {
        if let Some(uri) = url_from_path(&uri_str) {
            idx.add(
                IndexKind::Enum,
                IndexEntry {
                    uri: uri.clone(),
                    span: (span[0], span[1]),
                    name,
                },
            );
            idx.add_file(uri);
        }
    }
    for (name, uri_str) in modules {
        if let Some(uri) = url_from_path(&uri_str) {
            idx.add(
                IndexKind::Module,
                IndexEntry {
                    uri: uri.clone(),
                    span: (0, 0),
                    name,
                },
            );
            idx.add_file(uri);
        }
    }
    for entry in enum_values {
        if let Some(uri) = url_from_path(&entry.uri) {
            let span = (entry.span[0], entry.span[1]);
            let value_name = entry.name.clone();
            idx.add_enum_value(
                entry.class,
                value_name.clone(),
                IndexEntry {
                    uri: uri.clone(),
                    span,
                    name: value_name,
                },
            );
            idx.add_file(uri);
        }
    }

    idx
}

/// File path string → LSP URL
fn url_from_path(path: &str) -> Option<Url> {
    Url::from_file_path(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn empty_index() {
        let idx = ProjectIndex::new();
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
        assert!(idx.lookup(IndexKind::Component, "X").is_empty());
    }

    #[test]
    fn add_and_lookup() {
        let mut idx = ProjectIndex::new();
        idx.add(
            IndexKind::Component,
            IndexEntry {
                uri: url("file:///a.mc"),
                span: (0, 5),
                name: "X".into(),
            },
        );
        idx.add_file(url("file:///a.mc"));
        assert!(!idx.is_empty());

        let entries = idx.lookup(IndexKind::Component, "X");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "X");
    }

    #[test]
    fn remove_file_clears_entries() {
        let mut idx = ProjectIndex::new();
        idx.add(
            IndexKind::Component,
            IndexEntry {
                uri: url("file:///a.mc"),
                span: (0, 5),
                name: "X".into(),
            },
        );
        idx.add_file(url("file:///a.mc"));
        idx.add(
            IndexKind::Interface,
            IndexEntry {
                uri: url("file:///b.mc"),
                span: (10, 20),
                name: "Y".into(),
            },
        );

        idx.remove_file(&url("file:///a.mc"));

        assert!(idx.lookup(IndexKind::Component, "X").is_empty());
        assert_eq!(idx.lookup(IndexKind::Interface, "Y").len(), 1);
    }

    #[test]
    fn multiple_entries_same_name() {
        let mut idx = ProjectIndex::new();
        for i in 0..3 {
            idx.add(
                IndexKind::Component,
                IndexEntry {
                    uri: url(&format!("file:///f{i}.mc")),
                    span: (0, 5),
                    name: "Shared".into(),
                },
            );
        }
        let entries = idx.lookup(IndexKind::Component, "Shared");
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn build_from_mcb_iter_test() {
        let idx = build_from_mcb_iter(
            Some(PathBuf::from("/proj")),
            vec![("USB".into(), "/proj/usb.mc".into())],
            vec![("Power".into(), "/proj/power.mc".into())],
            vec![],
            vec![],
            vec![],
        );
        assert_eq!(idx.lookup(IndexKind::Component, "USB").len(), 1);
        assert_eq!(idx.lookup(IndexKind::Interface, "Power").len(), 1);
        assert!(idx.lookup(IndexKind::Component, "Nonexistent").is_empty());
    }

    #[test]
    fn enum_value_lookup_by_class_and_value() {
        use crate::rpc::EnumValueEntry;
        let idx = build_from_mcb_iter(
            Some(PathBuf::from("/proj")),
            vec![],
            vec![],
            vec![("PKG".into(), "/proj/pkg.mc".into(), [0, 0])],
            vec![],
            vec![
                EnumValueEntry {
                    class: "PKG".into(),
                    name: "SOP8".into(),
                    uri: "/proj/pkg.mc".into(),
                    span: [10, 14],
                },
                EnumValueEntry {
                    class: "PKG".into(),
                    name: "QFN20".into(),
                    uri: "/proj/pkg.mc".into(),
                    span: [20, 25],
                },
            ],
        );
        let e = idx
            .lookup_enum_value("PKG", "SOP8")
            .expect("SOP8 row registered");
        assert_eq!(e.span, (10, 14));
        assert_eq!(e.name, "SOP8");
        assert!(idx.lookup_enum_value("PKG", "Nonexistent").is_none());
    }
}
