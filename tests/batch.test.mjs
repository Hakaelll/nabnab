import test from "node:test";
import assert from "node:assert/strict";
import { batchCandidates, batchVisible, batchCounts } from "../src/batch-types.ts";

test("mixed batch retains invalid rows and exact meaningful query strings", () => {
  assert.deepEqual(batchCandidates("https://www.youtube.com/watch?v=a&list=x invalid https://example.org/a?token=x https://example.org/a?token=y"), [
    { url: "https://www.youtube.com/watch?v=a&list=x", valid: true },
    { url: "invalid", valid: false },
    { url: "https://example.org/a?token=x", valid: true },
    { url: "https://example.org/a?token=y", valid: true },
  ]);
});

test("batch visible selection and queue summary describe distinct scopes", () => {
  const items = [
    { id: "1", title: "Camera", url: "https://example.org/a", provider: "HTTP", state: "ready", picked: true, excluded: false },
    { id: "2", title: "Sound", url: "https://example.org/b", provider: "HTTP", state: "ready", picked: true, excluded: true },
    { id: "3", title: "Camera detail", url: "https://example.org/c", provider: "HTTP", state: "authentication", picked: true, excluded: false },
    { id: "4", title: "Landscape", url: "https://example.org/d", provider: "HTTP", state: "ready", picked: false, excluded: false },
  ];
  assert.deepEqual(batchVisible(items, "camera").map(item => item.id), ["1", "3"]);
  assert.deepEqual(batchCounts(items), { selected: 2, queueable: 1, attention: 1, excluded: 1 });
});
