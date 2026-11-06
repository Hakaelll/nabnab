// Drives captureDocument against rendered DOM in Edge. No provider account is used.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const source = await readFile(new URL("../browser-extension/archive.js", import.meta.url));
let fixture = "";
const server = createServer((request, response) => {
  response.setHeader("Content-Type", request.url === "/archive.js" ? "text/javascript" : "text/html; charset=utf-8");
  response.end(request.url === "/archive.js" ? source : `<!doctype html><title>Archive privacy fixture</title><main>${fixture}</main>`);
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const browser = await chromium.launch({ headless: true, ...(process.env.EDGE_EXECUTABLE ? { executablePath: process.env.EDGE_EXECUTABLE } : { channel: "msedge" }) });
const page = await browser.newPage();
const base = `http://127.0.0.1:${server.address().port}`;
async function open(html) { fixture = html; await page.goto(base); await page.evaluate(async () => { window.captureArchive = (await import("/archive.js")).captureDocument; }); }
async function capture(mode) { return page.evaluate(mode => { try { return { capture: window.captureArchive(mode) }; } catch (error) { return { error: error.message }; } }, mode); }
try {
  await open('<p>Public article.</p><div contenteditable="">EMPTY_ATTRIBUTE_DRAFT<p>INHERITED_DRAFT</p></div><div contenteditable="plaintext-only">PLAINTEXT_DRAFT</div><div contenteditable="true">TRUE_DRAFT</div><div contenteditable="false">Explicitly static content.</div><form><p>FORM_SECRET</p><input value="INPUT_SECRET"><textarea>TEXTAREA_SECRET</textarea></form>');
  for (const mode of ["reading", "page"]) {
    const result = await capture(mode);
    assert.ok(result.capture, result.error);
    const serialized = JSON.stringify(result.capture);
    for (const secret of ["EMPTY_ATTRIBUTE_DRAFT", "INHERITED_DRAFT", "PLAINTEXT_DRAFT", "TRUE_DRAFT", "FORM_SECRET", "INPUT_SECRET", "TEXTAREA_SECRET"]) assert.ok(!serialized.includes(secret), `${mode} retained ${secret}`);
    assert.ok(serialized.includes("Explicitly static content."));
  }
  await page.evaluate(() => { document.designMode = "on"; });
  for (const mode of ["reading", "page", "selection"]) assert.match((await capture(mode)).error, /modo de edición/);

  for (const forbidden of ['<form><p>FORM_SECRET</p></form>', '<div contenteditable="">EMPTY_DRAFT</div>', '<div contenteditable="plaintext-only">PLAINTEXT_DRAFT</div>', '<input value="INPUT_SECRET">']) {
    await open(`<p id="before">Public before.</p>${forbidden}<p id="after">Public after.</p>`);
    await page.evaluate(() => {
      const range = document.createRange(); range.setStart(document.querySelector("#before").firstChild, 0); range.setEnd(document.querySelector("#after").firstChild, 13);
      const selection = window.getSelection(); selection.removeAllRanges(); selection.addRange(range);
    });
    assert.match((await capture("selection")).error, /formularios|editable/);
  }
  await open('<p id="context">Public prefix <span hidden>HIDDEN_CONTEXT</span><span contenteditable="plaintext-only">PRIVATE_CONTEXT</span>chosen words and public suffix</p>');
  await page.evaluate(() => {
    const range = document.createRange(); range.setStart(document.querySelector("#context").lastChild, 0); range.setEnd(document.querySelector("#context").lastChild, 12);
    const selection = window.getSelection(); selection.removeAllRanges(); selection.addRange(range);
  });
  const selected = (await capture("selection")).capture;
  assert.equal(selected.blocks[0].text, "chosen words");
  assert.ok(!JSON.stringify(selected).includes("PRIVATE_CONTEXT") && !JSON.stringify(selected).includes("HIDDEN_CONTEXT"));
  assert.match(selected.selectionContext.prefix, /Public prefix/);

  await open(`<p>Visible opening text.</p>${"<span></span>".repeat(30000)}<p>BEYOND_TRAVERSAL_LIMIT</p>`);
  const bounded = (await capture("page")).capture;
  assert.ok(bounded.blocks.some(block => block.text.includes("Visible opening text.")));
  assert.ok(!JSON.stringify(bounded.blocks).includes("BEYOND_TRAVERSAL_LIMIT"));
  assert.ok(bounded.warnings.some(warning => warning.includes("parcial")));
  await open(`<p>${"x".repeat(500000)}<span>${"y".repeat(1000000)}</span></p>`);
  const longText = (await capture("page")).capture;
  assert.equal(longText.blocks[0].text.length, 500000);
  assert.ok(longText.warnings.some(warning => warning.includes("parcial")));
  console.log("Rendered capture privacy passed: editable states, form-spanning selections, safe selection context, designMode and bounded DOM traversal.");
} finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
