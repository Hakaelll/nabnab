export type OutputFormat = "mp4" | "mp3" | "m4a" | "opus";

export function durationLabel(seconds: number | null): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return "Duration unavailable";
  const value = Math.floor(seconds);
  const minutes = Math.floor(value / 60);
  const remaining = String(value % 60).padStart(2, "0");
  return minutes >= 60
    ? Math.floor(minutes / 60) + ":" + String(minutes % 60).padStart(2, "0") + ":" + remaining
    : minutes + ":" + remaining;
}

export function bytesLabel(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const index = Math.min(3, Math.floor(Math.log(bytes) / Math.log(1024)));
  return (bytes / 1024 ** index).toFixed(index < 2 ? 0 : 1) + " " + units[index];
}

export function validWebUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return ["http:", "https:"].includes(url.protocol) && Boolean(url.hostname) && !url.username && !url.password;
  } catch { return false; }
}

export function errorLabel(error: unknown): string {
  if (isFailure(error)) return error.message;
  const raw = String(error).replace(/\x1b\[[0-9;]*m/g, "").replace(/^ERROR:\s*/gm, "").trim();
  if (/no space left|disk full|not enough space|WinError 112/i.test(raw)) return "Your disk is full. Free up space or choose another folder.";
  if (/permission denied|access is denied|WinError 5/i.test(raw)) return "This folder is not writable. Choose another destination.";
  if (/too many requests|HTTP Error 429/i.test(raw)) return "The server is limiting requests. Wait before trying again.";
  if (/confirm.*not.*bot|automated queries/i.test(raw)) return "This source requires sign-in or session verification. Try a public video or try again later.";
  if (/HTTP Error 403|forbidden/i.test(raw)) return "The source denied access (HTTP 403). Its formats or session requirements may have changed.";
  if (/private video|sign in|login required|members.only|age.restricted/i.test(raw)) return "This video requires access or sign-in. Try a publicly available video.";
  if (/requested format.*not available/i.test(raw)) return "This quality is no longer available. Load the link again or select Best available.";
  if (/not available|unavailable|removed|not found|404/i.test(raw)) return "This video is unavailable. Check the link or try another video.";
  if (/unsupported url|no suitable extractor/i.test(raw)) return "This link is not supported. Paste a direct video URL.";
  if (/timed out|name resolution|unable to download|connection|network/i.test(raw)) return "Could not reach the source. Check your connection and try again.";
  if (/ffmpeg|ffprobe|conversion failed/i.test(raw) && !/missing/.test(raw)) return "The media could not be processed. Try another format or source.";
  return raw.slice(0, 500) || "Something went wrong. Try again.";
}

interface Failure { code: string; message: string; technical: string; retryable: boolean; retryAfter: number | null; phase: string }
function isFailure(error: unknown): error is Failure {
  return typeof error === "object" && error !== null
    && "code" in error && typeof error.code === "string"
    && "message" in error && typeof error.message === "string"
    && "technical" in error && typeof error.technical === "string"
    && "retryable" in error && typeof error.retryable === "boolean"
    && "phase" in error && typeof error.phase === "string"
    && "retryAfter" in error && (error.retryAfter === null || (typeof error.retryAfter === "number" && Number.isFinite(error.retryAfter) && error.retryAfter >= 0));
}
export function isCancelled(error: unknown): boolean { return isFailure(error) ? error.code === "cancelled" : String(error) === "Cancelled."; }
export function canRetry(error: unknown): boolean { return isFailure(error) ? error.retryable && !error.retryAfter : true; }
export function errorDetails(error: unknown): string {
  return isFailure(error) ? `${error.code} · ${error.phase}\n${error.technical}${error.retryAfter ? `\nRetry after at least ${error.retryAfter} seconds.` : ""}` : errorLabel(error);
}
