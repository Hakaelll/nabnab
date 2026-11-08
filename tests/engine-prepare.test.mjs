import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { appendFile, cp, mkdir, mkdtemp, readFile, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { publishEngineBundle, withEngineWorkspace } from "../scripts/engine-workspace.mjs";
import { engineChecksums, ffmpegBuildInputs, verifyEngineBundle } from "../scripts/engine-integrity.mjs";

const windows = { skip: process.platform !== "win32" || process.arch !== "x64" };

async function withFixture(check) {
  const parent = await realpath(tmpdir());
  const fixture = await mkdtemp(path.join(parent, "nabnab-engine-preparation-"));
  try {
    const scripts = path.join(fixture, "scripts");
    await cp(fileURLToPath(new URL("../scripts/", import.meta.url)), scripts, { recursive: true });
    const installed = path.join(fixture, "src-tauri", "engines");
    await mkdir(installed, { recursive: true });
    const previous = path.join(installed, "ffmpeg.exe");
    await writeFile(previous, "previous working engine");
    await check({ fixture, scripts, installed, previous });
  } finally {
    assert.equal(path.dirname(await realpath(fixture)), parent);
    await rm(fixture, { recursive: true, force: true });
  }
}

function prepareFixture(fixture, scripts, preload) {
  return spawnSync(process.execPath, ["--import", `data:text/javascript,${encodeURIComponent(preload)}`, path.join(scripts, "prepare-engines.mjs")], {
    cwd: fixture, encoding: "utf8", windowsHide: true,
  });
}

test("a failed engine download leaves the installed bundle and removes temporary work", windows, async () => {
  await withFixture(async ({ fixture, scripts, previous }) => {
    const offline = `globalThis.fetch = async () => { throw new Error("fixture network failure"); };`;
    const result = prepareFixture(fixture, scripts, offline);
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /fixture network failure/);
    assert.equal(await readFile(previous, "utf8"), "previous working engine");
    assert.deepEqual(await readdir(path.join(fixture, ".cache")), []);
  });
});

test("a failed FFmpeg build cleans its tree and leaves the installed bundle", windows, async t => {
  await withFixture(async ({ fixture, scripts, previous }) => {
    if (fixture.includes(" ")) return t.skip("The vcpkg recipe requires a path without spaces.");
    await writeFile(path.join(scripts, "engine-manifest.json"), JSON.stringify([{ name: "ffmpeg", build: "vcpkg" }]));
    const failedBuild = `
      import assert from "node:assert/strict";
      import child from "node:child_process";
      import { mkdirSync, writeFileSync } from "node:fs";
      import { syncBuiltinESMExports } from "node:module";
      import path from "node:path";
      child.execFileSync = (program, args) => {
        if (path.basename(program) === "vswhere.exe") return "C:/Fixture Visual Studio";
        assert.equal(path.basename(program), "vcpkg.exe");
        assert.ok(args.includes("--clean-after-build"));
        const tree = args.find(arg => arg.startsWith("--x-buildtrees-root=")).split("=")[1];
        mkdirSync(tree, { recursive: true });
        writeFileSync(path.join(tree, "partially-built.bin"), "build residue");
        throw new Error("fixture compiler failure");
      };
      syncBuiltinESMExports();`;
    const result = prepareFixture(fixture, scripts, failedBuild);
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /fixture compiler failure/);
    assert.equal(await readFile(previous, "utf8"), "previous working engine");
    assert.deepEqual(await readdir(path.join(fixture, ".cache")), []);
  });
});

test("engine publication replaces the previous bundle and removes its backup", async () => {
  await withFixture(async ({ fixture, installed, previous }) => {
    await withEngineWorkspace(fixture, "eng-", async (workspace, cache) => {
      const staged = path.join(workspace, "bundle");
      await mkdir(staged);
      await writeFile(path.join(staged, "ffmpeg.exe"), "new working engine");
      await publishEngineBundle(staged, installed, cache);
    });
    assert.equal(await readFile(previous, "utf8"), "new working engine");
    assert.deepEqual(await readdir(path.join(fixture, ".cache")), []);
  });
});

test("a blocked final move restores the previous engine bundle", async () => {
  await withFixture(async ({ fixture, installed, previous }) => {
    const script = new URL("../scripts/engine-workspace.mjs", import.meta.url).href;
    const testPublish = `
      import fs from "node:fs/promises";
      import { syncBuiltinESMExports } from "node:module";
      import path from "node:path";
      const rename = fs.rename;
      fs.rename = async (from, to) => {
        if (path.basename(from) === "bundle") throw new Error("fixture replacement failure");
        return rename(from, to);
      };
      syncBuiltinESMExports();
      const { publishEngineBundle, withEngineWorkspace } = await import(${JSON.stringify(script)});
      await withEngineWorkspace(process.cwd(), "eng-", async (workspace, cache) => {
        const staged = path.join(workspace, "bundle");
        await fs.mkdir(staged);
        await publishEngineBundle(staged, ${JSON.stringify(installed)}, cache);
      });`;
    const result = spawnSync(process.execPath, ["--input-type=module", "-e", testPublish], { cwd: fixture, encoding: "utf8", windowsHide: true });
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /fixture replacement failure/);
    assert.equal(await readFile(previous, "utf8"), "previous working engine");
    assert.deepEqual(await readdir(path.join(fixture, ".cache")), []);
  });
});

test("a changed FFmpeg triplet invalidates a reusable engine bundle", async () => {
  await withFixture(async ({ scripts, installed }) => {
    const recipe = path.join(scripts, "ffmpeg");
    const build = await ffmpegBuildInputs(recipe);
    await writeFile(path.join(installed, "versions.json"), "[]");
    await writeFile(path.join(installed, "ffmpeg-build.json"), JSON.stringify(build));
    for (const name of ["yt-dlp.exe", "deno.exe", "ffprobe.exe"]) await writeFile(path.join(installed, name), "fixture");
    await writeFile(path.join(installed, "checksums.json"), JSON.stringify(await engineChecksums(installed)));
    await verifyEngineBundle(installed, [], build);
    await appendFile(path.join(recipe, "triplets", "x64-ytnabber.cmake"), "\nset(VCPKG_BUILD_TYPE debug)\n");
    await assert.rejects(verifyEngineBundle(installed, [], await ffmpegBuildInputs(recipe)), /FFmpeg build triplet changed/);
  });
});
