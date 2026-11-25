// Run with `npm test` (Node strips the TypeScript types).
import assert from "node:assert/strict";
import { test } from "node:test";
import { toKey } from "./b2.ts";
import { wikilinks } from "./share.ts";

test("toKey keeps the vault hierarchy and rejects unsafe paths", () => {
	assert.deepEqual(toKey("/files/work/projects/My%20Note.md"), { key: "work/projects/My Note.md", list: false });
	assert.deepEqual(toKey("/files/work/projects/"), { key: "work/projects/", list: true });
	assert.deepEqual(toKey("/files/work/"), { key: "work/", list: true });
	for (const bad of ["/files/", "/files/a//b", "/files/a/../b", "/files/a/%2E%2E/b", "/files/a\\b", "/files/%E0%A4%A", "/other/a"])
		assert.equal(toKey(bad), null, bad);
});

test("wikilinks resolve by vault path, then by file name, like the app", () => {
	const notes = ["Ideas.md", "Projects/Ideas.md", "Projects/Big plan.md"];
	assert.equal(wikilinks("see [[Projects/Ideas]]", notes, "x"), "see [Projects/Ideas](</s/x/Projects/Ideas.md>)");
	assert.equal(wikilinks("[[big plan|the plan]]", notes, "x"), "[the plan](</s/x/Projects/Big%20plan.md>)");
	assert.equal(wikilinks("[[Missing]]", notes, "x"), "Missing");
});
