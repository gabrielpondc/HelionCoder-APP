import { describe, it, expect } from "vitest";
import {
  IMAGE_EXTENSIONS,
  OFFICE_PREVIEW_EXTENSIONS,
  PREVIEWABLE_EXTENSIONS,
  classifyPath,
  getExtension,
  isImage,
  isOfficePreviewable,
  isPreviewable,
} from "../preview-ext";

describe("getExtension", () => {
  it("extracts lowercase extension", () => {
    expect(getExtension("README.md")).toBe("md");
    expect(getExtension("photo.PNG")).toBe("png");
    expect(getExtension("path/to/file.TS")).toBe("ts");
  });
  it("returns empty string for files with no extension", () => {
    expect(getExtension("Makefile")).toBe("makefile");
    expect(getExtension("")).toBe("");
  });
  it("uses only the last dot segment", () => {
    expect(getExtension("archive.tar.gz")).toBe("gz");
  });
});

describe("isPreviewable", () => {
  it("matches markdown extensions", () => {
    expect(isPreviewable("md")).toBe(true);
    expect(isPreviewable("markdown")).toBe(true);
  });
  it("rejects others", () => {
    expect(isPreviewable("ts")).toBe(false);
    expect(isPreviewable("png")).toBe(false);
    expect(isPreviewable("")).toBe(false);
  });
});

describe("isImage", () => {
  it("matches common image extensions", () => {
    for (const ext of ["png", "jpg", "jpeg", "gif", "svg", "webp", "ico", "bmp", "avif"]) {
      expect(isImage(ext)).toBe(true);
    }
  });
  it("rejects non-image", () => {
    expect(isImage("md")).toBe(false);
    expect(isImage("ts")).toBe(false);
    expect(isImage("")).toBe(false);
  });
});

describe("isOfficePreviewable", () => {
  it("matches supported Office preview extensions", () => {
    expect(isOfficePreviewable("docx")).toBe(true);
    expect(isOfficePreviewable("pptx")).toBe(true);
    expect(isOfficePreviewable("xlsx")).toBe(true);
  });
  it("rejects legacy Office and non-Office extensions", () => {
    expect(isOfficePreviewable("doc")).toBe(false);
    expect(isOfficePreviewable("ppt")).toBe(false);
    expect(isOfficePreviewable("xls")).toBe(false);
    expect(isOfficePreviewable("md")).toBe(false);
  });
});

describe("classifyPath", () => {
  it("classifies markdown", () => {
    expect(classifyPath("notes.md")).toBe("markdown");
    expect(classifyPath("foo/bar.markdown")).toBe("markdown");
  });
  it("classifies images", () => {
    expect(classifyPath("a.png")).toBe("image");
    expect(classifyPath("dir/photo.JPEG")).toBe("image");
  });
  it("classifies Office preview files", () => {
    expect(classifyPath("report.docx")).toBe("office");
    expect(classifyPath("deck.PPTX")).toBe("office");
    expect(classifyPath("model.xlsx")).toBe("office");
  });
  it("falls back to text", () => {
    expect(classifyPath("src/main.ts")).toBe("text");
    expect(classifyPath("Makefile")).toBe("text");
    expect(classifyPath("")).toBe("text");
  });
});

describe("constants", () => {
  it("PREVIEWABLE includes md and markdown only", () => {
    expect(PREVIEWABLE_EXTENSIONS.has("md")).toBe(true);
    expect(PREVIEWABLE_EXTENSIONS.has("markdown")).toBe(true);
    expect(PREVIEWABLE_EXTENSIONS.size).toBe(2);
  });
  it("IMAGE has expected entries", () => {
    expect(IMAGE_EXTENSIONS.has("png")).toBe(true);
    expect(IMAGE_EXTENSIONS.has("avif")).toBe(true);
    expect(IMAGE_EXTENSIONS.has("md")).toBe(false);
  });
  it("OFFICE_PREVIEW has expected entries", () => {
    expect(OFFICE_PREVIEW_EXTENSIONS.has("docx")).toBe(true);
    expect(OFFICE_PREVIEW_EXTENSIONS.has("pptx")).toBe(true);
    expect(OFFICE_PREVIEW_EXTENSIONS.has("xlsx")).toBe(true);
  });
});
