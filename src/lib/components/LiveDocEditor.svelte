<script lang="ts">
  import "$lib/vendor/aieditor/style.css";
  import { onDestroy, onMount } from "svelte";
  import { readTextFile, writeTextFile } from "$lib/api";
  import { currentLocale } from "$lib/i18n/index.svelte";
  import { dbg, dbgWarn } from "$lib/utils/debug";
  import { createLiveDocAiConfig } from "$lib/utils/live-doc-ai";

  type AiEditorInstance = import("$lib/vendor/aieditor").AiEditor;

  const AUTOSAVE_KEY = "ocv:live-docs:draft";
  const AUTOSAVE_PATH_KEY = "ocv:live-docs:path";

  let editorHost: HTMLDivElement | undefined = $state();
  let editor: AiEditorInstance | null = null;
  let editorReady = $state(false);
  let docPath = $state("");
  let fileName = $state("Untitled.md");
  let dirty = $state(false);
  let saving = $state(false);
  let loading = $state(false);
  let statusText = $state("");
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let suppressChange = false;
  let currentMarkdown = "";
  let originalMarkdown = "";
  let themeObserver: MutationObserver | null = null;
  let destroyed = false;

  function isZh(): boolean {
    return currentLocale().startsWith("zh");
  }

  function defaultMarkdown(): string {
    return isZh() ? "# 未命名文档\n\n" : "# Untitled document\n\n";
  }

  function isDark(): boolean {
    return document.documentElement.classList.contains("dark");
  }

  function pathFileName(path: string): string {
    return path.split(/[/\\]/).filter(Boolean).at(-1) || "Untitled.md";
  }

  function setEditorMarkdown(markdown: string, path = docPath) {
    currentMarkdown = markdown;
    originalMarkdown = markdown;
    dirty = false;
    docPath = path;
    fileName = path ? pathFileName(path) : "Untitled.md";
    suppressChange = true;
    editor?.setMarkdownContent(markdown);
    queueMicrotask(() => {
      suppressChange = false;
    });
  }

  function rememberDraft() {
    localStorage.setItem(AUTOSAVE_KEY, currentMarkdown);
    if (docPath) localStorage.setItem(AUTOSAVE_PATH_KEY, docPath);
    else localStorage.removeItem(AUTOSAVE_PATH_KEY);
  }

  function scheduleDraftSave() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(rememberDraft, 350);
  }

  function handleEditorChange(nextEditor: AiEditorInstance) {
    if (suppressChange) return;
    currentMarkdown = String(nextEditor.getMarkdown() ?? "");
    dirty = currentMarkdown !== originalMarkdown;
    statusText = isZh() ? "草稿已更新" : "Draft updated";
    scheduleDraftSave();
  }

  async function openDocument() {
    loading = true;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Markdown", extensions: ["md", "markdown", "txt"] }],
      });
      if (!selected || Array.isArray(selected)) return;
      const content = await readTextFile(selected);
      setEditorMarkdown(content, selected);
      rememberDraft();
      statusText = isZh() ? "已打开" : "Opened";
    } catch (e) {
      dbgWarn("live-docs", "open failed", e);
      statusText = String(e);
    } finally {
      loading = false;
    }
  }

  async function saveDocument(forcePick = false) {
    if (!editor || saving) return;
    saving = true;
    try {
      let target = docPath;
      if (!target || forcePick) {
        const { save } = await import("@tauri-apps/plugin-dialog");
        const projectCwd = localStorage.getItem("ocv:project-cwd") || undefined;
        const selected = await save({
          defaultPath: projectCwd ? `${projectCwd}/Untitled.md` : "Untitled.md",
          filters: [{ name: "Markdown", extensions: ["md"] }],
        });
        if (!selected) return;
        target = selected;
      }
      currentMarkdown = String(editor.getMarkdown() ?? "");
      await writeTextFile(target, currentMarkdown);
      originalMarkdown = currentMarkdown;
      docPath = target;
      fileName = pathFileName(target);
      dirty = false;
      rememberDraft();
      statusText = isZh() ? "已保存" : "Saved";
    } catch (e) {
      dbgWarn("live-docs", "save failed", e);
      statusText = String(e);
    } finally {
      saving = false;
    }
  }

  function newDocument() {
    if (dirty && !confirm(isZh() ? "放弃当前未保存修改？" : "Discard unsaved changes?")) return;
    setEditorMarkdown(defaultMarkdown(), "");
    rememberDraft();
    statusText = isZh() ? "新文档" : "New document";
  }

  function cleanupAiEditorOrphans() {
    if (typeof document === "undefined") return;
    document.querySelectorAll(".aie-container").forEach((node) => {
      if (!node.closest("[data-live-doc-editor]")) node.remove();
    });
  }

  onMount(async () => {
    if (!editorHost) return;
    destroyed = false;
    cleanupAiEditorOrphans();
    editorHost.replaceChildren();
    const { AiEditor } = await import("$lib/vendor/aieditor");
    if (destroyed || !editorHost) return;
    const cached = localStorage.getItem(AUTOSAVE_KEY) || defaultMarkdown();
    const cachedPath = localStorage.getItem(AUTOSAVE_PATH_KEY) || "";
    currentMarkdown = cached;
    originalMarkdown = cached;
    docPath = cachedPath;
    fileName = cachedPath ? pathFileName(cachedPath) : "Untitled.md";

    editor = new AiEditor({
      element: editorHost,
      content: cached,
      contentIsMarkdown: true,
      lang: isZh() ? "zh" : "en",
      theme: isDark() ? "dark" : "light",
      placeholder: isZh() ? "开始写文档..." : "Start writing...",
      toolbarExcludeKeys: ["video", "attachment"],
      toolbarSize: "small",
      ai: createLiveDocAiConfig(isZh()),
      onCreated: () => {
        editorReady = true;
        statusText = isZh() ? "就绪" : "Ready";
      },
      onChange: handleEditorChange,
    });

    themeObserver = new MutationObserver(() => {
      editor?.changeTheme(isDark() ? "dark" : "light");
    });
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });
    dbg("live-docs", "editor mounted");
  });

  onDestroy(() => {
    destroyed = true;
    if (saveTimer) clearTimeout(saveTimer);
    themeObserver?.disconnect();
    try {
      editor?.destroy();
    } finally {
      editor = null;
      editorHost?.replaceChildren();
      cleanupAiEditorOrphans();
    }
  });
</script>

<div class="flex h-full flex-col overflow-hidden bg-background" data-live-doc-editor>
  <div class="flex min-h-12 shrink-0 items-center gap-2 border-b bg-background px-4">
    <div class="min-w-0 flex-1">
      <div class="flex items-center gap-2">
        <span class="truncate text-sm font-semibold text-foreground">{fileName}</span>
        {#if dirty}
          <span class="h-2 w-2 shrink-0 rounded-full bg-amber-400"></span>
        {/if}
      </div>
      {#if docPath}
        <div class="truncate text-[11px] text-muted-foreground">{docPath}</div>
      {/if}
    </div>
    {#if statusText}
      <span class="hidden text-[11px] text-muted-foreground sm:inline">{statusText}</span>
    {/if}
    <button
      class="rounded-md px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      onclick={newDocument}
    >
      {isZh() ? "新建" : "New"}
    </button>
    <button
      class="rounded-md px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-50"
      disabled={loading}
      onclick={openDocument}
    >
      {loading ? (isZh() ? "打开中" : "Opening") : isZh() ? "打开" : "Open"}
    </button>
    <button
      class="rounded-md px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      onclick={() => saveDocument(true)}
    >
      {isZh() ? "另存为" : "Save As"}
    </button>
    <button
      class="rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50"
      disabled={!editorReady || saving || !dirty}
      onclick={() => saveDocument(false)}
    >
      {saving ? (isZh() ? "保存中" : "Saving") : isZh() ? "保存" : "Save"}
    </button>
  </div>

  <div class="live-docs-editor min-h-0 flex-1 overflow-hidden p-3">
    <div
      bind:this={editorHost}
      class="h-full overflow-hidden rounded-lg border bg-background"
    ></div>
  </div>
</div>

<style>
  .live-docs-editor :global(.aie-container) {
    height: 100%;
    border: 0;
    border-radius: 8px;
    overflow: hidden;
    background: hsl(var(--background));
  }

  .live-docs-editor :global(.aie-container *),
  .live-docs-editor :global(.aie-container *::before),
  .live-docs-editor :global(.aie-container *::after) {
    box-sizing: border-box;
  }

  .live-docs-editor :global(.aie-content) {
    min-height: 100%;
    padding: 28px min(8vw, 72px);
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, sans-serif;
  }

  .live-docs-editor :global(.aie-container aie-header) {
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, sans-serif;
    font-size: 13px;
    font-weight: 500;
    line-height: 1;
  }

  .live-docs-editor :global(.aie-container aie-header > div) {
    min-height: 36px;
    align-items: center;
    gap: 1px;
  }

  .live-docs-editor :global(.aie-container aie-header .aie-menu-item) {
    min-height: 30px;
    padding: 3px 1px !important;
  }

  .live-docs-editor :global(.aie-container aie-header .aie-menu-item > div) {
    height: 18px !important;
    min-width: 18px;
    padding: 4px !important;
    border-radius: 5px;
  }

  .live-docs-editor :global(.aie-container aie-header .aie-menu-item #text) {
    font-size: 13px !important;
    font-weight: 500;
    letter-spacing: 0;
    line-height: 18px;
  }

  .live-docs-editor :global(.aie-container aie-header .aie-menu-item svg) {
    width: 17px !important;
    height: 17px !important;
  }

  .live-docs-editor :global(.aie-container aie-header .aie-menu-divider) {
    height: 15px;
    margin: auto 5px;
  }

  .live-docs-editor :global(.aie-container .aie-dropdown-container .aie-dropdown-item .text) {
    font-size: 13px !important;
    font-weight: 500;
    line-height: 18px;
  }

  .live-docs-editor :global(.aie-container aie-header input[type="file"]),
  .live-docs-editor :global(.aie-container .aie-upload-input) {
    position: absolute !important;
    display: none !important;
    width: 0 !important;
    height: 0 !important;
    padding: 0 !important;
    margin: 0 !important;
    overflow: hidden !important;
    appearance: none !important;
    opacity: 0 !important;
    pointer-events: none !important;
  }

  .live-docs-editor :global(.aie-container aie-header .aie-menu-item .aie-helion-ai-menu) {
    width: 30px !important;
    height: 22px !important;
    gap: 1px !important;
    border-radius: 7px !important;
    background: rgb(239 242 255) !important;
    color: rgb(55 48 163) !important;
    box-shadow:
      inset 0 0 0 1px rgb(99 102 241 / 0.18),
      0 1px 2px rgb(15 23 42 / 0.08);
  }

  .live-docs-editor :global(.aie-container .aie-helion-icon-badge) {
    overflow: hidden;
    border-radius: 4px;
    background: rgb(255 255 255 / 0.94);
    box-shadow: inset 0 0 0 1px rgb(99 102 241 / 0.16);
  }

  .live-docs-editor :global(.aie-container .aie-helion-ai-icon) {
    width: 12px !important;
    height: 12px !important;
    max-width: 12px !important;
    max-height: 12px !important;
    display: block !important;
    border: 0 !important;
    object-fit: contain;
  }

  .live-docs-editor :global(.aie-container .aie-helion-ai-chevron svg) {
    width: 9px !important;
    height: 9px !important;
    fill: currentColor !important;
  }

  .live-docs-editor :global(.aie-container .aie-bubble-menu-item#ai) {
    min-width: 30px;
  }

  .live-docs-editor :global(.aie-container .aie-helion-bubble-trigger) {
    width: 32px;
  }

  .live-docs-editor :global(.aie-container .aie-helion-bubble-trigger .aie-helion-icon-badge) {
    width: 18px !important;
    height: 18px !important;
    flex-basis: 18px !important;
  }

  .live-docs-editor :global(.aie-container .aie-helion-bubble-trigger .aie-helion-ai-icon) {
    width: 14px !important;
    height: 14px !important;
    max-width: 14px !important;
    max-height: 14px !important;
  }

  :global(.dark)
    .live-docs-editor
    :global(.aie-container aie-header .aie-menu-item .aie-helion-ai-menu) {
    background: rgb(49 46 129 / 0.58) !important;
    color: rgb(224 231 255) !important;
    box-shadow:
      inset 0 0 0 1px rgb(129 140 248 / 0.34),
      0 1px 2px rgb(0 0 0 / 0.3);
  }

  :global(.dark) .live-docs-editor :global(.aie-container .aie-helion-icon-badge) {
    background: rgb(15 23 42 / 0.92);
    box-shadow: inset 0 0 0 1px rgb(165 180 252 / 0.28);
  }

  :global(body > .aie-container) {
    display: none !important;
  }
</style>
