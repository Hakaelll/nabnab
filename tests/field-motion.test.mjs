import test from "node:test";
import assert from "node:assert/strict";
import { fieldMix, fieldVariation } from "../src/field-motion.ts";

test("the home field cycles through four compositions and returns to its opening current", () => {
  assert.deepEqual([0, 14, 28, 42, 56].map(fieldMix), [
    [0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1], [0, 0, 0],
  ]);
});

test("transitions blend masks smoothly at their midpoint and stay continuous at scene boundaries", () => {
  assert.deepEqual([11, 25, 39, 53].map(fieldMix), [[.5, 0, 0], [.5, .5, 0], [0, .5, .5], [0, 0, .5]]);
  for (const edge of [8, 14, 22, 28, 36, 42, 50, 56]) {
    const before = fieldMix(edge - .01);
    const after = fieldMix(edge + .01);
    assert.ok(before.every((value, index) => Math.abs(value - after[index]) < .00001));
  }
  const masks = [14, 28, 42].map(time => fieldVariation(.6, -.2, 2, fieldMix(time)));
  assert.equal(new Set(masks).size, 3);
  assert.equal(fieldVariation(.6, -.2, 2, fieldMix(25)), (masks[0] + masks[1]) / 2);
});
