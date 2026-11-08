import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";

export async function openSftpFixture(module, identity) {
  const { Server, utils } = createRequire(import.meta.url)(module);
  const { OPEN_MODE, STATUS_CODE: status } = utils.sftp;
  const keygen = path.join(process.env.WINDIR, "System32/OpenSSH/ssh-keygen.exe");
  execFileSync(keygen, ["-q", "-t", "ed25519", "-N", "", "-f", identity], { windowsHide: true });
  const loginKey = utils.parseKey(await readFile(`${identity}.pub`));
  const hostname = "127.0.0.1", username = "reader", root = "/vault";
  const folder = `${root}/Lecturas`, name = "Informe ñ.txt", remote = `${folder}/${name}`;
  const link = `${root}/Atajo`;
  let data = Buffer.from("Primera edición\n" + "a".repeat(265_000));
  let modified = 1_751_000_000, hostKey = utils.generateKeyPairSync("ed25519");
  let listener, port = 0, authenticationRequests = 0;
  let interruptNext = false, readGate = null;
  const clients = new Set();
  const activity = [];
  const version = () => ({ size: data.length, mode: 0o100644, atime: modified, mtime: modified });
  const directory = { mode: 0o40755 };
  const kind = (file) => ["/", root, folder].includes(file) ? directory
    : file === remote ? version() : file === link ? { mode: 0o120777 } : null;

  async function start() {
    listener = new Server({ hostKeys: [hostKey.private] }, client => {
      clients.add(client);
      client.on("error", () => {});
      client.once("close", () => clients.delete(client));
      client.on("authentication", request => {
        authenticationRequests++;
        if (request.method !== "publickey" || request.username !== username || request.key.algo !== loginKey.type
          || !request.key.data.equals(loginKey.getPublicSSH())
          || (request.signature && loginKey.verify(request.blob, request.signature, request.hashAlgo) !== true)) {
          request.reject(["publickey"]);
          return;
        }
        request.accept();
      });
      client.on("ready", () => {
        client.on("session", accept => {
          const session = accept();
          session.on("exec", (_, reject) => { activity.push(["EXEC"]); reject(); });
          session.on("shell", (_, reject) => { activity.push(["SHELL"]); reject(); });
          session.on("sftp", acceptSftp => {
            const channel = acceptSftp();
            const handles = new Map();
            let next = 0;
            function allocate(request, state) {
              const handle = Buffer.alloc(4);
              handle.writeUInt32BE(++next);
              handles.set(handle.toString("hex"), state);
              channel.handle(request, handle);
            }
            function entry(filename, file) { return { filename, longname: filename, attrs: kind(file) }; }
            channel.on("LSTAT", (request, file) => {
              activity.push(["LSTAT", file]);
              const attrs = kind(file);
              if (attrs) channel.attrs(request, attrs); else channel.status(request, status.NO_SUCH_FILE);
            });
            channel.on("REALPATH", (request, file) => {
              activity.push(["REALPATH", file]);
              if (kind(file)) channel.name(request, [entry(file, file)]); else channel.status(request, status.NO_SUCH_FILE);
            });
            channel.on("OPENDIR", (request, file) => {
              activity.push(["OPENDIR", file]);
              if (file === root || file === folder) allocate(request, { directory: file, read: false });
              else channel.status(request, status.PERMISSION_DENIED);
            });
            channel.on("READDIR", (request, handle) => {
              const opened = handles.get(handle.toString("hex"));
              if (!opened?.directory) return channel.status(request, status.FAILURE);
              if (opened.read) return channel.status(request, status.EOF);
              opened.read = true;
              channel.name(request, opened.directory === root ? [entry("Lecturas", folder), entry("Atajo", link)] : [entry(name, remote)]);
            });
            channel.on("OPEN", (request, file, flags) => {
              activity.push(["OPEN", file, flags]);
              if (file === remote && flags === OPEN_MODE.READ) allocate(request, { contents: data, attrs: version() });
              else channel.status(request, status.PERMISSION_DENIED);
            });
            channel.on("FSTAT", (request, handle) => {
              const opened = handles.get(handle.toString("hex"));
              if (opened?.attrs) channel.attrs(request, opened.attrs); else channel.status(request, status.FAILURE);
            });
            channel.on("READ", (request, handle, offset, count) => {
              activity.push(["READ", offset, count]);
              const opened = handles.get(handle.toString("hex"));
              if (!opened?.contents) return channel.status(request, status.FAILURE);
              if (offset >= opened.contents.length) return channel.status(request, status.EOF);
              if (offset > 0 && interruptNext) { interruptNext = false; client.end(); return; }
              const send = () => channel.data(request, opened.contents.subarray(offset, offset + count));
              if (offset > 0 && readGate) {
                const pending = readGate;
                readGate = null;
                void pending.then(send);
              } else send();
            });
            channel.on("CLOSE", (request, handle) => {
              const removed = handles.delete(handle.toString("hex"));
              channel.status(request, removed ? status.OK : status.FAILURE);
            });
          });
        });
      });
    });
    await new Promise((resolve, reject) => {
      listener.once("error", reject);
      listener.listen(port, hostname, resolve);
    });
    port = listener.address().port;
  }

  async function close() {
    if (!listener) return;
    for (const client of clients) client.end();
    const stopping = listener;
    listener = null;
    await new Promise(resolve => stopping.close(resolve));
  }

  await start();
  return {
    hostname, port, root, username, name, activity, close,
    get fingerprint() {
      const key = Buffer.from(hostKey.public.split(" ")[1], "base64");
      return `SHA256:${createHash("sha256").update(key).digest("base64").replace(/=+$/, "")}`;
    },
    get authenticationRequests() { return authenticationRequests; },
    get data() { return data; },
    interruptNextCopy() { interruptNext = true; },
    holdAfterFirstChunk(gate) { readGate = gate; },
    replace() { data = Buffer.from("Segunda edición\n" + "b".repeat(265_000)); modified += 2; },
    async rotateHostKey() { await close(); hostKey = utils.generateKeyPairSync("ed25519"); await start(); },
  };
}
