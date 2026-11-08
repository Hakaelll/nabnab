import test from "node:test";
import assert from "node:assert/strict";
import { fieldMix, fieldScenes, fieldVariation } from "../src/field-motion.ts";

const { current, dunes, orbit, interference, weave, prism, tide } = fieldScenes;
const sequence = [current, dunes, orbit, interference, weave, prism, tide, current];

test("the home field interleaves seven compositions before returning to its opening current", () => {
  assert.deepEqual(sequence.slice(0, -1).map((_, index) => fieldMix(index * 12)),
    sequence.slice(0, -1).map((scene, index) => [scene, sequence[index + 1], 0]));
  assert.deepEqual(fieldMix(84), [current, dunes, 0]);
});

test("each composition has a distinct spatial profile and crossfades continuously into the next", () => {
  const samples = [[-.64, -.32], [.6, -.2], [.48, .34], [-.3, .38]];
  const profile = mix => samples.map(([u, v]) => fieldVariation(u, v, 2, mix));
  const profiles = sequence.map((_, index) => profile(fieldMix(index * 12)));
  assert.equal(new Set(profiles.slice(0, -1).map(values => values.map(value => value.toFixed(3)).join())).size, 7);

  for (let index = 0; index < sequence.length - 1; index++) {
    const start = index * 12;
    assert.deepEqual(fieldMix(start + 9), [sequence[index], sequence[index + 1], .5]);
    const midpoint = profile(fieldMix(start + 9));
    assert.ok(midpoint.every((value, sample) => Math.abs(value - (profiles[index][sample] + profiles[index + 1][sample]) / 2) < 1e-8));
    for (const edge of [start + 6, start + 12]) {
      const before = profile(fieldMix(edge - .01));
      const after = profile(fieldMix(edge + .01));
      assert.ok(before.every((value, sample) => Math.abs(value - after[sample]) < .00001));
    }
  }
});
