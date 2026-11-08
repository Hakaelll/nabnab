import "./style.css";
import "./audio-library.css";
import "./playback-stage.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createTrackWheel, type WheelAction, type WheelEntry, type WheelSession } from "./track-wheel";

interface Configuration extends WheelSession {
  language: string;
  reducedMotion: boolean;
  reducedTransparency: boolean;
}

type Input = { gesture: number } & (
  | { type: "open"; x: number; y: number; step: number; configuration: Configuration }
  | { type: "move"; step: number }
  | { type: "playing"; playing: boolean }
  | { type: "commit" | "cancel" | "toggle" }
);

let session: Configuration | undefined;
let gesture = 0;
let language: string | undefined;
let pending: Promise<void> = Promise.resolve();

function send(action: { type: "hide" } | { type: "transport"; action: WheelAction }) {
  const message = { ...action, gesture };
  pending = pending.then(() => invoke<void>("quick_skip_overlay", { action: message })).catch(console.error);
}

function apply(target?: WheelEntry) {
  const source = session?.entries.find(entry => entry.position === session?.cursor);
  if (!source) return;
  send({ type: "transport", action: target ? { type: "play", source, target } : { type: "toggle", source } });
}

function thumbnails() {
  for (const element of document.querySelectorAll<HTMLElement>("[data-thumbnail-id]:not([data-thumbnail-queued])")) {
    element.dataset.thumbnailQueued = "";
    void invoke<string>("thumbnail", { id: element.dataset.thumbnailId }).then(url => {
      const cover = new Image();
      cover.alt = "";
      cover.decoding = "async";
      cover.onload = () => { if (element.isConnected) element.replaceChildren(cover); };
      cover.src = url;
    }).catch(() => {});
  }
}

const wheel = createTrackWheel({
  t: (es, en) => language === "es" ? es : en,
  enabled: () => Boolean(session),
  entries: () => session?.entries ?? [],
  cursor: () => session?.cursor ?? -1,
  play: apply,
  toggle: () => apply(),
  playing: () => Boolean(session?.playing),
  thumbnails,
  closed: () => { send({ type: "hide" }); },
});

await listen<Input>("quick-skip-input", ({ payload }) => {
  if (payload.type === "open") {
    gesture = payload.gesture;
    session = payload.configuration;
    language = session.language;
    document.documentElement.lang = language;
    document.documentElement.classList.toggle("reduced-motion", session.reducedMotion);
    document.documentElement.classList.toggle("reduced-transparency", session.reducedTransparency);
    if (!wheel.show(payload.x, payload.y, undefined, payload.step)) send({ type: "hide" });
    return;
  }
  if (payload.gesture !== gesture || !session) return;
  switch (payload.type) {
    case "move": wheel.move(payload.step); break;
    case "playing": session.playing = payload.playing; wheel.update(); break;
    case "commit": wheel.commit(); break;
    case "cancel": wheel.dismiss(); break;
    case "toggle": apply(); break;
  }
}, { target: { kind: "Webview", label: "quick-skip" } });
await invoke("quick_skip_overlay", { action: { type: "ready" } });
