<script lang="ts">
  import "@xterm/xterm/css/xterm.css";
  import { onMount } from "svelte";
  import { dbg } from "$lib/utils/debug";

  let {
    onResize,
    onReady,
    onData: onDataProp,
    class: className = "",
  }: {
    onResize: (cols: number, rows: number) => void;
    onReady: (cols: number, rows: number) => void;
    onData?: (data: string) => void;
    class?: string;
  } = $props();

  let containerEl: HTMLDivElement | undefined = $state();
  let terminal: import("@xterm/xterm").Terminal | undefined = $state();
  let fitAddon: import("@xterm/addon-fit").FitAddon | undefined = $state();

  function isDarkMode(): boolean {
    return typeof document !== "undefined" && document.documentElement.classList.contains("dark");
  }

  function terminalTheme(dark: boolean) {
    return dark
      ? {
          background: "#0a0a0a",
          foreground: "#e5e5e5",
          cursor: "#e5e5e5",
          cursorAccent: "#0a0a0a",
          selectionBackground: "rgba(255,255,255,0.2)",
          black: "#0a0a0a",
          red: "#ef4444",
          green: "#22c55e",
          yellow: "#eab308",
          blue: "#3b82f6",
          magenta: "#a855f7",
          cyan: "#06b6d4",
          white: "#e5e5e5",
          brightBlack: "#737373",
          brightRed: "#f87171",
          brightGreen: "#4ade80",
          brightYellow: "#facc15",
          brightBlue: "#60a5fa",
          brightMagenta: "#c084fc",
          brightCyan: "#22d3ee",
          brightWhite: "#ffffff",
        }
      : {
          background: "#f8fafc",
          foreground: "#111827",
          cursor: "#111827",
          cursorAccent: "#f8fafc",
          selectionBackground: "rgba(15,23,42,0.16)",
          black: "#111827",
          red: "#dc2626",
          green: "#16a34a",
          yellow: "#ca8a04",
          blue: "#2563eb",
          magenta: "#9333ea",
          cyan: "#0891b2",
          white: "#f8fafc",
          brightBlack: "#6b7280",
          brightRed: "#ef4444",
          brightGreen: "#22c55e",
          brightYellow: "#eab308",
          brightBlue: "#3b82f6",
          brightMagenta: "#a855f7",
          brightCyan: "#06b6d4",
          brightWhite: "#ffffff",
        };
  }

  export function writeData(data: Uint8Array) {
    terminal?.write(data);
  }

  export function writeText(text: string) {
    terminal?.write(text);
  }

  export function clear() {
    terminal?.clear();
  }

  onMount(() => {
    let resizeObserver: ResizeObserver | undefined;
    let resizeTimer: ReturnType<typeof setTimeout> | undefined;
    let themeObserver: MutationObserver | undefined;

    (async () => {
      const { Terminal } = await import("@xterm/xterm");
      const { FitAddon } = await import("@xterm/addon-fit");
      const { WebLinksAddon } = await import("@xterm/addon-web-links");

      if (!containerEl) return;

      const hasInput = !!onDataProp;
      const term = new Terminal({
        disableStdin: !hasInput,
        cursorBlink: hasInput,
        fontSize: 13,
        fontFamily: "'SF Mono', 'Menlo', 'Consolas', monospace",
        theme: terminalTheme(isDarkMode()),
        scrollback: 10000,
        convertEol: true,
        allowProposedApi: true,
      });

      const fit = new FitAddon();
      term.loadAddon(fit);
      term.loadAddon(new WebLinksAddon());

      term.open(containerEl);
      fit.fit();

      // Forward keystrokes to PTY when input is enabled
      if (onDataProp) {
        term.onData((data) => {
          onDataProp(data);
        });
      }

      terminal = term;
      fitAddon = fit;

      themeObserver = new MutationObserver(() => {
        term.options.theme = terminalTheme(isDarkMode());
      });
      themeObserver.observe(document.documentElement, {
        attributes: true,
        attributeFilter: ["class"],
      });

      // Resize observer with debounce
      resizeObserver = new ResizeObserver(() => {
        clearTimeout(resizeTimer);
        resizeTimer = setTimeout(() => {
          if (fitAddon && terminal) {
            fitAddon.fit();
            dbg("xterm", "resize", { cols: terminal.cols, rows: terminal.rows });
            onResize(terminal.cols, terminal.rows);
          }
        }, 100);
      });
      resizeObserver.observe(containerEl);

      // Signal ready
      dbg("xterm", "ready", { cols: term.cols, rows: term.rows });
      onReady(term.cols, term.rows);
    })();

    return () => {
      clearTimeout(resizeTimer);
      themeObserver?.disconnect();
      resizeObserver?.disconnect();
      terminal?.dispose();
    };
  });
</script>

<div
  bind:this={containerEl}
  class="xterm-container {className}"
  style="width: 100%; height: 100%;"
></div>

<style>
  :global(.xterm-container .xterm) {
    height: 100%;
    padding: 4px 8px;
  }
  :global(.xterm-container .xterm-viewport) {
    overflow-y: auto;
  }
  :global(.xterm-container .xterm-viewport::-webkit-scrollbar) {
    width: 6px;
  }
  :global(.xterm-container .xterm-viewport::-webkit-scrollbar-thumb) {
    background: hsl(var(--foreground) / 0.16);
    border-radius: 3px;
  }
  :global(.xterm-container .xterm-viewport::-webkit-scrollbar-thumb:hover) {
    background: hsl(var(--foreground) / 0.28);
  }
</style>
