import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { readFile, readdir } from "node:fs/promises";
import path from "node:path";

export async function engineChecksums(directory, prefix = "") {
  const files = {};
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const name = prefix + entry.name;
    if (name === ".gitkeep" || name === "checksums.json") continue;
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      assert.equal(name, "licenses", "Unexpected engine resource directory: " + name);
      Object.assign(files, await engineChecksums(file, name + "/"));
    } else {
      assert(entry.isFile() && /^(?:(?:yt-dlp|deno|ffmpeg|ffprobe)\.exe|[\w-]+\.dll|(?:versions|ffmpeg-build)\.json|licenses\/[\w-]+\.txt)$/i.test(name), "Unexpected engine resource: " + name);
      const hash = createHash("sha256");
      for await (const chunk of createReadStream(file)) hash.update(chunk);
      files[name] = hash.digest("hex");
    }
  }
  return Object.fromEntries(Object.entries(files).sort(([a], [b]) => a.localeCompare(b)));
}

export async function ffmpegBuildInputs(directory) {
  const recipe = JSON.parse(await readFile(path.join(directory, "vcpkg.json"), "utf8"));
  const triplet = await readFile(path.join(directory, "triplets", "x64-ytnabber.cmake"));
  return { recipe, tripletSha256: createHash("sha256").update(triplet).digest("hex") };
}

export async function verifyEngineBundle(directory, manifest, buildInputs) {
  const read = async name => JSON.parse(await readFile(path.join(directory, name), "utf8"));
  assert.deepEqual(await read("versions.json"), manifest, "Prepared engines are stale; run npm run engines:prepare.");
  const build = await read("ffmpeg-build.json");
  assert.deepEqual(build.recipe, buildInputs.recipe, "The FFmpeg build recipe changed; run npm run engines:prepare.");
  // The original release recorded the recipe without a separate hash for its triplet.
  const triplet = build.tripletSha256 ?? "52f4a5c6851c3a4690c0dbb5411e1db38413f688d63e53a0119f208189ab60e5";
  assert.equal(triplet, buildInputs.tripletSha256, "The FFmpeg build triplet changed; run npm run engines:prepare.");
  const files = await engineChecksums(directory);
  assert.deepEqual(files, await read("checksums.json"), "Engine resources changed or are incomplete; run npm run engines:prepare.");
  for (const name of ["yt-dlp.exe", "deno.exe", "ffmpeg.exe", "ffprobe.exe"]) {
    assert(files[name], `Missing engine: ${name}. Run npm run engines:prepare.`);
  }
  for (const engine of manifest) {
    if (engine.filename?.endsWith(".exe")) assert.equal(files[engine.filename], engine.sha256, `Pinned checksum mismatch: ${engine.name}.`);
  }
}
