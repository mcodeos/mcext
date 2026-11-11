// Copyright (c) 2026 MCode
//
// Licensed under the MIT License.

/**
 * 行注释切换的纯逻辑——MCode 有两种行注释形式（`//` 与 `#`），而 VS Code 的
 * 内置 toggleLineComment 只认 `language-configuration.json` 里那一个
 * `lineComment`（单值约束），认不出 `#`。故扩展自带命令 `mcode.toggleLineComment`。
 *
 * 规则：
 *   - 整行以 `//` 或 `#` 开头 → 去掉该前缀（连同其后一个空格）；
 *   - 否则 → 在首个非空白字符前插入 `// `；
 *   - 行内已有行尾注释则跳过该行（不制造 `// x // tail` 双注释）；
 *   - 空行不包裹。
 *
 * 本文件不含 `vscode` 依赖（纯函数），故可被 node 直接测。
 */

/** 一处行注释改动：`len > 0` 表示删除前缀，否则在 `char` 处插入 `insert`。 */
export interface LineCommentEdit {
  /** 0-based 行号（相对传入的 lines 数组）。 */
  line: number;
  /** 前缀起点（删除时）／插入点（插入时）的列偏移。 */
  char: number;
  /** 要删除的字符数（0 = 插入）。 */
  len: number;
  /** 要插入的文本（"" = 删除）。 */
  insert: string;
}

/** 行内是否已有行尾注释（跳过字符串字面量）。 */
export function hasTrailingComment(text: string): boolean {
  let inStr = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (inStr) {
      if (c === "\\") { i++; continue; }
      if (c === '"') inStr = false;
      continue;
    }
    if (c === '"') { inStr = true; continue; }
    if (c === "/" && text[i + 1] === "/") return true;
    if (c === "#") return true;
  }
  return false;
}

/** 给一批行文本，算出该加/去哪些行注释。 */
export function computeLineCommentEdits(lines: string[]): LineCommentEdit[] {
  const edits: LineCommentEdit[] = [];
  lines.forEach((text, i) => {
    const m = /^(\s*)(\/\/|#)( ?)/.exec(text);
    if (m) {
      edits.push({
        line: i,
        char: m[1].length,
        len: m[2].length + m[3].length,
        insert: "",
      });
      return;
    }
    if (!text.trim()) return;
    if (hasTrailingComment(text)) return;
    const indent = /^\s*/.exec(text)![0].length;
    edits.push({ line: i, char: indent, len: 0, insert: "// " });
  });
  return edits;
}
