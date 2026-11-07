// Verify the actual rendered NSIS input and its staged console executable, without installing or changing PATH.
import assert from "node:assert/strict";
import {readFile,readdir} from "node:fs/promises";
import {spawnSync} from "node:child_process";
import path from "node:path";

const root=path.resolve(import.meta.dirname,"..");
const rendered=process.argv[2],binary=process.argv[3]??path.join(root,"src-tauri/target/release/nabnab-cli.exe");
assert.ok(rendered,"Usage: node scripts/verify-cli-package.mjs RENDERED_INSTALLER_NSI [RELEASE_CLI_EXE]");
const source=await readFile(rendered,"utf8");
assert.ok(!source.includes("{{#each binaries}}"),"Use the rendered installer, not its source template.");
assert.match(source,/^\s*File\b[^\r\n]*nabnab-cli\.exe/im,"The actual installer must copy the CLI executable.");
assert.match(source,/^\s*Delete\b[^\r\n]*nabnab-cli\.exe/im,"The actual uninstaller must remove the CLI executable.");
const bytes=await readFile(binary),pe=bytes.readUInt32LE(0x3c);assert.equal(bytes.subarray(pe,pe+4).toString(),"PE\0\0");
assert.equal(bytes.readUInt16LE(pe+24+68),3,"The CLI must use the Windows console subsystem.");
const version=spawnSync(binary,["--version"],{encoding:"utf8",windowsHide:true,timeout:10000});
assert.equal(version.status,0);assert.match(version.stdout,/^nabnab-cli wire1 \/ rmcp3\.9\.0\r?\n$/);assert.equal(version.stderr,"");
assert.deepEqual((await readdir(path.join(root,"src-tauri/src/bin"))).sort(),["nabnab-cli.rs"],"Tauri scans every src/bin entry; helpers must live outside this directory.");
console.log(JSON.stringify({passed:true,rendered:path.resolve(rendered),binary:path.resolve(binary),consoleSubsystem:true,installerCopiesCli:true,uninstallerRemovesCli:true}));
