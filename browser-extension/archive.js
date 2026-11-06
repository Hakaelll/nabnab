// Injected only after an explicit action on the active tab. No network requests.
export function captureDocument(mode) {
  if (!["reading", "selection", "page"].includes(mode)) throw new Error("Elige artículo, selección o página.");
  const excluded = 'script,style,noscript,form,input,textarea,select,button,iframe,object,embed,canvas,svg,[hidden],[aria-hidden=true],[contenteditable]:not([contenteditable="false" i])';
  if (document.designMode.toLowerCase() === "on") throw new Error("No se archivan documentos en modo de edición. Sal del editor y vuelve a capturar.");
  const forbidden = element => element.isContentEditable || Boolean(element.closest(excluded));
  const nodeLimit = 25000;
  let visited = 0, partial = false;
  const warnings = ["Copia estática. No conserva funciones interactivas ni recursos que el navegador no permita copiar."];
  const resources = [];
  const blocks = [];
  let textBytes = 0, imageBytes = 0;
  const append = (kind, text, extra = {}) => {
    text = String(text || "").replace(/\r/g, "").trim();
    if (!text && kind !== "image") return;
    if (blocks.length >= 4000 || textBytes + text.length > 500000) { partial = true; return; }
    textBytes += text.length;
    blocks.push({ kind, text, ...extra });
  };
  const source = new URL(location.href);
  source.username = ""; source.password = ""; source.hash = "";
  for (const key of [...source.searchParams.keys()]) if (/token|auth|secret|key|signature|credential|session|password|^x-amz-|^x-goog-|^utm_/i.test(key)) source.searchParams.delete(key);
  const title = (document.querySelector("meta[property='og:title']")?.content || document.title || source.hostname).slice(0, 500);
  const author = document.querySelector("meta[name='author']")?.content?.slice(0, 500) || null;
  const publishedAt = document.querySelector("meta[property='article:published_time']")?.content?.slice(0, 80) || null;
  if (mode === "selection") {
    const selection = window.getSelection();
    if (!selection?.rangeCount || !selection.toString().trim()) throw new Error("Selecciona el texto que quieres guardar.");
    const range = selection.getRangeAt(0);
    const node = range.commonAncestorContainer;
    const element = node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
    if (element && forbidden(element)) throw new Error("No se guardan campos de formularios ni texto editable.");
    const walker = document.createTreeWalker(element || document.body, NodeFilter.SHOW_ELEMENT);
    while (walker.nextNode()) {
      if (++visited > nodeLimit) throw new Error("La selección es demasiado amplia. Selecciona un fragmento más pequeño.");
      if (forbidden(walker.currentNode) && range.intersectsNode(walker.currentNode)) throw new Error("La selección incluye formularios o texto editable. Selecciona solo el contenido que quieres archivar.");
    }
    append("paragraph", selection.toString());
    const contextWalker = document.createTreeWalker(element || document.body, NodeFilter.SHOW_ALL, { acceptNode(node) {
      if (++visited > nodeLimit) return NodeFilter.FILTER_ACCEPT;
      if (node.nodeType === Node.ELEMENT_NODE && (forbidden(node) || getComputedStyle(node).display === "none" || getComputedStyle(node).visibility === "hidden")) return NodeFilter.FILTER_REJECT;
      return node.nodeType === Node.TEXT_NODE ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP;
    } });
    let context = "";
    while (contextWalker.nextNode() && visited <= nodeLimit && context.length < 500000) context += contextWalker.currentNode.textContent.slice(0, 500000 - context.length);
    const at = context.indexOf(selection.toString());
    const prefix = at < 0 ? "" : context.slice(Math.max(0, at - 120), at);
    const suffix = at < 0 ? "" : context.slice(at + selection.toString().length, at + selection.toString().length + 120);
    return { schemaVersion: 1, mode, title, author, url: source.href, publishedAt, archivedAt: new Date().toISOString(), blocks, resources, warnings, selectionContext: { prefix, suffix } };
  }
  const root = mode === "reading" ? document.querySelector("main,[role=main]") || document.querySelector("article") || document.body : document.body;
  const visible = element => {
    if (forbidden(element) || mode === "reading" && element.closest("nav,aside,footer,header")) return false;
    const style = getComputedStyle(element);
    return style.display !== "none" && style.visibility !== "hidden" && element.getClientRects().length > 0;
  };
  const images = new Map();
  const elements = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, { acceptNode(element) {
    if (++visited > nodeLimit) return NodeFilter.FILTER_ACCEPT;
    return forbidden(element) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT;
  } });
  while (elements.nextNode()) {
    if (visited > nodeLimit || textBytes >= 500000 || blocks.length >= 4000) { partial = true; break; }
    const element = elements.currentNode;
    if (!element.matches("h1,h2,h3,h4,h5,h6,p,li,pre,blockquote,table,img,div")) continue;
    if (!visible(element)) continue;
    if (element instanceof HTMLImageElement) {
      if (!element.complete || !element.naturalWidth || resources.length >= 32) { warnings.push("Hay imágenes sin cargar o fuera del límite de 32 recursos."); continue; }
      const imageKey = element.currentSrc || element.src;
      const known = images.get(imageKey);
      if (known) { append("image", element.alt, { resourceId: known }); continue; }
      try {
        const scale = Math.min(1, 2048 / element.naturalWidth, 2048 / element.naturalHeight);
        const canvas = document.createElement("canvas");
        canvas.width = Math.max(1, Math.round(element.naturalWidth * scale)); canvas.height = Math.max(1, Math.round(element.naturalHeight * scale));
        const context = canvas.getContext("2d"); if (!context) continue;
        context.drawImage(element, 0, 0, canvas.width, canvas.height);
        const dataUrl = canvas.toDataURL("image/png");
        if (dataUrl.length > 350000 || imageBytes + dataUrl.length > 2000000) { warnings.push("Una imagen supera el límite de recursos de esta captura."); continue; }
        const id = "image-" + resources.length;
        resources.push({ id, dataUrl, width: canvas.width, height: canvas.height }); images.set(imageKey, id); imageBytes += dataUrl.length;
        append("image", element.alt, { resourceId: id });
        if (scale < 1) warnings.push("Se ha reducido una imagen a un máximo de 2048 píxeles.");
      } catch { warnings.push("No se pudo conservar una imagen por las restricciones del origen. No se solicitó de nuevo."); }
      continue;
    }
    if (element.parentElement?.closest("li,pre,blockquote,table") && !element.matches("li")) continue;
    if (element.matches("div") && (!Array.from(element.childNodes).some(node => node.nodeType === Node.TEXT_NODE && node.textContent.trim()) || element.querySelector("h1,h2,h3,h4,h5,h6,p,li,pre,blockquote,table,div"))) continue;
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_ALL, { acceptNode(node) {
      if (++visited > nodeLimit) return NodeFilter.FILTER_ACCEPT;
      if (node.nodeType === Node.ELEMENT_NODE && forbidden(node)) return NodeFilter.FILTER_REJECT;
      return node.nodeType === Node.TEXT_NODE ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP;
    } });
    const parts = [];
    let blockLength = 0;
    while (walker.nextNode()) {
      if (visited > nodeLimit) { partial = true; break; }
      const parent = walker.currentNode.parentElement;
      const nestedList = parent?.closest("ul,ol");
      if (parent && visible(parent) && !(nestedList && element.contains(nestedList))) {
        const separator = parts.length ? 1 : 0;
        const remaining = 500000 - textBytes - blockLength - separator;
        if (remaining <= 0) { partial = true; break; }
        const text = walker.currentNode.textContent;
        if (text.length > remaining) { parts.push(text.slice(0, remaining)); partial = true; break; }
        parts.push(text); blockLength += text.length + separator;
      }
    }
    const text = parts.join(" ");
    const tag = element.tagName.toLowerCase();
    append(/^h[1-6]$/.test(tag) ? "heading" : tag === "pre" ? "code" : tag === "blockquote" ? "quote" : tag === "li" ? "listItem" : "paragraph", text, /^h[1-6]$/.test(tag) ? { level: Number(tag[1]) } : {});
  }
  if (!blocks.some(block => block.kind !== "image" && block.text)) throw new Error("No se encontró texto visible para archivar. Prueba Guardar selección.");
  if (partial || blocks.length >= 4000 || textBytes >= 490000) warnings.push("La captura está limitada por tamaño o exploración del documento y puede ser parcial.");
  if (mode === "page") warnings.push("La página conserva texto, encabezados e imágenes disponibles en su orden. El diseño se simplifica y no se rastrean enlaces.");
  return { schemaVersion: 1, mode, title, author, url: source.href, publishedAt, archivedAt: new Date().toISOString(), blocks, resources, warnings: [...new Set(warnings)], selectionContext: null };
}

export function validateCapture(value) {
  if (!value || value.schemaVersion !== 1 || !["reading", "selection", "page"].includes(value.mode) || typeof value.title !== "string" || value.title.length > 500 || !Array.isArray(value.blocks) || !value.blocks.length || value.blocks.length > 4000 || !Array.isArray(value.resources) || value.resources.length > 32 || JSON.stringify(value).length > 3500000) throw new Error("La captura supera los límites o no es válida.");
  const url = new URL(value.url);
  if (!["http:", "https:"].includes(url.protocol) || url.username || url.password) throw new Error("La captura necesita una URL HTTP o HTTPS válida.");
  const resources = new Set();
  for (const resource of value.resources) {
    if (!/^image-\d{1,2}$/.test(resource.id) || resources.has(resource.id) || typeof resource.dataUrl !== "string" || !/^data:image\/png;base64,[A-Za-z0-9+/]+={0,2}$/.test(resource.dataUrl) || resource.dataUrl.length > 350000) throw new Error("Un recurso de la captura no es válido.");
    resources.add(resource.id);
  }
  for (const block of value.blocks) {
    if (!["heading", "paragraph", "image", "quote", "code", "listItem"].includes(block.kind) || typeof block.text !== "string" || block.text.length > 500000 || block.kind === "heading" && (!Number.isInteger(block.level) || block.level < 1 || block.level > 6) || block.kind === "image" && !resources.has(block.resourceId)) throw new Error("Un bloque de la captura no es válido.");
  }
  return value;
}
