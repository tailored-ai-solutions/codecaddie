import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path) => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");

test("the pinned Native SDK patch keeps large local-core responses bounded per spawn", () => {
  const patch = read("patches/@native-sdk__cli@0.10.1.patch");
  const desktop = read("apps/desktop/src/main.zig");
  const notice = read("THIRD_PARTY_NOTICES.md");

  assert.match(patch, /max_effect_collect_bytes_ceiling/);
  assert.match(patch, /max_collect_bytes: usize = max_effect_collect_bytes/);
  assert.match(patch, /raised per-spawn collect bound preserves a framed local response/);
  assert.match(desktop, /workspace_resume_collect_bytes: usize = core_ipc\.max_core_frame_bytes \+ 4/);
  assert.match(desktop, /\.max_collect_bytes = workspace_resume_collect_bytes/);
  assert.match(notice, /bounded per-spawn collected\s+stdout override/);
});
