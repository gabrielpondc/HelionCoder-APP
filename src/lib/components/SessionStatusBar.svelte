<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import type { TaskRun, McpServerInfo, CliModelInfo } from "$lib/types";
  import type { TurnUsage } from "$lib/stores/types";
  import { dbg } from "$lib/utils/debug";
  import { getCliModels } from "$lib/stores/cli-info.svelte";
  import { t } from "$lib/i18n/index.svelte";
  import { fmtNumber } from "$lib/i18n/format";
  import { truncate, formatTokenCount, formatDuration, formatCostDisplay } from "$lib/utils/format";

  interface WorkspaceTool {
    id: string;
    name: string;
    available?: boolean;
    source?: string | null;
    iconDataUrl?: string | null;
  }

  let {
    run = null,
    agent = "claude",
    model = "",
    cost = 0,
    inputTokens = 0,
    outputTokens = 0,
    cacheReadTokens = 0,
    cacheWriteTokens = 0,
    running = false,
    parentRunId,
    onEndSession,
    onFork,
    onModelChange,
    onNavigateParent,
    onToggleSidebar,
    mcpServers,
    onMcpToggle,
    cliVersion,
    permissionMode,
    fastModeState,
    numTurns,
    durationMs,
    persistedFiles,
    onRewind,
    contextUtilization,
    contextWarningLevel,
    contextWindow,
    cwd = "",
    lastCompactedAt = 0,
    compactCount = 0,
    microcompactCount = 0,
    turnUsages = [],
    activeTaskCount = 0,
    mode = "",
    toolsCount = 0,
    onToolsClick,
    remoteHostName,
    onRename,
    platformModels = [],
    authSourceLabel,
    authSourceCategory,
    verbose = false,
    apiKeySource,
    effort,
    onEffortChange,
    onPreviewToggle,
    previewOpen = false,
    onStatusClick,
    onExportHtml,
    workspaceTools = [],
    onOpenWorkspaceTool,
  }: {
    run?: TaskRun | null;
    agent?: string;
    model?: string;
    cost?: number;
    inputTokens?: number;
    outputTokens?: number;
    cacheReadTokens?: number;
    cacheWriteTokens?: number;
    running?: boolean;
    parentRunId?: string;
    onEndSession?: () => void;
    onFork?: () => void;
    onModelChange?: (model: string) => void;
    onNavigateParent?: () => void;
    onToggleSidebar?: () => void;
    mcpServers?: McpServerInfo[];
    onMcpToggle?: () => void;
    cliVersion?: string;
    permissionMode?: string;
    fastModeState?: string;
    numTurns?: number;
    durationMs?: number;
    persistedFiles?: unknown[];
    onRewind?: () => void;
    contextUtilization?: number;
    contextWarningLevel?: string;
    cwd?: string;
    contextWindow?: number;
    lastCompactedAt?: number;
    compactCount?: number;
    microcompactCount?: number;
    turnUsages?: TurnUsage[];
    activeTaskCount?: number;
    mode?: string;
    toolsCount?: number;
    onToolsClick?: () => void;
    remoteHostName?: string | null;
    onRename?: (name: string) => void;
    platformModels?: CliModelInfo[];
    authSourceLabel?: string;
    authSourceCategory?: string;
    verbose?: boolean;
    apiKeySource?: string;
    effort?: string;
    onEffortChange?: (effort: string) => void;
    onPreviewToggle?: () => void;
    previewOpen?: boolean;
    onStatusClick?: () => void;
    onExportHtml?: () => void;
    workspaceTools?: WorkspaceTool[];
    onOpenWorkspaceTool?: (id: string, source?: string | null) => void;
  } = $props();

  $effect(() => {
    dbg("status", "state", { agent, model, running, runId: run?.id });
  });

  // ── Compact indicator (fades after 8s) ──
  let compactVisible = $state(false);
  let compactTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    if (lastCompactedAt && lastCompactedAt > 0) {
      compactVisible = true;
      clearTimeout(compactTimer);
      compactTimer = setTimeout(() => {
        compactVisible = false;
      }, 8000);
    }
  });

  // ── Overflow actions menu ──
  let actionsMenuOpen = $state(false);
  let actionBtnEl: HTMLButtonElement | undefined = $state();
  let actionMenuEl: HTMLDivElement | undefined = $state();
  let actionMenuStyle = $state("");

  function toggleActionsMenu() {
    actionsMenuOpen = !actionsMenuOpen;
    if (actionsMenuOpen && actionBtnEl) {
      const rect = actionBtnEl.getBoundingClientRect();
      const left = Math.max(12, Math.min(rect.left, window.innerWidth - 312));
      actionMenuStyle = `position:fixed; top:${rect.bottom + 6}px; left:${left}px; z-index:50;`;
      requestAnimationFrame(() => actionMenuEl?.focus());
    }
  }

  function closeActionsMenu() {
    actionsMenuOpen = false;
  }

  function runAction(action?: () => void) {
    closeActionsMenu();
    action?.();
  }

  let cwdShort = $derived.by(() => {
    const val = cwd || run?.cwd || "";
    if (!val || val === "/") return "";
    const home = val
      .replace(/^\/Users\/[^/]+/, "~")
      .replace(/^\/home\/[^/]+/, "~")
      .replace(/^[A-Za-z]:[/\\](?:Users|users)[/\\][^/\\]+/, "~");
    return home.length > 30 ? "..." + home.slice(-27) : home;
  });

  let sessionIdShort = $derived(run?.session_id ? run.session_id.slice(0, 8) : "");
  let sidCopied = $state(false);

  async function copySessionId() {
    if (!run?.session_id) return;
    try {
      await navigator.clipboard.writeText(run.session_id);
      sidCopied = true;
      setTimeout(() => (sidCopied = false), 1500);
    } catch {
      /* ignore */
    }
  }

  // ── Title inline editing ──
  let titleEditing = $state(false);
  let titleEditValue = $state("");
  let titleInputEl: HTMLInputElement | undefined = $state();

  function startTitleEdit() {
    if (!onRename || !run) return;
    titleEditValue = run.name || run.prompt;
    titleEditing = true;
    requestAnimationFrame(() => titleInputEl?.select());
  }

  function commitTitleEdit() {
    titleEditing = false;
    const trimmed = titleEditValue.trim();
    if (trimmed && run && trimmed !== (run.name || run.prompt)) {
      onRename?.(trimmed);
    }
  }

  function cancelTitleEdit() {
    titleEditing = false;
  }

  const formatCost = formatCostDisplay;

  let permissionBadge = $derived.by(() => {
    if (!permissionMode || permissionMode === "default") return null;
    const map: Record<string, { label: string; cls: string }> = {
      acceptEdits: { label: "accept-edits", cls: "bg-blue-500/15 text-blue-400" },
      bypassPermissions: { label: "bypass", cls: "bg-amber-500/15 text-amber-500" },
      plan: { label: "plan", cls: "bg-purple-500/15 text-purple-400" },
      auto: { label: "auto", cls: "bg-teal-500/15 text-teal-400" },
      dontAsk: { label: "no-ask", cls: "bg-red-500/15 text-red-400" },
    };
    return (
      map[permissionMode] ?? { label: permissionMode, cls: "bg-foreground/10 text-foreground/60" }
    );
  });

  // ── Model selector dropdown ──
  // Use platform-specific models when a third-party provider is active
  let models = $derived(platformModels.length > 0 ? platformModels : getCliModels());
  let dropdownOpen = $state(false);
  let focusedModelIdx = $state(-1);
  let modelBtnEl: HTMLButtonElement | undefined = $state();
  let dropdownEl: HTMLDivElement | undefined = $state();
  let dropdownStyle = $state("");
  let workspaceMenuOpen = $state(false);
  let preferredWorkspaceToolId = $state("");
  let workspaceMenuBtnEl: HTMLButtonElement | undefined = $state();
  let workspaceMenuEl: HTMLDivElement | undefined = $state();
  let workspaceMenuStyle = $state("");

  let primaryWorkspaceTool = $derived.by(() => {
    const preferred = workspaceTools.find((tool) => tool.id === preferredWorkspaceToolId);
    return preferred ?? workspaceTools[0] ?? null;
  });

  function positionWorkspaceMenu() {
    if (!workspaceMenuBtnEl) return;
    const rect = workspaceMenuBtnEl.getBoundingClientRect();
    const width = 220;
    const left = Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8));
    workspaceMenuStyle = `position:fixed; top:${rect.bottom + 6}px; left:${left}px; width:${width}px; z-index:60;`;
  }

  function rememberWorkspaceTool(id: string) {
    preferredWorkspaceToolId = id;
    try {
      localStorage.setItem("helion:workspace-tool", id);
    } catch {
      /* ignore */
    }
  }

  function openWorkspaceTool(tool: WorkspaceTool | null) {
    if (!tool || !onOpenWorkspaceTool) return;
    rememberWorkspaceTool(tool.id);
    workspaceMenuOpen = false;
    onOpenWorkspaceTool(tool.id, tool.source ?? null);
  }

  function toggleWorkspaceMenu() {
    workspaceMenuOpen = !workspaceMenuOpen;
    if (workspaceMenuOpen) {
      positionWorkspaceMenu();
      requestAnimationFrame(() => workspaceMenuEl?.focus());
    }
  }

  function toggleModelDropdown() {
    dropdownOpen = !dropdownOpen;
    if (dropdownOpen && modelBtnEl) {
      const rect = modelBtnEl.getBoundingClientRect();
      const spaceBelow = window.innerHeight - rect.bottom;
      if (spaceBelow < 200) {
        dropdownStyle = `position:fixed; bottom:${window.innerHeight - rect.top + 4}px; left:${rect.left}px; z-index:50;`;
      } else {
        dropdownStyle = `position:fixed; top:${rect.bottom + 4}px; left:${rect.left}px; z-index:50;`;
      }
      focusedModelIdx = models.findIndex((m) => m.value === model);
      if (focusedModelIdx < 0) focusedModelIdx = 0;
      requestAnimationFrame(() => dropdownEl?.focus());
    }
  }

  export function openModelDropdown() {
    dropdownOpen = true;
    const anchorEl = modelBtnEl ?? actionBtnEl;
    if (anchorEl) {
      const rect = anchorEl.getBoundingClientRect();
      const spaceBelow = window.innerHeight - rect.bottom;
      if (spaceBelow < 200) {
        dropdownStyle = `position:fixed; bottom:${window.innerHeight - rect.top + 4}px; left:${rect.left}px; z-index:50;`;
      } else {
        dropdownStyle = `position:fixed; top:${rect.bottom + 4}px; left:${rect.left}px; z-index:50;`;
      }
    }
    focusedModelIdx = models.findIndex((m) => m.value === model);
    if (focusedModelIdx < 0) focusedModelIdx = 0;
    requestAnimationFrame(() => dropdownEl?.focus());
  }

  function selectModel(val: string) {
    dropdownOpen = false;
    onModelChange?.(val);
  }

  function handleDropdownKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      focusedModelIdx = Math.min(focusedModelIdx + 1, models.length - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      focusedModelIdx = Math.max(focusedModelIdx - 1, 0);
    } else if (e.key === "Enter" && focusedModelIdx >= 0 && focusedModelIdx < models.length) {
      e.preventDefault();
      dbg("statusbar", "model selected via keyboard", { model: models[focusedModelIdx].value });
      selectModel(models[focusedModelIdx].value);
    } else if (e.key === "Escape") {
      e.preventDefault();
      dropdownOpen = false;
    }
    // Tab: allow focus to leave dropdown; all other keys: stop propagation to prevent global shortcuts
    if (e.key !== "Tab") {
      e.stopPropagation();
    }
  }

  onMount(() => {
    try {
      preferredWorkspaceToolId = localStorage.getItem("helion:workspace-tool") ?? "";
    } catch {
      preferredWorkspaceToolId = "";
    }

    function onDocClick(e: MouseEvent) {
      if (
        dropdownOpen &&
        modelBtnEl &&
        !modelBtnEl.contains(e.target as Node) &&
        dropdownEl &&
        !dropdownEl.contains(e.target as Node)
      ) {
        dropdownOpen = false;
      }
      if (
        actionsMenuOpen &&
        actionBtnEl &&
        !actionBtnEl.contains(e.target as Node) &&
        actionMenuEl &&
        !actionMenuEl.contains(e.target as Node)
      ) {
        actionsMenuOpen = false;
      }
      if (
        workspaceMenuOpen &&
        workspaceMenuBtnEl &&
        !workspaceMenuBtnEl.contains(e.target as Node) &&
        workspaceMenuEl &&
        !workspaceMenuEl.contains(e.target as Node)
      ) {
        workspaceMenuOpen = false;
      }
    }
    function onDocKeydown(e: KeyboardEvent) {
      if (dropdownOpen && e.key === "Escape") {
        dropdownOpen = false;
        e.preventDefault();
        e.stopPropagation(); // Prevent bubble to window → keybindingStore.dispatch → chat:interrupt
      }
      if (actionsMenuOpen && e.key === "Escape") {
        actionsMenuOpen = false;
        e.preventDefault();
        e.stopPropagation();
      }
      if (workspaceMenuOpen && e.key === "Escape") {
        workspaceMenuOpen = false;
        e.preventDefault();
        e.stopPropagation();
      }
    }
    document.addEventListener("mousedown", onDocClick, true);
    document.addEventListener("keydown", onDocKeydown);
    return () => {
      document.removeEventListener("mousedown", onDocClick, true);
      document.removeEventListener("keydown", onDocKeydown);
    };
  });

  // ── End Session confirmation ──
  let confirmingEnd = $state(false);
  let confirmTimer: ReturnType<typeof setTimeout> | undefined;

  function requestEnd() {
    confirmingEnd = true;
    confirmTimer = setTimeout(() => {
      confirmingEnd = false;
    }, 3000);
  }

  function confirmEnd() {
    clearTimeout(confirmTimer);
    confirmingEnd = false;
    onEndSession?.();
  }

  function cancelEnd() {
    clearTimeout(confirmTimer);
    confirmingEnd = false;
  }

  let mcpAggregateStatus = $derived.by(() => {
    if (!mcpServers || mcpServers.length === 0) return "none";
    const hasFailure = mcpServers.some((s) => s.status === "failed" || s.status === "needs-auth");
    const hasPending = mcpServers.some((s) => s.status === "pending");
    const allDisabled = mcpServers.every((s) => s.status === "disabled");
    if (hasFailure) return "error";
    if (hasPending) return "pending";
    if (allDisabled) return "disabled";
    return "ok";
  });

  let mcpDotClass = $derived(
    mcpAggregateStatus === "error"
      ? "bg-destructive"
      : mcpAggregateStatus === "pending"
        ? "bg-amber-500"
        : mcpAggregateStatus === "disabled"
          ? "bg-muted-foreground/30"
          : "bg-emerald-500",
  );

  // Find model info: exact match first, then fuzzy (model ID contains alias)
  let currentModelInfo = $derived.by(() => {
    const exact = models.find((m) => m.value === model);
    if (exact) return exact;
    return models.find((m) => model.includes(m.value) && m.value !== "default");
  });
  // Effort: always collect levels from any model that supports them (for always-visible UI)
  let anyModelEffortLevels = $derived.by(() => {
    const supporting = models.find(
      (m) => m.supportsEffort === true && m.supportedEffortLevels?.length,
    );
    return supporting?.supportedEffortLevels ?? [];
  });
  let effortLevels = $derived(currentModelInfo?.supportedEffortLevels ?? anyModelEffortLevels);
  let effortDisabled = $derived(currentModelInfo?.supportsEffort !== true);

  let modelLabel = $derived.by(() => {
    // Check platform models first, then CLI models
    const all = [...(platformModels ?? []), ...getCliModels()];
    const found = all.find((m) => m.value === model);
    if (found) return found.displayName;
    const fuzzy = all.find((m) => model.includes(m.value) && m.value !== "default");
    if (fuzzy) return fuzzy.displayName;
    return model;
  });

  let titleRaw = $derived(run?.name || run?.prompt || t("statusbar_sessionTitle"));
  let titleText = $derived(truncate(titleRaw, 72));
</script>

{#snippet workspaceToolIcon(id: string)}
  {#if id === "vscode" || id === "vscode-insiders"}
    <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M19.4 4.1 14.9 2.4a1.15 1.15 0 0 0-1.2.26L4.1 11.1a1.2 1.2 0 0 0 0 1.8l9.6 8.44c.33.29.8.39 1.2.25l4.5-1.7c.46-.17.76-.62.76-1.11V5.22c0-.49-.3-.94-.76-1.12Z"
        fill="#007ACC"
      />
      <path d="m14.5 7.25-5.9 4.74 5.9 4.76V7.25Z" fill="#1F9CF0" />
      <path d="M14.5 2.55v18.9l4.9-1.86V4.41L14.5 2.55Z" fill="#0065A9" />
      <path d="M8.6 11.99 4.33 8.74v6.5l4.27-3.25Z" fill="#0E86D4" />
    </svg>
  {:else if id === "sublime"}
    <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <rect x="3.5" y="3.5" width="17" height="17" rx="4" fill="#1F1F1F" />
      <path d="m7.4 7.7 9.2-2.25v4.4l-9.2 2.25V7.7Z" fill="#FFB84D" />
      <path d="m7.4 16.3 9.2 2.25v-4.4L7.4 11.9v4.4Z" fill="#FF8C00" />
      <path d="m7.4 11.9 9.2-2.25v4.5l-9.2 2.25v-4.5Z" fill="#FF9800" />
    </svg>
  {:else if id === "intellij" || id === "webstorm" || id === "pycharm" || id === "goland" || id === "rustrover" || id === "clion" || id === "rider" || id === "phpstorm" || id === "rubymine" || id === "datagrip" || id === "dataspell" || id === "android-studio"}
    <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <rect x="4" y="4" width="16" height="16" rx="3.5" fill="#111827" />
      <path d="M4 8.5 8.5 4H14L4 14V8.5Z" fill="#18d6a6" />
      <path d="M20 9.5 9.5 20H15l5-5V9.5Z" fill="#7c3aed" />
      <path d="M8 15h6" stroke="#fff" stroke-width="1.6" stroke-linecap="round" />
    </svg>
  {:else if id === "terminal"}
    <svg
      class="h-4 w-4"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.9"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <rect x="3" y="5" width="18" height="14" rx="2.5" />
      <path d="m7 9 3 3-3 3" /><path d="M13 15h4" />
    </svg>
  {:else if id === "editor" || id === "finder" || id === "explorer"}
    <svg
      class="h-4 w-4"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.8"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <rect x="4" y="4" width="16" height="16" rx="3" />
      <path d="M8 9h8" /><path d="M8 13h5" /><path d="M8 17h3" />
    </svg>
  {:else if id === "xcode"}
    <svg
      class="h-4 w-4"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.8"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="m14.5 4 5.5 5.5" /><path d="m13 7 4 4" />
      <path d="M4 20 15.5 8.5" /><path d="m6.5 17.5 2 2" />
      <path d="m3 21 3.5-1.5L4.5 17 3 21Z" />
    </svg>
  {:else if id === "windows-terminal" || id === "powershell"}
    <svg
      class="h-4 w-4"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.9"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <rect x="3" y="5" width="18" height="14" rx="3" />
      <path d="m7 9 3 3-3 3" /><path d="M13 15h4" />
    </svg>
  {:else if id === "cursor" || id === "windsurf" || id === "zed" || id === "trae"}
    <svg
      class="h-4 w-4"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.8"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M5 4.5 19 12 5 19.5v-15Z" />
      <path d="m8.5 9.5 5.3 2.5-5.3 2.5" />
    </svg>
  {:else}
    <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <rect x="4" y="4" width="16" height="16" rx="4" stroke="currentColor" stroke-width="1.7" />
      <path d="M8 16 16 8" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
      <path d="M8 8h8v8" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
    </svg>
  {/if}
{/snippet}

{#snippet workspaceToolGlyph(tool: WorkspaceTool)}
  {#if tool.iconDataUrl}
    <img
      class="h-4 w-4 rounded-[3px] object-contain"
      src={tool.iconDataUrl}
      alt=""
      draggable="false"
    />
  {:else}
    {@render workspaceToolIcon(tool.id)}
  {/if}
{/snippet}

<div class="border-b border-border bg-background/95">
  <div class="flex h-11 min-w-0 items-center gap-2 px-4">
    {#if run && onRename && titleEditing}
      <input
        bind:this={titleInputEl}
        bind:value={titleEditValue}
        class="min-w-0 max-w-[520px] flex-1 border-b border-primary bg-transparent px-0.5 text-[15px] font-semibold text-foreground outline-none"
        onkeydown={(e) => {
          if (e.key === "Enter") commitTitleEdit();
          else if (e.key === "Escape") cancelTitleEdit();
        }}
        onblur={commitTitleEdit}
      />
    {:else if run && onRename}
      <button
        class="min-w-0 max-w-[520px] truncate text-left text-[15px] font-semibold text-foreground transition-colors hover:text-primary"
        onclick={startTitleEdit}
        title={titleRaw}
      >
        {titleText}
      </button>
    {:else}
      <h1 class="min-w-0 max-w-[520px] truncate text-[15px] font-semibold text-foreground">
        {titleText}
      </h1>
    {/if}

    <button
      bind:this={actionBtnEl}
      class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      onclick={toggleActionsMenu}
      title={t("statusbar_moreActions")}
      aria-label={t("statusbar_moreActions")}
      aria-expanded={actionsMenuOpen}
    >
      <svg
        class="h-4 w-4"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /><circle
          cx="5"
          cy="12"
          r="1"
        />
      </svg>
    </button>

    <div class="min-w-0 flex-1"></div>

    {#if primaryWorkspaceTool && onOpenWorkspaceTool}
      <div
        class="hidden shrink-0 items-center overflow-hidden rounded-xl border border-border/70 bg-background/85 shadow-[0_10px_28px_-24px_rgba(15,23,42,0.8)] backdrop-blur-xl md:flex dark:bg-background/75"
      >
        <button
          type="button"
          class="flex h-8 w-9 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/45"
          onclick={() => openWorkspaceTool(primaryWorkspaceTool)}
          title={`Open ${primaryWorkspaceTool.name}`}
          aria-label={`Open ${primaryWorkspaceTool.name}`}
        >
          {@render workspaceToolGlyph(primaryWorkspaceTool)}
        </button>
        <button
          bind:this={workspaceMenuBtnEl}
          type="button"
          class="flex h-8 w-7 items-center justify-center border-l border-border/60 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/45"
          onclick={toggleWorkspaceMenu}
          title="选择打开方式"
          aria-label="选择打开方式"
          aria-expanded={workspaceMenuOpen}
        >
          <svg
            class="h-3.5 w-3.5"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="m6 9 6 6 6-6" />
          </svg>
        </button>
      </div>
    {/if}
  </div>
</div>

{#if workspaceMenuOpen && workspaceTools.length > 0}
  <div
    bind:this={workspaceMenuEl}
    tabindex="-1"
    role="menu"
    class="rounded-xl border border-border/70 bg-popover/98 p-1.5 text-popover-foreground shadow-xl outline-none backdrop-blur-xl animate-fade-in"
    style={workspaceMenuStyle}
    onkeydown={(e) => {
      if (e.key === "Escape") {
        workspaceMenuOpen = false;
        e.preventDefault();
        e.stopPropagation();
      }
    }}
  >
    {#each workspaceTools as tool (tool.id)}
      <button
        type="button"
        role="menuitem"
        class="flex h-8 w-full items-center gap-2 rounded-lg px-2.5 text-left text-sm text-foreground transition-colors hover:bg-accent focus-visible:bg-accent focus-visible:outline-none"
        onclick={() => openWorkspaceTool(tool)}
      >
        <span class="flex h-5 w-5 shrink-0 items-center justify-center text-muted-foreground">
          {@render workspaceToolGlyph(tool)}
        </span>
        <span class="min-w-0 flex-1 truncate">{tool.name}</span>
      </button>
    {/each}
  </div>
{/if}

{#if dropdownOpen}
  <div
    bind:this={dropdownEl}
    tabindex="-1"
    role="listbox"
    class="min-w-[560px] w-max rounded-md border bg-background shadow-lg animate-fade-in outline-none"
    style={dropdownStyle}
    onkeydown={handleDropdownKeydown}
  >
    <div class="p-1">
      {#each models as m, i}
        <button
          class="flex w-full items-center gap-2 rounded-sm px-3 py-2 text-xs hover:bg-accent transition-colors {model ===
          m.value
            ? 'bg-accent font-medium'
            : ''} {i === focusedModelIdx ? 'ring-1 ring-primary/50' : ''}"
          onclick={() => selectModel(m.value)}
        >
          {#if model === m.value}
            <svg
              class="h-3 w-3 text-primary shrink-0"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"><path d="M20 6 9 17l-5-5" /></svg
            >
          {:else}
            <span class="w-3 shrink-0"></span>
          {/if}
          <span class="shrink-0 text-foreground">{m.displayName}</span>
          <span class="text-[10px] text-foreground/70 truncate">{m.description}</span>
        </button>
      {/each}
    </div>
    {#if effortLevels.length > 0 && onEffortChange}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        onkeydown={(e) => {
          if (["Enter", " ", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(e.key)) {
            e.stopPropagation();
          }
        }}
      >
        <div class="border-t mx-1 my-1"></div>
        <div class="px-3 py-2">
          <div class="text-[10px] text-muted-foreground mb-1.5">
            {t("effort_label")}{#if effortDisabled}<span class="ml-1 opacity-50"
                >— {currentModelInfo?.displayName ?? model} not supported</span
              >{/if}
          </div>
          <div class="flex gap-1">
            {#each effortLevels as level}
              <button
                class="flex-1 rounded px-2 py-1 text-xs transition-colors
                  {effortDisabled
                  ? 'bg-muted/30 text-muted-foreground/40 cursor-not-allowed'
                  : effort === level
                    ? 'bg-primary text-primary-foreground font-medium'
                    : 'bg-muted/50 text-muted-foreground hover:bg-accent'}"
                disabled={effortDisabled}
                onclick={() => onEffortChange(level)}>{level}</button
              >
            {/each}
          </div>
        </div>
      </div>
    {/if}
  </div>
{/if}

{#if actionsMenuOpen}
  <div
    bind:this={actionMenuEl}
    tabindex="-1"
    role="menu"
    class="w-[300px] overflow-hidden rounded-lg border border-border/80 bg-popover/95 p-1.5 text-xs shadow-xl outline-none backdrop-blur"
    style={actionMenuStyle}
  >
    <div class="px-2 py-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
      {t("statusbar_actions")}
    </div>
    {#if model && onModelChange}
      <button
        role="menuitem"
        class="flex w-full items-center justify-between gap-3 rounded-md px-2 py-1.5 text-left text-foreground/80 transition-colors hover:bg-accent hover:text-foreground"
        onclick={() => {
          closeActionsMenu();
          openModelDropdown();
        }}
      >
        <span>{t("quickAction_model")}</span>
        <span class="min-w-0 truncate text-[10px] text-muted-foreground">{modelLabel}</span>
      </button>
    {/if}
    {#if onPreviewToggle}
      <button
        role="menuitem"
        class="flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left text-foreground/80 transition-colors hover:bg-accent hover:text-foreground"
        onclick={() => runAction(onPreviewToggle)}
      >
        <span>{t("preview_label")}</span>
        <span class="text-[10px] text-muted-foreground"
          >{previewOpen ? t("preview_close") : t("preview_pick")}</span
        >
      </button>
    {/if}
    {#if onExportHtml}
      <button
        role="menuitem"
        class="flex w-full items-center rounded-md px-2 py-1.5 text-left text-foreground/80 transition-colors hover:bg-accent hover:text-foreground"
        onclick={() => runAction(onExportHtml)}
      >
        {t("export_htmlButton")}
      </button>
    {/if}
    {#if onRewind}
      <button
        role="menuitem"
        class="flex w-full items-center rounded-md px-2 py-1.5 text-left text-foreground/80 transition-colors hover:bg-accent hover:text-foreground"
        onclick={() => runAction(onRewind)}
        title={t("statusbar_rewindTitle")}
      >
        {t("statusbar_rewind")}
      </button>
    {/if}
    {#if onFork}
      <button
        role="menuitem"
        class="flex w-full items-center rounded-md px-2 py-1.5 text-left text-foreground/80 transition-colors hover:bg-accent hover:text-foreground"
        onclick={() => runAction(onFork)}
        title={t("statusbar_forkTitle")}
      >
        {t("statusbar_fork")}
      </button>
    {/if}
    {#if parentRunId && onNavigateParent}
      <button
        role="menuitem"
        class="flex w-full items-center rounded-md px-2 py-1.5 text-left text-blue-400/80 transition-colors hover:bg-accent hover:text-blue-400"
        onclick={() => runAction(onNavigateParent)}
      >
        {t("statusbar_viewParent")}
      </button>
    {/if}
    {#if toolsCount > 0 && onToolsClick}
      <button
        role="menuitem"
        class="flex w-full items-center rounded-md px-2 py-1.5 text-left text-foreground/80 transition-colors hover:bg-accent hover:text-foreground"
        onclick={() => runAction(onToolsClick)}
      >
        {t("statusbar_tools", { count: String(toolsCount) })}
      </button>
    {/if}
    {#if mcpServers && mcpServers.length > 0 && onMcpToggle}
      <button
        role="menuitem"
        class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-foreground/80 transition-colors hover:bg-accent hover:text-foreground"
        onclick={() => runAction(onMcpToggle)}
      >
        <span class="inline-block h-1.5 w-1.5 rounded-full {mcpDotClass}"></span>
        <span>{t("statusbar_mcpLabel", { count: String(mcpServers.length) })}</span>
      </button>
    {/if}
    {#if sessionIdShort}
      <button
        role="menuitem"
        class="flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left text-foreground/80 transition-colors hover:bg-accent hover:text-foreground"
        onclick={() => runAction(() => void copySessionId())}
      >
        <span>{t("statusbar_sessionLabel", { id: sessionIdShort })}</span>
        <span class="text-[10px] text-muted-foreground"
          >{sidCopied ? t("statusbar_copied") : t("statusbar_clickToCopy")}</span
        >
      </button>
    {/if}

    <div class="my-1 border-t border-border/70"></div>
    <div class="space-y-1 px-2 py-1 text-[11px] text-muted-foreground">
      {#if cwdShort}
        <div class="flex gap-2">
          <span class="shrink-0 text-foreground/40">{t("statusbar_cwd")}</span>
          <span class="min-w-0 flex-1 truncate text-right" title={cwd || run?.cwd || ""}
            >{cwdShort}</span
          >
        </div>
      {/if}
      {#if cost > 0}
        <div class="flex justify-between gap-2">
          <span class="text-foreground/40">{t("statusbar_cost")}</span>
          <span>{formatCost(cost)}</span>
        </div>
      {/if}
      {#if inputTokens > 0 || outputTokens > 0}
        <div
          class="flex justify-between gap-2"
          title={`${t("statusbar_inputLabel")}: ${fmtNumber(inputTokens)} / ${t("statusbar_outputLabel")}: ${fmtNumber(outputTokens)}${cacheReadTokens ? `\n${t("statusbar_cacheReadLabel")}: ${fmtNumber(cacheReadTokens)}` : ""}${cacheWriteTokens ? `\n${t("statusbar_cacheWriteLabel")}: ${fmtNumber(cacheWriteTokens)}` : ""}`}
        >
          <span class="text-foreground/40">{t("statusbar_tokens")}</span>
          <span
            >{formatTokenCount(inputTokens)} / {formatTokenCount(outputTokens)}
            {t("statusbar_tok")}</span
          >
        </div>
      {/if}
      {#if numTurns && numTurns > 0}
        <div class="flex justify-between gap-2">
          <span class="text-foreground/40">{t("statusbar_turnsTitle")}</span>
          <span>{t("statusbar_turns", { count: String(numTurns) })}</span>
        </div>
      {/if}
      {#if durationMs && durationMs > 0}
        {@const turnDetail = turnUsages
          .filter((tu) => tu.durationMs && tu.durationMs > 0)
          .map((tu) => `T${tu.turnIndex}: ${formatDuration(tu.durationMs!)}`)
          .join(", ")}
        <div
          class="flex justify-between gap-2"
          title={t("statusbar_durationTitle") +
            (turnDetail ? `\n${t("statusbar_durationPerTurn")}: ${turnDetail}` : "")}
        >
          <span class="text-foreground/40">{t("statusbar_duration")}</span>
          <span>{formatDuration(durationMs)}</span>
        </div>
      {/if}
      {#if permissionBadge}
        <div class="flex justify-between gap-2">
          <span class="text-foreground/40">{t("statusbar_permission")}</span>
          <span class="{permissionBadge.cls} rounded px-1.5 py-0.5 text-[10px] font-medium"
            >{permissionBadge.label}</span
          >
        </div>
      {/if}
      {#if fastModeState === "on" || verbose || authSourceLabel || remoteHostName || mode}
        <div class="flex flex-wrap justify-end gap-1 pt-1">
          {#if fastModeState === "on"}
            <span
              class="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-500"
              >{t("statusbar_fastMode")}</span
            >
          {/if}
          {#if verbose}
            <span class="rounded bg-sky-500/15 px-1.5 py-0.5 text-[10px] font-medium text-sky-400"
              >{t("statusbar_verbose")}</span
            >
          {/if}
          {#if authSourceLabel}
            <span class="rounded bg-foreground/10 px-1.5 py-0.5 text-[10px] font-medium"
              >{authSourceLabel}</span
            >
          {/if}
          {#if remoteHostName}
            <span class="rounded bg-blue-500/15 px-1.5 py-0.5 text-[10px] font-medium text-blue-500"
              >{t("statusbar_sshLabel", { name: remoteHostName ?? "" })}</span
            >
          {/if}
          {#if mode}
            <span class="rounded bg-foreground/10 px-1.5 py-0.5 text-[10px] font-medium"
              >{mode}</span
            >
          {/if}
        </div>
      {/if}
      {#if persistedFiles && persistedFiles.length > 0}
        <div class="flex justify-between gap-2">
          <span class="text-foreground/40">{t("statusbar_files")}</span>
          <span>{persistedFiles.length}</span>
        </div>
      {/if}
    </div>
    {#if cliVersion}
      <div class="mt-1 border-t border-border/70 pt-1">
        <button
          role="menuitem"
          class="flex w-full items-center rounded-md px-2 py-1.5 text-left text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          onclick={() => runAction(() => goto("/release-notes"))}
          title={t("statusbar_cliVersionTitle", { version: cliVersion ?? "" })}
        >
          CLI v{cliVersion}
        </button>
      </div>
    {/if}
  </div>
{/if}
