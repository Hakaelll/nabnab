import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFile, mkdir, readdir, readFile, writeFile, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
export async function buildFFmpeg() {
  assert(process.platform === "win32" && process.arch === "x64", "FFmpeg targets Windows x64.");
  const recipe = path.join(root, "scripts", "ffmpeg");
  const cache = path.join(root, ".cache", "ffmpeg-build");
  const installed = path.join(cache, "installed", "x64-ytnabber");
  const output = path.join(cache, "bundle");
  assert(!cache.includes(" "), "The vcpkg FFmpeg build needs a checkout path without spaces; installed app paths may contain spaces.");
  const vswhere = path.join(process.env["ProgramFiles(x86)"] ?? "C:/Program Files (x86)", "Microsoft Visual Studio", "Installer", "vswhere.exe");
  const visualStudio = execFileSync(vswhere, ["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"], { encoding: "utf8", windowsHide: true }).trim();
  assert(visualStudio, "Visual Studio C++ Build Tools are required to build FFmpeg.");
  await mkdir(path.join(cache, "downloads"), { recursive: true });
  await mkdir(path.join(cache, "registries"), { recursive: true });
  execFileSync(path.join(visualStudio, "VC", "vcpkg", "vcpkg.exe"), [
    "install", `--x-manifest-root=${recipe}`, "--triplet=x64-ytnabber", `--overlay-triplets=${path.join(recipe, "triplets")}`,
    `--x-install-root=${path.join(cache, "installed")}`, `--x-buildtrees-root=${path.join(cache, "buildtrees")}`,
    `--x-packages-root=${path.join(cache, "packages")}`, "--binarysource=clear",
  ], { stdio: "inherit", windowsHide: true, env: { ...process.env, VCPKG_DOWNLOADS: path.join(cache, "downloads"), X_VCPKG_REGISTRIES_CACHE: path.join(cache, "registries") } });
  assert(path.dirname(path.resolve(output)) === path.resolve(cache), "Bundle must stay within the build cache.");
  await rm(output, { recursive: true, force: true });
  await mkdir(output, { recursive: true });
  for (const name of await readdir(path.join(installed, "bin"))) {
    if (name.endsWith(".dll")) await copyFile(path.join(installed, "bin", name), path.join(output, name));
  }
  for (const name of ["ffmpeg.exe", "ffprobe.exe"]) await copyFile(path.join(installed, "tools", "ffmpeg", name), path.join(output, name));
  const licenses = path.join(output, "licenses");
  await mkdir(licenses, { recursive: true });
  for (const entry of await readdir(path.join(installed, "share"), { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const license = path.join(installed, "share", entry.name, "copyright");
    try { await copyFile(license, path.join(licenses, entry.name + ".txt")); }
    catch (error) { if (error.code !== "ENOENT") throw error; }
  }
  const run = args => execFileSync(path.join(output, "ffmpeg.exe"), ["-hide_banner", ...args], { encoding: "utf8", windowsHide: true, maxBuffer: 1024 * 1024 });
  const encoders = run(["-encoders"]);
  for (const codec of ["libmp3lame", "aac", "libopus", "libopenh264", "libvpx-vp9", "libwebp", "libwebp_anim"]) assert(encoders.includes(codec), "Missing encoder: " + codec);
  assert(run(["-filters"]).includes("subtitles"), "FFmpeg must retain local subtitle burning.");
  const decoders = run(["-decoders"]);
  for (const codec of ["h264", "aac", "opus", "vp9", "libdav1d"]) assert(decoders.includes(codec), "Missing decoder: " + codec);
  assert(run(["-protocols"]).includes("https"), "FFmpeg must retain HTTPS.");
  assert(run(["-devices"]).includes("lavfi"), "FFmpeg must retain local fixture generation.");
  execFileSync(path.join(output, "ffprobe.exe"), ["-version"], { windowsHide: true });
  const files = {};
  for (const name of await readdir(output)) {
    if (name.endsWith(".dll") || name.endsWith(".exe")) files[name] = createHash("sha256").update(await readFile(path.join(output, name))).digest("hex");
  }
  const configuration = spawnSync(path.join(output, "ffmpeg.exe"), ["-hide_banner", "-buildconf"], { encoding: "utf8", windowsHide: true });
  assert.equal(configuration.status, 0, "FFmpeg configuration must be readable.");
  const buildConfiguration = (configuration.stdout + configuration.stderr).replaceAll(root.replaceAll("\\", "/"), "$BUILD_ROOT/").replaceAll(root, "$BUILD_ROOT/");
  await writeFile(path.join(output, "ffmpeg-build.json"), JSON.stringify({ recipe: JSON.parse(await readFile(path.join(recipe, "vcpkg.json"), "utf8")), configuration: buildConfiguration, files }, null, 2));
  return output;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) console.log(await buildFFmpeg());
