import { structuredPatch } from "diff";
import type { BusToolItem, TimelineEntry } from "$lib/types";

export interface EditedPatchHunk {
  oldStart: number;
  newStart: number;
  lines: string[];
}

export interface EditedFileSummary {
  path: string;
  toolUseId?: string;
  toolName: string;
  status: "running" | "success" | "other";
  additions: number;
  deletions: number;
  hunks: EditedPatchHunk[];
  updatedAtSeq: number;
}

export interface EditedFilesSummary {
  files: EditedFileSummary[];
  activeFile: EditedFileSummary | null;
  totalFiles: number;
  totalAdditions: number;
  totalDeletions: number;
  hasChanges: boolean;
  isEditing: boolean;
}

const EDIT_TOOL_NAMES = new Set(["Edit", "edit_file", "Write", "write_file", "NotebookEdit"]);
const MAX_STORED_PATCH_LINES = 240;

function normalizePath(path: string): string {
  return path.replaceAll("\\", "/").replace(/\/+/g, "/");
}

function filePathFromTool(tool: BusToolItem): string {
  const input = tool.input ?? {};
  return String(input.file_path ?? input.path ?? input.notebook_path ?? "");
}

function countLines(text: string): number {
  if (!text) return 0;
  return text.endsWith("\n") ? text.split("\n").length - 1 : text.split("\n").length;
}

function patchCounts(hunks: EditedPatchHunk[]): { additions: number; deletions: number } {
  let additions = 0;
  let deletions = 0;
  for (const hunk of hunks) {
    for (const line of hunk.lines) {
      if (line.startsWith("+")) additions++;
      else if (line.startsWith("-")) deletions++;
    }
  }
  return { additions, deletions };
}

function clipHunks(hunks: EditedPatchHunk[]): EditedPatchHunk[] {
  let remaining = MAX_STORED_PATCH_LINES;
  const result: EditedPatchHunk[] = [];
  for (const hunk of hunks) {
    if (remaining <= 0) break;
    const lines = hunk.lines.slice(0, remaining);
    result.push({ ...hunk, lines });
    remaining -= lines.length;
  }
  return result;
}

function normalizeStructuredPatch(raw: unknown): EditedPatchHunk[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .map((hunk): EditedPatchHunk | null => {
      if (!hunk || typeof hunk !== "object") return null;
      const obj = hunk as Record<string, unknown>;
      const lines = Array.isArray(obj.lines) ? obj.lines.map(String) : [];
      if (lines.length === 0) return null;
      return {
        oldStart: typeof obj.oldStart === "number" ? obj.oldStart : 1,
        newStart: typeof obj.newStart === "number" ? obj.newStart : 1,
        lines,
      };
    })
    .filter((hunk): hunk is EditedPatchHunk => hunk !== null);
}

function fallbackPatchFromStrings(oldString: string, newString: string): EditedPatchHunk[] {
  const patch = structuredPatch("", "", oldString, newString, "", "", { context: 2 });
  return patch.hunks.map((hunk) => ({
    oldStart: hunk.oldStart,
    newStart: hunk.newStart,
    lines: hunk.lines,
  }));
}

function additionHunkFromContent(content: string): EditedPatchHunk[] {
  const lines = content.split("\n");
  const normalized = content.endsWith("\n") ? lines.slice(0, -1) : lines;
  return [
    {
      oldStart: 1,
      newStart: 1,
      lines: normalized.map((line) => `+${line}`),
    },
  ];
}

function summarizeTool(tool: BusToolItem, seq: number): EditedFileSummary | null {
  if (!EDIT_TOOL_NAMES.has(tool.tool_name)) return null;
  if (tool.status !== "running" && tool.status !== "success") return null;

  const path = filePathFromTool(tool);
  if (!path) return null;

  const result = tool.tool_use_result as Record<string, unknown> | undefined;
  let hunks = normalizeStructuredPatch(result?.structuredPatch);
  let additions =
    typeof result?._patchAdded === "number" ? (result._patchAdded as number) : undefined;
  let deletions =
    typeof result?._patchRemoved === "number" ? (result._patchRemoved as number) : undefined;

  if (hunks.length === 0) {
    const oldString =
      typeof result?.oldString === "string"
        ? result.oldString
        : typeof tool.input?.old_string === "string"
          ? tool.input.old_string
          : undefined;
    const newString =
      typeof result?.newString === "string"
        ? result.newString
        : typeof tool.input?.new_string === "string"
          ? tool.input.new_string
          : typeof tool.input?.new_source === "string"
            ? tool.input.new_source
            : undefined;
    if (oldString !== undefined || newString !== undefined) {
      hunks = fallbackPatchFromStrings(oldString ?? "", newString ?? "");
    } else if (typeof tool.input?.content === "string") {
      hunks = additionHunkFromContent(tool.input.content);
    }
  }

  if (additions === undefined || deletions === undefined) {
    const counts = patchCounts(hunks);
    additions = additions ?? counts.additions;
    deletions = deletions ?? counts.deletions;
  }

  if (additions === 0 && deletions === 0) {
    if (typeof tool.input?.content === "string") additions = countLines(tool.input.content);
    if (typeof tool.input?.old_string === "string") deletions = countLines(tool.input.old_string);
    if (typeof tool.input?.new_string === "string") additions = countLines(tool.input.new_string);
  }

  return {
    path,
    toolUseId: tool.tool_use_id,
    toolName: tool.tool_name,
    status: tool.status === "running" ? "running" : "success",
    additions,
    deletions,
    hunks: clipHunks(hunks),
    updatedAtSeq: seq,
  };
}

function collectTools(timeline: TimelineEntry[]): BusToolItem[] {
  const tools: BusToolItem[] = [];
  function walk(entries: TimelineEntry[]) {
    for (const entry of entries) {
      if (entry.kind !== "tool") continue;
      tools.push(entry.tool);
      if (entry.subTimeline) walk(entry.subTimeline);
    }
  }
  walk(timeline);
  return tools;
}

export function summarizeEditedFiles(timeline: TimelineEntry[]): EditedFilesSummary {
  const byPath = new Map<string, EditedFileSummary>();
  let activeFile: EditedFileSummary | null = null;

  collectTools(timeline).forEach((tool, seq) => {
    const summary = summarizeTool(tool, seq);
    if (!summary) return;
    const key = normalizePath(summary.path);
    const existing = byPath.get(key);
    if (!existing) {
      byPath.set(key, summary);
    } else {
      byPath.set(key, {
        ...summary,
        additions: existing.additions + summary.additions,
        deletions: existing.deletions + summary.deletions,
        hunks: clipHunks([...existing.hunks, ...summary.hunks]),
        status:
          existing.status === "running" || summary.status === "running"
            ? "running"
            : summary.status,
        toolUseId: summary.toolUseId ?? existing.toolUseId,
      });
    }
    if (summary.status === "running") activeFile = summary;
  });

  const files = [...byPath.values()].sort((a, b) => b.updatedAtSeq - a.updatedAtSeq);
  return {
    files,
    activeFile,
    totalFiles: files.length,
    totalAdditions: files.reduce((n, file) => n + file.additions, 0),
    totalDeletions: files.reduce((n, file) => n + file.deletions, 0),
    hasChanges: files.length > 0,
    isEditing: activeFile !== null,
  };
}
