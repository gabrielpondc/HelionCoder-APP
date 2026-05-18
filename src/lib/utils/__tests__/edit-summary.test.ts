import { describe, expect, it } from "vitest";
import { summarizeEditedFiles } from "$lib/utils/edit-summary";
import type { TimelineEntry } from "$lib/types";

function toolEntry(tool: Record<string, unknown>): TimelineEntry {
  return {
    kind: "tool",
    ts: "2026-05-14T00:00:00.000Z",
    tool: {
      tool_name: "Edit",
      tool_use_id: "tool-1",
      status: "success",
      input: {},
      ...tool,
    },
  } as unknown as TimelineEntry;
}

describe("summarizeEditedFiles", () => {
  it("aggregates edits by file and totals additions/deletions", () => {
    const summary = summarizeEditedFiles([
      toolEntry({
        tool_use_id: "edit-1",
        input: {
          file_path: "/repo/src/main.ts",
          old_string: "const a = 1;\n",
          new_string: "const a = 2;\nconst b = 3;\n",
        },
      }),
      toolEntry({
        tool_use_id: "edit-2",
        input: {
          file_path: "/repo/src/main.ts",
          old_string: "const b = 3;\n",
          new_string: "const b = 4;\n",
        },
      }),
    ]);

    expect(summary.totalFiles).toBe(1);
    expect(summary.files[0].path).toBe("/repo/src/main.ts");
    expect(summary.totalAdditions).toBeGreaterThan(0);
    expect(summary.totalDeletions).toBeGreaterThan(0);
    expect(summary.hasChanges).toBe(true);
  });

  it("tracks the active running edit file", () => {
    const summary = summarizeEditedFiles([
      toolEntry({
        tool_use_id: "write-1",
        tool_name: "Write",
        status: "running",
        input: {
          file_path: "/repo/report.md",
          content: "# Report\n\nDraft",
        },
      }),
    ]);

    expect(summary.isEditing).toBe(true);
    expect(summary.activeFile?.path).toBe("/repo/report.md");
    expect(summary.totalAdditions).toBe(3);
  });
});
