import { createHash } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import { mkdir, readFile, writeFile, readdir, rename, unlink } from "node:fs/promises";
import assert from "node:assert/strict";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { buildFFmpeg } from "./build-ffmpeg.mjs";
import { engineChecksums, ffmpegBuildInputs, verifyEngineBundle } from "./engine-integrity.mjs";
import { publishEngineBundle, withEngineWorkspace } from "./engine-workspace.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const target = path.join(root, "src-tauri", "engines");
const read = async url => JSON.parse(await readFile(new URL(url, import.meta.url), "utf8"));
const manifest = await read("./engine-manifest.json");
const buildInputs = await ffmpegBuildInputs(path.join(root, "scripts", "ffmpeg"));
if (process.platform !== "win32" || process.arch !== "x64") throw new Error("The bundled engines target Windows x64.");

async function download(engine, archive) {
  console.log("Downloading " + engine.name + " " + engine.version);
  const response = await fetch(engine.url);
  if (!response.ok || !response.body) throw new Error("Download failed: " + engine.url);
  await pipeline(Readable.fromWeb(response.body), createWriteStream(archive));
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(archive)) hash.update(chunk);
  assert.equal(hash.digest("hex"), engine.sha256, "SHA-256 mismatch: " + engine.name);
}

async function moveResources(engine, folder, output) {
  for (const entry of await readdir(folder, { withFileTypes: true })) {
    const file = path.join(folder, entry.name);
    if (entry.isDirectory()) await moveResources(engine, file, output);
    else if (entry.isFile() && (entry.name.endsWith(".dll") || ["deno.exe", "ffmpeg.exe", "ffprobe.exe", "LICENSE", "LICENSE.txt"].includes(entry.name))) {
      const name = entry.name.startsWith("LICENSE") ? engine.name + "-" + entry.name : entry.name;
      await rename(file, path.join(output, name));
    }
  }
}

async function prepare(workspace, cache) {
  const output = path.join(workspace, "bundle");
  await mkdir(output);
  for (const engine of manifest) {
    if (engine.build) {
      assert(engine.name === "ffmpeg" && engine.build === "vcpkg", "Unknown engine build recipe.");
      await buildFFmpeg(path.join(workspace, "v"), output);
      continue;
    }
    const archive = path.join(workspace, engine.filename);
    await download(engine, archive);
    if (engine.filename.endsWith(".exe")) {
      await rename(archive, path.join(output, engine.filename));
    } else {
      const unpacked = path.join(workspace, engine.name);
      await mkdir(unpacked);
      execFileSync("tar.exe", ["-xf", archive, "-C", unpacked], { windowsHide: true });
      await unlink(archive);
      await moveResources(engine, unpacked, output);
    }
    console.log("Verified " + engine.name);
  }
  await writeFile(path.join(output, "versions.json"), JSON.stringify(manifest, null, 2));
  await writeFile(path.join(output, ".gitkeep"), "\n");
  await writeFile(path.join(output, "checksums.json"), JSON.stringify(await engineChecksums(output), null, 2));
  for (const [name, flag] of [["yt-dlp.exe", "--version"], ["deno.exe", "--version"], ["ffmpeg.exe", "-version"], ["ffprobe.exe", "-version"]]) {
    execFileSync(path.join(output, name), [flag], { windowsHide: true });
  }
  await publishEngineBundle(output, target, cache);
}

let ready = false;
if (!process.argv.includes("--rebuild")) {
  try { await verifyEngineBundle(target, manifest, buildInputs); ready = true; }
  catch (error) { console.log(`Preparing engine bundle (${error.code === "ENOENT" ? "installed bundle is incomplete" : error.message}).`); }
}
if (ready) console.log("Installed engines match the manifest and checksums.");
else {
  await withEngineWorkspace(root, "eng-", prepare);
  console.log("Engines ready.");
}
