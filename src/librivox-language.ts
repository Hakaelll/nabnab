import type { Copy } from "./contracts";

const spanish = new Map(Object.entries({
  English: "Inglés", Spanish: "Español", French: "Francés", German: "Alemán", Italian: "Italiano",
  Portuguese: "Portugués", Catalan: "Catalán", Galician: "Gallego", Chinese: "Chino",
  Japanese: "Japonés", Russian: "Ruso", Arabic: "Árabe", Dutch: "Neerlandés",
  Multilingual: "Multilingüe",
}));

export function librivoxLanguageLabel(name: string, t: Copy): string {
  return t(spanish.get(name) ?? name, name);
}
