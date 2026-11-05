import assert from "node:assert/strict";
import { readdir, readFile, stat } from "node:fs/promises";
import path from "node:path";

const root = path.resolve(process.argv[2] ?? ".");
const { version } = JSON.parse(await readFile(path.join(root, "package.json"), "utf8"));
async function inventory(directory, prefix = "") {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const name = path.join(prefix, entry.name);
    if (entry.isDirectory()) files.push(...await inventory(path.join(directory, entry.name), name));
    else if (entry.isFile()) files.push({ name, bytes: (await stat(path.join(directory, entry.name))).size });
  }
  return files;
}
const native = { name: "nabnab.exe", bytes: (await stat(path.join(root, "src-tauri/target/release/nabnab.exe"))).size };
const engines = (await inventory(path.join(root, "src-tauri/engines"))).map(f => ({ ...f, name: "engines/" + f.name.replaceAll("\\", "/") }));
const licenses = (await inventory(path.join(root, "licenses"))).map(f => ({ ...f, name: "licenses/" + f.name.replaceAll("\\", "/") }));
const notices = { name: "THIRD_PARTY_NOTICES.md", bytes: (await stat(path.join(root, "THIRD_PARTY_NOTICES.md"))).size };
const payload = [native, ...engines, ...licenses, notices];
assert(payload.length > 5 && payload.every(f => f.bytes > 0), "Release payload must contain nonempty runtime files.");
assert.equal(new Set(payload.map(f => f.name)).size, payload.length, "Release payload paths must be unique.");
const total = files => files.reduce((sum, file) => sum + file.bytes, 0);
const installer = path.join(root, `src-tauri/target/release/bundle/nsis/nabnab_${version}_x64-setup.exe`);
console.log(JSON.stringify({ version, installerBytes: (await stat(installer)).size, payloadBytes: total(payload), nativeBytes: native.bytes,
  engineBytes: total(engines), licenseBytes: total(licenses) + notices.bytes,
  note: "Logical payload bytes; excludes uninstaller, shared WebView2, filesystem allocation and user data. This is not a clean-machine installed-size measurement.",
  largest: [...payload].sort((a, b) => b.bytes - a.bytes).slice(0, 10), files: payload }, null, 2));
