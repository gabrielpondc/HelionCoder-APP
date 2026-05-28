export function formatSelectionQuote(text: string): string {
  const cleaned = text
    .replace(/\u00a0/g, " ")
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((line) => line.trimEnd())
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();

  if (!cleaned) return "";

  return cleaned
    .split("\n")
    .map((line) => (line ? `> ${line}` : ">"))
    .join("\n");
}
