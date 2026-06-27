//! Semantic Tokens legend definition
//!
//! This is the contract negotiated between server and client: token_type index and modifier bitmask positions.
//! Once published, must be stable. Modifications require new version.

use tower_lsp::lsp_types::{SemanticTokenModifier, SemanticTokenType};

/// Token types (index is LSP `token_type`)
pub const LEGEND_TYPE: &[SemanticTokenType] = &[
    SemanticTokenType::STRING,         // 0
    SemanticTokenType::NUMBER,         // 1
    SemanticTokenType::TYPE,           // 2
    SemanticTokenType::CLASS,          // 3
    SemanticTokenType::FUNCTION,       // 4
    SemanticTokenType::INTERFACE,      // 5
    SemanticTokenType::ENUM,           // 6
    SemanticTokenType::TYPE_PARAMETER, // 7
    SemanticTokenType::PARAMETER,      // 8
    SemanticTokenType::VARIABLE,       // 9
    SemanticTokenType::PROPERTY,       // 10
    SemanticTokenType::METHOD,         // 11
    SemanticTokenType::MACRO,          // 12
    SemanticTokenType::KEYWORD,        // 13
    SemanticTokenType::OPERATOR,       // 14
    SemanticTokenType::REGEXP,         // 15
    SemanticTokenType::COMMENT,        // 16
];

/// Token 修饰符（位掩码：`1 << index`）
pub const LEGEND_MODIFIER: &[SemanticTokenModifier] = &[
    SemanticTokenModifier::DECLARATION,     // 0 (bit 1<<0 = 1)
    SemanticTokenModifier::DEFINITION,      // 1 (bit 1<<1 = 2)
    SemanticTokenModifier::READONLY,        // 2 (bit 1<<2 = 4)
    SemanticTokenModifier::STATIC,          // 3
    SemanticTokenModifier::DEPRECATED,      // 4
    SemanticTokenModifier::ABSTRACT,        // 5
    SemanticTokenModifier::ASYNC,           // 6
    SemanticTokenModifier::MODIFICATION,    // 7
    SemanticTokenModifier::DOCUMENTATION,   // 8
    SemanticTokenModifier::DEFAULT_LIBRARY, // 9
];

/// mcode element → (legend_type_index, modifier_bitmap)
///
/// These are **conventions**: in `features::semantic_tokens::convert`, map mcc's
/// `McSemToken.type_` field to the indices defined here.
pub mod type_map {
    pub const T_STRING: u32 = 0;
    pub const T_NUMBER: u32 = 1;
    pub const T_TYPE: u32 = 2;
    pub const T_CLASS: u32 = 3;
    pub const T_FUNCTION: u32 = 4;
    pub const T_INTERFACE: u32 = 5;
    pub const T_ENUM: u32 = 6;
    pub const T_PARAMETER: u32 = 8;
    pub const T_VARIABLE: u32 = 9;
    pub const T_PROPERTY: u32 = 10;
    pub const T_KEYWORD: u32 = 13;
    pub const T_OPERATOR: u32 = 14;
    pub const T_COMMENT: u32 = 16;

    pub const M_DECLARATION: u32 = 1 << 0;
    pub const M_DEFINITION: u32 = 1 << 1;
    pub const M_READONLY: u32 = 1 << 2;
    pub const M_STATIC: u32 = 1 << 3;
    pub const M_DEFAULT_LIBRARY: u32 = 1 << 9;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legend_type_count_matches_index_range() {
        // LEGEND_TYPE indices must start from 0 and increase consecutively
        assert_eq!(LEGEND_TYPE.len(), 17);
    }

    #[test]
    fn legend_modifier_bitmap_layout() {
        use type_map::*;
        // Modifier bitmask should match index
        assert_eq!(M_DECLARATION, 1 << 0);
        assert_eq!(M_DEFINITION, 1 << 1);
        assert_eq!(M_READONLY, 1 << 2);
        assert_eq!(M_DEFAULT_LIBRARY, 1 << 9);
    }
}
