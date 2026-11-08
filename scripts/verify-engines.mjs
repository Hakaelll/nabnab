import { readFile } from "node:fs/promises";
import { ffmpegBuildInputs, verifyEngineBundle } from "./engine-integrity.mjs";
import { fileURLToPath } from "node:url";

const read = async url => JSON.parse(await readFile(new URL(url, import.meta.url), "utf8"));
const root = new URL("../src-tauri/engines/", import.meta.url);
await verifyEngineBundle(fileURLToPath(root), await read("./engine-manifest.json"), await ffmpegBuildInputs(fileURLToPath(new URL("./ffmpeg/", import.meta.url))));
console.log("Verified production engine manifest, recipe and file integrity.");
