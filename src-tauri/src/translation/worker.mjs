import { join } from "node:path";

const [engine, model, input] = Deno.args;
const { pair, modelFile, shortlistFile, blocks } = JSON.parse(await Deno.readTextFile(input));
const base = new URL("file:///nabnab-bergamot/");
const glue = await Deno.readTextFile(join(engine, "bergamot-translator-worker.js"));
let receive;
let response;
const scope = {
  location: base,
  async fetch(url) {
    const name = "bergamot-translator-worker.wasm";
    if (url.href !== new URL(name, base).href) throw Error("Unexpected translation resource");
    return new Response(await Deno.readFile(join(engine, name)), { headers: { "Content-Type": "application/wasm" } });
  },
  importScripts(...scripts) {
    if (scripts.length !== 1 || scripts[0] !== "bergamot-translator-worker.js") {
      throw Error("Unexpected translation script");
    }
    globalThis.Module = this.Module;
    (0, eval)(glue);
  },
  addEventListener(kind, callback) {
    if (kind === "message") receive = callback;
  },
  postMessage(message) {
    response = message;
  },
};

// The launcher verifies these two upstream scripts before granting Deno read access.
new Function("self", await Deno.readTextFile(join(engine, "translator-worker.js")))(scope);
async function call(name, ...args) {
  response = null;
  await receive({ data: { id: 1, name, args } });
  if (!response || response.error) throw Error(response?.error?.message ?? "Bergamot returned no response");
  return response.result;
}

await call("initialize", { useNativeIntGemm: false, cacheSize: 0 });
const read = async (root, name) => (await Deno.readFile(join(root, name))).buffer;
await call("loadTranslationModel", pair, {
  model: await read(model, modelFile),
  shortlist: await read(model, shortlistFile),
  vocabs: [await read(engine, "vocab.esen.spm")],
});

async function translate(texts) {
  const translated = await call("translate", {
    models: [pair],
    texts: texts.map(text => ({ text, html: true, qualityScores: false })),
  });
  if (!Array.isArray(translated) || translated.length !== texts.length) throw Error("Unexpected translation count");
  return translated.map(item => item.target.text);
}

function unpack(text, length) {
  const spans = [...text.matchAll(/<span data-nab-cue="(\d+)">([\s\S]*?)<\/span>/g)];
  if (spans.length !== length) return null;
  let end = 0;
  for (const [index, span] of spans.entries()) {
    if (text.slice(end, span.index).trim() || Number(span[1]) !== index || !span[2].trim()) return null;
    end = span.index + span[0].length;
  }
  if (text.slice(end).trim()) return null;
  return spans.map(span => span[2]);
}

const cues = [];
let contextualBlocks = 0;
for (let offset = 0; offset < blocks.length; offset += 16) {
  const group = blocks.slice(offset, offset + 16);
  const grouped = group.map(block => block.length === 1 ? block[0]
    : block.map((cue, index) => `<span data-nab-cue="${index}">${cue}</span>`).join(" "));
  const results = await translate(grouped);
  for (const [index, result] of results.entries()) {
    const block = group[index];
    if (block.length === 1) {
      cues.push(result);
      continue;
    }
    const aligned = unpack(result, block.length);
    if (aligned) {
      contextualBlocks++;
      cues.push(...aligned);
    } else {
      cues.push(...await translate(block));
    }
  }
}
console.log(JSON.stringify({ cues, contextualBlocks }));
