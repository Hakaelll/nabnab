import type { LibraryItem } from "./contracts";

export function arxivCopies(library: readonly LibraryItem[]): Map<string, LibraryItem> {
  const copies = new Map<string, LibraryItem>();
  for (const item of library) {
    const id = item.details?.contentId;
    if (!id?.startsWith("arxiv:") || item.kind !== "document" || item.missing || !/\.pdf$/i.test(item.name)) continue;
    const previous = copies.get(id);
    if (!previous || previous.addedAt < item.addedAt) copies.set(id, item);
  }
  return copies;
}
