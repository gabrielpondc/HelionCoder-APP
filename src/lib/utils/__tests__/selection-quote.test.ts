import { describe, expect, it } from "vitest";
import { formatSelectionQuote } from "../selection-quote";

describe("formatSelectionQuote", () => {
  it("turns selected text into a markdown blockquote", () => {
    expect(formatSelectionQuote("hello world")).toBe("> hello world");
  });

  it("preserves multiple selected lines", () => {
    expect(formatSelectionQuote("first\nsecond")).toBe("> first\n> second");
  });

  it("normalizes whitespace and blank lines", () => {
    expect(formatSelectionQuote("  first  \r\n\r\n\r\nsecond\u00a0")).toBe("> first\n>\n> second");
  });

  it("returns an empty string for empty selections", () => {
    expect(formatSelectionQuote(" \n\t ")).toBe("");
  });
});
