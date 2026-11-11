// Copyright (c) 2026 MCode
//
// Licensed under the MIT License.

// 行注释切换的纯逻辑回归钉（MCode 两种行注释形式 `//` 与 `#`）。
//   node --test --experimental-strip-types client/test/line-comment.test.ts

import test from "node:test";
import assert from "node:assert/strict";

import { computeLineCommentEdits, hasTrailingComment } from "../src/line-comment.ts";

test("computeLineCommentEdits: 两种前缀都能加与去", () => {
  const edits = computeLineCommentEdits([
    "// a",
    "# b",
    "  let x = 1;",
    "let y = 2; // tail",
    "",
  ]);
  assert.deepEqual(edits, [
    { line: 0, char: 0, len: 3, insert: "" },     // "// a" → " a"
    { line: 1, char: 0, len: 2, insert: "" },     // "# b" → " b"
    { line: 2, char: 2, len: 0, insert: "// " },  // 缩进后插入
  ]);
});

test("computeLineCommentEdits: 串内 # 不算已有注释", () => {
  const edits = computeLineCommentEdits(['partno = "LTC#4365";']);
  assert.equal(edits.length, 1);
  assert.equal(edits[0].insert, "// ");
});

test("hasTrailingComment: 串内不算，行尾算", () => {
  assert.equal(hasTrailingComment('partno = "LTC#4365";'), false);
  assert.equal(hasTrailingComment("let x = 1; // tail"), true);
  assert.equal(hasTrailingComment("# head"), true);
});
