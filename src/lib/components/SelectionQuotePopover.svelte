<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n/index.svelte";

  let {
    rootEl,
    disabled = false,
    onAddQuote,
  }: {
    rootEl?: HTMLElement;
    disabled?: boolean;
    onAddQuote: (text: string) => void;
  } = $props();

  type PopoverState = {
    visible: boolean;
    text: string;
    x: number;
    y: number;
  };

  let popover = $state<PopoverState>({ visible: false, text: "", x: 0, y: 0 });
  let updateTimer: ReturnType<typeof setTimeout> | null = null;

  const INTERACTIVE_SELECTOR = [
    "button",
    "input",
    "textarea",
    "select",
    "a",
    "[contenteditable='true']",
    "[role='button']",
    "[role='textbox']",
    "[role='menu']",
    "[role='listbox']",
    ".cm-editor",
    ".xterm",
  ].join(",");

  function hide() {
    popover = { visible: false, text: "", x: 0, y: 0 };
  }

  function elementFromNode(node: Node | null): Element | null {
    if (!node) return null;
    return node.nodeType === Node.ELEMENT_NODE ? (node as Element) : node.parentElement;
  }

  function rangeBelongsToRoot(range: Range, root: HTMLElement): boolean {
    return (
      root.contains(range.commonAncestorContainer) &&
      root.contains(range.startContainer) &&
      root.contains(range.endContainer)
    );
  }

  function isInteractiveRange(range: Range): boolean {
    const start = elementFromNode(range.startContainer);
    const end = elementFromNode(range.endContainer);
    return !!start?.closest(INTERACTIVE_SELECTOR) || !!end?.closest(INTERACTIVE_SELECTOR);
  }

  function rangeRect(range: Range): DOMRect | null {
    const rects = Array.from(range.getClientRects()).filter(
      (rect) => rect.width > 0 && rect.height > 0,
    );
    if (rects.length > 0) return rects[0];
    const rect = range.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0 ? rect : null;
  }

  function clamp(value: number, min: number, max: number) {
    return Math.min(Math.max(value, min), max);
  }

  function updateFromSelection() {
    if (disabled || !rootEl) {
      hide();
      return;
    }

    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0 || selection.isCollapsed) {
      hide();
      return;
    }

    const text = selection
      .toString()
      .replace(/\u00a0/g, " ")
      .trim();
    if (text.length < 2) {
      hide();
      return;
    }

    const range = selection.getRangeAt(0);
    if (!rangeBelongsToRoot(range, rootEl) || isInteractiveRange(range)) {
      hide();
      return;
    }

    const rect = rangeRect(range);
    if (!rect) {
      hide();
      return;
    }

    const viewportPadding = 12;
    const x = clamp(rect.left + rect.width / 2, 110, window.innerWidth - 110);
    const y = clamp(rect.top - 42, viewportPadding, window.innerHeight - 48);
    popover = { visible: true, text, x, y };
  }

  function scheduleSelectionUpdate() {
    if (updateTimer) clearTimeout(updateTimer);
    updateTimer = setTimeout(() => {
      updateTimer = null;
      updateFromSelection();
    }, 0);
  }

  function addSelectionToConversation() {
    const text = popover.text.trim();
    if (!text) return;
    onAddQuote(text);
    window.getSelection()?.removeAllRanges();
    hide();
  }

  onMount(() => {
    document.addEventListener("selectionchange", scheduleSelectionUpdate);
    document.addEventListener("mouseup", scheduleSelectionUpdate);
    document.addEventListener("keyup", scheduleSelectionUpdate);
    window.addEventListener("resize", hide);

    return () => {
      if (updateTimer) clearTimeout(updateTimer);
      document.removeEventListener("selectionchange", scheduleSelectionUpdate);
      document.removeEventListener("mouseup", scheduleSelectionUpdate);
      document.removeEventListener("keyup", scheduleSelectionUpdate);
      window.removeEventListener("resize", hide);
    };
  });

  $effect(() => {
    if (disabled) hide();
  });

  $effect(() => {
    const root = rootEl;
    if (!root) return;
    root.addEventListener("scroll", hide, { passive: true });
    return () => root.removeEventListener("scroll", hide);
  });
</script>

{#if popover.visible}
  <button
    type="button"
    class="fixed z-[70] inline-flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-border/70 bg-background/95 px-3 py-1.5 text-xs font-medium text-foreground shadow-lg shadow-black/10 backdrop-blur-xl transition-colors hover:bg-accent"
    style:left={popover.x + "px"}
    style:top={popover.y + "px"}
    onmousedown={(event) => event.preventDefault()}
    onclick={addSelectionToConversation}
  >
    <svg
      class="h-3.5 w-3.5 text-primary"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M5 12h14" />
      <path d="M12 5v14" />
    </svg>
    {t("chat_addSelectionToConversation")}
  </button>
{/if}
