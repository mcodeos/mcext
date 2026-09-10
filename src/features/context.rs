//! Completion context (§4) — what kind of completion position a cursor sits at.
//!
//! Given a cursor byte offset in the document rope plus the semantic lapper
//! entries for that file, [`detect`] computes:
//!
//! - [`ContextKind`]: the completion position kind (§4.1),
//! - `prefix`: the partial token being typed (filter), with lexical boundary
//!   handling for `\`-escaped identifiers (`D\+`), leading `_`, and dotted
//!   member chains,
//! - `member_root`: the expression before the final `.` in member access
//!   (`uC` in `uC.PA`, `this` in `this.VDD`, `PKG` in `PKG.SOP8`),
//! - `container_scope` / `func_scope`: the enclosing container and function
//!   scope strings (P2 / P1 candidate filtering), inferred from the lapper,
//! - `suppressed`: completion must return no symbols (comment / string).

use crate::rpc::LapperEntry;
use ropey::Rope;

// SymbolKind ordinals — must stay in sync with mcc `kind_names` ordering
// (see `features::symbols::kind_rank`).
pub const CLASS_DEF: u8 = 0;
pub const INST_DEF: u8 = 2;
pub const PORT_DEF: u8 = 4;
pub const LABEL_DEF: u8 = 6;
pub const FUNC_DEF: u8 = 8;
pub const PIN_ID_DEF: u8 = 10;
pub const PIN_NAME_DEF: u8 = 12;
pub const PIN_IFACE_DEF: u8 = 14;
pub const ENUM_DEF: u8 = 16;
pub const ENUM_VAL_DEF: u8 = 18;
pub const ROLE_DEF: u8 = 20;
pub const PARAM_DEF: u8 = 21;
pub const DEFINE_DEF: u8 = 22;
pub const ATTR_DEF: u8 = 23;
pub const BUS_DEF: u8 = 25;
pub const UNKNOWN_DEF: u8 = 27;
pub const BUS_MEMBER_DEF: u8 = 28;

/// Completion position kinds (§4.1). Extensions are recognized but only get
/// candidate sources in later stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextKind {
    TopLevel,
    ContainerBody,
    FuncBody,
    MemberAccess,
    UsePath,
    NetExpr,
    InstanceDecl,
    AttrAssign,
    IfExpr,
    // Extensions (recognized, filled in later stages)
    PinsList,
    ParamsList,
    CurlyRef,
    IfaceBind,
    SpecBlock,
    ReturnStmt,
}

impl ContextKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ContextKind::TopLevel => "top_level",
            ContextKind::ContainerBody => "container_body",
            ContextKind::FuncBody => "func_body",
            ContextKind::MemberAccess => "member_access",
            ContextKind::UsePath => "use_path",
            ContextKind::NetExpr => "net_expr",
            ContextKind::InstanceDecl => "instance_decl",
            ContextKind::AttrAssign => "attr_assign",
            ContextKind::IfExpr => "if_expr",
            ContextKind::PinsList => "pins_list",
            ContextKind::ParamsList => "params_list",
            ContextKind::CurlyRef => "curly_ref",
            ContextKind::IfaceBind => "iface_bind",
            ContextKind::SpecBlock => "spec_block",
            ContextKind::ReturnStmt => "return_stmt",
        }
    }
}

/// Suppression reason (§7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressReason {
    Comment,
    StringLit,
}

/// Fully resolved completion context for a cursor position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionContext {
    pub kind: ContextKind,
    /// Partial token being typed (filter prefix); may be empty.
    pub prefix: String,
    /// Member-access root before the final `.`; `None` outside member access.
    pub member_root: Option<String>,
    /// Scope string of the enclosing container (class name, e.g. `main`,
    /// `CAP.CER`), empty at file level.
    pub container_scope: String,
    /// Scope string of the enclosing function (e.g. `main.i2c`), if inside one.
    pub func_scope: Option<String>,
    /// Suppression (comment / string) — completion must return no symbols.
    pub suppressed: Option<SuppressReason>,
}

/// Detect the completion context at `offset` in `rope`.
pub fn detect(rope: &Rope, offset: usize, lapper: &[LapperEntry]) -> CompletionContext {
    let offset = offset.min(rope.len_bytes());
    let line_start = line_start_offset(rope, offset);
    let line_before = rope.byte_slice(line_start..offset).to_string();

    let suppressed = detect_suppression(&line_before);
    let (member_root, prefix) = extract_prefix(rope, offset);
    let (container_scope, func_scope) = infer_scopes(lapper, rope, offset);
    let kind = detect_kind(
        rope,
        offset,
        &line_before,
        member_root.is_some(),
        &container_scope,
        func_scope.as_deref(),
    );

    CompletionContext {
        kind,
        prefix,
        member_root,
        container_scope,
        func_scope,
        suppressed,
    }
}

// ── Lexical helpers ──

/// Byte offset of the start of the line containing `offset`.
fn line_start_offset(rope: &Rope, offset: usize) -> usize {
    let mut i = offset;
    while i > 0 && rope.byte(i - 1) != b'\n' {
        i -= 1;
    }
    i
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'\\' || b >= 0x80
}

/// True when the byte at `idx` is part of a `\X` escape pair (preceded by `\`).
fn is_escaped_byte(rope: &Rope, idx: usize) -> bool {
    idx > 0 && rope.byte(idx - 1) == b'\\'
}

/// Scan back from `from` over identifier bytes (including `\` escapes and
/// non-ASCII), returning the start of the token.
fn scan_ident_back(rope: &Rope, from: usize) -> usize {
    let mut i = from;
    while i > 0 {
        let b = rope.byte(i - 1);
        if is_ident_byte(b) || (is_escaped_byte(rope, i - 1) && !is_ident_byte(b)) {
            i -= 1;
        } else {
            break;
        }
    }
    i
}

/// Extract `(member_root, prefix)` at the cursor.
///
/// `prefix` is the partial token being typed. When the token is preceded by a
/// `.`, the dotted chain before it becomes `member_root` (`uC.PA` →
/// (`uC`, `PA`); `this.VDD` → (`this`, `VDD`); `U_MCU.UART0.TX` →
/// (`U_MCU.UART0`, `TX`)).
fn extract_prefix(rope: &Rope, offset: usize) -> (Option<String>, String) {
    let start = scan_ident_back(rope, offset);
    let prefix = rope.byte_slice(start..offset).to_string();

    if start == 0 || rope.byte(start - 1) != b'.' {
        return (None, prefix);
    }

    // Walk back over the dotted chain before the final '.'.
    let mut p = start - 1;
    let mut chain: Vec<u8> = Vec::new();
    loop {
        let q = scan_ident_back(rope, p);
        let seg = rope.byte_slice(q..p);
        chain.splice(0..0, seg.bytes());
        if q > 0 && rope.byte(q - 1) == b'.' {
            chain.splice(0..0, [b'.']);
            p = q - 1;
        } else {
            break;
        }
    }
    let root = String::from_utf8_lossy(&chain).to_string();
    (Some(root), prefix)
}

// ── Suppression (§7.5) ──

/// Detect comment / string suppression on the current line before the cursor.
/// Block comments spanning multiple lines are not tracked (rare in MCode).
fn detect_suppression(line_before: &str) -> Option<SuppressReason> {
    let bytes = line_before.as_bytes();
    let mut i = 0;
    let mut block_depth = 0usize;
    let mut in_string = false;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                return Some(SuppressReason::Comment)
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                block_depth += 1;
                i += 2;
                continue;
            }
            b'*' if i + 1 < bytes.len() && bytes[i + 1] == b'/' && block_depth > 0 => {
                block_depth -= 1;
                i += 2;
                continue;
            }
            b'"' => in_string = !in_string,
            _ => {}
        }
        i += 1;
    }
    if block_depth > 0 {
        Some(SuppressReason::Comment)
    } else if in_string {
        Some(SuppressReason::StringLit)
    } else {
        None
    }
}

// ── Scope inference ──

/// Parse the declaration header at `pos` (the class name token) into the
/// declaration keyword and the class name. Handles `module main {`,
/// `component CAP.CER {`, `interface UART.TTL {`, `enum PKG {`, `define X`.
/// Shared with the P3 candidate collector in `comp`.
pub(crate) fn header_def(rope: &Rope, pos: usize) -> Option<(String, String)> {
    let pos = pos.min(rope.len_bytes());
    let mut ls = pos;
    while ls > 0 && rope.byte(ls - 1) != b'\n' {
        ls -= 1;
    }
    let mut le = pos;
    while le < rope.len_bytes() && rope.byte(le) != b'\n' {
        le += 1;
    }
    let line = rope.byte_slice(ls..le).to_string();
    let t = line.trim_start();
    for kw in ["component ", "interface ", "enum ", "module ", "define "] {
        if let Some(rest) = t.strip_prefix(kw) {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '_' || *c == '\\')
                .collect();
            if !name.is_empty() {
                return Some((kw.trim().to_string(), name));
            }
        }
    }
    None
}

/// Infer `(container_scope, func_scope)` from the lapper and document text.
///
/// mcc lapper spans cover the **name tokens** only (ClassDef = class name,
/// FuncDef = func name), never whole bodies — so scope inference cannot rely
/// on "an entry covering the cursor". Instead the container/func bodies are
/// located structurally: the nearest preceding ClassDef names the container;
/// the first brace pair after its name that encloses the cursor is its body.
/// The same applies to the nearest preceding FuncDef.
fn infer_scopes(lapper: &[LapperEntry], rope: &Rope, offset: usize) -> (String, Option<String>) {
    // 1. Nearest preceding ClassDef — the enclosing container declaration.
    let class = lapper
        .iter()
        .filter(|e| e.kind == CLASS_DEF && e.start <= offset)
        .max_by_key(|e| e.start);
    let Some(class) = class else {
        return (String::new(), None);
    };

    // 2. The cursor must sit inside the container body.
    if find_enclosing_brace(rope, class.stop, offset).is_none() {
        return (String::new(), None);
    }

    // 3. Container name from the declaration header line.
    let container_scope = header_def(rope, class.start)
        .map(|(_kw, name)| name)
        .unwrap_or_default();
    if container_scope.is_empty() {
        return (String::new(), None);
    }

    // 4. Nearest preceding FuncDef inside this container whose body encloses
    //    the cursor.
    let func = lapper
        .iter()
        .filter(|e| e.kind == FUNC_DEF && e.start >= class.start && e.start <= offset)
        .max_by_key(|e| e.start);
    let func_scope = func
        .filter(|f| find_enclosing_brace(rope, f.stop, offset).is_some())
        .map(|f| f.scope.clone())
        .filter(|s| !s.is_empty());

    (container_scope, func_scope)
}

/// First `{` at/after `from` whose matching `}` encloses `offset` (or that
/// stays unterminated — then treated as enclosing from its position on).
/// Braces that close before `offset` are skipped, so `{}` groups inside
/// declaration params (`dc{VDD,GND}::DC(3.3V)`) do not mask the body brace.
fn find_enclosing_brace(rope: &Rope, from: usize, offset: usize) -> Option<usize> {
    let mut i = from.min(rope.len_bytes());
    while i < rope.len_bytes() {
        if rope.byte(i) == b'{' {
            if let Some(close) = find_matching_brace(rope, i) {
                if i <= offset && offset <= close {
                    return Some(i);
                }
                i = close; // closed before the cursor — skip the pair
            } else {
                return (i <= offset).then_some(i); // unterminated — encloses
            }
        }
        i += 1;
    }
    None
}

/// Match the `{` at `open` to its `}`, counting nesting. `None` if the brace
/// is never closed before the end of the document.
fn find_matching_brace(rope: &Rope, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < rope.len_bytes() {
        match rope.byte(i) {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

// ── Kind detection (§4.3) ──

fn detect_kind(
    rope: &Rope,
    offset: usize,
    line_before: &str,
    is_member: bool,
    container_scope: &str,
    func_scope: Option<&str>,
) -> ContextKind {
    if is_member {
        return ContextKind::MemberAccess;
    }

    let trimmed = line_before.trim_start();

    // `use` / `pub use` path (checked before `::` because `$::lib@ver`).
    if trimmed.starts_with("use") && is_word_boundary(trimmed, 3)
        || trimmed.starts_with("pub use") && is_word_boundary(trimmed, 7)
    {
        return ContextKind::UsePath;
    }

    // `::` contexts (§4.3 step 4): pins rows bind an interface, other lines
    // are inline construction `NAME::TYPE(...)` (or class-static calls). The
    // `::` may sit several tokens before the cursor (`R442::RES(1M`), so scan
    // the whole line. S1 simplification: `=` on the line means a pins-row
    // bind; the exact bracket-level split is refined in a later stage.
    if line_before.contains("::") {
        return if trimmed.contains('=') {
            ContextKind::IfaceBind
        } else {
            ContextKind::InstanceDecl
        };
    }

    // Curly reference `DC2{`, `SPK{`.
    if last_non_ws_before(rope, offset) == Some('{') {
        return ContextKind::CurlyRef;
    }

    // Line-start keywords.
    if (trimmed.starts_with("if ") || trimmed.starts_with("if("))
        || (trimmed.starts_with("else if ") || trimmed.starts_with("else if("))
    {
        return ContextKind::IfExpr;
    }
    if trimmed.starts_with("return") && is_word_boundary(trimmed, 6) {
        return ContextKind::ReturnStmt;
    }
    if trimmed.starts_with("pins") && is_word_boundary(trimmed, 4) {
        return ContextKind::PinsList;
    }
    if trimmed.starts_with("spec") && is_word_boundary(trimmed, 4) {
        return ContextKind::SpecBlock;
    }

    // Attribute assignment `key = value` (except `=>` drive operator).
    if is_attr_assign(rope, offset, trimmed) {
        return ContextKind::AttrAssign;
    }

    if container_scope.is_empty() {
        return ContextKind::TopLevel;
    }
    if func_scope.is_some() {
        return if is_net_expr(trimmed) {
            ContextKind::NetExpr
        } else {
            ContextKind::FuncBody
        };
    }

    // Container body: net line vs instance declaration vs plain member area.
    if is_net_expr(trimmed) {
        return ContextKind::NetExpr;
    }
    if is_instance_decl(trimmed) {
        return ContextKind::InstanceDecl;
    }
    ContextKind::ContainerBody
}

/// True when `s[idx]` is not an identifier char (word boundary).
fn is_word_boundary(s: &str, idx: usize) -> bool {
    s.len() <= idx || !s.as_bytes()[idx].is_ascii_alphanumeric() && s.as_bytes()[idx] != b'_'
}

/// Last non-whitespace char before `offset`.
fn last_non_ws_before(rope: &Rope, offset: usize) -> Option<char> {
    let mut i = offset;
    while i > 0 {
        let b = rope.byte(i - 1);
        if b != b' ' && b != b'\t' {
            return Some(char::from(b));
        }
        i -= 1;
    }
    None
}

/// `key = value` at line start, excluding the `=>` drive operator. The char
/// after `=` may sit beyond the cursor (`V3V3 =|>` with the cursor right
/// after `=`), so it is read from the rope.
fn is_attr_assign(rope: &Rope, offset: usize, trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    if i >= bytes.len() || !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        return false;
    }
    while i < bytes.len() && is_attr_key_byte(bytes[i]) {
        i += 1;
    }
    let mut j = i;
    while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
        j += 1;
    }
    if j >= bytes.len() || bytes[j] != b'=' {
        return false;
    }
    // `=>` is the drive operator, not an attribute assignment.
    let after = if j + 1 < bytes.len() {
        Some(bytes[j + 1])
    } else if offset < rope.len_bytes() {
        Some(rope.byte(offset))
    } else {
        None
    };
    after != Some(b'>')
}

fn is_attr_key_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'\\'
}

/// A net line: starts with a continuation operator or contains a net arrow.
fn is_net_expr(trimmed: &str) -> bool {
    if trimmed.starts_with("->")
        || trimmed.starts_with("- ")
        || trimmed.starts_with('|')
        || trimmed.starts_with('+')
        || trimmed.starts_with('(')
        || trimmed.starts_with(')')
        || trimmed.starts_with(',')
    {
        return true;
    }
    trimmed.contains(" -> ") || trimmed.contains(" <- ") || trimmed.contains("<- ")
}

/// `TYPE(.TYPE)* NAME` declaration, where NAME is followed by `(` / `[` / `,`
/// or end of line (instance declarations like `DC.SRC PWR(5V)`).
fn is_instance_decl(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    // TYPE: ident ( '.' ident )* — up to 3 dotted segments.
    let mut segs = 0;
    loop {
        let start = i;
        while i < bytes.len() && is_attr_key_byte(bytes[i]) {
            i += 1;
        }
        if i == start {
            return false;
        }
        segs += 1;
        if i < bytes.len() && bytes[i] == b'.' && segs < 3 {
            i += 1;
            if i < bytes.len() && bytes[i] == b'.' {
                return false;
            }
            continue;
        }
        break;
    }
    // Whitespace, then the instance name.
    let mut j = i;
    while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
        j += 1;
    }
    if j >= bytes.len() || !(bytes[j].is_ascii_alphabetic() || bytes[j] == b'_') {
        return false;
    }
    let mut k = j;
    while k < bytes.len() && is_attr_key_byte(bytes[k]) {
        k += 1;
    }
    // After the name: `(` / `[` / `,` / EOL (allow spaces before them).
    let mut m = k;
    while m < bytes.len() && (bytes[m] == b' ' || bytes[m] == b'\t') {
        m += 1;
    }
    m >= bytes.len() || bytes[m] == b'(' || bytes[m] == b'[' || bytes[m] == b','
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    /// Build a lapper for `module main { ... }` with realistic spans: as mcc
    /// emits them, ClassDef / FuncDef / port entries cover the **name
    /// tokens**, not whole bodies. `"main"` sits at 7..11 in every module
    /// source below; a synthetic `i2c` func name at 23..26 and its `a` param
    /// label at 27..28 exist only in the func-body source.
    fn lapper_with_scopes() -> Vec<LapperEntry> {
        let e = |kind: u8, start: usize, stop: usize, scope: &str| LapperEntry {
            kind,
            start,
            stop,
            id: 0,
            scope: scope.to_string(),
            file: String::new(),
        };
        vec![
            e(CLASS_DEF, 7, 11, ""),          // name token "main"
            e(PORT_DEF, 18, 31, "main"),      // container-body source port
            e(FUNC_DEF, 23, 26, "main.i2c"),  // name token "i2c"
            e(LABEL_DEF, 27, 28, "main.i2c"), // func param `a`
        ]
    }

    fn ctx(source: &str, byte_offset: usize, lapper: &[LapperEntry]) -> CompletionContext {
        let r = rope(source);
        detect(&r, byte_offset, lapper)
    }

    #[test]
    fn top_level_declaration_position() {
        let src = "module main {\n    pins = []\n}";
        let c = ctx(src, 7, &[]); // after "module "
        assert_eq!(c.kind, ContextKind::TopLevel);
        assert!(c.container_scope.is_empty());
        assert!(c.func_scope.is_none());
    }

    #[test]
    fn container_body_member_area() {
        let src = "module main {\n    GPIO_EXPANDER u_small\n}";
        let lapper = lapper_with_scopes();
        let c = ctx(src, 39, &lapper); // after "u_small"
        assert_eq!(c.kind, ContextKind::InstanceDecl);
        assert_eq!(c.container_scope, "main");
    }

    #[test]
    fn func_body_scope() {
        let src = "module main {\n    func i2c(a) {\n        a -> \n    }\n}";
        let lapper = lapper_with_scopes();
        let c = ctx(src, 45, &lapper); // after "a -> "
        assert_eq!(c.kind, ContextKind::NetExpr);
        assert_eq!(c.func_scope.as_deref(), Some("main.i2c"));
        assert_eq!(c.container_scope, "main");
    }

    #[test]
    fn member_access_root_and_prefix() {
        let src = "uC.PA";
        let c = ctx(src, 5, &[]);
        assert_eq!(c.kind, ContextKind::MemberAccess);
        assert_eq!(c.member_root.as_deref(), Some("uC"));
        assert_eq!(c.prefix, "PA");
    }

    #[test]
    fn this_member_access() {
        let src = "this.VDD";
        let c = ctx(src, 8, &[]);
        assert_eq!(c.kind, ContextKind::MemberAccess);
        assert_eq!(c.member_root.as_deref(), Some("this"));
    }

    #[test]
    fn deep_member_chain() {
        let src = "U_MCU.UART0.TX";
        let c = ctx(src, 15, &[]);
        assert_eq!(c.member_root.as_deref(), Some("U_MCU.UART0"));
        assert_eq!(c.prefix, "TX");
    }

    #[test]
    fn use_path() {
        let src = "use ./po";
        let c = ctx(src, 8, &[]);
        assert_eq!(c.kind, ContextKind::UsePath);
        assert_eq!(c.prefix, "po");
    }

    #[test]
    fn net_expr_arrow() {
        let src = "module main {\n    V3V3 -> R_L\n}";
        let lapper = lapper_with_scopes();
        let c = ctx(src, 29, &lapper); // after "R_L"
        assert_eq!(c.kind, ContextKind::NetExpr);
        assert_eq!(c.prefix, "R_L");
    }

    #[test]
    fn instance_decl_type_slot() {
        let src = "module main {\n    DC.SRC PWR(5\n}";
        let lapper = lapper_with_scopes();
        let c = ctx(src, 30, &lapper); // after "(5"
        assert_eq!(c.kind, ContextKind::InstanceDecl);
        assert_eq!(c.prefix, "5");
    }

    #[test]
    fn attr_assign_key() {
        let src = "partno = \"HUM";
        let c = ctx(src, 14, &[]);
        assert_eq!(c.kind, ContextKind::AttrAssign);
    }

    #[test]
    fn drive_operator_is_not_attr() {
        let src = "V3V3 => CAP(1uF)";
        let c = ctx(src, 6, &[]);
        assert_ne!(c.kind, ContextKind::AttrAssign);
    }

    #[test]
    fn if_expr() {
        let src = "if (partno == \"GPI";
        let c = ctx(src, 18, &[]);
        assert_eq!(c.kind, ContextKind::IfExpr);
    }

    #[test]
    fn comment_suppressed() {
        let src = "// a comment here";
        let c = ctx(src, 15, &[]);
        assert_eq!(c.suppressed, Some(SuppressReason::Comment));
    }

    #[test]
    fn string_suppressed() {
        let src = "name = \"str";
        let c = ctx(src, 10, &[]);
        assert_eq!(c.suppressed, Some(SuppressReason::StringLit));
    }

    #[test]
    fn escaped_identifier_prefix() {
        let src = "bat\\+ -> R";
        let c = ctx(src, 11, &[]);
        assert_eq!(c.prefix, "R");
        let c2 = ctx(src, 5, &[]); // after "bat\+"
        assert_eq!(c2.prefix, "bat\\+");
    }

    #[test]
    fn curly_ref() {
        let src = "DC2{";
        let c = ctx(src, 4, &[]);
        assert_eq!(c.kind, ContextKind::CurlyRef);
    }

    #[test]
    fn double_colon_inline_construct() {
        let src = "R442::RES(1M";
        let c = ctx(src, 11, &[]);
        assert_eq!(c.kind, ContextKind::InstanceDecl);
    }

    #[test]
    fn double_colon_iface_bind() {
        let src = "io 1:2 = UART0::UART.TT";
        // Cursor right after `::` — the interface-name slot of a pins-row
        // bind. (After `UART.` the position is member access of the
        // interface type, which is a separate context.)
        let c = ctx(src, 16, &[]);
        assert_eq!(c.kind, ContextKind::IfaceBind);
    }

    #[test]
    fn dotted_component_container_scope() {
        let src = "component CAP.CER {\n    pins = [1, 2]\n}";
        let lapper = vec![LapperEntry {
            kind: CLASS_DEF,
            start: 10, // name token "CAP.CER"
            stop: 17,
            id: 0,
            scope: String::new(),
            file: String::new(),
        }];
        let c = ctx(src, 30, &lapper); // inside the body
        assert_eq!(c.container_scope, "CAP.CER");
        assert!(c.func_scope.is_none());
    }

    #[test]
    fn curly_param_does_not_mask_body() {
        // module US513(psnk dc{VDD,GND}::DC(3.3V)) { IO1 -> ... } — the `{VDD,GND}`
        // param group must not be mistaken for the container body brace.
        let src = "module US513(psnk dc{VDD,GND}::DC(3.3V)) {\n    IO1 -> \n}";
        let lapper = vec![LapperEntry {
            kind: CLASS_DEF,
            start: 7, // name token "US513"
            stop: 12,
            id: 0,
            scope: String::new(),
            file: String::new(),
        }];
        let c = ctx(src, 54, &lapper); // after "IO1 -> "
        assert_eq!(c.container_scope, "US513");
        assert_eq!(c.kind, ContextKind::NetExpr);
    }

    #[test]
    fn no_member_access_after_space() {
        let src = "module main {\n    uC. PA\n}";
        let lapper = lapper_with_scopes();
        let c = ctx(src, 24, &lapper); // after "PA"
        assert_ne!(c.kind, ContextKind::MemberAccess);
        assert!(c.member_root.is_none());
        assert_eq!(c.container_scope, "main");
    }
}
