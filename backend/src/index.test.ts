// Run with `npm test` (Node strips the TypeScript types).
import assert from "node:assert/strict";
import { test } from "node:test";
import { toKey } from "./index.ts";

test("toKey keeps the vault hierarchy and rejects unsafe paths", () => {
	assert.deepEqual(toKey("/files/work/projects/My%20Note.md"), { key: "work/projects/My Note.md", list: false });
	assert.deepEqual(toKey("/files/work/projects/"), { key: "work/projects/", list: true });
	assert.deepEqual(toKey("/files/work/"), { key: "work/", list: true });
	for (const bad of ["/files/", "/files/a//b", "/files/a/../b", "/files/a/%2E%2E/b", "/files/a\\b", "/files/%E0%A4%A", "/other/a"])
		assert.equal(toKey(bad), null, bad);
});
