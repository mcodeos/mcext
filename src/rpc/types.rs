//! RPC wire types for the `sem` method payload.
//!
//! Inlined from the former `mc-rpc-types` crate (same fields, same serde
//! defaults) so mcext has no dependency outside this repository. mcc emits the
//! payload as JSON; field layout here must match what mcc's `sem` handler
//! serializes.

use serde::{Deserialize, Serialize};

// ============================================================================
// LapperEntry — single lapper interval sent over sem RPC
// ============================================================================

/// One entry in the lapper interval tree, sent via `sem` RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LapperEntry {
    /// SymbolKind ordinal (u8). Maps to kind_names[] on RefDefMapData.
    pub kind: u8,
    /// Byte start offset in the source file.
    pub start: usize,
    /// Byte end offset in the source file.
    pub stop: usize,
    /// DeclareId or ReferenceId as raw u32 (sequential allocation).
    pub id: u32,
    /// Scope string for LSP hover/goto-def.
    #[serde(default)]
    pub scope: String,
    /// Source file URI for this entry (fixes cross-file span lookup).
    #[serde(default)]
    pub file: String,
}

// ============================================================================
// RefDefEntryData — single ref→def mapping
// ============================================================================

/// One entry in the unified RefDefMap, sent via `sem` RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefDefEntryData {
    /// SymbolKind ordinal for the reference side.
    pub ref_kind: u8,
    /// Reference ID (raw u32 — sequential DeclareId).
    pub ref_id: u32,
    /// File ID (index into RefDefMapData.files[]).
    pub file_id: u32,
    /// Byte span [start, end) of the definition in the source file.
    pub def_span: [u32; 2],
    /// SymbolKind ordinal for the definition side.
    pub def_kind: u8,
    /// Container ID (index into RefDefMapData.containers[]).
    pub container_id: u32,
    /// CMIE table kind: 0=Component, 1=Module, 2=Interface, 3=Enum, 255=unknown.
    #[serde(default = "default_cmie_kind")]
    pub cmie_kind: u8,
    /// Exact def name captured by mcc at registration from the AST node
    /// (e.g. `RES`, `QFN20`). Lets mcext hover show the def name without
    /// text-slicing the def line.
    #[serde(default)]
    pub def_name: String,
}

fn default_cmie_kind() -> u8 {
    255
}
