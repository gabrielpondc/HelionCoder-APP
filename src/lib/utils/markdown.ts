import { Marked, type Token } from "marked";
import { escapeHtml } from "$lib/utils/ansi";
import { perfMark } from "$lib/utils/perf";
import { hljs } from "$lib/utils/hljs-init";
import DOMPurify from "dompurify";
import "highlight.js/styles/github-dark.min.css";

const marked = new Marked();
const codeCopyIcon = `<svg class="code-block-copy-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="9" width="13" height="13" rx="2"></rect><rect x="2" y="2" width="13" height="13" rx="2"></rect></svg>`;

marked.use({
  gfm: true,
  breaks: false,
  renderer: {
    // marked v15: table(token) receives a Token with header[] and rows[][]
    table(token: {
      header: Array<{ tokens: Token[]; align: string | null; header: boolean }>;
      rows: Array<Array<{ tokens: Token[]; align: string | null; header: boolean }>>;
    }) {
      // Build header cells
      let headerCells = "";
      for (const cell of token.header) {
        const content = this.parser.parseInline(cell.tokens);
        const tag = cell.align ? `<th align="${cell.align}">` : "<th>";
        headerCells += `${tag}${content}</th>\n`;
      }
      const headerRow = `<tr>\n${headerCells}</tr>\n`;

      // Build body rows
      let body = "";
      for (const row of token.rows) {
        let rowCells = "";
        for (const cell of row) {
          const content = this.parser.parseInline(cell.tokens);
          const tag = cell.align ? `<td align="${cell.align}">` : "<td>";
          rowCells += `${tag}${content}</td>\n`;
        }
        body += `<tr>\n${rowCells}</tr>\n`;
      }
      if (body) body = `<tbody>${body}</tbody>`;

      return `<div class="table-wrapper"><table><thead>${headerRow}</thead>${body}</table></div>`;
    },
    code({ text, lang }: { text: string; lang?: string }) {
      const language = lang || "";
      let highlighted: string;

      if (language && hljs.getLanguage(language)) {
        try {
          highlighted = hljs.highlight(text, { language }).value;
        } catch {
          highlighted = escapeHtml(text);
        }
      } else {
        // Skip highlightAuto() — it tries all ~190 languages synchronously
        // and can freeze the UI for seconds on large code blocks
        highlighted = escapeHtml(text);
      }

      const displayLang = language || "text";

      return `<div class="code-block"><div class="code-block-header"><span class="code-block-lang">${escapeHtml(displayLang)}</span><button type="button" class="code-block-copy" data-code-copy aria-label="Copy code" title="Copy code">${codeCopyIcon}</button></div><pre><code class="hljs language-${escapeHtml(language)}">${highlighted}</code></pre></div>`;
    },
  },
});

export function renderMarkdown(text: string): string {
  return perfMark(
    "md-render",
    () => {
      const raw = marked.parse(text);
      if (typeof raw !== "string") return "";
      return DOMPurify.sanitize(raw, {
        ADD_ATTR: ["class", "target", "data-code-copy"],
      });
    },
    { chars: text.length, codeFenceCount: text.match(/```/g)?.length ?? 0 },
  );
}
