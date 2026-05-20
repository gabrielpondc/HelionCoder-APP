<script lang="ts">
  import type { HookEvent, ContextSnapshot, SessionInfoData, FileEntry } from "$lib/types";
  import type { TimelineEntry, BusToolItem } from "$lib/types";
  import type { TurnUsage } from "$lib/stores/types";
  import { getToolColor } from "$lib/utils/tool-colors";
  import { truncate, formatTokenCount, formatDuration } from "$lib/utils/format";
  import { getToolDetail as getToolDetailRaw } from "$lib/utils/tool-rendering";
  import { dbg } from "$lib/utils/debug";
  import { t } from "$lib/i18n/index.svelte";
  import ContextHistoryPanel from "$lib/components/ContextHistoryPanel.svelte";
  import FilesPanel from "$lib/components/FilesPanel.svelte";
  import FilePreviewPane from "$lib/components/FilePreviewPane.svelte";
  import { onMount } from "svelte";
  import { fpsCounter, isPerfEnabled } from "$lib/utils/perf";
  import SessionInfoPanel from "$lib/components/SessionInfoPanel.svelte";
  import StatusIcon from "$lib/components/StatusIcon.svelte";
  import {
    extractFilesFromTimeline,
    extractFilesFromHooks,
    extractFilesFromPersisted,
    mergeFileEntries,
  } from "$lib/utils/file-entries";
  import {
    summarizeEditedFiles,
    type EditedFileSummary,
    type EditedFilesSummary,
  } from "$lib/utils/edit-summary";
  import { getExtension, isOfficePreviewable } from "$lib/utils/preview-ext";
  import { extractTaskToolMeta, type TaskToolMeta } from "$lib/utils/tool-rendering";
  import type { TaskNotificationItem } from "$lib/stores/session-store.svelte";

  type PreviewMode = "preview" | "diff";

  const EMPTY_EDITED_SUMMARY: EditedFilesSummary = {
    files: [],
    activeFile: null,
    totalFiles: 0,
    totalAdditions: 0,
    totalDeletions: 0,
    hasChanges: false,
    isEditing: false,
  };

  let {
    timeline = [],
    tools = [],
    turnUsages = [],
    contextHistory = [],
    persistedFiles = [],
    sessionInfo = null,
    collapsed = false,
    onToggle,
    onScrollToTool,
    onScrollToTurn,
    requestedTab = $bindable(null as "tools" | "context" | "files" | "info" | "tasks" | null),
    backgroundTasks = new Map(),
    activeBackgroundTasks = [],
    cwd = "",
    runId = "",
    isRemote = false,
    appMode = "code",
    requestedPreviewPath = $bindable(null as string | null),
    requestedPreviewMode = $bindable(null as PreviewMode | null),
  }: {
    timeline: TimelineEntry[];
    tools: HookEvent[];
    turnUsages?: TurnUsage[];
    contextHistory?: ContextSnapshot[];
    persistedFiles?: unknown[];
    sessionInfo?: SessionInfoData | null;
    collapsed: boolean;
    onToggle: () => void;
    onScrollToTool?: (toolUseId: string) => void;
    onScrollToTurn?: (anchorId: string) => void;
    requestedTab?: "tools" | "context" | "files" | "info" | "tasks" | null;
    backgroundTasks?: Map<string, TaskNotificationItem>;
    activeBackgroundTasks?: TaskNotificationItem[];
    /** Working directory for file preview (typically store.effectiveCwd). */
    cwd?: string;
    /** Run id — when it changes the preview is cleared. */
    runId?: string;
    /** Remote run flag — disables file preview (file APIs are local-only). */
    isRemote?: boolean;
    /** Current app mode. Cowork mode auto-opens generated Office previews. */
    appMode?: string;
    /** External request to open preview for a path (auto-switches to files tab). */
    requestedPreviewPath?: string | null;
    /** External request for preview vs git diff mode. */
    requestedPreviewMode?: PreviewMode | null;
  } = $props();

  // ── Tab state ──
  type SidebarPanel = "tools" | "context" | "files" | "info" | "tasks";
  let activeTab: SidebarPanel = $state("tools");

  // Lazy keep-alive: a tab is mounted on first activation and stays mounted thereafter.
  // Switching back to a previously-opened tab is then visibility-only (no remount).
  // Svelte 5: $state(Set) requires reassignment to trigger reactivity (mutation methods
  // alone won't), mirroring the existing collapsedTurns pattern below.
  let mountedTabs = $state(new Set<SidebarPanel>(["tools"]));
  $effect(() => {
    if (!mountedTabs.has(activeTab)) {
      mountedTabs = new Set(mountedTabs).add(activeTab);
    }
  });

  // Perf: measure tab-switch frame cost. Tracks the gap between activeTab change and the next
  // animation frame — proxy for "how much work was queued by switching".
  // Gated by isPerfEnabled() so non-debug runs don't pay performance.now() + rAF overhead.
  let _prevTab: SidebarPanel | null = null;
  $effect(() => {
    const cur = activeTab;
    const from = _prevTab;
    _prevTab = cur;
    if (from === null || from === cur) return;
    if (!isPerfEnabled()) return;
    const t0 = performance.now();
    requestAnimationFrame(() => {
      const dt = performance.now() - t0;
      if (dt > 1) dbg("perf", "tab-switch", { from, to: cur, ms: +dt.toFixed(2) });
    });
  });

  // ── External tab request ──
  $effect(() => {
    if (requestedTab) {
      activeTab = requestedTab;
      if (collapsed && !pinned && compactMode !== "hidden") {
        lockedExpanded = true;
      }
      requestedTab = null;
    }
  });

  // ── Preview state ──
  let previewPath = $state<string | null>(null);
  let previewMode = $state<PreviewMode>("preview");

  function openPreview(path: string) {
    previewPath = path;
    previewMode = "preview";
  }

  function isOfficePath(path: string): boolean {
    return isOfficePreviewable(getExtension(path));
  }

  function openDiff(path: string) {
    if (!path) return;
    previewPath = path;
    previewMode = "diff";
    activeTab = "files";
  }

  // External preview request → set path + switch tab; consume by setting $bindable to null
  $effect(() => {
    if (requestedPreviewPath) {
      previewPath = requestedPreviewPath;
      previewMode = requestedPreviewMode ?? "preview";
      activeTab = "files";
      if (collapsed && !pinned && compactMode !== "hidden") {
        lockedExpanded = true;
      }
      requestedPreviewPath = null;
      requestedPreviewMode = null;
    } else if (requestedPreviewMode) {
      previewMode = requestedPreviewMode;
      activeTab = "files";
      if (collapsed && !pinned && compactMode !== "hidden") {
        lockedExpanded = true;
      }
      requestedPreviewMode = null;
    }
  });

  // Clear preview when run changes (different session — paths from previous run no longer relevant)
  $effect(() => {
    void runId;
    previewPath = null;
    previewMode = "preview";
  });

  // ── Width state (browser-safe initialization) ──
  // Note: auto-widening on previewPath change was removed because the width change forced
  // chat-main to reflow its thousands of message nodes every time previewPath transitioned,
  // causing perceptible lag. Users drag the handle to adjust (persisted to localStorage).
  // Default bumped from 320 → 420 to give more reasonable starting room for code preview;
  // 320 was too narrow for typical lines. Users can still drag narrower if desired.
  const WIDTH_MIN = 280;
  const WIDTH_MAX = 720;
  const WIDTH_DEFAULT = 420;
  const COMPACT_RAIL_MIN_WIDTH = 760;
  const FLOATING_PANEL_RIGHT_GAP = 12;
  const FLOATING_PANEL_TOP = 72;
  const FLOATING_PANEL_BOTTOM_GAP = 36;
  const FLOATING_PANEL_MAX_HEIGHT = 720;
  const FLOATING_PANEL_HEIGHT_CSS = `min(${FLOATING_PANEL_MAX_HEIGHT}px, calc(100vh - ${
    FLOATING_PANEL_TOP + FLOATING_PANEL_BOTTOM_GAP
  }px))`;
  const PINNED_STORAGE_KEY = "ocv:toolactivity-pinned";
  const EDGE_REVEAL_ZONE_WIDTH = 56;
  const EDGE_HANDLE_WIDTH = 28;
  const FLOATING_PANEL_CLASS =
    "group fixed right-3 top-[72px] z-30 overflow-hidden rounded-[22px] border border-border/70 bg-background shadow-[0_22px_60px_-28px_rgba(15,23,42,0.75)] backdrop-blur transition-[width,height,opacity,transform] duration-200 dark:shadow-[0_22px_60px_-28px_rgba(0,0,0,0.9)]";
  const DOCKED_PANEL_CLASS =
    "group relative z-10 h-full shrink-0 overflow-hidden border-l border-border/70 bg-background transition-[width] duration-200";

  type CompactMode = "rail" | "hidden";

  function clampWidth(v: number): number {
    return Math.max(WIDTH_MIN, Math.min(WIDTH_MAX, v));
  }

  let savedWidth = $state(WIDTH_DEFAULT);
  let compactHostWidth = $state(0);
  let hoverExpanded = $state(false);
  let lockedExpanded = $state(false);
  let edgeHintVisible = $state(false);
  let pinned = $state(false);

  onMount(() => {
    if (typeof window === "undefined") return;
    const stored = window.localStorage.getItem("ocv:toolactivity-width");
    if (stored) {
      const n = parseInt(stored, 10);
      if (Number.isFinite(n)) savedWidth = clampWidth(n);
    }
    pinned = window.localStorage.getItem(PINNED_STORAGE_KEY) === "true";

    const updateCompactVisibility = () => {
      const parentWidth = asideEl?.parentElement?.clientWidth ?? window.innerWidth;
      compactHostWidth = parentWidth;
    };

    updateCompactVisibility();
    const resizeObserver = new ResizeObserver(updateCompactVisibility);
    if (asideEl?.parentElement) resizeObserver.observe(asideEl.parentElement);
    window.addEventListener("resize", updateCompactVisibility);

    return () => {
      resizeObserver.disconnect();
      window.removeEventListener("resize", updateCompactVisibility);
    };
  });

  let effectiveWidth = $derived(clampWidth(savedWidth));
  let panelWidth = $derived(effectiveWidth);
  let compactMode = $derived.by<CompactMode>(() => {
    if (compactHostWidth > 0 && compactHostWidth < COMPACT_RAIL_MIN_WIDTH) return "hidden";
    return "rail";
  });
  let panelOpen = $derived(!collapsed || hoverExpanded || lockedExpanded || pinned);
  let panelVisible = $derived(panelOpen && (pinned || !collapsed || compactMode !== "hidden"));
  let panelWorkEnabled = $derived(panelVisible);
  let canShowEdgeHint = $derived(collapsed && !panelOpen && !pinned && compactMode !== "hidden");
  let showEdgeHint = $derived(canShowEdgeHint && edgeHintVisible);

  // ── Resize handle (VS Code-style: ghost line during drag, single commit on release) ──
  // Why this approach: in-place live resize forces chat-main reflow on every pointermove,
  // which is too expensive with thousands of chat DOM nodes. Instead, during drag we DON'T
  // move any panel — only render a fixed-position vertical line at the cursor that previews
  // the new boundary. On release we commit savedWidth ONCE → single reflow.
  let resizing = $state(false);
  let ghostX = $state(0);
  let resizeStartX = 0;
  let resizeStartWidth = 0;
  let pendingWidth: number | null = null;
  let rafId: number | null = null;
  let asideEl: HTMLElement | undefined = $state();
  let dragFpsStop: (() => void) | null = null;

  function openCollapsedPanel(lock = false) {
    if (collapsed && !pinned && compactMode !== "hidden") {
      edgeHintVisible = false;
      if (lock) lockedExpanded = true;
      hoverExpanded = true;
    }
  }

  function handlePanelEnter() {
    if (collapsed && !pinned && panelVisible) hoverExpanded = true;
  }

  function handlePanelLeave(e: MouseEvent | PointerEvent) {
    if (collapsed && !pinned && !lockedExpanded && !pointerInPanelOrEdge(e.clientX, e.clientY)) {
      hoverExpanded = false;
    }
  }

  function getFloatingPanelRect() {
    if (typeof window === "undefined") return null;
    const height = Math.min(
      FLOATING_PANEL_MAX_HEIGHT,
      Math.max(0, window.innerHeight - FLOATING_PANEL_TOP - FLOATING_PANEL_BOTTOM_GAP),
    );
    const right = window.innerWidth - FLOATING_PANEL_RIGHT_GAP;
    const left = right - panelWidth;
    const top = FLOATING_PANEL_TOP;
    const bottom = top + height;
    return { left, right, top, bottom };
  }

  function pointerInPanelOrEdge(clientX: number, clientY: number): boolean {
    const futurePanel = getFloatingPanelRect();
    if (!futurePanel) return false;
    const inPanelHeight = clientY >= futurePanel.top && clientY <= futurePanel.bottom;
    if (
      inPanelHeight &&
      clientX >= window.innerWidth - EDGE_HANDLE_WIDTH &&
      clientX <= window.innerWidth
    ) {
      return true;
    }
    const rect = asideEl?.getBoundingClientRect();
    if (!rect) return false;
    const inPanel =
      clientX >= rect.left &&
      clientX <= rect.right &&
      clientY >= rect.top &&
      clientY <= rect.bottom;
    const inRightGap =
      clientX >= rect.right &&
      clientX <= window.innerWidth &&
      clientY >= rect.top &&
      clientY <= rect.bottom;
    return inPanel || inRightGap;
  }

  function handleWindowPointerMove(e: PointerEvent) {
    if (!collapsed || pinned || compactMode === "hidden") {
      edgeHintVisible = false;
      return;
    }
    if (lockedExpanded) {
      edgeHintVisible = false;
      return;
    }
    if (hoverExpanded) {
      edgeHintVisible = false;
      if (!pointerInPanelOrEdge(e.clientX, e.clientY)) hoverExpanded = false;
      return;
    }
    const futurePanel = getFloatingPanelRect();
    edgeHintVisible =
      !!futurePanel &&
      e.clientX >= window.innerWidth - EDGE_REVEAL_ZONE_WIDTH &&
      e.clientX <= window.innerWidth &&
      e.clientY >= futurePanel.top &&
      e.clientY <= futurePanel.bottom;
    if (
      edgeHintVisible &&
      e.clientX >= window.innerWidth - EDGE_HANDLE_WIDTH &&
      e.clientX <= window.innerWidth
    ) {
      openCollapsedPanel(false);
    }
  }

  function handleManualToggle() {
    if (collapsed) {
      openCollapsedPanel(true);
      return;
    }
    lockedExpanded = false;
    hoverExpanded = false;
    onToggle();
  }

  function handlePanelToggle() {
    if (pinned) {
      pinned = false;
      if (typeof window !== "undefined") {
        window.localStorage.setItem(PINNED_STORAGE_KEY, "false");
      }
      lockedExpanded = false;
      hoverExpanded = false;
      if (!collapsed) onToggle();
      return;
    }
    if (panelOpen) {
      lockedExpanded = false;
      hoverExpanded = false;
      if (!collapsed) onToggle();
      return;
    }
    handleManualToggle();
  }

  function togglePinned() {
    const next = !pinned;
    pinned = next;
    if (typeof window !== "undefined") {
      window.localStorage.setItem(PINNED_STORAGE_KEY, String(next));
    }
    if (next) {
      lockedExpanded = false;
      hoverExpanded = false;
      if (!collapsed) onToggle();
      return;
    }
    if (compactMode !== "hidden") {
      hoverExpanded = true;
    }
  }

  function onResizeStart(e: PointerEvent) {
    resizing = true;
    resizeStartX = e.clientX;
    resizeStartWidth = panelWidth;
    pendingWidth = resizeStartWidth;
    ghostX = e.clientX;
    (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
    e.preventDefault();
    dragFpsStop = fpsCounter("aside-drag");
  }

  function flushGhostFrame() {
    rafId = null;
    // ghostX state already updated; this is just a frame-aligned re-render gate.
  }

  function onResizeMove(e: PointerEvent) {
    if (!resizing) return;
    const delta = resizeStartX - e.clientX; // dragging left grows the panel
    pendingWidth = clampWidth(resizeStartWidth + delta);
    // Snap ghost line to the new panel boundary (clamped). For docked mode, use the
    // panel's actual right edge so app padding/window chrome don't skew the preview.
    const rightEdge =
      asideEl?.getBoundingClientRect().right ??
      (typeof window !== "undefined" ? window.innerWidth : e.clientX);
    const wantedX = rightEdge - pendingWidth;
    if (rafId === null && typeof window !== "undefined") {
      rafId = window.requestAnimationFrame(flushGhostFrame);
    }
    ghostX = wantedX;
  }

  function onResizeEnd(e: PointerEvent) {
    if (!resizing) return;
    resizing = false;
    if (rafId !== null && typeof window !== "undefined") {
      window.cancelAnimationFrame(rafId);
      rafId = null;
    }
    (e.target as HTMLElement).releasePointerCapture?.(e.pointerId);

    if (pendingWidth !== null && pendingWidth !== savedWidth) {
      savedWidth = pendingWidth;
      if (typeof window !== "undefined") {
        window.localStorage.setItem("ocv:toolactivity-width", String(savedWidth));
      }
    }
    pendingWidth = null;
    dragFpsStop?.();
    dragFpsStop = null;
  }

  // ── Helpers ──

  function getToolDetail(tool: BusToolItem): string {
    return truncate(getToolDetailRaw(tool.input as Record<string, unknown>), 50);
  }

  function getHookDetail(event: HookEvent): string {
    return truncate(getToolDetailRaw(event.tool_input as Record<string, unknown>), 50);
  }

  type StatusCategory = "done" | "running" | "error" | "other";

  function categorizeBusStatus(status: string): StatusCategory {
    switch (status) {
      case "success":
        return "done";
      case "running":
        return "running";
      case "error":
      case "denied":
      case "permission_denied":
        return "error";
      case "ask_pending":
      case "permission_prompt":
        return "other";
      default:
        return "other";
    }
  }

  function categorizeHookStatus(status: string | undefined): StatusCategory {
    if (!status) return "other";
    switch (status) {
      case "done":
      case "success":
        return "done";
      case "running":
      case "pending":
        return "running";
      case "error":
      case "denied":
        return "error";
      default:
        return "other";
    }
  }

  // ── Tree structure for hierarchical tool display ──

  interface ToolNode {
    tool: BusToolItem;
    children: ToolNode[];
  }

  /** Build a tree from TimelineEntries, preserving parent→child hierarchy. */
  function buildToolTree(entries: TimelineEntry[], seen: Set<string>): ToolNode[] {
    const result: ToolNode[] = [];
    for (const entry of entries) {
      if (entry.kind === "tool" && !seen.has(entry.tool.tool_use_id)) {
        seen.add(entry.tool.tool_use_id);
        result.push({
          tool: entry.tool,
          children: entry.subTimeline ? buildToolTree(entry.subTimeline, seen) : [],
        });
      }
    }
    return result;
  }

  /** Flatten tree nodes for counting/statistics. */
  function flattenNodes(nodes: ToolNode[]): BusToolItem[] {
    const result: BusToolItem[] = [];
    for (const node of nodes) {
      result.push(node.tool);
      if (node.children.length > 0) result.push(...flattenNodes(node.children));
    }
    return result;
  }

  /** Recursively count all nodes in a tree. */
  function countToolNodes(nodes: ToolNode[]): number {
    let count = 0;
    for (const node of nodes) count += 1 + countToolNodes(node.children);
    return count;
  }

  // ── Dual-source strategy ──

  // ── Background tasks (sorted: active first, then by recency) ──

  let sortedBgTasks = $derived.by(() => {
    if (!panelWorkEnabled) return [];
    const items = [...backgroundTasks.values()];
    return items.sort((a, b) => {
      const aActive =
        a.status !== "completed" && a.status !== "failed" && a.status !== "error" ? 0 : 1;
      const bActive =
        b.status !== "completed" && b.status !== "failed" && b.status !== "error" ? 0 : 1;
      if (aActive !== bActive) return aActive - bActive;
      return b.startedAt - a.startedAt;
    });
  });

  function bgElapsed(startedAt: number): string {
    const ms = Date.now() - startedAt;
    if (ms < 1000) return "<1s";
    return `${Math.floor(ms / 1000)}s`;
  }

  let fileTrackingEnabled = $derived(panelWorkEnabled || appMode === "cowork");
  let useTimeline = $derived(fileTrackingEnabled && timeline.length > 0);

  // ── Turn grouping (timeline mode) ──

  interface ToolTurn {
    turnIndex: number;
    userPreview: string;
    tools: ToolNode[];
    anchorId?: string;
  }

  let turns = $derived.by(() => {
    if (!useTimeline) return [];
    const result: ToolTurn[] = [];
    let currentTools: ToolNode[] = [];
    let currentPreview = "";
    let currentAnchorId: string | undefined;
    let turnIdx = 0;
    // Defensive dedup: CLI can emit events with missing parent_tool_use_id,
    // causing the same tool_use_id to appear in both main timeline and a subTimeline.
    // Track seen IDs to prevent each_key_duplicate crashes in {#each} blocks.
    const seen = new Set<string>();

    for (const entry of timeline) {
      if (entry.kind === "separator") continue;
      if (entry.kind === "user") {
        // Flush previous turn (guard: don't flush initial empty state)
        if (currentTools.length > 0 || currentPreview || currentAnchorId) {
          result.push({
            turnIndex: turnIdx,
            userPreview: currentPreview,
            tools: currentTools,
            anchorId: currentAnchorId,
          });
        }
        turnIdx++;
        currentPreview = entry.content.slice(0, 40);
        currentAnchorId = entry.anchorId;
        currentTools = [];
      } else if (entry.kind === "tool") {
        if (!seen.has(entry.tool.tool_use_id)) {
          seen.add(entry.tool.tool_use_id);
          currentTools.push({
            tool: entry.tool,
            children: entry.subTimeline ? buildToolTree(entry.subTimeline, seen) : [],
          });
        }
      }
    }
    // Flush last turn
    if (currentTools.length > 0 || currentPreview || currentAnchorId) {
      result.push({
        turnIndex: turnIdx,
        userPreview: currentPreview,
        tools: currentTools,
        anchorId: currentAnchorId,
      });
    }
    return result;
  });

  // ── HookEvent fallback (pipe/PTY mode) ──

  let hookToolEvents = $derived(fileTrackingEnabled ? tools.filter((e) => e.tool_name) : []);

  // ── File entries (dual-source + persisted merge) ──

  let fileEntries: FileEntry[] = $derived.by(() => {
    if (!fileTrackingEnabled) return [];
    const timelineFiles = useTimeline
      ? extractFilesFromTimeline(timeline)
      : extractFilesFromHooks(hookToolEvents);
    const persistedEntries = extractFilesFromPersisted(persistedFiles ?? []);
    return mergeFileEntries(
      { entries: timelineFiles, hasTemporalOrder: true },
      { entries: persistedEntries, hasTemporalOrder: false },
    );
  });

  let lastAutoOfficePreviewKey = "";
  $effect(() => {
    if (appMode !== "cowork" || isRemote) return;
    const officeEntry = fileEntries.find(
      (entry) => entry.action !== "read" && isOfficePath(entry.path),
    );
    if (!officeEntry) return;
    const key = `${runId}:${officeEntry.path}:${officeEntry.action}:${fileEntries.length}`;
    if (key === lastAutoOfficePreviewKey) return;
    lastAutoOfficePreviewKey = key;
    previewPath = officeEntry.path;
    previewMode = "preview";
    activeTab = "files";
    if (collapsed && !pinned && compactMode !== "hidden") {
      lockedExpanded = true;
    }
  });
  let editedSummary = $derived(
    panelWorkEnabled ? summarizeEditedFiles(timeline) : EMPTY_EDITED_SUMMARY,
  );

  function shortPath(path: string): string {
    const normalized = path.replaceAll("\\", "/");
    const parts = normalized.split("/").filter(Boolean);
    return parts.length > 2 ? ".../" + parts.slice(-2).join("/") : normalized;
  }

  function patchLineClass(line: string): string {
    if (line.startsWith("+")) return "text-emerald-500";
    if (line.startsWith("-")) return "text-red-400";
    if (line.startsWith("@")) return "text-blue-400";
    return "text-muted-foreground";
  }

  function patchPreviewLines(file: EditedFileSummary): string[] {
    const firstHunk = file.hunks[0];
    if (!firstHunk) return [];
    return firstHunk.lines.slice(0, 6);
  }

  // ── Subagent extraction (for info tab) ──

  interface SubagentInfo {
    toolUseId: string;
    meta: TaskToolMeta;
    status: string;
    durationMs?: number;
    toolCount: number;
  }

  let subagents: SubagentInfo[] = $derived.by(() => {
    if (!panelWorkEnabled || !useTimeline) return [];
    const result: SubagentInfo[] = [];
    for (const turn of turns) {
      for (const node of flattenNodes(turn.tools)) {
        if (node.tool_name === "Task") {
          const meta = extractTaskToolMeta(node.input);
          if (!meta) continue;
          // Count nested tools from the result
          let toolCount = 0;
          let durationMs: number | undefined;
          const tur = node.tool_use_result as Record<string, unknown> | undefined;
          if (tur && typeof tur === "object") {
            if ("totalToolUseCount" in tur) toolCount = tur.totalToolUseCount as number;
            if ("totalDurationMs" in tur) durationMs = tur.totalDurationMs as number;
          }
          result.push({
            toolUseId: node.tool_use_id,
            meta,
            status: node.status,
            durationMs,
            toolCount,
          });
        }
      }
    }
    return result;
  });

  // ── Summary + status counts (single-pass) ──

  let toolStats = $derived.by(() => {
    if (!panelWorkEnabled) {
      return {
        summary: [],
        doneCount: 0,
        runningCount: 0,
        errorCount: 0,
        totalToolCount: 0,
      };
    }
    const counts: Record<string, number> = {};
    let done = 0,
      running = 0,
      errors = 0,
      total = 0;
    if (useTimeline) {
      for (const turn of turns) {
        for (const t of flattenNodes(turn.tools)) {
          counts[t.tool_name] = (counts[t.tool_name] ?? 0) + 1;
          total++;
          const cat = categorizeBusStatus(t.status);
          if (cat === "done") done++;
          else if (cat === "running") running++;
          else if (cat === "error") errors++;
        }
      }
    } else {
      for (const ev of hookToolEvents) {
        const name = ev.tool_name ?? "other";
        counts[name] = (counts[name] ?? 0) + 1;
        total++;
        const cat = categorizeHookStatus(ev.status);
        if (cat === "done") done++;
        else if (cat === "running") running++;
        else if (cat === "error") errors++;
      }
    }
    return {
      summary: Object.entries(counts).sort((a, b) => b[1] - a[1]),
      doneCount: done,
      runningCount: running,
      errorCount: errors,
      totalToolCount: total,
    };
  });

  // ── Per-turn usage lookup ──

  let usageByTurn = $derived(new Map(turnUsages.map((tu) => [tu.turnIndex, tu])));

  // ── Collapsible turn state ──
  // Default: collapse all turns except the latest to reduce initial DOM count

  let collapsedTurns = $state(new Set<number>());

  // Auto-collapse older turns when turn count changes (session load / new turn)
  let prevTurnCount = 0;
  $effect(() => {
    const count = turns.length;
    if (count !== prevTurnCount && count > 1) {
      const collapsed = new Set<number>();
      for (const turn of turns) {
        // Collapse all except the last turn
        if (turn !== turns[turns.length - 1]) {
          collapsed.add(turn.turnIndex);
        }
      }
      collapsedTurns = collapsed;
    }
    prevTurnCount = count;
  });

  function toggleTurn(turnIndex: number) {
    if (collapsedTurns.has(turnIndex)) {
      collapsedTurns.delete(turnIndex);
    } else {
      collapsedTurns.add(turnIndex);
    }
    collapsedTurns = new Set(collapsedTurns);
  }

  $effect(() => {
    dbg("tools", "sidebar updated", {
      useTimeline,
      turns: turns.length,
      hookTools: hookToolEvents.length,
      total: toolStats.totalToolCount,
      files: fileEntries.length,
    });
  });
</script>

{#snippet statusIcon(category: StatusCategory)}
  <StatusIcon status={category} size="sm" />
{/snippet}

{#snippet pinGlyph()}
  <svg
    class="h-3.5 w-3.5 transition-transform {pinned ? '-rotate-45' : ''}"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path d="M12 17v5" />
    <path d="M5 17h14" />
    <path d="M9 3h6l1 7 3 3v4H5v-4l3-3 1-7Z" />
  </svg>
{/snippet}

{#snippet toolNodeView(node: ToolNode)}
  {@const style = getToolColor(node.tool.tool_name)}
  {@const detail = getToolDetail(node.tool)}
  {@const cat = categorizeBusStatus(node.tool.status)}
  <button
    class="w-full text-left px-2.5 py-1 hover:bg-accent/50 rounded-sm transition-colors group"
    onclick={() => onScrollToTool?.(node.tool.tool_use_id)}
    title={t("toolActivity_scrollToTool")}
  >
    <div class="flex items-center gap-1.5">
      {@render statusIcon(cat)}
      <div class="flex h-4 w-4 shrink-0 items-center justify-center rounded {style.bg}">
        <svg
          class="h-2.5 w-2.5 {style.text}"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d={style.icon} />
        </svg>
      </div>
      <span class="text-[11px] font-medium text-foreground shrink-0">{node.tool.tool_name}</span>
      {#if detail}
        <span
          class="text-[10px] text-muted-foreground truncate min-w-0 opacity-70 group-hover:opacity-100"
          >{detail}</span
        >
      {/if}
    </div>
  </button>
  {#if node.children.length > 0}
    <div class="ml-5 border-l-2 border-cyan-500/25">
      {#each node.children as child (child.tool.tool_use_id)}
        {@render toolNodeView(child)}
      {/each}
    </div>
  {/if}
{/snippet}

<svelte:window
  onpointermove={handleWindowPointerMove}
  onpointerleave={() => (edgeHintVisible = false)}
/>

<!--
  Expanded panel stays mounted while collapsed so heavyweight previews are not torn down.
  Unpinned collapsed mode is fully hidden until the future panel edge opens it.
-->
{#if resizing}
  <!-- Ghost line during drag: zero-cost preview, no layout reflow elsewhere -->
  <div
    class="fixed top-0 bottom-0 z-[9999] pointer-events-none bg-primary"
    style="left: {ghostX - 1}px; width: 3px; box-shadow: 0 0 8px hsl(var(--primary) / 0.6);"
  ></div>
{/if}
{#if showEdgeHint}
  <button
    type="button"
    class="fixed z-40 flex cursor-pointer items-center justify-end bg-transparent"
    style="top: {FLOATING_PANEL_TOP}px; height: {FLOATING_PANEL_HEIGHT_CSS}; right: 0; width: {EDGE_HANDLE_WIDTH}px"
    title={t("toolActivity_show")}
    aria-label={t("toolActivity_show")}
    onmouseenter={() => openCollapsedPanel(false)}
    onpointerenter={() => openCollapsedPanel(false)}
    onclick={() => openCollapsedPanel(true)}
  >
    <span
      class="h-full w-[2px] rounded-full bg-muted-foreground/45 shadow-[0_0_14px_rgba(15,23,42,0.12)] transition-colors hover:bg-primary/70 dark:shadow-[0_0_16px_rgba(255,255,255,0.08)]"
    ></span>
  </button>
{/if}
<aside
  bind:this={asideEl}
  class={pinned ? DOCKED_PANEL_CLASS : FLOATING_PANEL_CLASS}
  style="width: {panelWidth}px; height: {pinned
    ? '100%'
    : FLOATING_PANEL_HEIGHT_CSS}; opacity: {panelVisible ? 1 : 0}; transform: {pinned
    ? 'none'
    : `translateX(${panelVisible ? '0' : 'calc(100% + 18px)'})`}; pointer-events: {panelVisible
    ? 'auto'
    : 'none'}; contain: layout style;"
  data-tool-activity-state={pinned
    ? "pinned"
    : collapsed
      ? panelOpen
        ? "hover-open"
        : compactMode
      : "open"}
  onmouseenter={handlePanelEnter}
  onmouseleave={handlePanelLeave}
  onpointerenter={handlePanelEnter}
  onpointerleave={handlePanelLeave}
>
  <!-- Always-mounted expanded panel. Use opacity instead of visibility because child tabs set
    their own visibility and can otherwise bleed through the compact card while collapsed. -->
  <div
    class="absolute top-0 left-0 h-full flex flex-col transition-opacity duration-150"
    style="width: {panelWidth}px; opacity: {panelVisible ? 1 : 0}; pointer-events: {panelVisible
      ? 'auto'
      : 'none'};"
    aria-hidden={!panelVisible}
  >
    <!-- Resize handle on the left edge -->
    <div
      role="separator"
      aria-orientation="vertical"
      tabindex="-1"
      class="absolute left-0 top-0 h-full w-1 cursor-col-resize hover:bg-primary/30 active:bg-primary/50 z-20 {resizing
        ? 'bg-primary/50'
        : ''}"
      onpointerdown={onResizeStart}
      onpointermove={onResizeMove}
      onpointerup={onResizeEnd}
      onpointercancel={onResizeEnd}
    ></div>
    <!-- Header: 4 icon tabs -->
    <div class="border-b border-border bg-background/95 px-3 py-2 backdrop-blur">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-1">
          <!-- Tools icon -->
          <button
            class="flex h-8 w-8 items-center justify-center rounded-md transition-colors {activeTab ===
            'tools'
              ? 'bg-primary/10 text-primary ring-1 ring-primary/15'
              : 'text-muted-foreground hover:text-foreground hover:bg-accent'}"
            onclick={() => (activeTab = "tools")}
            title={t("toolActivity_tabTools")}
          >
            <svg
              class="h-3.5 w-3.5"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path
                d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"
              />
            </svg>
          </button>
          <!-- Context icon -->
          <button
            class="relative flex h-8 w-8 items-center justify-center rounded-md transition-colors {activeTab ===
            'context'
              ? 'bg-primary/10 text-primary ring-1 ring-primary/15'
              : 'text-muted-foreground hover:text-foreground hover:bg-accent'}"
            onclick={() => (activeTab = "context")}
            title={t("toolActivity_tabContext")}
          >
            <svg
              class="h-3.5 w-3.5"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <circle cx="12" cy="12" r="10" />
              <polyline points="12 6 12 12 16 14" />
            </svg>
            {#if contextHistory.length > 0}
              <span class="absolute top-0.5 right-0.5 h-1.5 w-1.5 rounded-full bg-emerald-500"
              ></span>
            {/if}
          </button>
          <!-- Files icon -->
          <button
            class="relative flex h-8 w-8 items-center justify-center rounded-md transition-colors {activeTab ===
            'files'
              ? 'bg-primary/10 text-primary ring-1 ring-primary/15'
              : 'text-muted-foreground hover:text-foreground hover:bg-accent'}"
            onclick={() => (activeTab = "files")}
            title={t("toolActivity_tabFiles")}
          >
            <svg
              class="h-3.5 w-3.5"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
              <polyline points="14 2 14 8 20 8" />
            </svg>
            {#if fileEntries.length > 0}
              <span class="absolute top-0.5 right-0.5 h-1.5 w-1.5 rounded-full bg-amber-400"></span>
            {/if}
          </button>
          <!-- Info icon -->
          <button
            class="flex h-8 w-8 items-center justify-center rounded-md transition-colors {activeTab ===
            'info'
              ? 'bg-primary/10 text-primary ring-1 ring-primary/15'
              : 'text-muted-foreground hover:text-foreground hover:bg-accent'}"
            onclick={() => (activeTab = "info")}
            title={t("toolActivity_tabInfo")}
          >
            <svg
              class="h-3.5 w-3.5"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <circle cx="12" cy="12" r="10" />
              <line x1="12" y1="16" x2="12" y2="12" />
              <line x1="12" y1="8" x2="12.01" y2="8" />
            </svg>
          </button>
          <!-- Tasks icon -->
          <button
            class="relative flex h-8 w-8 items-center justify-center rounded-md transition-colors {activeTab ===
            'tasks'
              ? 'bg-primary/10 text-primary ring-1 ring-primary/15'
              : 'text-muted-foreground hover:text-foreground hover:bg-accent'}"
            onclick={() => (activeTab = "tasks")}
            title={t("toolActivity_tabTasks")}
          >
            <svg
              class="h-3.5 w-3.5"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <rect x="3" y="3" width="18" height="18" rx="2" />
              <path d="M9 12l2 2 4-4" />
            </svg>
            {#if activeBackgroundTasks.length > 0}
              <span
                class="absolute top-0.5 right-0.5 h-1.5 w-1.5 rounded-full bg-blue-400 animate-pulse"
              ></span>
            {/if}
          </button>
        </div>
        <div class="flex items-center gap-1">
          <button
            type="button"
            class="flex h-8 w-8 items-center justify-center rounded-md transition-colors {pinned
              ? 'bg-primary/10 text-primary ring-1 ring-primary/15'
              : 'text-muted-foreground hover:bg-accent hover:text-foreground'}"
            onclick={togglePinned}
            title={pinned ? t("toolActivity_unpin") : t("toolActivity_pin")}
            aria-pressed={pinned}
          >
            {@render pinGlyph()}
          </button>
          <button
            type="button"
            class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            onclick={handlePanelToggle}
            title={t("toolActivity_collapse")}
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
              <polyline points="15 18 9 12 15 6" />
            </svg>
          </button>
        </div>
      </div>
    </div>

    <!-- Lazy keep-alive: each tab mounts on first activation and stays mounted (visibility-only after).
         Tab content is absolutely positioned within this relative wrapper so all mounted tabs share
         the same layout slot but only the active one is visible/interactive. -->
    <div class="flex-1 flex flex-col min-h-0 relative">
      {#if mountedTabs.has("tasks")}
        <div
          class="absolute inset-0 flex flex-col"
          style="visibility: {activeTab === 'tasks'
            ? 'visible'
            : 'hidden'}; pointer-events: {activeTab === 'tasks' ? 'auto' : 'none'};"
        >
          <!-- Background tasks panel -->
          <div class="flex-1 overflow-y-auto">
            {#if backgroundTasks.size === 0}
              <div class="flex items-center justify-center h-32 text-xs text-muted-foreground/50">
                {t("bgTask_empty")}
              </div>
            {:else}
              <div class="py-1 space-y-0.5">
                {#each sortedBgTasks as item (item.task_id)}
                  {@const isDone = item.status === "completed"}
                  {@const isFailed = item.status === "failed" || item.status === "error"}
                  {@const isActive = !isDone && !isFailed}
                  {@const rawData = (item.data as Record<string, unknown> | undefined)?.data as
                    | Record<string, unknown>
                    | undefined}
                  {@const usage = rawData?.usage as
                    | { duration_ms?: number; tool_uses?: number; total_tokens?: number }
                    | undefined}
                  {@const toolUseId = item.tool_use_id}
                  <button
                    class="w-full text-left mx-1.5 rounded px-2 py-1.5 transition-colors {isDone
                      ? 'text-foreground/40 hover:bg-accent/30'
                      : isFailed
                        ? 'bg-destructive/5 text-foreground/50 hover:bg-destructive/10'
                        : 'bg-blue-500/5 text-foreground/70 hover:bg-blue-500/10'}"
                    onclick={() => {
                      if (toolUseId) onScrollToTool?.(toolUseId);
                    }}
                    title={toolUseId ? t("toolActivity_scrollToTool") : ""}
                  >
                    <div class="flex items-center gap-2">
                      <StatusIcon
                        status={isActive ? "running" : isDone ? "done" : "error"}
                        size="sm"
                      />
                      <span class="flex-1 min-w-0 truncate text-[11px]"
                        >{item.summary || item.message}</span
                      >
                      {#if isActive}
                        <span class="shrink-0 text-[10px] text-foreground/30 tabular-nums"
                          >{bgElapsed(item.startedAt)}</span
                        >
                      {/if}
                    </div>
                    {#if usage && (usage.tool_uses || usage.total_tokens || usage.duration_ms)}
                      <div class="mt-0.5 text-[10px] text-muted-foreground/60 pl-5">
                        {#if usage.tool_uses}{usage.tool_uses} tools{/if}
                        {#if usage.tool_uses && usage.duration_ms}
                          ·
                        {/if}
                        {#if usage.duration_ms}{formatDuration(usage.duration_ms)}{/if}
                        {#if (usage.tool_uses || usage.duration_ms) && usage.total_tokens}
                          ·
                        {/if}
                        {#if usage.total_tokens}{formatTokenCount(usage.total_tokens)} tok{/if}
                      </div>
                    {/if}
                  </button>
                {/each}
              </div>
            {/if}
          </div>
        </div>
      {/if}
      {#if mountedTabs.has("context")}
        <div
          class="absolute inset-0 flex flex-col"
          style="visibility: {activeTab === 'context'
            ? 'visible'
            : 'hidden'}; pointer-events: {activeTab === 'context' ? 'auto' : 'none'};"
        >
          <ContextHistoryPanel history={contextHistory} {turnUsages} {sessionInfo} />
        </div>
      {/if}
      {#if mountedTabs.has("files")}
        <div
          class="absolute inset-0 flex flex-col"
          style="visibility: {activeTab === 'files'
            ? 'visible'
            : 'hidden'}; pointer-events: {activeTab === 'files' ? 'auto' : 'none'};"
        >
          <div class="flex flex-1 flex-col min-h-0">
            <div class="flex-shrink-0 max-h-[40vh] overflow-y-auto border-b border-border/50">
              {#if editedSummary.hasChanges}
                <div class="border-b border-border/50 px-2.5 py-2">
                  <div class="mb-1.5 flex items-center gap-2">
                    <span
                      class="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground"
                      >{t("toolActivity_editedFiles")}</span
                    >
                    <span class="ml-auto text-[10px] tabular-nums text-muted-foreground">
                      {editedSummary.totalFiles}
                      {#if editedSummary.totalAdditions > 0}
                        <span class="text-emerald-500">+{editedSummary.totalAdditions}</span>
                      {/if}
                      {#if editedSummary.totalDeletions > 0}
                        <span class="text-red-400">-{editedSummary.totalDeletions}</span>
                      {/if}
                    </span>
                  </div>
                  <div class="space-y-1">
                    {#each editedSummary.files.slice(0, 4) as file (file.path)}
                      {@const isDiffSelected = previewMode === "diff" && previewPath === file.path}
                      <button
                        type="button"
                        class="w-full rounded-md border px-2 py-1.5 text-left transition-colors {isDiffSelected
                          ? 'border-primary/50 bg-primary/10'
                          : 'border-border/50 hover:bg-accent/40'}"
                        onclick={() => openDiff(file.path)}
                        title={file.path}
                      >
                        <div class="flex items-center gap-1.5">
                          <span
                            class="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground"
                            >{shortPath(file.path)}</span
                          >
                          {#if file.additions > 0}
                            <span class="text-[10px] tabular-nums text-emerald-500"
                              >+{file.additions}</span
                            >
                          {/if}
                          {#if file.deletions > 0}
                            <span class="text-[10px] tabular-nums text-red-400"
                              >-{file.deletions}</span
                            >
                          {/if}
                        </div>
                        {#if patchPreviewLines(file).length > 0}
                          <div
                            class="mt-1 overflow-hidden rounded border border-border/40 bg-muted/30 px-1.5 py-1 font-mono text-[10px] leading-4"
                          >
                            {#each patchPreviewLines(file) as line}
                              <div class="truncate {patchLineClass(line)}">{line}</div>
                            {/each}
                          </div>
                        {/if}
                      </button>
                    {/each}
                  </div>
                  <button
                    type="button"
                    class="mt-1.5 flex w-full items-center justify-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-primary hover:bg-primary/10 transition-colors"
                    onclick={() => openDiff(editedSummary.files[0]?.path ?? "")}
                  >
                    {t("toolActivity_viewChanges")}
                  </button>
                </div>
              {/if}
              <FilesPanel
                {fileEntries}
                {onScrollToTool}
                onPreview={openPreview}
                onReview={openDiff}
                selectedPath={previewPath ?? undefined}
                selectedMode={previewMode}
              />
            </div>
            <div class="flex-1 min-h-0 overflow-hidden">
              <FilePreviewPane
                {cwd}
                path={previewPath ?? ""}
                mode={previewMode}
                editable={false}
                {isRemote}
                scopeKey={runId}
                active={activeTab === "files"}
                onCloseDiff={() => (previewMode = "preview")}
              />
            </div>
          </div>
        </div>
      {/if}
      {#if mountedTabs.has("info")}
        <div
          class="absolute inset-0 flex flex-col overflow-y-auto"
          style="visibility: {activeTab === 'info'
            ? 'visible'
            : 'hidden'}; pointer-events: {activeTab === 'info' ? 'auto' : 'none'};"
        >
          <!-- Subagents section (shown above session info when Task tools exist) -->
          {#if subagents.length > 0}
            <div class="px-3 py-2 border-b border-border/50">
              <div
                class="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider mb-1.5"
              >
                {t("tool_subagents", { count: String(subagents.length) })}
              </div>
              <div class="space-y-1.5">
                {#each subagents as sa (sa.toolUseId)}
                  {@const isDone = sa.status === "success"}
                  {@const isError = sa.status === "error" || sa.status === "denied"}
                  {@const isRunning = !isDone && !isError}
                  <button
                    class="w-full text-left rounded-md border border-border/50 bg-background/50 px-2.5 py-1.5 hover:bg-accent/30 transition-colors"
                    onclick={() => onScrollToTool?.(sa.toolUseId)}
                    title="Scroll to tool"
                  >
                    <div class="flex items-center gap-1.5">
                      <span class="text-[11px] font-medium text-foreground"
                        >{sa.meta.subagentType}</span
                      >
                      {#if sa.meta.model}
                        <span
                          class="text-[10px] px-1 py-0.5 rounded bg-cyan-500/15 text-cyan-600 dark:text-cyan-400 font-medium"
                          >{sa.meta.model}</span
                        >
                      {/if}
                      <span class="ml-auto">
                        {#if isDone}
                          <StatusIcon status="done" size="sm" />
                        {:else if isError}
                          <StatusIcon status="error" size="sm" />
                        {:else if isRunning}
                          <StatusIcon status="running" size="sm" />
                        {/if}
                      </span>
                    </div>
                    {#if sa.meta.description}
                      <div class="text-[10px] text-muted-foreground truncate mt-0.5">
                        {sa.meta.description}
                      </div>
                    {/if}
                    {#if sa.toolCount > 0 || sa.durationMs != null}
                      <div class="text-[10px] text-muted-foreground/60 mt-0.5">
                        {#if sa.toolCount > 0}{sa.toolCount} tools{/if}
                        {#if sa.toolCount > 0 && sa.durationMs != null}
                          ·
                        {/if}
                        {#if sa.durationMs != null}{formatDuration(sa.durationMs)}{/if}
                      </div>
                    {/if}
                  </button>
                {/each}
              </div>
            </div>
          {/if}
          <SessionInfoPanel info={sessionInfo} {activeTab} />
        </div>
      {/if}
      {#if mountedTabs.has("tools")}
        <div
          class="absolute inset-0 flex flex-col"
          style="visibility: {activeTab === 'tools'
            ? 'visible'
            : 'hidden'}; pointer-events: {activeTab === 'tools' ? 'auto' : 'none'};"
        >
          <!-- Tools panel -->
          <!-- Summary chips -->
          {#if toolStats.summary.length > 1}
            <div class="flex flex-wrap gap-1 px-2.5 py-1.5 border-b border-border/50">
              {#each toolStats.summary as [name, count]}
                {@const style = getToolColor(name)}
                <span
                  class="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded {style.bg} {style.text} font-medium"
                >
                  {name}
                  <span class="opacity-70">{count}</span>
                </span>
              {/each}
            </div>
          {/if}

          <!-- Tool list -->
          <div class="flex-1 overflow-y-auto py-0.5">
            {#if toolStats.totalToolCount === 0}
              <div class="flex items-center justify-center h-32 text-xs text-muted-foreground/50">
                {t("toolActivity_noToolCalls")}
              </div>
            {:else if useTimeline}
              <!-- Timeline mode: grouped by turn -->
              {#each turns as turn (turn.turnIndex)}
                {@const isCollapsed = collapsedTurns.has(turn.turnIndex)}
                {@const tu = usageByTurn.get(turn.turnIndex)}
                {@const hasTools = turn.tools.length > 0}
                <!-- Turn header: div with two sibling buttons (no nesting) -->
                <div
                  class="flex items-center w-full px-2.5 py-1.5 hover:bg-accent/50 transition-colors border-b border-border/30"
                >
                  <button
                    class="flex-1 flex items-center gap-1.5 text-left min-w-0"
                    onclick={() => {
                      if (hasTools) {
                        toggleTurn(turn.turnIndex);
                      } else if (turn.anchorId) {
                        dbg("tool-activity", "scroll to turn (no tools)", {
                          turnIndex: turn.turnIndex,
                          anchorId: turn.anchorId,
                        });
                        onScrollToTurn?.(turn.anchorId);
                      }
                    }}
                  >
                    {#if hasTools}
                      <svg
                        class="h-3 w-3 text-muted-foreground/50 shrink-0 transition-transform {isCollapsed
                          ? ''
                          : 'rotate-90'}"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      >
                        <polyline points="9 18 15 12 9 6" />
                      </svg>
                    {/if}
                    <span class="text-[11px] font-medium text-muted-foreground truncate">
                      {#if turn.userPreview}
                        {t("toolActivity_turn", { index: String(turn.turnIndex) })}
                        <span class="text-foreground/70">{truncate(turn.userPreview, 25)}</span>
                      {:else}
                        <span class="text-muted-foreground/60"
                          >{t("toolActivity_systemResume")}</span
                        >
                      {/if}
                    </span>
                    <span class="ml-auto flex items-center gap-1.5 shrink-0">
                      {#if tu}
                        <span class="text-[10px] text-muted-foreground"
                          >{formatTokenCount(tu.inputTokens + tu.outputTokens)}</span
                        >
                      {/if}
                      {#if hasTools}
                        <span
                          class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground font-medium"
                          >{countToolNodes(turn.tools)}</span
                        >
                      {/if}
                    </span>
                  </button>
                  {#if turn.anchorId}
                    <button
                      class="shrink-0 ml-1 p-0.5 rounded text-muted-foreground/40 hover:text-foreground hover:bg-muted transition-colors"
                      onclick={() => {
                        dbg("tool-activity", "scroll to turn", {
                          turnIndex: turn.turnIndex,
                          anchorId: turn.anchorId,
                        });
                        onScrollToTurn?.(turn.anchorId!);
                      }}
                      title={t("toolActivity_scrollToTurn")}
                    >
                      <svg
                        class="h-3 w-3"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        ><circle cx="12" cy="12" r="3" /><path
                          d="M12 2v4m0 12v4M2 12h4m12 0h4"
                        /></svg
                      >
                    </button>
                  {/if}
                </div>

                <!-- Tools in this turn (only render if turn has tools) -->
                {#if hasTools && !isCollapsed}
                  <div class="py-0.5">
                    {#each turn.tools as node (node.tool.tool_use_id)}
                      {@render toolNodeView(node)}
                    {/each}
                  </div>
                {/if}
              {/each}
            {:else}
              <!-- HookEvent fallback mode (pipe/PTY) -->
              {#each hookToolEvents as event, ei (ei)}
                {@const style = getToolColor(event.tool_name ?? "")}
                {@const detail = getHookDetail(event)}
                {@const cat = categorizeHookStatus(event.status)}
                <div class="px-2.5 py-1">
                  <div class="flex items-center gap-1.5">
                    {@render statusIcon(cat)}
                    <div
                      class="flex h-4 w-4 shrink-0 items-center justify-center rounded {style.bg}"
                    >
                      <svg
                        class="h-2.5 w-2.5 {style.text}"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      >
                        <path d={style.icon} />
                      </svg>
                    </div>
                    <span class="text-[11px] font-medium text-foreground shrink-0"
                      >{event.tool_name ?? event.hook_type}</span
                    >
                    {#if detail}
                      <span class="text-[10px] text-muted-foreground truncate min-w-0"
                        >{detail}</span
                      >
                    {/if}
                  </div>
                </div>
              {/each}
            {/if}
          </div>

          <!-- Stats footer (status counts only, tools tab only) -->
          {#if toolStats.totalToolCount > 0}
            <div class="border-t border-border px-3 py-1.5">
              <div class="flex items-center gap-3 text-[11px]">
                {#if toolStats.doneCount > 0}
                  <span class="flex items-center gap-1 text-emerald-500 dark:text-emerald-400">
                    <StatusIcon status="done" size="sm" />
                    {toolStats.doneCount}
                  </span>
                {/if}
                {#if toolStats.runningCount > 0}
                  <span class="flex items-center gap-1 text-muted-foreground">
                    <StatusIcon status="running" size="sm" />
                    {toolStats.runningCount}
                  </span>
                {/if}
                {#if toolStats.errorCount > 0}
                  <span class="flex items-center gap-1 text-destructive">
                    <StatusIcon status="error" size="sm" />
                    {toolStats.errorCount}
                  </span>
                {/if}
              </div>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  </div>
  {#if collapsed && !panelOpen && compactMode === "rail"}
    <div class="flex h-full w-full items-center justify-center">
      <div
        class="h-24 w-1.5 rounded-full bg-muted-foreground/25 transition-colors group-hover:bg-primary/50"
      ></div>
    </div>
  {/if}
</aside>
