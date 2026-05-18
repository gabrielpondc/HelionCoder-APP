/**
 * File conversion module for office documents (docx, xlsx, pptx).
 *
 * Converts office files to markdown text for injection into the chat
 * as pastedBlocks. Zero backend changes — conversion happens entirely
 * in the frontend.
 */

import mammoth from "mammoth";
import TurndownService from "turndown";
import ExcelJS from "exceljs";
import JSZip from "jszip";

/** Maximum characters in converted output. Prevents context explosion from huge spreadsheets. */
export const MAX_CONVERTED_CHARS = 200_000;

/**
 * Convert a File (docx, xlsx, or pptx) to markdown text.
 * @returns `{ text, format }` where format is always "markdown"
 * @throws User-friendly error message on failure
 */
export async function convertFile(file: File): Promise<{ text: string; format: string }> {
  const ext = file.name.split(".").pop()?.toLowerCase() ?? "";
  const arrayBuffer = await file.arrayBuffer();

  let text: string;
  if (ext === "docx") {
    text = await convertDocx(arrayBuffer);
  } else if (ext === "xlsx") {
    text = await convertXlsx(arrayBuffer);
  } else if (ext === "pptx") {
    text = await convertPptx(arrayBuffer);
  } else {
    throw new Error(`Unsupported conversion format: .${ext}`);
  }

  // Truncate if too large
  if (text.length > MAX_CONVERTED_CHARS) {
    text =
      text.slice(0, MAX_CONVERTED_CHARS) +
      `\n\n[Truncated: original was ${text.length} characters, showing first ${MAX_CONVERTED_CHARS}]`;
  }

  return { text, format: "markdown" };
}

function decodeXmlText(text: string): string {
  return text
    .replace(/&#x([0-9a-fA-F]+);/g, (_m, hex: string) => String.fromCodePoint(parseInt(hex, 16)))
    .replace(/&#(\d+);/g, (_m, dec: string) => String.fromCodePoint(parseInt(dec, 10)))
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, "&");
}

function slideNumber(path: string): number {
  const match = path.match(/slide(\d+)\.xml$/);
  return match ? Number(match[1]) : 0;
}

function extractSlideText(xml: string): string {
  const chunks = [...xml.matchAll(/<a:t[^>]*>([\s\S]*?)<\/a:t>/g)]
    .map((match) => decodeXmlText(match[1]).trim())
    .filter(Boolean);
  return chunks.join("\n");
}

/** Convert a pptx ArrayBuffer to markdown sections (one section per slide). */
async function convertPptx(arrayBuffer: ArrayBuffer): Promise<string> {
  try {
    const zip = await JSZip.loadAsync(arrayBuffer);
    const slidePaths = Object.keys(zip.files)
      .filter((path) => /^ppt\/slides\/slide\d+\.xml$/.test(path))
      .sort((a, b) => slideNumber(a) - slideNumber(b));

    const sections: string[] = [];
    for (const path of slidePaths) {
      const file = zip.files[path];
      if (!file) continue;
      const xml = await file.async("text");
      const text = extractSlideText(xml);
      if (text) sections.push(`## Slide ${slideNumber(path)}\n\n${text}`);
    }

    if (sections.length === 0) {
      throw new Error("Presentation appears to be empty");
    }

    return sections.join("\n\n");
  } catch (e) {
    if (e instanceof Error && e.message === "Presentation appears to be empty") throw e;
    throw new Error(
      `Failed to read PowerPoint document: ${e instanceof Error ? e.message : String(e)}`,
    );
  }
}

/** Convert a docx ArrayBuffer to markdown via mammoth → turndown. */
async function convertDocx(arrayBuffer: ArrayBuffer): Promise<string> {
  try {
    const result = await mammoth.convertToHtml({ arrayBuffer });
    const html = result.value;
    if (!html || html.trim().length === 0) {
      throw new Error("Document appears to be empty");
    }
    const td = new TurndownService({ headingStyle: "atx" });
    return td.turndown(html);
  } catch (e) {
    if (e instanceof Error && e.message === "Document appears to be empty") throw e;
    throw new Error(`Failed to read Word document: ${e instanceof Error ? e.message : String(e)}`);
  }
}

/** Convert an xlsx ArrayBuffer to markdown tables (one section per sheet). */
async function convertXlsx(arrayBuffer: ArrayBuffer): Promise<string> {
  try {
    const workbook = new ExcelJS.Workbook();
    await workbook.xlsx.load(arrayBuffer);

    const sections: string[] = [];

    workbook.eachSheet((sheet) => {
      const rows: string[][] = [];
      sheet.eachRow((row) => {
        const cells: string[] = [];
        row.eachCell({ includeEmpty: true }, (cell) => {
          cells.push(String(cell.value ?? ""));
        });
        rows.push(cells);
      });

      if (rows.length === 0) return;

      // Normalize column count (pad shorter rows)
      const maxCols = Math.max(...rows.map((r) => r.length));
      const normalized = rows.map((r) => {
        while (r.length < maxCols) r.push("");
        return r;
      });

      // Build markdown table
      const header = "| " + normalized[0].map((c) => c.replace(/\|/g, "\\|")).join(" | ") + " |";
      const separator = "| " + normalized[0].map(() => "---").join(" | ") + " |";
      const body = normalized
        .slice(1)
        .map((row) => "| " + row.map((c) => c.replace(/\|/g, "\\|")).join(" | ") + " |")
        .join("\n");

      const table = [header, separator, body].filter(Boolean).join("\n");
      sections.push(`## Sheet: ${sheet.name}\n\n${table}`);
    });

    if (sections.length === 0) {
      throw new Error("Spreadsheet appears to be empty");
    }

    return sections.join("\n\n");
  } catch (e) {
    if (
      e instanceof Error &&
      (e.message === "Spreadsheet appears to be empty" ||
        e.message === "Document appears to be empty")
    )
      throw e;
    throw new Error(`Failed to read spreadsheet: ${e instanceof Error ? e.message : String(e)}`);
  }
}
