// Copyright (c) 2026 MCode
//
// Licensed under the MIT License.

import { Position, Range, TextEdit, window } from "vscode";

import { computeLineCommentEdits } from "./line-comment";

/**
 * 在活动编辑器上切换行注释（MCode 的两种形式 `//` 与 `#` 都认）。
 * 逐选区处理：每个选区覆盖的行整体加或整体去。
 */
export function toggleLineComment(): void {
  const editor = window.activeTextEditor;
  if (!editor) return;
  const doc = editor.document;
  const edits: TextEdit[] = [];
  for (const sel of editor.selections) {
    const first = doc.lineAt(sel.start.line).lineNumber;   // 1-based
    const last = doc.lineAt(sel.end.line).lineNumber;
    const texts: string[] = [];
    for (let ln = first; ln <= last; ln++) texts.push(doc.lineAt(ln - 1).text);
    for (const e of computeLineCommentEdits(texts)) {
      const row = first - 1 + e.line;                       // 0-based
      if (e.len > 0) {
        edits.push(TextEdit.delete(new Range(row, e.char, row, e.char + e.len)));
      } else {
        edits.push(TextEdit.insert(new Position(row, e.char), e.insert));
      }
    }
  }
  if (!edits.length) return;
  void editor.edit((b) => {
    for (const e of edits) b.edit(e);
  });
}
