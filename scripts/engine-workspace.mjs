import assert from "node:assert/strict";
import { mkdir, mkdtemp, realpath, rename, rm } from "node:fs/promises";
import path from "node:path";

async function assertUnredirected(directory) {
  assert.equal(path.relative(directory, await realpath(directory)), "", `Engine workspace path is redirected: ${directory}`);
}

async function removeWorkspace(directory) {
  await assertUnredirected(directory);
  await rm(directory, { recursive: true, force: true, maxRetries: 3 });
}

export async function withEngineWorkspace(root, prefix, prepare) {
  const cache = path.join(await realpath(root), ".cache");
  await mkdir(cache, { recursive: true });
  await assertUnredirected(cache);
  const workspace = await mkdtemp(path.join(cache, prefix));
  try { return await prepare(workspace, cache); }
  finally { await removeWorkspace(workspace); }
}

export async function publishEngineBundle(staged, target, cache) {
  await assertUnredirected(staged);
  await assertUnredirected(path.dirname(target));
  const backup = path.join(cache, `${path.basename(target)}-previous`);
  try { await mkdir(backup); }
  catch (error) {
    if (error.code !== "EEXIST") throw error;
    throw new Error(`An engine publication is in progress or needs recovery. Check ${backup}.`, { cause: error });
  }
  const previous = path.join(backup, "files");
  let saved = false, preserve = false;
  try {
    try {
      await assertUnredirected(target);
      await rename(target, previous);
      saved = true;
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    try { await rename(staged, target); }
    catch (error) {
      if (saved) {
        try { await rename(previous, target); }
        catch (restoreError) {
          preserve = true;
          throw new AggregateError([error, restoreError], `Could not restore the previous engines. They are saved in ${previous}.`);
        }
      }
      throw error;
    }
  } finally {
    if (!preserve) await removeWorkspace(backup);
  }
}
