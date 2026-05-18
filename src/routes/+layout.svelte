<script lang="ts">
  import "../app.css";
  import { escapeHtml } from "$lib/utils/ansi";
  import {
    listRuns,
    getUserSettings,
    updateUserSettings,
    listDirectory,
    getGitSummary,
    listPromptFavorites,
    searchPrompts,
    listMemoryFiles,
    renameRun,
    softDeleteRuns,
  } from "$lib/api";
  import ProjectFolderItem from "$lib/components/ProjectFolderItem.svelte";
  import CommandPalette from "$lib/components/CommandPalette.svelte";
  import SetupWizard from "$lib/components/SetupWizard.svelte";
  import AboutModal from "$lib/components/AboutModal.svelte";
  import PermissionsModal from "$lib/components/PermissionsModal.svelte";
  import Modal from "$lib/components/Modal.svelte";
  import CliSessionBrowser from "$lib/components/CliSessionBrowser.svelte";
  import UpdateBanner from "$lib/components/UpdateBanner.svelte";
  import FolderPicker from "$lib/components/FolderPicker.svelte";
  import type {
    TaskRun,
    UserSettings,
    DirEntry,
    GitSummary,
    PromptFavorite,
    PromptSearchResult,
    MemoryFileCandidate,
  } from "$lib/types";
  import { cwdDisplayLabel, truncate, snippetAround, relativeTime } from "$lib/utils/format";
  import { filterVisibleCandidates } from "$lib/utils/memory-helpers";
  import {
    buildProjectFolders,
    autoExpandForRun,
    expandForProjectChange,
    normalizeCwd,
    type ConversationGroup,
  } from "$lib/utils/sidebar-groups";
  import { loadRemovedCwds } from "$lib/utils/removed-cwds";
  import {
    getLastTarget,
    setLastTarget,
    getStoredRemoteCwd,
    setStoredRemoteCwd,
  } from "$lib/utils/remote-cwd";
  import { page } from "$app/stores";
  import { goto, afterNavigate } from "$app/navigation";
  import { onMount, setContext, tick, untrack } from "svelte";
  import { dbg, dbgWarn } from "$lib/utils/debug";
  import { fpsCounter } from "$lib/utils/perf";
  import { PLATFORM_PRESETS } from "$lib/utils/platform-presets";
  import { loadAgentSettingsCache } from "$lib/stores/agent-settings-cache.svelte";
  import type { PlatformCredential } from "$lib/types";
  import { TeamStore } from "$lib/stores/team-store.svelte";
  import { KeybindingStore, normalizeKeyEvent } from "$lib/stores/keybindings.svelte";
  import { getTransport } from "$lib/transport";
  import {
    t,
    LOCALE_REGISTRY,
    getEntry,
    initLocale,
    switchLocale,
    currentLocale,
  } from "$lib/i18n/index.svelte";

  // Wire reactive locale before any t() usage
  initLocale();

  let localePopupOpen = $state(false);

  function handleLocaleSelect(code: string) {
    switchLocale(code);
    localePopupOpen = false;
  }

  let commandPaletteOpen = $state(false);
  let showSetupWizard = $state(false);
  let showAbout = $state(false);
  let showCliBrowser = $state(false);
  let permissionsModalOpen = $state(false);
  let searchPanelOpen = $state(false);
  let themeMenuOpen = $state(false);
  let profileModalOpen = $state(false);
  let profileNameDraft = $state("");
  let profileSaving = $state(false);
  let profileError = $state("");
  let isDesktopApp = $state(false);
  let isMacDesktop = $state(false);
  let isWindowsDesktop = $state(false);
  let isWindowFullscreen = $state(false);
  let isGlassDesktop = $derived(isMacDesktop || isWindowsDesktop);
  let reserveMacTrafficLights = $derived(isMacDesktop && !isWindowFullscreen);
  type WindowEffectsConfig = {
    effects: string[];
    state?: string;
    radius?: number;
    color?: [number, number, number, number];
  };
  type TauriWindowHandle = {
    setBackgroundColor: (color: [number, number, number, number]) => Promise<void>;
    setEffects: (effects: WindowEffectsConfig) => Promise<void>;
    startDragging: () => Promise<void>;
    isFullscreen: () => Promise<boolean>;
    setFullscreen: (fullscreen: boolean) => Promise<void>;
    toggleMaximize: () => Promise<void>;
    setTheme: (theme?: "light" | "dark" | null) => Promise<void>;
    onResized: (handler: () => void) => Promise<() => void>;
    onFocusChanged: (handler: () => void) => Promise<() => void>;
  };
  let tauriWindowPromise: Promise<TauriWindowHandle> | null = null;

  // Team store (shared via context with /teams page)
  const teamStore = new TeamStore();
  setContext("teamStore", teamStore);

  // Keybinding store (shared via context with all pages)
  const keybindingStore = new KeybindingStore();
  setContext("keybindings", keybindingStore);

  let { children } = $props();

  let runs = $state<TaskRun[]>([]);
  let sidebarFavorites = $state<PromptFavorite[]>([]);
  let favoriteRunIds = $derived(new Set(sidebarFavorites.map((f) => f.runId)));
  let settings = $state<UserSettings | null>(null);
  let sidebarOpen = $state(true);
  let projectCwd = $state("");
  type ThemeMode = "light" | "dark" | "system";
  type ColorScheme = "warm" | "neutral";

  function getInitialTheme(): ThemeMode {
    if (typeof window === "undefined") return "dark";
    const saved = localStorage.getItem("helion:theme") ?? localStorage.getItem("ocv:theme");
    if (saved === "light" || saved === "dark" || saved === "system") return saved;
    return "dark";
  }

  function getInitialScheme(): ColorScheme {
    if (typeof window === "undefined") return "neutral";
    const saved = localStorage.getItem("ocv:colorScheme");
    return saved === "warm" ? "warm" : "neutral";
  }

  let themeMode = $state<ThemeMode>(getInitialTheme());
  let colorScheme = $state<ColorScheme>(getInitialScheme());
  let systemDark = $state(
    typeof window !== "undefined"
      ? window.matchMedia("(prefers-color-scheme: dark)").matches
      : false,
  );
  let effectiveDark = $derived(themeMode === "system" ? systemDark : themeMode === "dark");
  let pinnedCwds = $state<string[]>([]);
  let removedCwds = $state<string[]>([]);

  let panelTab = $state<"chats" | "teams">("chats");
  let runSearchQuery = $state("");
  type AppMode = "chat" | "cowork" | "code";

  function getInitialAppMode(): AppMode {
    if (typeof window === "undefined") return "code";
    const saved = localStorage.getItem("helion:mode");
    return saved === "chat" || saved === "cowork" || saved === "code" ? saved : "code";
  }

  let appMode = $state<AppMode>(getInitialAppMode());
  let modeMoreOpen = $state(false);
  const appModeItems = [
    { id: "chat", label: () => t("appMode_chat"), title: () => t("appMode_chatTitle") },
    { id: "cowork", label: () => t("appMode_cowork"), title: () => t("appMode_coworkTitle") },
    { id: "code", label: () => t("appMode_code"), title: () => t("appMode_codeTitle") },
  ] as const;
  type ModeMenuItem = { icon: string; label: string; action: string };

  const modeMenuItems = $derived.by<ModeMenuItem[]>(() => {
    const zh = currentLocale().startsWith("zh");
    if (appMode === "chat") {
      return [
        { icon: "plus", label: zh ? "新聊天" : "New chat", action: "new" },
        { icon: "sparkles", label: "Skills", action: "skills" },
        { icon: "package", label: "Plugins", action: "plugins" },
        { icon: "folder", label: zh ? "打开文件夹" : "Open folder", action: "projects" },
      ];
    }
    if (appMode === "cowork") {
      return [
        { icon: "plus", label: zh ? "新任务" : "New task", action: "new" },
        { icon: "book", label: zh ? "实时文档" : "Live docs", action: "live-docs" },
        { icon: "sparkles", label: "Skills", action: "skills" },
        { icon: "package", label: "Plugins", action: "plugins" },
        { icon: "folder", label: zh ? "打开文件夹" : "Open folder", action: "projects" },
      ];
    }
    return [
      { icon: "plus", label: zh ? "新会话" : "New session", action: "new" },
      { icon: "chart", label: zh ? "查看更改" : "Review changes", action: "changes" },
      { icon: "sparkles", label: "Skills", action: "skills" },
      { icon: "package", label: "Plugins", action: "plugins" },
      { icon: "chevron", label: zh ? "更多" : "More", action: "more" },
    ];
  });

  const modeMoreItems = $derived.by<ModeMenuItem[]>(() => {
    const zh = currentLocale().startsWith("zh");
    if (appMode === "chat") {
      return [
        { icon: "server", label: "MCP", action: "mcp" },
        { icon: "webhook", label: "Hooks", action: "hooks" },
        { icon: "agents", label: "Agents", action: "agents" },
      ];
    }
    if (appMode === "cowork") {
      return [
        { icon: "server", label: "MCP", action: "mcp" },
        { icon: "webhook", label: "Hooks", action: "hooks" },
        { icon: "agents", label: "Agents", action: "agents" },
        { icon: "chart", label: zh ? "查看更改" : "Review changes", action: "changes" },
      ];
    }
    return [
      { icon: "server", label: "MCP", action: "mcp" },
      { icon: "webhook", label: "Hooks", action: "hooks" },
      { icon: "agents", label: "Agents", action: "agents" },
    ];
  });

  function handleModeMenuAction(action: string) {
    if (action === "more") {
      modeMoreOpen = !modeMoreOpen;
      return;
    }
    modeMoreOpen = ["mcp", "hooks", "agents"].includes(action);
    if (action === "new") {
      newChat();
      return;
    }
    if (action === "projects") {
      pickFolder();
      return;
    }
    if (action === "skills") {
      pluginActiveSection = "skills";
      goto("/plugins?section=skills&source=discover");
      return;
    }
    if (action === "live-docs") {
      goto("/live-docs");
      return;
    }
    if (action === "plugins" || action === "artifacts") {
      pluginActiveSection = "plugins";
      goto("/plugins?section=plugins&source=marketplace");
      return;
    }
    if (action === "mcp") {
      pluginActiveSection = "mcp";
      goto("/plugins?section=mcp&source=discover");
      return;
    }
    if (action === "hooks") {
      pluginActiveSection = "hooks";
      goto("/plugins?section=hooks");
      return;
    }
    if (action === "agents") {
      pluginActiveSection = "agents";
      goto("/plugins?section=agents");
      return;
    }
    if (action === "changes") {
      goto("/explorer");
      return;
    }
    goto("/chat");
  }

  function selectAppMode(mode: AppMode) {
    appMode = mode;
    modeMoreOpen = false;
    panelTab = "chats";
    localStorage.setItem("helion:mode", mode);
    window.dispatchEvent(new CustomEvent("helion:mode-change", { detail: { mode } }));
    if (!isChatPage) goto("/chat");
  }

  async function openModelSelectorFromLayout() {
    if (!currentPath.startsWith("/chat") && currentPath !== "/") {
      await goto("/chat");
      await tick();
    }
    window.dispatchEvent(new CustomEvent("ocv:command-open-model-selector"));
  }

  // ── Folder tree state ──
  let expandedProjects = $state<Set<string>>(new Set());
  let runsLoadSucceededOnce = $state(false);

  // ── Deep search (backend full-text) ──
  let searchResults = $state<PromptSearchResult[]>([]);
  let searching = $state(false);
  let searchRequestId = $state(0);
  let debounceTimer: ReturnType<typeof setTimeout> | undefined;

  // ── Sidebar resize (ghost-line strategy, same as right panel) ──
  // chat-main reflow during drag is still expensive even with cv-auto: visible tool cards
  // contain markdown / hljs code blocks that re-measure on every width change. Ghost line
  // gives zero-reflow drag preview and commits once on release.
  const SIDEBAR_DEFAULT_WIDTH = 300;
  const SIDEBAR_MIN_WIDTH = 236;
  const SIDEBAR_COLLAPSE_WIDTH = 190;
  const SIDEBAR_MAX_WIDTH = 380;

  function clampSidebarWidth(width: number): number {
    return Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, width));
  }

  function loadSidebarWidth(): number {
    if (typeof window === "undefined") return SIDEBAR_DEFAULT_WIDTH;
    const raw = parseInt(localStorage.getItem("ocv:sidebar-width") ?? "", 10);
    return Number.isFinite(raw) ? clampSidebarWidth(raw) : SIDEBAR_DEFAULT_WIDTH;
  }
  let sidebarWidth = $state(loadSidebarWidth());
  let sidebarResizing = $state(false);
  let sidebarGhostX = $state(0);
  let resizeCleanup: (() => void) | null = null;

  /** Ghost line DOM element — bound after sidebarResizing toggles true.
   *  Used only for imperative DOM writes during drag, but declared as $state to satisfy
   *  Svelte 5's bind:this reactivity contract (silences non_reactive_update warning). */
  let sidebarGhostEl: HTMLElement | null = $state(null);

  function startResize(e: PointerEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = sidebarWidth;
    let pendingWidth = startWidth;
    sidebarResizing = true;
    sidebarGhostX = e.clientX; // initial position via Svelte (single render)
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture?.(e.pointerId);
    dbg("layout", "sidebar resize start", { startWidth });
    document.body.style.userSelect = "none";
    document.body.style.cursor = "col-resize";
    const stopFps = fpsCounter("sidebar-drag");

    function onMove(ev: PointerEvent) {
      const rawWidth = startWidth + (ev.clientX - startX);
      pendingWidth = rawWidth < SIDEBAR_COLLAPSE_WIDTH ? 0 : clampSidebarWidth(rawWidth);
      const x = startX + (pendingWidth - startWidth);
      // BYPASS Svelte: write DOM directly. Svelte's reactive batching + WKWebView's pointer
      // capture don't cooperate during drag — updates pile up until pointerup. Direct DOM
      // write is synchronous and the browser repaints on the next frame regardless.
      if (sidebarGhostEl) {
        sidebarGhostEl.style.left = x - 1 + "px";
      }
    }
    function cleanup() {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
      try {
        handle.releasePointerCapture?.(e.pointerId);
      } catch {
        // The window-level listeners still complete the resize if capture is already gone.
      }
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
      if (pendingWidth <= 0) {
        sidebarOpen = false;
      } else {
        sidebarWidth = pendingWidth;
        sidebarOpen = true;
        localStorage.setItem("ocv:sidebar-width", String(sidebarWidth));
      }
      sidebarResizing = false;
      sidebarGhostEl = null;
      resizeCleanup = null;
      dbg("layout", "sidebar resize end", { width: sidebarWidth, open: sidebarOpen });
      stopFps();
    }
    function onUp() {
      cleanup();
    }

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    resizeCleanup = cleanup;
  }

  // ── File tree state (shown in sidebar when on /explorer) ──
  interface TreeNode {
    name: string;
    fullPath: string;
    is_dir: boolean;
    size: number;
    expanded: boolean;
    loaded: boolean;
    children: TreeNode[];
    depth: number;
  }

  let fileTree = $state<TreeNode[]>([]);
  let treeLoading = $state(false);
  let explorerSelectedFile = $state("");
  let explorerTab = $state<"files" | "git">("files");
  let explorerProjectOpen = $state(false);

  // ── Git state (shown in sidebar Git tab when on /explorer) ──
  let gitSummary = $state<GitSummary | null>(null);
  let gitLoading = $state(false);

  const GIT_STATUS_COLORS: Record<string, string> = {
    M: "text-blue-400",
    A: "text-green-400",
    D: "text-red-400",
    R: "text-purple-400",
    "?": "text-muted-foreground",
  };

  function entriesToNodes(entries: DirEntry[], parentPath: string, depth: number): TreeNode[] {
    return entries.map((e) => ({
      name: e.name,
      fullPath: `${parentPath}/${e.name}`,
      is_dir: e.is_dir,
      size: e.size,
      expanded: false,
      loaded: false,
      children: [],
      depth,
    }));
  }

  let _treeSeq = 0;
  async function loadRootTree() {
    if (!projectCwd) {
      fileTree = [];
      return;
    }
    const seq = ++_treeSeq;
    treeLoading = true;
    try {
      const listing = await listDirectory(projectCwd, true);
      if (seq !== _treeSeq) return; // stale response, discard
      fileTree = entriesToNodes(listing.entries, projectCwd, 0);
      dbg("layout", "file tree loaded", { count: fileTree.length });
    } catch (e) {
      if (seq !== _treeSeq) return;
      dbgWarn("layout", "file tree load error", e);
      fileTree = [];
    } finally {
      if (seq === _treeSeq) treeLoading = false;
    }
  }

  async function toggleFolder(node: TreeNode) {
    if (!node.loaded) {
      try {
        const listing = await listDirectory(node.fullPath, true);
        node.children = entriesToNodes(listing.entries, node.fullPath, node.depth + 1);
        node.loaded = true;
        dbg("layout", "folder loaded", { path: node.fullPath, count: node.children.length });
      } catch (e) {
        dbgWarn("layout", "folder load error", e);
        node.children = [];
        node.loaded = true;
      }
    }
    node.expanded = !node.expanded;
  }

  function selectFile(node: TreeNode) {
    explorerSelectedFile = node.fullPath;
    // Notify explorer page via custom event
    window.dispatchEvent(new CustomEvent("ocv:explorer-file", { detail: { path: node.fullPath } }));
  }

  let _gitSeq = 0;
  let _gitLoadedCwd = "";
  async function loadGitSummary() {
    if (!projectCwd) {
      gitSummary = null;
      _gitLoadedCwd = "";
      return;
    }
    const requestedCwd = projectCwd;
    const seq = ++_gitSeq;
    gitLoading = true;
    try {
      const result = await getGitSummary(requestedCwd);
      if (seq !== _gitSeq) return; // stale response, discard
      gitSummary = result;
      _gitLoadedCwd = requestedCwd;
      dbg("layout", "git summary loaded", {
        branch: result.branch,
        files: result.total_files,
      });
    } catch (e) {
      if (seq !== _gitSeq) return;
      dbgWarn("layout", "git summary load error", e);
      gitSummary = null;
      _gitLoadedCwd = "";
    } finally {
      if (seq === _gitSeq) gitLoading = false;
    }
  }

  function selectDiffFile(filePath: string) {
    // Notify explorer page to show diff
    window.dispatchEvent(new CustomEvent("ocv:explorer-diff", { detail: { path: filePath } }));
  }

  // ── Memory sidebar state (shown when on /memory) ──
  let memoryCandidates = $state<MemoryFileCandidate[]>([]);
  let memorySelectedFile = $state("");
  let memoryLoading = $state(false);
  let memoryScopeExpanded = $state<Record<string, boolean>>({
    global: false,
  });

  let memoryScopeProject = $derived(memoryCandidates.filter((c) => c.scope === "project"));
  let memoryScopeGlobal = $derived(memoryCandidates.filter((c) => c.scope === "global"));
  let memoryScopeMemory = $derived(memoryCandidates.filter((c) => c.scope === "memory"));
  // Merged project + auto memory for flat folder view
  let memoryScopeFolder = $derived([...memoryScopeProject, ...memoryScopeMemory]);

  let memoryCandidateSeq = 0;

  async function loadMemoryCandidates(opts?: { soft?: boolean }) {
    const seq = ++memoryCandidateSeq;
    if (!opts?.soft) memoryLoading = true;
    try {
      const result = await listMemoryFiles(projectCwd || undefined);
      if (seq !== memoryCandidateSeq) return; // stale — discard
      memoryCandidates = result;
      dbg("layout", "memory candidates loaded", {
        count: result.length,
        existing: result.filter((f) => f.exists).length,
      });
    } catch (e) {
      if (seq !== memoryCandidateSeq) return;
      if (opts?.soft) {
        dbgWarn("layout", "soft memory refresh failed, keeping old data", e);
      } else {
        dbgWarn("layout", "memory candidates load error", e);
        memoryCandidates = [];
      }
    } finally {
      if (seq === memoryCandidateSeq) memoryLoading = false;
    }
  }

  function selectMemoryFile(file: MemoryFileCandidate) {
    // Don't set highlight immediately — page will confirm dirty state first.
    // If confirmed, page sends ocv:memory-file-selected to ack the switch.
    window.dispatchEvent(
      new CustomEvent("ocv:memory-select", { detail: { path: file.path, exists: file.exists } }),
    );
  }

  function toggleMemoryScope(scope: string) {
    memoryScopeExpanded = { ...memoryScopeExpanded, [scope]: !memoryScopeExpanded[scope] };
  }

  // Load tree when switching to explorer page or changing project
  // Git summary is lazy-loaded when user clicks the Git tab (see below)
  let _prevExplorerCwd: string | undefined;
  $effect(() => {
    const _path = currentPath;
    const _cwd = projectCwd;
    if (_path?.startsWith("/explorer")) {
      if (_cwd) {
        loadRootTree();
        // Invalidate git cache when cwd changes so Git tab reloads on next switch
        if (_prevExplorerCwd !== undefined && _prevExplorerCwd !== _cwd) {
          ++_gitSeq; // cancel in-flight request so it can't backfill _gitLoadedCwd
          gitLoading = false;
          _gitLoadedCwd = "";
        }
        _prevExplorerCwd = _cwd;
      } else {
        // Increment seq to invalidate any in-flight requests
        ++_treeSeq;
        ++_gitSeq;
        // Clear state
        fileTree = [];
        gitSummary = null;
        gitLoading = false;
        treeLoading = false;
        _gitLoadedCwd = "";
        _prevExplorerCwd = _cwd;
      }
    }
  });

  // Lazy-load git summary when user switches to Git tab (only on Explorer page)
  $effect(() => {
    if (
      currentPath?.startsWith("/explorer") &&
      explorerTab === "git" &&
      projectCwd &&
      _gitLoadedCwd !== projectCwd
    ) {
      loadGitSummary();
    }
  });

  // Load memory candidates when switching to memory page or changing project
  let _prevMemoryCwd: string | undefined;
  $effect(() => {
    const _path = currentPath;
    const _cwd = projectCwd;
    if (_path?.startsWith("/memory")) {
      const cwdChanged = _cwd !== _prevMemoryCwd;
      _prevMemoryCwd = _cwd;
      if (cwdChanged) {
        // Only clear project scope, keep Global/Memory to avoid visual jitter
        // Use untrack to read memoryCandidates without adding it as a dependency
        memoryCandidates = untrack(() => memoryCandidates).filter((c) => c.scope !== "project");
      }
      loadMemoryCandidates();
    }
  });

  // Navigation items (declared before pageName derivation)
  const navItems = [
    { path: "/chat", label: () => t("nav_chat"), icon: "message" },
    { path: "/explorer", label: () => t("nav_explorer"), icon: "folder" },
    { path: "/plugins", label: () => t("nav_extend"), icon: "zap" },
    { path: "/memory", label: () => t("nav_memory"), icon: "book" },
    { path: "/usage", label: () => t("nav_usage"), icon: "chart" },
    { path: "/history", label: () => t("nav_history"), icon: "clock" },
    { path: "/settings", label: () => t("nav_settings"), icon: "settings" },
  ];

  // Load initial data
  async function loadRuns() {
    try {
      runs = await listRuns();
      runsLoadSucceededOnce = true;
    } catch {
      // Silently fail
    }
  }

  async function loadSidebarFavorites() {
    try {
      sidebarFavorites = await listPromptFavorites();
    } catch {
      // Silently fail
    }
  }

  // ── Deep search ──

  function onDeepQueryInput() {
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => doDeepSearch(), 300);
  }

  async function doDeepSearch() {
    const q = runSearchQuery.trim();
    if (!q) {
      searchResults = [];
      searching = false;
      return;
    }
    searching = true;
    const reqId = ++searchRequestId;
    try {
      const results = await searchPrompts(q);
      if (reqId !== searchRequestId) return;
      searchResults = results;
      dbg("layout", "search results", { count: results.length });
    } catch (e) {
      if (reqId !== searchRequestId) return;
      dbg("layout", "search error", e);
      searchResults = [];
    } finally {
      if (reqId === searchRequestId) searching = false;
    }
  }

  function highlightMatch(text: string, query: string): string {
    if (!query.trim()) return escapeHtml(text);
    const escaped = escapeHtml(text);
    const q = escapeHtml(query.trim());
    const re = new RegExp(`(${q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`, "gi");
    return escaped.replace(re, "<mark>$1</mark>");
  }

  async function loadSettings() {
    try {
      settings = await getUserSettings();
      const normalizedWd = normalizeCwd(settings.working_directory);
      if (normalizedWd) {
        localStorage.setItem("ocv:settings-cwd", normalizedWd);
        if (!projectCwd) projectCwd = normalizedWd;
      } else {
        localStorage.removeItem("ocv:settings-cwd");
      }
      // Show setup wizard if onboarding not completed
      if (!settings.onboarding_completed) {
        showSetupWizard = true;
      }
      if (settings.auth_mode !== "cli") {
        settings = await updateUserSettings({ auth_mode: "cli" } as Partial<UserSettings>);
      }
      // One-time migration: if platform_credentials is empty but api_key exists,
      // create an initial credential from current settings
      await migrateCredentialsIfNeeded(settings);
      applyZoom(settings.ui_zoom);
    } catch {
      // Silently fail
    }
  }

  function applyZoom(zoom?: number) {
    const factor = Math.min(1.5, Math.max(0.75, zoom ?? 1.0));
    import("@tauri-apps/api/webviewWindow")
      .then(({ getCurrentWebviewWindow }) => {
        getCurrentWebviewWindow()
          .setZoom(factor)
          .then(() => dbg("layout", "applyZoom success", { factor }))
          .catch((e) => dbgWarn("layout", "setZoom failed", e));
      })
      .catch((e) => dbgWarn("layout", "import webviewWindow failed", e));
  }

  /** Migrate existing api_key into platform_credentials (one-time). */
  async function migrateCredentialsIfNeeded(s: UserSettings) {
    if (s.platform_credentials && s.platform_credentials.length > 0) return;
    if (!s.anthropic_api_key) return;

    // Detect platform from base_url
    let platformId = "anthropic";
    if (s.anthropic_base_url) {
      const match = PLATFORM_PRESETS.find((p) => p.base_url && s.anthropic_base_url === p.base_url);
      platformId = match?.id ?? "custom-migrated";
    }

    const cred: PlatformCredential = {
      platform_id: platformId,
      api_key: s.anthropic_api_key,
      base_url: s.anthropic_base_url || undefined,
      auth_env_var: s.auth_env_var || undefined,
      ...(platformId === "custom-migrated" ? { name: "Migrated" } : {}),
    };

    try {
      await updateUserSettings({
        platform_credentials: [cred],
        active_platform_id: platformId,
      } as Partial<UserSettings>);
      dbg("layout", "migrated credentials", { platformId });
    } catch (e) {
      dbgWarn("layout", "credential migration failed:", e);
    }
  }

  function handleSetupComplete() {
    showSetupWizard = false;
    loadSettings();
  }

  let displayName = $derived(settings?.display_name?.trim() || "Helion");
  let displayInitial = $derived((displayName.trim()[0] || "H").toUpperCase());

  function openProfileModal() {
    profileNameDraft = settings?.display_name?.trim() || "";
    profileError = "";
    profileModalOpen = true;
  }

  async function saveProfileName() {
    const name = profileNameDraft.trim();
    if (!name) {
      profileError = currentLocale().startsWith("zh") ? "请输入用户名" : "Please enter a name";
      return;
    }
    profileSaving = true;
    profileError = "";
    try {
      settings = await updateUserSettings({ display_name: name } as Partial<UserSettings>);
      profileModalOpen = false;
      window.dispatchEvent(new Event("helion:profile-changed"));
    } catch (e) {
      profileError = String((e as Error)?.message ?? e);
    } finally {
      profileSaving = false;
    }
  }

  function getTauriWindowHandle() {
    if (!tauriWindowPromise) {
      tauriWindowPromise = import("@tauri-apps/api/window").then(
        ({ getCurrentWindow }) => getCurrentWindow() as unknown as TauriWindowHandle,
      );
    }
    return tauriWindowPromise;
  }

  async function applyDesktopWindowEffects() {
    if (!isGlassDesktop || typeof window === "undefined") return;

    try {
      const win = await getTauriWindowHandle();
      await win.setTheme(themeMode === "system" ? null : themeMode);
      await win.setBackgroundColor([0, 0, 0, 0]);

      if (isMacDesktop) {
        await win.setEffects({
          effects: ["underWindowBackground"],
          state: "active",
          radius: 16,
        });
        return;
      }

      if (isWindowsDesktop) {
        try {
          await win.setEffects({
            effects: ["acrylic"],
            color: effectiveDark ? [12, 12, 14, 118] : [255, 255, 255, 150],
          });
        } catch {
          await win.setEffects({
            effects: ["blur"],
            color: effectiveDark ? [12, 12, 14, 112] : [255, 255, 255, 142],
          });
        }
      }
    } catch (e) {
      dbgWarn("layout", "apply desktop window effects failed", e);
    }
  }

  async function refreshWindowChromeState() {
    if (!isDesktopApp || typeof window === "undefined") return;

    try {
      const win = await getTauriWindowHandle();
      isWindowFullscreen = await win.isFullscreen();
    } catch (e) {
      dbgWarn("layout", "refresh window chrome state failed", e);
    }
  }

  function isWindowDragTarget(event: PointerEvent) {
    if (!isDesktopApp || event.button !== 0 || event.detail > 1 || event.defaultPrevented)
      return false;

    const target = event.target;
    if (!(target instanceof Element)) return false;

    if (
      target.closest(
        [
          ".titlebar-no-drag",
          "[data-titlebar-no-drag]",
          "[data-no-window-drag]",
          "[data-conversation-menu]",
          "a",
          "button",
          "input",
          "label",
          "select",
          "summary",
          "textarea",
          "[contenteditable='true']",
          "[role='button']",
          "[role='link']",
          ".cm-editor",
          ".xterm",
        ].join(","),
      )
    ) {
      return false;
    }

    return !!target.closest(
      "[data-window-control-strip], .titlebar-drag, [data-tauri-drag-region]",
    );
  }

  function handleWindowDragPointerDown(event: PointerEvent) {
    if (!isWindowDragTarget(event)) return;
    event.preventDefault();
    getTauriWindowHandle()
      .then((win) => win.startDragging())
      .catch((e) => dbgWarn("layout", "start window dragging failed", e));
  }

  function toggleWindowMaximize(event: MouseEvent) {
    if (!isDesktopApp) return;
    event.preventDefault();
    getTauriWindowHandle()
      .then(async (win) => {
        if (isWindowFullscreen) {
          await win.setFullscreen(false);
        } else {
          await win.toggleMaximize();
        }
        setTimeout(() => void refreshWindowChromeState(), 160);
      })
      .catch((e) => dbgWarn("layout", "toggle window maximize failed", e));
  }

  // Use onMount for initialization (not $effect - avoids accidental reactive tracking)
  onMount(() => {
    isDesktopApp = getTransport().isDesktop();
    const platformText = `${navigator.platform ?? ""} ${navigator.userAgent ?? ""}`;
    isMacDesktop = isDesktopApp && /mac/i.test(platformText);
    isWindowsDesktop = isDesktopApp && /win/i.test(platformText);
    document.documentElement.classList.toggle("desktop-window-transparent", isGlassDesktop);
    document.body.classList.toggle("desktop-window-transparent", isGlassDesktop);
    void applyDesktopWindowEffects();
    void refreshWindowChromeState();

    function syncDesktopWindowChrome() {
      void refreshWindowChromeState();
    }
    window.addEventListener("resize", syncDesktopWindowChrome);

    // Remove splash screen
    const splash = document.getElementById("app-splash");
    if (splash) {
      splash.style.opacity = "0";
      setTimeout(() => splash.remove(), 300);
    }

    loadRuns();
    loadSettings();
    loadSidebarFavorites();
    loadAgentSettingsCache();
    window.dispatchEvent(new CustomEvent("helion:mode-change", { detail: { mode: appMode } }));

    // Load saved CWD and pinned folders from localStorage
    const saved = localStorage.getItem("ocv:project-cwd");
    if (saved) projectCwd = normalizeCwd(saved) || "";

    // Load expanded projects from localStorage (defensive parse)
    try {
      const rawExpanded = localStorage.getItem("ocv:expanded-projects");
      if (rawExpanded) {
        const parsed = JSON.parse(rawExpanded);
        if (Array.isArray(parsed) && parsed.every((v: unknown) => typeof v === "string")) {
          expandedProjects = new Set(parsed as string[]);
        }
      }
    } catch {
      /* ignore corrupted data, keep empty Set */
    }
    try {
      const pinned = localStorage.getItem("ocv:pinned-cwds");
      if (pinned) pinnedCwds = JSON.parse(pinned);
    } catch {
      /* ignore parse errors */
    }
    pinnedConversationKeys = readStringArray(PINNED_CONVERSATIONS_KEY);
    archivedConversationKeys = readStringArray(ARCHIVED_CONVERSATIONS_KEY);
    unreadConversationKeys = readStringArray(UNREAD_CONVERSATIONS_KEY);
    removedCwds = loadRemovedCwds();

    // Poll for runs every 60s (fallback only — primary updates via ocv:runs-changed event)
    const interval = setInterval(loadRuns, 60000);

    // Team store: initial load + poll fallback (60s)
    teamStore.loadTeams();
    const teamPollInterval = setInterval(() => teamStore.loadTeams(), 60000);

    // Team/task event listeners — app-level lifecycle, independent of chat page
    type TeamUpdatePayload = { team_name: string; change: string };
    type TaskUpdatePayload = { team_name: string; task_id: string; change: string };

    let destroyed = false;
    let unlistenDesktopResize: (() => void) | undefined;
    let unlistenDesktopFocus: (() => void) | undefined;
    let unlistenTeam: (() => void) | undefined;
    let unlistenTask: (() => void) | undefined;
    const retryTimers: ReturnType<typeof setTimeout>[] = [];

    if (isDesktopApp) {
      const syncAfterNativeWindowEvent = () => {
        if (destroyed) return;
        setTimeout(() => {
          if (!destroyed) void refreshWindowChromeState();
        }, 80);
      };

      getTauriWindowHandle()
        .then(async (win) => {
          const resizeUnlisten = await win.onResized(syncAfterNativeWindowEvent);
          const focusUnlisten = await win.onFocusChanged(syncAfterNativeWindowEvent);
          if (destroyed) {
            resizeUnlisten();
            focusUnlisten();
            return;
          }
          unlistenDesktopResize = resizeUnlisten;
          unlistenDesktopFocus = focusUnlisten;
        })
        .catch((e) => dbgWarn("layout", "desktop window event listener failed", e));
    }

    // 首次+重试成功后都补偿同步（debounce 300ms）
    let resyncTimer: ReturnType<typeof setTimeout> | undefined;
    function scheduleResync() {
      if (resyncTimer) clearTimeout(resyncTimer);
      resyncTimer = setTimeout(() => {
        if (!destroyed) teamStore.forceRefresh();
      }, 300);
    }

    const transport = getTransport();

    function registerTeamListener<T>(
      name: string,
      handler: (payload: T) => void,
      assign: (fn: () => void) => void,
    ) {
      function tryListen(attempt: number) {
        transport
          .listen<T>(name, handler)
          .then((fn) => {
            if (destroyed) {
              fn();
              return;
            }
            assign(fn);
            scheduleResync();
          })
          .catch((e) => {
            if (destroyed) return;
            if (attempt < 2) {
              const delay = (attempt + 1) * 2000; // 2s, 4s
              dbgWarn(
                "layout",
                `${name} listen failed (attempt ${attempt + 1}/3), retry in ${delay}ms`,
                e,
              );
              const t = setTimeout(() => tryListen(attempt + 1), delay);
              retryTimers.push(t);
            } else {
              dbgWarn("layout", `${name} listen failed after 3 attempts, falling back to poll`, e);
            }
          });
      }
      tryListen(0);
    }

    registerTeamListener<TeamUpdatePayload>(
      "team-update",
      (payload) => {
        dbg("layout", "team-update", payload);
        teamStore.handleTeamUpdate(payload);
      },
      (fn) => (unlistenTeam = fn),
    );

    registerTeamListener<TaskUpdatePayload>(
      "task-update",
      (payload) => {
        dbg("layout", "task-update", payload);
        teamStore.handleTaskUpdate(payload);
      },
      (fn) => (unlistenTask = fn),
    );

    // Keybinding store: load overrides + CLI bindings, register app-level callbacks
    keybindingStore.loadOverrides();
    keybindingStore.loadCliBindings();
    keybindingStore.registerCallback("app:toggleSidebar", toggleSidebar);
    keybindingStore.registerCallback("app:commandPalette", () => {
      commandPaletteOpen = !commandPaletteOpen;
    });
    keybindingStore.registerCallback("app:newChat", newChat);

    function onShortcutHintKeydown(e: KeyboardEvent) {
      if (e.metaKey || e.ctrlKey || e.key === "Meta" || e.key === "Control") {
        updateWorkspaceShortcutHints(e);
      }
    }

    function onShortcutHintKeyup(e: KeyboardEvent) {
      if (e.key === "Meta" || e.key === "Control" || e.metaKey || e.ctrlKey) {
        updateWorkspaceShortcutHints(e);
      } else if (workspaceShortcutHintsVisible) {
        updateWorkspaceShortcutHints(e);
      }
    }

    function clearShortcutHints() {
      updateWorkspaceShortcutHints();
    }

    function onVisibilityChange() {
      if (document.visibilityState !== "visible") clearShortcutHints();
    }

    window.addEventListener("keydown", onShortcutHintKeydown, true);
    window.addEventListener("keyup", onShortcutHintKeyup, true);
    window.addEventListener("blur", clearShortcutHints);
    document.addEventListener("visibilitychange", onVisibilityChange);

    // Immediate refresh when chat page signals a status change
    function onRunsChanged() {
      loadRuns();
      // Invalidate git cache unconditionally; if currently viewing Git tab on Explorer,
      // reload immediately — otherwise lazy $effect picks it up on next visit.
      ++_gitSeq; // cancel in-flight request so it can't backfill _gitLoadedCwd
      gitLoading = false;
      _gitLoadedCwd = "";
      if (currentPath?.startsWith("/explorer") && explorerTab === "git") {
        loadGitSummary();
      }
    }
    window.addEventListener("ocv:runs-changed", onRunsChanged);

    // Refresh sidebar favorites when /runs page changes them
    function onFavoritesChanged() {
      loadSidebarFavorites();
    }
    window.addEventListener("ocv:favorites-changed", onFavoritesChanged);

    // Listen for Settings page requesting wizard re-open
    function onShowWizard() {
      showSetupWizard = true;
    }
    window.addEventListener("ocv:show-wizard", onShowWizard);

    // Memory page signals which file it selected (for sidebar highlight sync)
    function onMemoryFileSelected(e: Event) {
      const path = (e as CustomEvent).detail?.path ?? "";
      if (path) memorySelectedFile = path;
    }
    window.addEventListener("ocv:memory-file-selected", onMemoryFileSelected);

    // Memory page signals a file was saved (refresh candidates to update exists status)
    function onMemoryFileSaved() {
      if (currentPath?.startsWith("/memory")) loadMemoryCandidates({ soft: true });
    }
    window.addEventListener("ocv:memory-file-saved", onMemoryFileSaved);

    // Sync projectCwd when chat page picks a folder via dialog
    function handleCwdChanged() {
      const newCwd = normalizeCwd(localStorage.getItem("ocv:project-cwd") ?? "") || "";
      if (newCwd !== projectCwd) {
        projectCwd = newCwd;
      }
    }
    window.addEventListener("ocv:cwd-changed", handleCwdChanged);

    // Open permissions modal from any entry point (Command Palette, PromptInput button)
    function onOpenPermissions() {
      permissionsModalOpen = true;
    }
    window.addEventListener("ocv:open-permissions", onOpenPermissions);

    function onConversationMenuPointerDown(e: PointerEvent) {
      const target = e.target as HTMLElement | null;
      if (!conversationMenu.open || target?.closest("[data-conversation-menu]")) return;
      closeConversationMenu();
    }

    function onConversationMenuKeydown(e: KeyboardEvent) {
      if (e.key === "Escape" && conversationMenu.open) {
        closeConversationMenu();
      }
    }

    function onConversationMenuScroll() {
      if (conversationMenu.open) closeConversationMenu();
    }
    document.addEventListener("pointerdown", onConversationMenuPointerDown);
    document.addEventListener("pointerdown", handleWindowDragPointerDown, true);
    document.addEventListener("keydown", onConversationMenuKeydown);
    document.addEventListener("scroll", onConversationMenuScroll, true);

    // ── External link interceptor ──
    // Prevent webview from navigating away to external URLs.
    // Opens them in the system browser instead.
    function handleExternalLink(e: MouseEvent) {
      // Only intercept plain left-click (no modifier keys)
      if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;

      const anchor = (e.target as HTMLElement)?.closest?.("a");
      if (!anchor) return;

      const href = anchor.getAttribute("href");
      if (!href) return;

      // Parse URL — handles protocol-relative (//example.com), case-insensitive schemes
      let url: URL;
      try {
        url = new URL(href, window.location.origin);
      } catch {
        return;
      }

      // Only intercept http/https
      if (url.protocol !== "http:" && url.protocol !== "https:") return;
      // Skip internal SvelteKit routes (same origin)
      if (url.origin === window.location.origin) return;

      // Prevent webview navigation, don't stopPropagation (let other listeners see it)
      e.preventDefault();

      dbg("layout", "external-link: opening in system browser", { href });
      import("@tauri-apps/plugin-shell")
        .then(({ open }) => open(href))
        .catch((err) => {
          dbgWarn("layout", "external-link: plugin-shell failed, fallback to window.open", err);
          window.open(href, "_blank");
        });
    }
    document.addEventListener("click", handleExternalLink, true);
    dbg("layout", "external-link interceptor mounted");

    // Explorer → layout: sync sidebar highlight when explorer restores cached file
    function onExplorerFileSelected(e: Event) {
      explorerSelectedFile = (e as CustomEvent).detail?.path ?? "";
    }
    window.addEventListener("ocv:explorer-file-selected", onExplorerFileSelected);

    // Listen for run status changes (idle↔running) from backend
    let unlistenStatus: (() => void) | undefined;
    transport
      .listen("ocv:status-changed", (payload: unknown) => {
        dbg("layout", "status-changed", payload);
        loadRuns();
      })
      .then((fn) => {
        if (destroyed) {
          fn();
          return;
        }
        unlistenStatus = fn;
      });

    return () => {
      resizeCleanup?.(); // Clean up resize drag if component unmounts mid-drag
      unlistenStatus?.();
      unlistenDesktopResize?.();
      unlistenDesktopFocus?.();
      clearInterval(interval);
      clearInterval(teamPollInterval);
      if (debounceTimer) clearTimeout(debounceTimer);
      destroyed = true;
      unlistenTeam?.();
      unlistenTask?.();
      retryTimers.forEach(clearTimeout);
      if (resyncTimer) clearTimeout(resyncTimer);
      keybindingStore.unregisterCallback("app:toggleSidebar");
      keybindingStore.unregisterCallback("app:commandPalette");
      keybindingStore.unregisterCallback("app:newChat");
      window.removeEventListener("keydown", onShortcutHintKeydown, true);
      window.removeEventListener("keyup", onShortcutHintKeyup, true);
      window.removeEventListener("blur", clearShortcutHints);
      document.removeEventListener("visibilitychange", onVisibilityChange);
      window.removeEventListener("ocv:runs-changed", onRunsChanged);
      window.removeEventListener("ocv:favorites-changed", onFavoritesChanged);
      window.removeEventListener("ocv:show-wizard", onShowWizard);
      window.removeEventListener("ocv:cwd-changed", handleCwdChanged);
      window.removeEventListener("ocv:memory-file-selected", onMemoryFileSelected);
      window.removeEventListener("ocv:memory-file-saved", onMemoryFileSaved);
      window.removeEventListener("ocv:open-permissions", onOpenPermissions);
      document.removeEventListener("pointerdown", onConversationMenuPointerDown);
      document.removeEventListener("pointerdown", handleWindowDragPointerDown, true);
      document.removeEventListener("keydown", onConversationMenuKeydown);
      document.removeEventListener("scroll", onConversationMenuScroll, true);
      document.removeEventListener("click", handleExternalLink, true);
      window.removeEventListener("resize", syncDesktopWindowChrome);
      window.removeEventListener("ocv:explorer-file-selected", onExplorerFileSelected);
    };
  });

  // Save CWD to localStorage when changed (clear key for "All Projects")
  // Also pin manually-selected folders so they persist in the project list
  $effect(() => {
    if (typeof window !== "undefined") {
      if (projectCwd) {
        localStorage.setItem("ocv:project-cwd", projectCwd);
        // Pin this cwd so it stays in the dropdown after switching away
        if (projectCwd !== "/" && !pinnedCwds.includes(projectCwd)) {
          pinnedCwds = [...pinnedCwds, projectCwd];
          localStorage.setItem("ocv:pinned-cwds", JSON.stringify(pinnedCwds));
        }
      } else {
        localStorage.removeItem("ocv:project-cwd");
      }
      // Notify child pages (e.g. Memory) that project cwd changed
      window.dispatchEvent(new CustomEvent("ocv:project-changed", { detail: { cwd: projectCwd } }));
    }
  });

  afterNavigate(({ to }) => {
    dbg("layout", "navigated to:", to?.url.pathname);
    // Auto-switch sidebar tab when navigating to /teams
    if (to?.url.pathname === "/teams") panelTab = "teams";
    // Sync plugin section from URL when navigating to /plugins
    if (to?.url.pathname.startsWith("/plugins")) {
      const section = to.url.searchParams.get("section");
      if (section && pluginSections.some((s) => s.id === section)) {
        pluginActiveSection = section;
        modeMoreOpen = ["mcp", "hooks", "agents"].includes(section);
      }
    }
  });

  // Catch unhandled errors that could break the router
  onMount(() => {
    function onError(e: ErrorEvent) {
      dbgWarn("layout", "global error", e.message, e.filename, e.lineno);
    }
    function onRejection(e: PromiseRejectionEvent) {
      dbgWarn("layout", "unhandled rejection", e.reason);
      // Don't call e.preventDefault() — let rejections surface in devtools
    }
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);
    return () => {
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onRejection);
    };
  });

  // Get selected run from URL
  let selectedRunId = $derived.by(() => {
    const url = $page.url;
    return url.searchParams.get("run") ?? "";
  });

  // ── Conversation context menu ──
  const PINNED_CONVERSATIONS_KEY = "helion:pinned-conversations";
  const ARCHIVED_CONVERSATIONS_KEY = "helion:archived-conversations";
  const UNREAD_CONVERSATIONS_KEY = "helion:unread-conversations";

  type ConversationMenuState = {
    open: boolean;
    x: number;
    y: number;
    conv: ConversationGroup | null;
  };

  let conversationMenu = $state<ConversationMenuState>({
    open: false,
    x: 0,
    y: 0,
    conv: null,
  });
  let pinnedConversationKeys = $state<string[]>([]);
  let archivedConversationKeys = $state<string[]>([]);
  let unreadConversationKeys = $state<string[]>([]);
  let renameConversationOpen = $state(false);
  let renameConversationTarget: ConversationGroup | null = $state(null);
  let renameConversationValue = $state("");
  let renameConversationSaving = $state(false);
  let renameConversationError = $state("");
  let renameConversationInput: HTMLInputElement | null = $state(null);
  let conversationToast = $state("");
  let conversationToastTimer: ReturnType<typeof setTimeout> | null = null;

  function readStringArray(key: string): string[] {
    if (typeof window === "undefined") return [];
    try {
      const parsed = JSON.parse(localStorage.getItem(key) ?? "[]");
      return Array.isArray(parsed) && parsed.every((v: unknown) => typeof v === "string")
        ? parsed
        : [];
    } catch {
      return [];
    }
  }

  function persistStringArray(key: string, values: string[]) {
    if (typeof window === "undefined") return;
    localStorage.setItem(key, JSON.stringify(values));
  }

  function conversationStorageKey(conv: ConversationGroup): string {
    return `${conv.groupKey}::${normalizeCwd(conv.latestRun.cwd)}`;
  }

  function isConversationPinned(conv: ConversationGroup): boolean {
    return pinnedConversationKeys.includes(conversationStorageKey(conv));
  }

  function isConversationArchived(conv: ConversationGroup): boolean {
    return archivedConversationKeys.includes(conversationStorageKey(conv));
  }

  function isConversationUnread(conv: ConversationGroup): boolean {
    return unreadConversationKeys.includes(conversationStorageKey(conv));
  }

  function getConversationSessionId(conv: ConversationGroup): string {
    return conv.latestRun.session_id || conv.runs.find((r) => r.session_id)?.session_id || "";
  }

  function clampConversationMenuPosition(x: number, y: number) {
    if (typeof window === "undefined") return { x, y };
    const margin = 8;
    const width = 252;
    const height = 386;
    return {
      x: Math.max(margin, Math.min(x, window.innerWidth - width - margin)),
      y: Math.max(margin, Math.min(y, window.innerHeight - height - margin)),
    };
  }

  function openConversationMenuAt(x: number, y: number, conv: ConversationGroup) {
    const pos = clampConversationMenuPosition(x, y);
    conversationMenu = { open: true, x: pos.x, y: pos.y, conv };
  }

  function openConversationContextMenu(event: MouseEvent, conv: ConversationGroup) {
    event.preventDefault();
    event.stopPropagation();
    openConversationMenuAt(event.clientX, event.clientY, conv);
  }

  function openConversationActionMenu(event: MouseEvent | KeyboardEvent, conv: ConversationGroup) {
    event.preventDefault();
    event.stopPropagation();
    const el = event.currentTarget as HTMLElement;
    const rect = el.getBoundingClientRect();
    openConversationMenuAt(rect.right - 252, rect.bottom + 4, conv);
  }

  function closeConversationMenu() {
    conversationMenu = { open: false, x: 0, y: 0, conv: null };
  }

  function showConversationToast(message: string) {
    conversationToast = message;
    if (conversationToastTimer) clearTimeout(conversationToastTimer);
    conversationToastTimer = setTimeout(() => {
      conversationToast = "";
      conversationToastTimer = null;
    }, 1400);
  }

  async function copyConversationText(text: string) {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      showConversationToast(t("common_copied"));
    } catch (e) {
      dbgWarn("layout", "copy conversation menu text failed", e);
    }
  }

  function cleanupConversationLocalState(conv: ConversationGroup) {
    const key = conversationStorageKey(conv);
    pinnedConversationKeys = pinnedConversationKeys.filter((k) => k !== key);
    archivedConversationKeys = archivedConversationKeys.filter((k) => k !== key);
    unreadConversationKeys = unreadConversationKeys.filter((k) => k !== key);
    persistStringArray(PINNED_CONVERSATIONS_KEY, pinnedConversationKeys);
    persistStringArray(ARCHIVED_CONVERSATIONS_KEY, archivedConversationKeys);
    persistStringArray(UNREAD_CONVERSATIONS_KEY, unreadConversationKeys);
  }

  function openConversation(conv: ConversationGroup) {
    const key = conversationStorageKey(conv);
    if (unreadConversationKeys.includes(key)) {
      unreadConversationKeys = unreadConversationKeys.filter((k) => k !== key);
      persistStringArray(UNREAD_CONVERSATIONS_KEY, unreadConversationKeys);
    }
    closeConversationMenu();
    goto(`/chat?run=${conv.latestRun.id}`);
  }

  function togglePinConversation(conv: ConversationGroup) {
    const key = conversationStorageKey(conv);
    pinnedConversationKeys = pinnedConversationKeys.includes(key)
      ? pinnedConversationKeys.filter((k) => k !== key)
      : [key, ...pinnedConversationKeys];
    persistStringArray(PINNED_CONVERSATIONS_KEY, pinnedConversationKeys);
    closeConversationMenu();
  }

  function startRenameConversation(conv: ConversationGroup) {
    renameConversationTarget = conv;
    renameConversationValue = conv.title;
    renameConversationError = "";
    renameConversationOpen = true;
    closeConversationMenu();
    tick().then(() => {
      renameConversationInput?.focus();
      renameConversationInput?.select();
    });
  }

  async function confirmRenameConversation() {
    const conv = renameConversationTarget;
    const name = renameConversationValue.trim();
    if (!conv || renameConversationSaving) return;
    if (!name) {
      renameConversationError = currentLocale().startsWith("zh")
        ? "请输入对话名称"
        : "Please enter a conversation name";
      return;
    }
    renameConversationSaving = true;
    renameConversationError = "";
    try {
      await renameRun(conv.latestRun.id, name);
      renameConversationOpen = false;
      renameConversationTarget = null;
      await loadRuns();
      window.dispatchEvent(new Event("ocv:runs-changed"));
    } catch (e) {
      renameConversationError = String((e as Error)?.message ?? e);
      dbgWarn("layout", "renameConversation failed", e);
    } finally {
      renameConversationSaving = false;
    }
  }

  function cancelRenameConversation() {
    renameConversationOpen = false;
    renameConversationTarget = null;
    renameConversationError = "";
  }

  function archiveConversation(conv: ConversationGroup) {
    const key = conversationStorageKey(conv);
    if (!archivedConversationKeys.includes(key)) {
      archivedConversationKeys = [key, ...archivedConversationKeys];
      persistStringArray(ARCHIVED_CONVERSATIONS_KEY, archivedConversationKeys);
    }
    closeConversationMenu();
    if (conv.runs.some((r) => r.id === selectedRunId)) {
      goto("/chat");
    }
  }

  function toggleUnreadConversation(conv: ConversationGroup) {
    const key = conversationStorageKey(conv);
    unreadConversationKeys = unreadConversationKeys.includes(key)
      ? unreadConversationKeys.filter((k) => k !== key)
      : [key, ...unreadConversationKeys];
    persistStringArray(UNREAD_CONVERSATIONS_KEY, unreadConversationKeys);
    closeConversationMenu();
  }

  async function openConversationInFinder(conv: ConversationGroup) {
    closeConversationMenu();
    const cwd = normalizeCwd(conv.latestRun.cwd);
    if (!cwd) return;
    try {
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(cwd);
    } catch (e) {
      dbgWarn("layout", "open conversation cwd failed", e);
    }
  }

  async function copyConversationCwd(conv: ConversationGroup) {
    closeConversationMenu();
    await copyConversationText(normalizeCwd(conv.latestRun.cwd));
  }

  async function copyConversationSessionId(conv: ConversationGroup) {
    closeConversationMenu();
    await copyConversationText(getConversationSessionId(conv));
  }

  async function copyConversationDeepLink(conv: ConversationGroup) {
    closeConversationMenu();
    const url = new URL("/chat", window.location.origin);
    url.searchParams.set("run", conv.latestRun.id);
    await copyConversationText(url.toString());
  }

  function forkConversation(conv: ConversationGroup) {
    closeConversationMenu();
    goto(`/chat?run=${conv.latestRun.id}&resume=fork`);
  }

  async function openConversationNewWindow(conv: ConversationGroup) {
    closeConversationMenu();
    const href = `/chat?run=${encodeURIComponent(conv.latestRun.id)}`;
    const opened = window.open(href, "_blank", "noopener,noreferrer");
    if (!opened) {
      await goto(href);
    }
  }

  // ── Delete conversation confirm flow ──
  let deleteConfirmOpen = $state(false);
  let deleteTarget: ConversationGroup | null = $state(null);

  function requestDeleteConversation(conv: ConversationGroup) {
    deleteTarget = conv;
    deleteConfirmOpen = true;
  }

  async function confirmDeleteConversation() {
    const conv = deleteTarget;
    deleteConfirmOpen = false;
    deleteTarget = null;
    if (!conv) return;
    try {
      const ids = conv.runs.map((r) => r.id);
      await softDeleteRuns(ids);
      cleanupConversationLocalState(conv);
      dbg("layout", "deleteConversation success", { ids });
      window.dispatchEvent(new Event("ocv:runs-changed"));
      if (conv.runs.some((r) => r.id === selectedRunId)) {
        goto("/chat");
      }
    } catch (e) {
      dbgWarn("layout", "deleteConversation failed", e);
    }
  }

  function cancelDeleteConversation() {
    deleteConfirmOpen = false;
    deleteTarget = null;
  }

  // ── Remove project folder confirm flow ──
  let removeProjectConfirmOpen = $state(false);
  let removeProjectTarget = $state("");

  function persistRemovedCwds() {
    localStorage.setItem("ocv:removed-cwds", JSON.stringify(removedCwds));
  }

  function requestRemoveProject(cwd: string) {
    removeProjectTarget = normalizeCwd(cwd);
    removeProjectConfirmOpen = true;
  }

  function confirmRemoveProject() {
    const normalized = removeProjectTarget;
    removeProjectConfirmOpen = false;
    removeProjectTarget = "";
    if (!normalized) return;

    // Add to removedCwds
    if (!removedCwds.includes(normalized)) {
      removedCwds = [...removedCwds, normalized];
      persistRemovedCwds();
    }

    // Remove from pinnedCwds (compare normalized)
    const newPinned = pinnedCwds.filter((c) => normalizeCwd(c) !== normalized);
    if (newPinned.length !== pinnedCwds.length) {
      pinnedCwds = newPinned;
      localStorage.setItem("ocv:pinned-cwds", JSON.stringify(pinnedCwds));
    }

    // If currently viewing this project, switch to All Projects
    if (normalizeCwd(projectCwd) === normalized) {
      projectCwd = "";
    }

    dbg("layout", "removeProject", { cwd: normalized });
  }

  function cancelRemoveProject() {
    removeProjectConfirmOpen = false;
    removeProjectTarget = "";
  }

  // Build project folder tree for chats tab
  let projectFolders = $derived.by(() =>
    buildProjectFolders(runs, favoriteRunIds, pinnedCwds, removedCwds),
  );

  let sidebarProjectFolders = $derived.by(() =>
    projectFolders
      .map((folder) => {
        const conversations = folder.conversations
          .filter((conv) => !isConversationArchived(conv))
          .sort((a, b) => {
            const ap = isConversationPinned(a) ? 1 : 0;
            const bp = isConversationPinned(b) ? 1 : 0;
            if (ap !== bp) return bp - ap;
            const at = new Date(a.latestRun.last_activity_at ?? a.latestRun.started_at).getTime();
            const bt = new Date(b.latestRun.last_activity_at ?? b.latestRun.started_at).getTime();
            return bt - at;
          });
        return {
          ...folder,
          conversations,
          conversationCount: conversations.length,
          latestActivityAt:
            conversations[0]?.latestRun.last_activity_at ??
            conversations[0]?.latestRun.started_at ??
            folder.latestActivityAt,
        };
      })
      .filter((folder) => !folder.isUncategorized || folder.conversationCount > 0),
  );
  const workspaceShortcutFolders = $derived(
    sidebarProjectFolders.filter((folder) => !folder.isUncategorized).slice(0, 9),
  );
  const workspaceShortcutIndexes = $derived.by(
    () =>
      new Map(
        workspaceShortcutFolders.map((folder, index) => [folder.folderKey, index + 1] as const),
      ),
  );
  let workspaceShortcutHintsVisible = $state(false);

  function usesMetaWorkspaceShortcuts(): boolean {
    if (isMacDesktop) return true;
    if (isDesktopApp) return false;
    if (typeof navigator === "undefined") return false;
    return /mac/i.test(`${navigator.platform ?? ""} ${navigator.userAgent ?? ""}`);
  }

  function updateWorkspaceShortcutHints(e?: KeyboardEvent) {
    if (!e) {
      workspaceShortcutHintsVisible = false;
      return;
    }
    workspaceShortcutHintsVisible = usesMetaWorkspaceShortcuts() ? e.metaKey : e.ctrlKey;
  }

  // Selectable folders: real project folders (exclude Uncategorized)
  const selectableFolders = $derived(projectFolders.filter((f) => !f.isUncategorized));

  // Removed cwd set for O(1) lookup in search filtering
  let removedCwdSet = $derived(new Set(removedCwds.map(normalizeCwd)));

  // Filter search results to exclude removed project cwds
  let visibleSearchResults = $derived.by(() => {
    if (removedCwdSet.size === 0) return searchResults;
    // Build runId→cwd mapping from runs
    const runCwdMap = new Map<string, string>();
    for (const run of runs) {
      runCwdMap.set(run.id, normalizeCwd(run.cwd));
    }
    return searchResults.filter((result) => {
      const cwd = runCwdMap.get(result.runId);
      // Unknown runId (not in runs yet) → show by default (avoid async timing issues)
      if (cwd === undefined) return true;
      // "" = Uncategorized → always show
      if (!cwd) return true;
      return !removedCwdSet.has(cwd);
    });
  });

  // Debug log when folder tree rebuilds
  $effect(() => {
    dbg("layout", "folders rebuilt", {
      count: projectFolders.length,
      total: projectFolders.reduce((s, f) => s + f.conversationCount, 0),
    });
  });

  // Defensive fallback: reset projectCwd if it's no longer in selectable folders
  $effect(() => {
    if (!projectCwd) return; // "" is always valid (All Projects)
    const validCwds = new Set(selectableFolders.map((f) => f.cwd));
    if (!validCwds.has(projectCwd)) {
      dbg("layout", "projectCwd not in selectable folders, resetting", { projectCwd });
      projectCwd = "";
    }
  });

  // Current page detection
  let currentPath = $derived($page.url.pathname);
  let isChatPage = $derived(currentPath === "/chat" || currentPath === "/");
  let isPluginsPage = $derived(currentPath.startsWith("/plugins"));
  let isExplorerPage = $derived(currentPath.startsWith("/explorer"));
  let isMemoryPage = $derived(currentPath.startsWith("/memory"));

  // Plugin sidebar navigation (shown when on /plugins route)
  const pluginSections = [
    { id: "skills", label: () => t("sidebar_skills"), icon: "sparkles" },
    { id: "mcp", label: () => t("sidebar_mcpServers"), icon: "server" },
    { id: "hooks", label: () => t("sidebar_hooks"), icon: "webhook" },
    { id: "plugins", label: () => t("sidebar_plugins"), icon: "package" },
    { id: "agents", label: () => t("sidebar_agents"), icon: "agents" },
  ];

  let pluginActiveSection = $state<string>("skills");
  setContext("pluginSection", {
    get active() {
      return pluginActiveSection;
    },
    set active(v: string) {
      pluginActiveSection = v;
    },
  });

  // Breadcrumb for non-chat pages
  let pageName = $derived.by(() => {
    const nav = navItems.find((n) => currentPath.startsWith(n.path));
    if (nav) return nav.label();
    if (currentPath.startsWith("/release-notes")) return t("release_cliChangelog");
    return t("layout_appName");
  });

  function newChat() {
    goto("/chat");
  }

  function navigateBack() {
    window.history.back();
  }

  function navigateForward() {
    window.history.forward();
  }

  function switchToProjectFolder(cwd: string) {
    const normalized = normalizeCwd(cwd);
    if (!normalized) {
      newChat();
      return;
    }
    projectCwd = normalized;
    panelTab = "chats";
    expandedProjects =
      expandForProjectChange(`cwd:${normalized}`, expandedProjects) ?? expandedProjects;
    goto(`/chat?folder=${encodeURIComponent(normalized)}`);
  }

  function newChatInFolder(cwd: string) {
    switchToProjectFolder(cwd);
  }

  function handleWorkspaceShortcut(e: KeyboardEvent): boolean {
    const match = /^Cmd\+([1-9])$/.exec(normalizeKeyEvent(e));
    if (!match) return false;
    const folder = workspaceShortcutFolders[Number(match[1]) - 1];
    if (!folder) return false;
    e.preventDefault();
    e.stopPropagation();
    switchToProjectFolder(folder.cwd);
    return true;
  }

  function openConversationByRunId(runId: string) {
    const conv = projectFolders
      .flatMap((folder) => folder.conversations)
      .find((candidate) => candidate.runs.some((run) => run.id === runId));
    if (conv) {
      openConversation(conv);
      return;
    }
    goto(`/chat?run=${encodeURIComponent(runId)}`);
  }

  function resumeConversation(runId: string, mode: "resume") {
    goto(`/chat?run=${encodeURIComponent(runId)}&resume=${mode}`);
  }

  function toggleProject(folderKey: string) {
    const next = new Set(expandedProjects);
    if (next.has(folderKey)) next.delete(folderKey);
    else next.add(folderKey);
    expandedProjects = next;
  }

  // ── Folder picker (sidebar "+ Open folder") ──
  let folderPickerOpen = $state(false);
  let folderPickerInitialHost = $state<string | null>(null);
  let folderPickerInitialPath = $state("");
  let folderPickerStartChatOnPick = $state(true);

  async function pickLocalFolderNative(
    initialPath = "",
  ): Promise<{ path: string | null; failed: boolean }> {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        directory: true,
        title: t("layout_selectProjectFolder"),
        defaultPath: initialPath || undefined,
      });
      return { path: typeof selected === "string" ? selected : null, failed: false };
    } catch (e) {
      dbgWarn("layout", "native folder picker failed; falling back to FolderPicker", e);
      return { path: null, failed: true };
    }
  }

  async function pickFolder(startNewChat = true) {
    // Pre-fill from last-target so remote-using users don't lose their target.
    // Validate against current settings — a host removed/renamed since the
    // value was persisted should not silently leak through to the picker.
    const lastTarget = getLastTarget();
    const validatedTarget =
      lastTarget && (settings?.remote_hosts ?? []).some((h) => h.name === lastTarget)
        ? lastTarget
        : null;
    if (lastTarget && !validatedTarget) {
      dbgWarn("layout", "lastTarget references unknown remote — falling back to local", {
        lastTarget,
      });
    }
    const initialPath = validatedTarget
      ? getStoredRemoteCwd(validatedTarget)
      : projectCwd || settings?.working_directory || "";
    if (!validatedTarget && getTransport().isDesktop()) {
      const result = await pickLocalFolderNative(initialPath);
      if (result.path) {
        onFolderPicked({ hostName: null, path: result.path }, startNewChat);
        return;
      }
      if (!result.failed) return;
    }
    folderPickerInitialHost = validatedTarget;
    folderPickerInitialPath = initialPath;
    folderPickerStartChatOnPick = startNewChat;
    folderPickerOpen = true;
  }

  function onFolderPicked(
    result: { hostName: string | null; path: string },
    startNewChat = folderPickerStartChatOnPick,
  ) {
    const { hostName, path } = result;
    if (!path) return;
    if (hostName) {
      // Remote: persist and navigate to chat with host+folder
      setStoredRemoteCwd(hostName, path);
      setLastTarget(hostName);
      // Clear local projectCwd so the local file tree doesn't try to list a remote path
      projectCwd = "";
      dbg("layout", "pickFolder (remote)", { hostName, path });
      if (startNewChat) {
        goto(`/chat?host=${encodeURIComponent(hostName)}&folder=${encodeURIComponent(path)}`);
      }
    } else {
      // Local target
      const normalized = normalizeCwd(path) || "";
      if (normalized && removedCwds.includes(normalized)) {
        removedCwds = removedCwds.filter((c) => c !== normalized);
        persistRemovedCwds();
        dbg("layout", "pickFolder: un-removed cwd", { cwd: normalized });
      }
      projectCwd = normalized;
      setLastTarget(null);
      if (normalized && startNewChat) {
        goto(`/chat?folder=${encodeURIComponent(normalized)}`);
      }
    }
  }

  function toggleSidebar() {
    sidebarOpen = !sidebarOpen;
  }

  setContext("toggleSidebar", toggleSidebar);
  setContext("layoutSidebar", {
    get open() {
      return sidebarOpen;
    },
    toggle: toggleSidebar,
  });

  function cycleTheme() {
    const order: ThemeMode[] = ["light", "dark", "system"];
    const idx = order.indexOf(themeMode);
    themeMode = order[(idx + 1) % order.length];
    dbg("layout", "theme cycled", { themeMode, effectiveDark });
  }

  function setTheme(mode: ThemeMode) {
    themeMode = mode;
    themeMenuOpen = false;
    dbg("layout", "theme selected", { themeMode, effectiveDark });
  }

  function cycleScheme() {
    colorScheme = colorScheme === "warm" ? "neutral" : "warm";
    dbg("layout", "color scheme cycled", { colorScheme });
  }

  // Persist theme + apply class
  $effect(() => {
    localStorage.setItem("helion:theme", themeMode);
    localStorage.setItem("ocv:theme", themeMode);
    document.documentElement.classList.toggle("dark", effectiveDark);
    void applyDesktopWindowEffects();
  });

  // Persist color scheme + apply class
  $effect(() => {
    localStorage.setItem("ocv:colorScheme", colorScheme);
    document.documentElement.classList.toggle("scheme-neutral", colorScheme === "neutral");
  });

  // Auto-expand folder containing selected run (chats tab only)
  // Track runId + runs.length as change signals. runs.length is the most
  // reliable: it changes on any new run (including resume into existing
  // session where conversationCount stays the same).
  // Don't track expandedProjects itself (otherwise collapsing re-expands).
  let _prevAutoExpandRunId = "";
  let _prevAutoExpandRunsLen = 0;
  $effect(() => {
    if (!isChatPage || panelTab !== "chats") return;
    const runId = selectedRunId;
    const runsLen = runs.length;
    const runChanged = runId !== _prevAutoExpandRunId;
    const runsChanged = runsLen !== _prevAutoExpandRunsLen;
    if (!runChanged && !runsChanged) return; // early-return avoids tracking expandedProjects
    _prevAutoExpandRunId = runId;
    _prevAutoExpandRunsLen = runsLen;
    if (!runId) return;
    const next = autoExpandForRun(runId, projectFolders, expandedProjects);
    if (next) {
      dbg("layout", "auto-expand for run", { selectedRunId: runId });
      expandedProjects = next;
    }
  });

  // Auto-expand folder matching projectCwd (cross-tab sync)
  let _prevAutoExpandCwd = "";
  $effect(() => {
    const cwd = projectCwd;
    if (cwd === _prevAutoExpandCwd) return;
    _prevAutoExpandCwd = cwd;
    if (!cwd) return;
    const folderKey = `cwd:${cwd}`;
    const next = expandForProjectChange(folderKey, expandedProjects);
    if (next) {
      dbg("layout", "auto-expand for cwd change", { cwd });
      expandedProjects = next;
    }
  });

  // Persist expandedProjects + prune stale keys (only after first successful load)
  $effect(() => {
    if (!runsLoadSucceededOnce) return;
    const validKeys = new Set(projectFolders.map((f) => f.folderKey));
    const pruned = [...expandedProjects].filter((k) => validKeys.has(k));
    if (pruned.length !== expandedProjects.size) {
      expandedProjects = new Set(pruned);
    }
    localStorage.setItem("ocv:expanded-projects", JSON.stringify(pruned));
  });

  // Note: <html lang> is set by initLocale() and switchLocale() directly.

  // Listen for system preference changes
  onMount(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    function onSystemChange(e: MediaQueryListEvent) {
      systemDark = e.matches;
    }
    mq.addEventListener("change", onSystemChange);
    // Apply initial theme
    document.documentElement.classList.toggle("dark", effectiveDark);
    return () => mq.removeEventListener("change", onSystemChange);
  });

  function handleKeydown(e: KeyboardEvent) {
    if (handleWorkspaceShortcut(e)) return;
    keybindingStore.dispatch(e);
  }
</script>

{#snippet treeNodes(nodes: TreeNode[])}
  {#each nodes as node}
    <button
      class="flex w-full items-center gap-1 py-0.5 text-[13px] transition-colors
        text-sidebar-foreground hover:bg-sidebar-accent/50
        {explorerSelectedFile === node.fullPath ? 'bg-sidebar-accent/70' : ''}"
      style="padding-left: {8 + node.depth * 12}px"
      onclick={() => (node.is_dir ? toggleFolder(node) : selectFile(node))}
    >
      {#if node.is_dir}
        <svg
          class="h-3 w-3 shrink-0 transition-transform duration-150 {node.expanded
            ? 'rotate-90'
            : ''}"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"><path d="m9 18 6-6-6-6" /></svg
        >
        <svg
          class="h-3.5 w-3.5 shrink-0 text-blue-400/70"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          ><path
            d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"
          /></svg
        >
      {:else}
        <span class="w-3 shrink-0"></span>
        <svg
          class="h-3.5 w-3.5 shrink-0 opacity-40"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          ><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" /><path
            d="M14 2v4a2 2 0 0 0 2 2h4"
          /></svg
        >
      {/if}
      <span class="min-w-0 truncate">{node.name}</span>
    </button>
    {#if node.is_dir && node.expanded}
      {@render treeNodes(node.children)}
    {/if}
  {/each}
{/snippet}

<svelte:window onkeydown={handleKeydown} />

{#if isDesktopApp}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="desktop-window-control-strip"
    data-window-control-strip
    role="presentation"
    ondblclick={toggleWindowMaximize}
    title="Drag window"
  ></div>
{/if}

<div
  class="app-shell relative flex h-screen gap-0 overflow-hidden {isGlassDesktop
    ? 'bg-transparent'
    : 'bg-background'}"
>
  <!-- Claude Code-style single sidebar panel -->
  <aside
    class="relative z-10 flex shrink-0 overflow-hidden border-r border-sidebar-border text-sidebar-foreground transition-[width,opacity,transform] duration-200 ease-out {isGlassDesktop
        ? 'desktop-sidebar-glass'
        : 'bg-sidebar'} {sidebarOpen ? 'opacity-100' : 'pointer-events-none -translate-x-2 opacity-0'}"
    style:width="{sidebarOpen ? sidebarWidth : 0}px"
    aria-hidden={!sidebarOpen}
  >
      <!-- A. Icon Rail -->
      <div class="hidden w-[42px] flex-col items-center border-r border-sidebar-border bg-sidebar">
        <!-- Rail logo (OC) -->
        <div class="flex h-12 w-full items-center justify-center border-b border-sidebar-border">
          <img src="/logo.png?v=2" alt="HelionCoder" class="h-7 w-7 rounded-md" />
        </div>

        <!-- Rail nav icons -->
        <nav class="flex flex-1 flex-col items-center gap-1 py-2">
          {#each navItems as item}
            {@const isActive = currentPath.startsWith(item.path)}
            <a
              href={item.path}
              class="relative flex h-8 w-8 items-center justify-center rounded-md transition-colors duration-150 no-underline
                {isActive
                ? 'bg-sidebar-accent text-sidebar-accent-foreground'
                : 'hover:bg-sidebar-accent/50 text-sidebar-foreground'}"
              title={item.label()}
            >
              <!-- Active indicator bar -->
              {#if isActive}
                <span class="absolute left-0 top-1.5 h-5 w-[2px] rounded-r-full bg-primary"></span>
              {/if}
              {#if item.icon === "message"}
                <svg
                  class="h-[18px] w-[18px]"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"><path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z" /></svg
                >
              {:else if item.icon === "folder"}
                <svg
                  class="h-[18px] w-[18px]"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  ><path
                    d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"
                  /></svg
                >
              {:else if item.icon === "zap"}
                <svg
                  class="h-[18px] w-[18px]"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  ><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" /></svg
                >
              {:else if item.icon === "book"}
                <svg
                  class="h-[18px] w-[18px]"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  ><path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H20v20H6.5a2.5 2.5 0 0 1 0-5H20" /></svg
                >
              {:else if item.icon === "chart"}
                <svg
                  class="h-[18px] w-[18px]"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"><path d="M3 3v18h18" /><path d="m19 9-5 5-4-4-3 3" /></svg
                >
              {:else if item.icon === "clock"}
                <svg
                  class="h-[18px] w-[18px]"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  ><circle cx="12" cy="12" r="10" /><polyline points="12 6 12 12 16 14" /></svg
                >
              {:else if item.icon === "settings"}
                <svg
                  class="h-[18px] w-[18px]"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  ><path
                    d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"
                  /><circle cx="12" cy="12" r="3" /></svg
                >
              {/if}
              <span class="sr-only">{item.label()}</span>
            </a>
          {/each}
        </nav>

        <!-- Rail version + locale + dark mode toggle -->
        <div class="border-t border-sidebar-border py-2">
          <div class="flex items-center justify-center pb-1">
            <button
              class="text-xs text-muted-foreground hover:text-muted-foreground transition-colors cursor-pointer"
              onclick={() => (showAbout = true)}
              title="About HelionCoder">v0.1</button
            >
          </div>
          <div class="relative mx-auto mb-0.5">
            <button
              class="flex h-8 w-8 items-center justify-center rounded-md text-sidebar-foreground hover:bg-sidebar-accent/50 transition-colors duration-150"
              onclick={() => (localePopupOpen = !localePopupOpen)}
              title={currentLocale()}
            >
              <span class="text-xs font-medium"
                >{getEntry(currentLocale())?.shortLabel ?? currentLocale()}</span
              >
            </button>
            {#if localePopupOpen}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="fixed inset-0 z-40"
                onclick={() => (localePopupOpen = false)}
                onkeydown={(e) => e.key === "Escape" && (localePopupOpen = false)}
              ></div>
              <div
                class="absolute bottom-0 left-full ml-1 z-50 min-w-[140px] rounded-md border border-sidebar-border bg-popover py-1 shadow-lg"
              >
                {#each LOCALE_REGISTRY as entry}
                  <button
                    class="flex w-full items-center gap-2 px-3 py-1.5 text-xs transition-colors
                      {currentLocale() === entry.code
                      ? 'bg-accent text-accent-foreground'
                      : 'text-popover-foreground hover:bg-accent/50'}"
                    onclick={() => handleLocaleSelect(entry.code)}
                  >
                    <span class="w-5 text-center font-medium">{entry.shortLabel}</span>
                    <span>{entry.nativeName}</span>
                    {#if (entry.status as string) === "beta"}
                      <span
                        class="ml-auto text-[10px] text-muted-foreground/60 border border-muted-foreground/20 rounded px-1"
                        >Beta</span
                      >
                    {/if}
                  </button>
                {/each}
              </div>
            {/if}
          </div>
          <button
            class="flex h-8 w-8 items-center justify-center rounded-md text-sidebar-foreground hover:bg-sidebar-accent/50 transition-colors duration-150"
            onclick={cycleTheme}
            title={themeMode === "dark"
              ? t("layout_themeTitle_dark")
              : themeMode === "light"
                ? t("layout_themeTitle_light")
                : t("layout_themeTitle_system")}
          >
            {#if themeMode === "dark"}
              <!-- Moon icon (dark mode active) -->
              <svg
                class="h-[18px] w-[18px]"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" /></svg
              >
            {:else if themeMode === "light"}
              <!-- Sun icon (light mode active) -->
              <svg
                class="h-[18px] w-[18px]"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                ><circle cx="12" cy="12" r="4" /><path
                  d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"
                /></svg
              >
            {:else}
              <!-- Monitor icon (system mode active) -->
              <svg
                class="h-[18px] w-[18px]"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                ><rect width="20" height="14" x="2" y="3" rx="2" /><line
                  x1="8"
                  x2="16"
                  y1="21"
                  y2="21"
                /><line x1="12" x2="12" y1="17" y2="21" /></svg
              >
            {/if}
          </button>
          <button
            class="flex h-8 w-8 items-center justify-center rounded-md text-sidebar-foreground hover:bg-sidebar-accent/50 transition-colors duration-150"
            onclick={cycleScheme}
            title={colorScheme === "warm"
              ? t("layout_schemeTitle_warm")
              : t("layout_schemeTitle_neutral")}
          >
            <!-- Palette icon -->
            <svg
              class="h-[18px] w-[18px]"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              ><circle cx="13.5" cy="6.5" r=".5" fill="currentColor" /><circle
                cx="17.5"
                cy="10.5"
                r=".5"
                fill="currentColor"
              /><circle cx="8.5" cy="7.5" r=".5" fill="currentColor" /><circle
                cx="6.5"
                cy="12"
                r=".5"
                fill="currentColor"
              /><path
                d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.926 0 1.648-.746 1.648-1.688 0-.437-.18-.835-.437-1.125-.29-.289-.438-.652-.438-1.125a1.64 1.64 0 0 1 1.668-1.668h1.996c3.051 0 5.555-2.503 5.555-5.554C21.965 6.012 17.461 2 12 2z"
              /></svg
            >
          </button>
        </div>
      </div>

      <!-- B. Content Panel -->
      <div class="relative flex flex-none flex-col overflow-hidden" style:width="{sidebarWidth}px">
        <!-- Top chrome: menu / sidebar / search, matching Claude Code's compact rail. -->
        <div
          class="titlebar-drag flex h-12 items-center gap-1.5 {reserveMacTrafficLights
            ? 'pl-[92px] pr-3'
            : 'px-3'}"
          data-tauri-drag-region
        >
          <div class="flex items-center gap-1.5 {reserveMacTrafficLights ? 'mac-titlebar-control-strip' : ''}">
            {#if !isDesktopApp}
              <button
                class="titlebar-no-drag flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-sidebar-accent/60 hover:text-sidebar-foreground transition-colors"
                onclick={() => (commandPaletteOpen = true)}
                title="Menu"
              >
                <svg
                  class="h-4 w-4"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"><path d="M4 7h16" /><path d="M4 12h16" /><path d="M4 17h16" /></svg
                >
              </button>
            {/if}
            <button
              class="titlebar-no-drag flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-sidebar-accent/60 hover:text-sidebar-foreground transition-colors"
              onclick={toggleSidebar}
              title={t("layout_toggleSidebar")}
            >
              <svg
                class="h-4 w-4"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.8"
                stroke-linecap="round"
                stroke-linejoin="round"
                ><rect width="16" height="14" x="4" y="5" rx="2" /><path d="M10 5v14" /></svg
              >
            </button>
            <button
              class="titlebar-no-drag flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-sidebar-accent/60 hover:text-sidebar-foreground transition-colors"
              onclick={() => {
                searchPanelOpen = !searchPanelOpen;
                if (!searchPanelOpen) runSearchQuery = "";
              }}
              title={t("sidebar_searchChats")}
            >
              <svg
                class="h-4 w-4"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                ><circle cx="11" cy="11" r="7" /><path d="m21 21-4.3-4.3" /></svg
              >
            </button>
          </div>
        </div>

        <div class="px-2 pb-2">
          <div class="grid grid-cols-3 gap-1 rounded-lg bg-sidebar-accent/30 p-1">
            {#each appModeItems as mode}
              <button
                type="button"
                class="flex h-8 items-center justify-center gap-1 rounded-md text-[12px] font-medium transition-colors
                  {appMode === mode.id
                  ? 'bg-background text-foreground shadow-sm'
                  : 'text-muted-foreground hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'}"
                onclick={() => selectAppMode(mode.id)}
                title={mode.title()}
              >
                {#if mode.id === "chat"}
                  <svg
                    class="h-3.5 w-3.5"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"><path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z" /></svg
                  >
                {:else if mode.id === "cowork"}
                  <svg
                    class="h-3.5 w-3.5"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><path d="m9 11 3 3L22 4" /><path
                      d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"
                    /></svg
                  >
                {:else}
                  <svg
                    class="h-3.5 w-3.5"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"><path d="m8 9-4 3 4 3" /><path d="m16 9 4 3-4 3" /></svg
                  >
                {/if}
                <span>{mode.label()}</span>
              </button>
            {/each}
          </div>
        </div>

        {#if isExplorerPage}
          <!-- Explorer tab bar: Files / Git -->
          <div class="flex shrink-0 border-b border-sidebar-border">
            <button
              class="flex-1 py-1.5 text-xs font-medium text-center transition-colors
              {explorerTab === 'files'
                ? 'text-sidebar-foreground border-b-2 border-primary'
                : 'text-muted-foreground hover:text-sidebar-foreground'}"
              onclick={() => (explorerTab = "files")}>{t("sidebar_files")}</button
            >
            <button
              class="relative flex-1 py-1.5 text-xs font-medium text-center transition-colors
              {explorerTab === 'git'
                ? 'text-sidebar-foreground border-b-2 border-primary'
                : 'text-muted-foreground hover:text-sidebar-foreground'}"
              onclick={() => (explorerTab = "git")}
              >{t("sidebar_git")}
              {#if gitSummary && gitSummary.total_files > 0}
                <span
                  class="ml-0.5 inline-flex h-3.5 min-w-[14px] items-center justify-center rounded-full bg-blue-500/80 px-1 text-[10px] font-bold text-white"
                  >{gitSummary.total_files}</span
                >
              {/if}
            </button>
          </div>

          <!-- Compact project picker (below tabs) -->
          <div class="relative shrink-0 border-b border-sidebar-border">
            <button
              class="flex w-full items-center gap-1.5 px-2.5 py-1.5 text-xs transition-colors hover:bg-sidebar-accent/50"
              onclick={() => (explorerProjectOpen = !explorerProjectOpen)}
            >
              <svg
                class="h-3.5 w-3.5 shrink-0 text-muted-foreground/70"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                ><path
                  d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"
                /></svg
              >
              <span class="min-w-0 truncate text-sidebar-foreground"
                >{projectCwd ? cwdDisplayLabel(projectCwd) : t("sidebar_selectProjectBrowse")}</span
              >
              <svg
                class="ml-auto h-3 w-3 shrink-0 text-muted-foreground/50 transition-transform {explorerProjectOpen
                  ? 'rotate-180'
                  : ''}"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"><path d="m6 9 6 6 6-6" /></svg
              >
            </button>
            {#if explorerProjectOpen}
              <div class="border-b border-sidebar-border bg-sidebar">
                {#each selectableFolders as folder (folder.folderKey)}
                  <button
                    class="flex w-full items-center gap-1.5 px-2.5 py-1.5 text-xs transition-colors
                      {folder.cwd === projectCwd
                      ? 'bg-sidebar-accent text-sidebar-foreground'
                      : 'text-muted-foreground hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'}"
                    onclick={() => {
                      projectCwd = folder.cwd;
                      explorerProjectOpen = false;
                    }}
                  >
                    <svg
                      class="h-3 w-3 shrink-0 text-muted-foreground/70"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      ><path
                        d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"
                      /></svg
                    >
                    <span class="min-w-0 truncate">{cwdDisplayLabel(folder.cwd)}</span>
                  </button>
                {/each}
                <button
                  class="flex w-full items-center gap-1.5 px-2.5 py-1.5 text-xs text-muted-foreground hover:text-sidebar-foreground hover:bg-sidebar-accent/50 transition-colors"
                  onclick={() => {
                    pickFolder(false);
                    explorerProjectOpen = false;
                  }}
                >
                  <svg
                    class="h-3 w-3 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"><path d="M12 5v14" /><path d="M5 12h14" /></svg
                  >
                  <span>{t("project_openFolder")}</span>
                </button>
              </div>
            {/if}
          </div>

          <!-- Explorer tab content -->
          {#if explorerTab === "files"}
            <div class="flex-1 overflow-y-auto px-1 py-1">
              {#if !projectCwd}
                {@const lastRemote = getLastTarget()}
                <div class="flex items-center justify-center px-3 py-12">
                  <p class="text-xs text-muted-foreground text-center">
                    {lastRemote
                      ? t("layout_remoteFileTreeUnavailable")
                      : t("sidebar_selectProjectBrowse")}
                  </p>
                </div>
              {:else if treeLoading}
                <div class="flex items-center justify-center py-12">
                  <div
                    class="h-4 w-4 border-2 border-primary/30 border-t-primary rounded-full animate-spin"
                  ></div>
                </div>
              {:else if fileTree.length === 0}
                <p class="px-2 py-8 text-xs text-muted-foreground text-center">
                  {t("sidebar_emptyDirectory")}
                </p>
              {:else}
                {@render treeNodes(fileTree)}
              {/if}
            </div>
          {:else}
            <!-- Git tab -->
            {#if !projectCwd}
              <div class="flex-1 flex items-center justify-center px-3">
                <p class="text-xs text-muted-foreground text-center">
                  {t("sidebar_selectProjectGit")}
                </p>
              </div>
            {:else if gitLoading}
              <div class="flex-1 flex items-center justify-center">
                <div
                  class="h-4 w-4 border-2 border-primary/30 border-t-primary rounded-full animate-spin"
                ></div>
              </div>
            {:else if !gitSummary}
              <div class="flex-1 flex items-center justify-center px-3">
                <p class="text-xs text-muted-foreground text-center">{t("sidebar_notGitRepo")}</p>
              </div>
            {:else}
              <!-- Branch info -->
              <div
                class="flex items-center gap-1.5 px-3 py-2 border-b border-sidebar-border shrink-0"
              >
                <svg
                  class="h-3 w-3 shrink-0 text-muted-foreground"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  ><circle cx="12" cy="12" r="3" /><line x1="3" x2="9" y1="12" y2="12" /><line
                    x1="15"
                    x2="21"
                    y1="12"
                    y2="12"
                  /></svg
                >
                <span class="text-[12px] font-medium text-sidebar-foreground min-w-0 truncate"
                  >{gitSummary.branch || t("sidebar_detached")}</span
                >
                <button
                  class="ml-auto flex h-5 w-5 items-center justify-center rounded text-muted-foreground hover:text-sidebar-foreground hover:bg-sidebar-accent/50 transition-colors"
                  onclick={loadGitSummary}
                  title={t("sidebar_refresh")}
                >
                  <svg
                    class="h-3 w-3"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" /><path
                      d="M3 3v5h5"
                    /><path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16" /><path
                      d="M16 16h5v5"
                    /></svg
                  >
                </button>
              </div>
              <!-- Summary -->
              {#if gitSummary.total_files > 0}
                <div
                  class="flex items-center gap-2 px-3 py-1.5 text-xs text-muted-foreground border-b border-sidebar-border shrink-0"
                >
                  <span class="tabular-nums"
                    >{gitSummary.total_files !== 1
                      ? t("sidebar_changedFiles", { count: String(gitSummary.total_files) })
                      : t("sidebar_changedFile", { count: String(gitSummary.total_files) })}</span
                  >
                  {#if gitSummary.total_insertions > 0}
                    <span class="text-green-500 tabular-nums">+{gitSummary.total_insertions}</span>
                  {/if}
                  {#if gitSummary.total_deletions > 0}
                    <span class="text-red-400 tabular-nums">-{gitSummary.total_deletions}</span>
                  {/if}
                </div>
                <!-- Changed files list -->
                <div class="flex-1 overflow-y-auto">
                  {#each gitSummary.files as file}
                    <button
                      class="flex w-full items-center gap-1.5 px-3 py-1 text-[12px] hover:bg-sidebar-accent/50 transition-colors"
                      onclick={() => selectDiffFile(file.path)}
                    >
                      <span
                        class="w-3 shrink-0 text-center font-mono text-[10px] font-bold {GIT_STATUS_COLORS[
                          file.status
                        ] ?? 'text-muted-foreground'}">{file.status}</span
                      >
                      <span class="flex-1 min-w-0 truncate text-sidebar-foreground text-left"
                        >{file.path}</span
                      >
                      {#if file.insertions > 0}
                        <span class="text-[10px] text-green-500">+{file.insertions}</span>
                      {/if}
                      {#if file.deletions > 0}
                        <span class="text-[10px] text-red-400">-{file.deletions}</span>
                      {/if}
                    </button>
                  {/each}
                </div>
              {:else}
                <div class="flex-1 flex items-center justify-center px-3">
                  <div class="flex flex-col items-center gap-1.5 text-center">
                    <svg
                      class="h-6 w-6 text-muted-foreground/30"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.5"><path d="M20 6 9 17l-5-5" /></svg
                    >
                    <p class="text-xs text-muted-foreground">{t("sidebar_workingTreeClean")}</p>
                  </div>
                </div>
              {/if}
            {/if}
          {/if}
        {:else if isMemoryPage}
          <!-- Memory file tree -->
          <div class="flex-1 overflow-y-auto py-1">
            <!-- Project folders (accordion: only one expanded at a time) -->
            {#each selectableFolders as folder (folder.folderKey)}
              <ProjectFolderItem
                {folder}
                label={cwdDisplayLabel(folder.cwd)}
                expanded={folder.cwd === projectCwd}
                onToggle={() => {
                  projectCwd = projectCwd === folder.cwd ? "" : folder.cwd;
                }}
              >
                {#if memoryLoading}
                  <div class="flex items-center justify-center py-6">
                    <div
                      class="h-4 w-4 border-2 border-primary/30 border-t-primary rounded-full animate-spin"
                    ></div>
                  </div>
                {:else if memoryScopeFolder.length > 0}
                  {#each filterVisibleCandidates(memoryScopeFolder, true, memorySelectedFile) as file}
                    <button
                      class="flex w-full items-center gap-1.5 py-1 pl-4 pr-3 text-xs transition-colors
                        {memorySelectedFile === file.path
                        ? 'bg-sidebar-accent text-sidebar-foreground'
                        : 'text-muted-foreground hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'}"
                      onclick={() => selectMemoryFile(file)}
                      title={file.path}
                    >
                      <svg
                        class="h-3 w-3 shrink-0 {file.scope === 'memory'
                          ? 'text-amber-400'
                          : file.exists
                            ? 'text-blue-400'
                            : 'text-muted-foreground/40'}"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        ><path
                          d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"
                        /><path d="M14 2v4a2 2 0 0 0 2 2h4" /></svg
                      >
                      <span class="min-w-0 truncate">{file.label}</span>
                      {#if !file.exists}
                        <span class="ml-auto text-[10px] text-muted-foreground shrink-0"
                          >{t("memory_new")}</span
                        >
                      {/if}
                    </button>
                  {/each}
                {:else}
                  <p class="px-2 py-3 text-xs text-muted-foreground">
                    {t("memory_noProjectFiles")}
                  </p>
                {/if}
              </ProjectFolderItem>
            {/each}
            <!-- Global scope (same style as project folders, globe icon) -->
            {#if memoryScopeGlobal.length > 0}
              <div class="mb-0.5">
                <button
                  class="flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-xs font-medium text-sidebar-foreground hover:bg-sidebar-accent/50 transition-colors"
                  onclick={() => toggleMemoryScope("global")}
                >
                  <svg
                    class="h-3 w-3 shrink-0 text-muted-foreground/60 transition-transform duration-150 {memoryScopeExpanded[
                      'global'
                    ]
                      ? 'rotate-90'
                      : ''}"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"><path d="M9 18l6-6-6-6" /></svg
                  >
                  <!-- Globe icon -->
                  <svg
                    class="h-3.5 w-3.5 shrink-0 text-muted-foreground/70"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><circle cx="12" cy="12" r="10" /><path d="M2 12h20" /><path
                      d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"
                    /></svg
                  >
                  <span class="truncate">{t("memory_tabGlobal")}</span>
                </button>
                {#if memoryScopeExpanded["global"]}
                  <div class="pl-3">
                    {#each filterVisibleCandidates(memoryScopeGlobal, true, memorySelectedFile) as file}
                      <button
                        class="flex w-full items-center gap-1.5 py-1 pl-4 pr-3 text-xs transition-colors
                          {memorySelectedFile === file.path
                          ? 'bg-sidebar-accent text-sidebar-foreground'
                          : 'text-muted-foreground hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'}"
                        onclick={() => selectMemoryFile(file)}
                        title={file.path}
                      >
                        <svg
                          class="h-3 w-3 shrink-0 {file.exists
                            ? 'text-blue-400'
                            : 'text-muted-foreground/40'}"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="2"
                          stroke-linecap="round"
                          stroke-linejoin="round"
                          ><path
                            d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"
                          /><path d="M14 2v4a2 2 0 0 0 2 2h4" /></svg
                        >
                        <span class="min-w-0 truncate">{file.label}</span>
                        {#if !file.exists}
                          <span class="ml-auto text-[10px] text-muted-foreground shrink-0"
                            >{t("memory_new")}</span
                          >
                        {/if}
                      </button>
                    {/each}
                  </div>
                {/if}
              </div>
            {/if}

            <!-- Open folder button -->
            <button
              class="flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-xs text-muted-foreground hover:text-sidebar-foreground hover:bg-sidebar-accent/50 transition-colors"
              onclick={() => pickFolder(false)}
            >
              <svg
                class="h-3.5 w-3.5 shrink-0"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"><path d="M12 5v14" /><path d="M5 12h14" /></svg
              >
              <span>+ {t("project_openFolder")}</span>
            </button>
          </div>
        {:else}
          <!-- Mode-specific command group -->
          <div class="shrink-0 space-y-1 px-2 pb-3">
            {#each modeMenuItems as item}
              <button
                type="button"
                class="flex h-8 w-full items-center gap-2 rounded-md px-2.5 text-left text-[13px] text-sidebar-foreground transition-colors hover:bg-sidebar-accent/70 {item.action ===
                  'more' && modeMoreOpen
                  ? 'bg-sidebar-accent/70'
                  : ''}"
                onclick={() => handleModeMenuAction(item.action)}
              >
                {#if item.icon === "plus"}
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"><path d="M12 5v14" /><path d="M5 12h14" /></svg
                  >
                {:else if item.icon === "folder"}
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><path
                      d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"
                    /></svg
                  >
                {:else if item.icon === "calendar"}
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    ><path d="M8 2v4" /><path d="M16 2v4" /><rect
                      width="18"
                      height="18"
                      x="3"
                      y="4"
                      rx="2"
                    /><path d="M3 10h18" /></svg
                  >
                {:else if item.icon === "book"}
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    ><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" /><path d="M4 4v15.5" /><path
                      d="M20 22V6a2 2 0 0 0-2-2H6.5A2.5 2.5 0 0 0 4 6.5v13"
                    /></svg
                  >
                {:else if item.icon === "chart"}
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"><path d="M3 3v18h18" /><path d="m19 9-5 5-4-4-3 3" /></svg
                  >
                {:else if item.icon === "sparkles"}
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    ><path
                      d="M9.94 15.5A2 2 0 0 0 8.5 14.06l-6.14-1.58a.5.5 0 0 1 0-.96L8.5 9.94A2 2 0 0 0 9.94 8.5l1.58-6.14a.5.5 0 0 1 .96 0l1.58 6.14a2 2 0 0 0 1.44 1.44l6.14 1.58a.5.5 0 0 1 0 .96l-6.14 1.58a2 2 0 0 0-1.44 1.44l-1.58 6.14a.5.5 0 0 1-.96 0z"
                    /></svg
                  >
                {:else if item.icon === "package"}
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    ><path d="m7.5 4.27 9 5.15" /><path
                      d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z"
                    /><path d="m3.3 7 8.7 5 8.7-5" /><path d="M12 22V12" /></svg
                  >
                {:else if item.icon === "server"}
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    ><rect x="2" y="3" width="20" height="8" rx="2" /><rect
                      x="2"
                      y="13"
                      width="20"
                      height="8"
                      rx="2"
                    /><path d="M6 7h.01" /><path d="M6 17h.01" /></svg
                  >
                {:else if item.icon === "webhook"}
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    ><path
                      d="M18 16.98h-5.99c-1.1 0-1.95-.94-1.72-2.02.3-1.41 1.31-2.96 3.71-2.96h2"
                    /><circle cx="18" cy="17" r="3" /><circle cx="6" cy="5" r="3" /><circle
                      cx="6"
                      cy="19"
                      r="3"
                    /><path d="M9 5h1a6 6 0 0 1 6 6v1" /></svg
                  >
                {:else if item.icon === "agents"}
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    ><path d="M12 8V4H8" /><rect width="16" height="12" x="4" y="8" rx="2" /><path
                      d="M2 14h2"
                    /><path d="M20 14h2" /><path d="M9 13v2" /><path d="M15 13v2" /></svg
                  >
                {:else if item.icon === "zap"}
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" /></svg
                  >
                {:else if item.icon === "spark"}
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><path
                      d="m12 3-1.9 5.8a2 2 0 0 1-1.3 1.3L3 12l5.8 1.9a2 2 0 0 1 1.3 1.3L12 21l1.9-5.8a2 2 0 0 1 1.3-1.3L21 12l-5.8-1.9a2 2 0 0 1-1.3-1.3L12 3Z"
                    /></svg
                  >
                {:else if item.icon === "briefcase"}
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    ><rect width="20" height="14" x="2" y="7" rx="2" /><path
                      d="M16 7V5a2 2 0 0 0-2-2h-4a2 2 0 0 0-2 2v2"
                    /></svg
                  >
                {:else}
                  <svg
                    class="h-4 w-4 shrink-0 transition-transform {item.action === 'more' &&
                    modeMoreOpen
                      ? 'rotate-180'
                      : ''}"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"><path d="m6 9 6 6 6-6" /></svg
                  >
                {/if}
                <span class="min-w-0 truncate">{item.label}</span>
              </button>
            {/each}
            {#if modeMoreOpen && modeMoreItems.length > 0}
              <div class="ml-3 border-l border-sidebar-border/70 pl-2">
                {#each modeMoreItems as item}
                  <button
                    type="button"
                    class="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[12px] transition-colors {isPluginsPage &&
                    pluginActiveSection === item.action
                      ? 'bg-sidebar-accent text-sidebar-foreground'
                      : 'text-muted-foreground hover:bg-sidebar-accent/60 hover:text-sidebar-foreground'}"
                    onclick={() => handleModeMenuAction(item.action)}
                  >
                    {#if item.icon === "server"}
                      <svg
                        class="h-3.5 w-3.5 shrink-0"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.8"
                        ><rect x="2" y="3" width="20" height="8" rx="2" /><rect
                          x="2"
                          y="13"
                          width="20"
                          height="8"
                          rx="2"
                        /><path d="M6 7h.01" /><path d="M6 17h.01" /></svg
                      >
                    {:else if item.icon === "webhook"}
                      <svg
                        class="h-3.5 w-3.5 shrink-0"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.8"
                        ><path
                          d="M18 16.98h-5.99c-1.1 0-1.95-.94-1.72-2.02.3-1.41 1.31-2.96 3.71-2.96h2"
                        /><circle cx="18" cy="17" r="3" /><circle cx="6" cy="5" r="3" /><circle
                          cx="6"
                          cy="19"
                          r="3"
                        /><path d="M9 5h1a6 6 0 0 1 6 6v1" /></svg
                      >
                    {:else if item.icon === "agents"}
                      <svg
                        class="h-3.5 w-3.5 shrink-0"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.8"
                        ><path d="M12 8V4H8" /><rect
                          width="16"
                          height="12"
                          x="4"
                          y="8"
                          rx="2"
                        /><path d="M2 14h2" /><path d="M20 14h2" /><path d="M9 13v2" /><path
                          d="M15 13v2"
                        /></svg
                      >
                    {:else}
                      <svg
                        class="h-3.5 w-3.5 shrink-0"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.8"
                        ><path d="M3 3v18h18" /><path d="m19 9-5 5-4-4-3 3" /></svg
                      >
                    {/if}
                    <span class="min-w-0 truncate">{item.label}</span>
                  </button>
                {/each}
              </div>
            {/if}
          </div>

          <!-- Tab content -->
          {#if panelTab === "chats" || isChatPage}
            {#if searchPanelOpen || runSearchQuery.trim()}
              <div class="px-2 pb-2 shrink-0">
                <input
                  type="text"
                  bind:value={runSearchQuery}
                  oninput={onDeepQueryInput}
                  placeholder={t("sidebar_searchChats")}
                  class="h-8 w-full rounded-md border border-sidebar-border bg-background px-2 text-xs text-sidebar-foreground placeholder:text-muted-foreground/50 focus:outline-none focus:border-ring/50"
                />
                {#if runSearchQuery.trim()}
                  {#if searching}
                    <p class="text-xs text-muted-foreground px-1 pt-0.5">
                      {t("runs_searching")}
                    </p>
                  {:else if visibleSearchResults.length > 0}
                    <p
                      class="flex items-center justify-between text-xs text-muted-foreground px-1 pt-0.5"
                    >
                      <span
                        >{t("runs_resultsCount", {
                          count: String(visibleSearchResults.length),
                        })}</span
                      >
                      <a
                        href="/history?q={encodeURIComponent(runSearchQuery)}"
                        class="text-primary/70 hover:text-primary transition-colors"
                        >{t("history_advancedSearch")}</a
                      >
                    </p>
                  {/if}
                {/if}
              </div>
            {/if}

            {#if runSearchQuery.trim()}
              <!-- Search results -->
              <div class="flex-1 overflow-y-auto">
                {#if searching && visibleSearchResults.length === 0}
                  <div class="flex items-center justify-center py-10">
                    <div
                      class="h-4 w-4 border-2 border-primary/30 border-t-primary rounded-full animate-spin"
                    ></div>
                  </div>
                {:else if !searching && visibleSearchResults.length === 0}
                  <div class="flex items-center justify-center px-3 py-10 text-center">
                    <p class="text-xs text-muted-foreground">{t("runs_noMatching")}</p>
                  </div>
                {:else}
                  {#each visibleSearchResults as result}
                    <button
                      class="w-full text-left flex flex-col gap-0.5 px-3 py-2 hover:bg-sidebar-accent/50 transition-colors text-sidebar-foreground"
                      onclick={() => {
                        runSearchQuery = "";
                        searchResults = [];
                        goto(
                          `/chat?run=${result.runId}&scrollTo=${encodeURIComponent(result.matchedEventId || result.matchedTs)}`,
                        );
                      }}
                    >
                      <p class="text-[12px] min-w-0 line-clamp-2 break-all">
                        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                        {@html highlightMatch(
                          snippetAround(result.matchedText, runSearchQuery, 80),
                          runSearchQuery,
                        )}
                      </p>
                      <div class="flex items-center gap-1 text-xs text-muted-foreground min-w-0">
                        <span class="flex-1 min-w-0 truncate"
                          >{result.runName || truncate(result.runPrompt, 30)}</span
                        >
                        <span class="ml-auto shrink-0">{relativeTime(result.matchedTs)}</span>
                      </div>
                    </button>
                  {/each}
                {/if}
              </div>
            {:else}
              <!-- Project-grouped conversations -->
              <div class="flex-1 overflow-y-auto px-2 py-1">
                {#each sidebarProjectFolders as folder (folder.folderKey)}
                  {@const activeFolder =
                    folder.cwd === projectCwd ||
                    folder.conversations.some((conv) =>
                      conv.runs.some((run) => run.id === selectedRunId),
                    )}
                  <ProjectFolderItem
                    {folder}
                    label={folder.isUncategorized
                      ? t("sidebar_uncategorized")
                      : cwdDisplayLabel(folder.cwd)}
                    expanded={expandedProjects.has(folder.folderKey) || activeFolder}
                    active={activeFolder}
                    shortcutIndex={workspaceShortcutIndexes.get(folder.folderKey)}
                    showShortcutHints={workspaceShortcutHintsVisible}
                    {selectedRunId}
                    onToggle={() => toggleProject(folder.folderKey)}
                    onSelectConversation={openConversationByRunId}
                    onResume={resumeConversation}
                    onDelete={requestDeleteConversation}
                    onNewChat={folder.isUncategorized
                      ? undefined
                      : () => newChatInFolder(folder.cwd)}
                    onRemove={folder.isUncategorized
                      ? undefined
                      : () => requestRemoveProject(folder.cwd)}
                    onConversationContextMenu={openConversationContextMenu}
                    onConversationActionMenu={openConversationActionMenu}
                  />
                {/each}
                <!-- Open folder... -->
                <button
                  class="mt-2 flex h-8 w-full items-center gap-2 rounded-md px-2 text-[13px] text-muted-foreground hover:text-sidebar-foreground hover:bg-sidebar-accent/60 transition-colors"
                  onclick={() => pickFolder(true)}
                >
                  <svg
                    class="h-4 w-4 shrink-0"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"><path d="M12 5v14" /><path d="M5 12h14" /></svg
                  >
                  <span>{t("project_openFolder")}</span>
                </button>

                {#if sidebarProjectFolders.length === 0}
                  <div class="flex flex-col items-center gap-2 px-3 py-6 text-center">
                    <p class="text-xs text-muted-foreground">
                      {t("sidebar_noConversationsYet")}<br />{t("sidebar_startNewChat")}
                    </p>
                  </div>
                {/if}
              </div>
            {/if}
          {:else if panelTab === "teams"}
            <!-- Teams list in sidebar -->
            <div class="flex-1 overflow-y-auto px-2 py-1">
              {#if teamStore.loading}
                <div class="flex items-center justify-center py-6">
                  <div
                    class="h-4 w-4 border-2 border-primary/30 border-t-primary rounded-full animate-spin"
                  ></div>
                </div>
              {:else if teamStore.teams.length === 0}
                <div class="flex flex-col items-center gap-1 px-3 py-6 text-center">
                  <p class="text-xs text-muted-foreground">{t("sidebar_noActiveTeams")}</p>
                  <p class="text-[10px] text-muted-foreground/60">{t("sidebar_startTeamHint")}</p>
                </div>
              {:else}
                {#each teamStore.teams as team}
                  <button
                    class="flex w-full flex-col gap-0.5 rounded-md px-2.5 py-2 text-left transition-colors mb-0.5
                        {teamStore.selectedTeam === team.name
                      ? 'bg-sidebar-accent text-sidebar-foreground'
                      : 'hover:bg-sidebar-accent/50 text-sidebar-foreground'}"
                    onclick={() => {
                      teamStore.selectTeam(team.name);
                      goto("/teams");
                    }}
                  >
                    <div class="flex items-center gap-1.5">
                      <span class="h-2 w-2 rounded-full bg-teal-500 shrink-0"></span>
                      <span class="text-[13px] font-medium min-w-0 truncate">{team.name}</span>
                    </div>
                    {#if team.description}
                      <p class="text-xs text-muted-foreground truncate pl-3.5">
                        {team.description}
                      </p>
                    {/if}
                    <div class="flex items-center gap-2 pl-3.5 text-xs text-muted-foreground">
                      <span>{t("sidebar_members", { count: String(team.member_count) })}</span>
                      <span>{t("sidebar_tasks", { count: String(team.task_count) })}</span>
                    </div>
                  </button>
                {/each}
              {/if}
            </div>
          {/if}
        {/if}
        <div class="border-t border-sidebar-border px-3 py-3">
          <div class="flex items-center gap-2">
            <button
              type="button"
              class="flex min-w-0 flex-1 items-center gap-2 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-sidebar-accent/60"
              onclick={openProfileModal}
              title="HelionCoder"
            >
              <span
                class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-[hsl(42_48%_88%)] text-sm font-semibold text-neutral-900"
                >{displayInitial}</span
              >
              <span class="min-w-0">
                <span class="block truncate text-[13px] font-medium text-sidebar-foreground"
                  >{displayName}</span
                >
                <span class="block truncate text-[11px] text-muted-foreground">HelionCoder</span>
              </span>
            </button>
            <button
              type="button"
              class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-sidebar-accent/60 hover:text-sidebar-foreground"
              onclick={() => goto("/settings")}
              title={currentLocale().startsWith("zh") ? "设置" : "Settings"}
            >
              <svg
                class="h-4 w-4"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                ><path d="M4 21v-7" /><path d="M4 10V3" /><path d="M12 21v-9" /><path
                  d="M12 8V3"
                /><path d="M20 21v-5" /><path d="M20 12V3" /><path d="M2 14h4" /><path
                  d="M10 8h4"
                /><path d="M18 16h4" /></svg
              >
            </button>
            <div class="relative">
              <button
                type="button"
                class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-sidebar-accent/60 hover:text-sidebar-foreground"
                onclick={() => (themeMenuOpen = !themeMenuOpen)}
                title={themeMode === "dark"
                  ? t("layout_themeTitle_dark")
                  : themeMode === "light"
                    ? t("layout_themeTitle_light")
                    : t("layout_themeTitle_system")}
              >
                <svg
                  class="h-4 w-4"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  ><circle cx="12" cy="12" r="4" /><path d="M12 2v2" /><path d="M12 20v2" /><path
                    d="m4.93 4.93 1.41 1.41"
                  /><path d="m17.66 17.66 1.41 1.41" /><path d="M2 12h2" /><path
                    d="M20 12h2"
                  /><path d="m6.34 17.66-1.41 1.41" /><path d="m19.07 4.93-1.41 1.41" /></svg
                >
              </button>
              {#if themeMenuOpen}
                <div
                  class="fixed inset-0 z-40"
                  role="button"
                  tabindex="-1"
                  onclick={() => (themeMenuOpen = false)}
                  onkeydown={(e) => e.key === "Escape" && (themeMenuOpen = false)}
                ></div>
                <div
                  class="absolute bottom-full right-0 z-50 mb-2 w-40 rounded-lg border border-sidebar-border bg-popover p-1 shadow-lg"
                >
                  {#each [{ id: "dark", label: currentLocale().startsWith("zh") ? "深色" : "Dark" }, { id: "light", label: currentLocale().startsWith("zh") ? "浅色" : "Light" }, { id: "system", label: currentLocale().startsWith("zh") ? "跟随系统" : "Match system" }] as item}
                    <button
                      type="button"
                      class="flex w-full items-center justify-between rounded-md px-2.5 py-1.5 text-left text-xs transition-colors
                        {themeMode === item.id
                        ? 'bg-accent text-accent-foreground'
                        : 'text-popover-foreground hover:bg-accent/60'}"
                      onclick={() => setTheme(item.id as ThemeMode)}
                    >
                      <span>{item.label}</span>
                      {#if themeMode === item.id}
                        <span class="text-[10px]">✓</span>
                      {/if}
                    </button>
                  {/each}
                </div>
              {/if}
            </div>
          </div>
        </div>
        <!-- Resize handle -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="absolute right-0 top-0 bottom-0 w-1 cursor-col-resize hover:bg-primary/20 active:bg-primary/30 transition-colors z-10"
          onpointerdown={startResize}
        ></div>
      </div>
  </aside>

  {#if sidebarOpen}
    <button
      type="button"
      class="absolute top-0 bottom-0 z-40 w-3 cursor-col-resize bg-transparent transition-colors hover:bg-primary/10 active:bg-primary/20"
      style:left="{Math.max(0, sidebarWidth - 16)}px"
      title="Resize sidebar"
      aria-label="Resize sidebar"
      onpointerdown={startResize}
    ></button>
  {/if}

  <!-- Ghost line during sidebar drag (zero-reflow preview) -->
  {#if sidebarResizing}
    <div
      bind:this={sidebarGhostEl}
      class="fixed top-0 bottom-0 z-[9999] pointer-events-none bg-primary"
      style="left: {sidebarGhostX -
        1}px; width: 3px; box-shadow: 0 0 8px hsl(var(--primary) / 0.6);"
    ></div>
  {/if}

  <!-- Main content -->
  <div
    class="relative z-20 flex min-w-0 flex-1 flex-col overflow-hidden bg-background transition-[margin,padding,border-radius,box-shadow] duration-200 ease-out {sidebarOpen
      ? 'desktop-main-surface'
      : ''} {!sidebarOpen ? 'pt-12' : ''}"
  >
    <div
      class="titlebar-drag absolute left-0 right-0 top-0 z-40 flex h-12 items-center gap-1.5 bg-background/75 pr-4 backdrop-blur transition-[opacity,transform] duration-200 ease-out {reserveMacTrafficLights
        ? 'pl-[92px]'
        : 'pl-3'} {sidebarOpen
        ? 'pointer-events-none -translate-x-2 opacity-0'
        : 'opacity-100'}"
      data-tauri-drag-region
      aria-hidden={sidebarOpen}
    >
      <div class="flex min-w-0 items-center gap-1.5 {reserveMacTrafficLights ? 'mac-titlebar-control-strip' : ''}">
      <button
        type="button"
        class="titlebar-no-drag flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent/70 hover:text-foreground"
        onclick={toggleSidebar}
        title={t("layout_toggleSidebar")}
        aria-label={t("layout_toggleSidebar")}
        tabindex={sidebarOpen ? -1 : 0}
      >
        <svg
          class="h-4 w-4"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
          stroke-linejoin="round"
          ><rect width="16" height="14" x="4" y="5" rx="2" /><path d="M10 5v14" /></svg
        >
      </button>
      <button
        type="button"
        class="titlebar-no-drag flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent/70 hover:text-foreground"
        onclick={navigateBack}
        title={t("layout_goBack")}
        aria-label={t("layout_goBack")}
        tabindex={sidebarOpen ? -1 : 0}
      >
        <svg
          class="h-4 w-4"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"><path d="m15 18-6-6 6-6" /></svg
        >
      </button>
      <button
        type="button"
        class="titlebar-no-drag flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent/70 hover:text-foreground"
        onclick={navigateForward}
        title={t("layout_goForward")}
        aria-label={t("layout_goForward")}
        tabindex={sidebarOpen ? -1 : 0}
      >
        <svg
          class="h-4 w-4"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"><path d="m9 18 6-6-6-6" /></svg
        >
      </button>
      <button
        type="button"
        class="titlebar-no-drag flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent/70 hover:text-foreground"
        onclick={newChat}
        title={t("layout_newConversation")}
        aria-label={t("layout_newConversation")}
        tabindex={sidebarOpen ? -1 : 0}
      >
        <svg
          class="h-4 w-4"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"><path d="M12 5v14" /><path d="M5 12h14" /></svg
        >
      </button>
      {#if !isChatPage}
        <div class="ml-2 min-w-0 truncate text-sm font-medium text-foreground/85">
          {pageName}
        </div>
      {/if}
      </div>
    </div>
    <UpdateBanner />
    <!-- Top bar (non-chat pages only — chat uses SessionStatusBar) -->
    {#if !isChatPage}
      <header class="hidden h-12 items-center gap-3 border-b px-4">
        <button
          class="rounded-md p-1.5 hover:bg-accent transition-all duration-150"
          onclick={toggleSidebar}
          title={t("layout_toggleSidebar")}
        >
          <svg
            class="h-4 w-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            ><rect width="18" height="18" x="3" y="3" rx="2" /><path d="M9 3v18" /></svg
          >
        </button>

        <div class="flex items-center gap-2 text-sm">
          <span class="text-muted-foreground">{t("layout_appName")}</span>
          <svg
            class="h-3.5 w-3.5 text-muted-foreground/50"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"><path d="m9 18 6-6-6-6" /></svg
          >
          <span class="font-medium">{pageName}</span>
        </div>
      </header>
    {/if}

    <!-- Page content -->
    <main class="flex-1 overflow-y-auto">
      {@render children()}
    </main>
  </div>
</div>

<CommandPalette
  bind:open={commandPaletteOpen}
  cwd={projectCwd || "/"}
  onOpenFolderBrowser={pickFolder}
  onOpenModelSelector={openModelSelectorFromLayout}
/>

{#if conversationMenu.open && conversationMenu.conv}
  {@const menuConv = conversationMenu.conv}
  {@const menuPinned = isConversationPinned(menuConv)}
  {@const menuUnread = isConversationUnread(menuConv)}
  {@const menuCwd = normalizeCwd(menuConv.latestRun.cwd)}
  {@const menuSessionId = getConversationSessionId(menuConv)}
  <div
    data-conversation-menu
    class="fixed z-50 w-[252px] overflow-hidden rounded-lg border border-border/80 bg-popover/98 p-1 text-popover-foreground shadow-2xl backdrop-blur-xl"
    style={`left: ${conversationMenu.x}px; top: ${conversationMenu.y}px;`}
    role="menu"
  >
    <button
      type="button"
      role="menuitem"
      class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent"
      onclick={() => togglePinConversation(menuConv)}
    >
      <svg
        class="h-4 w-4 shrink-0 text-muted-foreground"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        ><path d="M12 17v5" /><path d="M9 10.8 4.6 6.4" /><path
          d="m14.6 3 6.4 6.4-3.7 3.7.3 3.4-2.1 2.1-4.1-4.1-4.7 4.7-1.9-1.9 4.7-4.7-4.1-4.1 2.1-2.1 3.4.3L14.6 3Z"
        /></svg
      >
      <span class="min-w-0 flex-1 truncate"
        >{menuPinned ? t("sidebar_contextUnpin") : t("sidebar_contextPin")}</span
      >
    </button>
    <button
      type="button"
      role="menuitem"
      class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent"
      onclick={() => startRenameConversation(menuConv)}
    >
      <svg
        class="h-4 w-4 shrink-0 text-muted-foreground"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        ><path d="M12 20h9" /><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5Z" /></svg
      >
      <span class="min-w-0 flex-1 truncate">{t("sidebar_contextRename")}</span>
    </button>
    <button
      type="button"
      role="menuitem"
      class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent"
      onclick={() => archiveConversation(menuConv)}
    >
      <svg
        class="h-4 w-4 shrink-0 text-muted-foreground"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        ><rect x="3" y="4" width="18" height="4" rx="1" /><path d="M5 8v11h14V8" /><path
          d="M10 12h4"
        /></svg
      >
      <span class="min-w-0 flex-1 truncate">{t("sidebar_contextArchive")}</span>
    </button>
    <button
      type="button"
      role="menuitem"
      class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent"
      onclick={() => toggleUnreadConversation(menuConv)}
    >
      <svg
        class="h-4 w-4 shrink-0 text-muted-foreground"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        ><path d="M22 12s-3.5 6-10 6S2 12 2 12s3.5-6 10-6 10 6 10 6Z" /><circle
          cx="12"
          cy="12"
          r="3"
        /></svg
      >
      <span class="min-w-0 flex-1 truncate"
        >{menuUnread ? t("sidebar_contextMarkRead") : t("sidebar_contextMarkUnread")}</span
      >
    </button>

    <div class="my-1 h-px bg-border/80"></div>

    <button
      type="button"
      role="menuitem"
      class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-45"
      disabled={!menuCwd}
      onclick={() => openConversationInFinder(menuConv)}
    >
      <svg
        class="h-4 w-4 shrink-0 text-muted-foreground"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        ><path
          d="M3 7.5A2.5 2.5 0 0 1 5.5 5H9l2 2h7.5A2.5 2.5 0 0 1 21 9.5v7A2.5 2.5 0 0 1 18.5 19h-13A2.5 2.5 0 0 1 3 16.5v-9Z"
        /></svg
      >
      <span class="min-w-0 flex-1 truncate">{t("sidebar_contextOpenFinder")}</span>
    </button>
    <button
      type="button"
      role="menuitem"
      class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-45"
      disabled={!menuCwd}
      onclick={() => void copyConversationCwd(menuConv)}
    >
      <svg
        class="h-4 w-4 shrink-0 text-muted-foreground"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        ><rect x="9" y="9" width="13" height="13" rx="2" /><rect
          x="2"
          y="2"
          width="13"
          height="13"
          rx="2"
        /></svg
      >
      <span class="min-w-0 flex-1 truncate">{t("sidebar_contextCopyCwd")}</span>
    </button>
    <button
      type="button"
      role="menuitem"
      class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-45"
      disabled={!menuSessionId}
      onclick={() => void copyConversationSessionId(menuConv)}
    >
      <svg
        class="h-4 w-4 shrink-0 text-muted-foreground"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        ><path d="M15 7h3a5 5 0 0 1 0 10h-3" /><path d="M9 17H6A5 5 0 0 1 6 7h3" /><path
          d="M8 12h8"
        /></svg
      >
      <span class="min-w-0 flex-1 truncate">{t("sidebar_contextCopySessionId")}</span>
    </button>
    <button
      type="button"
      role="menuitem"
      class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent"
      onclick={() => void copyConversationDeepLink(menuConv)}
    >
      <svg
        class="h-4 w-4 shrink-0 text-muted-foreground"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        ><path d="M10 13a5 5 0 0 0 7.1 0l2-2a5 5 0 0 0-7.1-7.1l-1.1 1.1" /><path
          d="M14 11a5 5 0 0 0-7.1 0l-2 2A5 5 0 0 0 12 20.1l1.1-1.1"
        /></svg
      >
      <span class="min-w-0 flex-1 truncate">{t("sidebar_contextCopyDeepLink")}</span>
    </button>

    <div class="my-1 h-px bg-border/80"></div>

    <button
      type="button"
      role="menuitem"
      class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-45"
      disabled={!menuSessionId}
      onclick={() => forkConversation(menuConv)}
    >
      <svg
        class="h-4 w-4 shrink-0 text-muted-foreground"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        ><circle cx="6" cy="18" r="3" /><circle cx="18" cy="6" r="3" /><path
          d="M8.6 16.4 15.4 9.6"
        /><path d="M12 6H9a3 3 0 0 0-3 3v6" /></svg
      >
      <span class="min-w-0 flex-1 truncate">{t("sidebar_contextForkLocal")}</span>
    </button>
    <button
      type="button"
      role="menuitem"
      class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent"
      onclick={() => void openConversationNewWindow(menuConv)}
    >
      <svg
        class="h-4 w-4 shrink-0 text-muted-foreground"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        ><rect x="4" y="4" width="14" height="14" rx="2" /><path d="M14 4h6v6" /><path
          d="m13 11 7-7"
        /></svg
      >
      <span class="min-w-0 flex-1 truncate">{t("sidebar_contextOpenNewWindow")}</span>
    </button>

    <div class="my-1 h-px bg-border/80"></div>

    <button
      type="button"
      role="menuitem"
      class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] text-destructive transition-colors hover:bg-destructive/10"
      onclick={() => {
        closeConversationMenu();
        requestDeleteConversation(menuConv);
      }}
    >
      <svg
        class="h-4 w-4 shrink-0"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        ><path d="M3 6h18" /><path d="M8 6V4h8v2" /><path d="M19 6l-1 14H6L5 6" /></svg
      >
      <span class="min-w-0 flex-1 truncate">{t("sidebar_contextDelete")}</span>
    </button>
  </div>
{/if}

{#if conversationToast}
  <div
    class="fixed bottom-5 left-1/2 z-50 -translate-x-1/2 rounded-full border border-border bg-popover px-3 py-1.5 text-xs text-popover-foreground shadow-lg"
  >
    {conversationToast}
  </div>
{/if}

{#if showSetupWizard}
  <SetupWizard onComplete={handleSetupComplete} />
{/if}

<AboutModal bind:open={showAbout} />

<PermissionsModal bind:open={permissionsModalOpen} cwd={projectCwd} />

<Modal
  bind:open={profileModalOpen}
  title={currentLocale().startsWith("zh") ? "用户资料" : "Profile"}
>
  <div class="space-y-4">
    <label class="block space-y-1.5">
      <span class="text-sm font-medium"
        >{currentLocale().startsWith("zh") ? "用户名" : "Display name"}</span
      >
      <input
        class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        bind:value={profileNameDraft}
        placeholder="Helion"
        onkeydown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            void saveProfileName();
          }
        }}
      />
    </label>
    <p class="text-xs text-muted-foreground">
      {currentLocale().startsWith("zh")
        ? "这个名字会用于侧边栏和欢迎语，统计数据会保存在 ~/.helioncoder。"
        : "Used in the sidebar and welcome screen. Usage stats are stored in ~/.helioncoder."}
    </p>
    {#if profileError}
      <p class="text-xs text-red-500">{profileError}</p>
    {/if}
    <div class="flex justify-end gap-2">
      <button
        class="px-3 py-1.5 text-sm rounded-md border border-border hover:bg-accent transition-colors"
        onclick={() => (profileModalOpen = false)}
      >
        {currentLocale().startsWith("zh") ? "取消" : "Cancel"}
      </button>
      <button
        class="px-3 py-1.5 text-sm rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50"
        disabled={profileSaving}
        onclick={() => void saveProfileName()}
      >
        {profileSaving
          ? currentLocale().startsWith("zh")
            ? "保存中"
            : "Saving"
          : currentLocale().startsWith("zh")
            ? "保存"
            : "Save"}
      </button>
    </div>
  </div>
</Modal>

<Modal bind:open={renameConversationOpen} title={t("sidebar_renameTitle")}>
  <div class="space-y-4">
    <label class="block space-y-1.5">
      <span class="text-sm font-medium">{t("sidebar_renameLabel")}</span>
      <input
        bind:this={renameConversationInput}
        bind:value={renameConversationValue}
        class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        placeholder={t("sidebar_renamePlaceholder")}
        onkeydown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            void confirmRenameConversation();
          }
        }}
      />
    </label>
    {#if renameConversationError}
      <p class="text-xs text-red-500">{renameConversationError}</p>
    {/if}
    <div class="flex justify-end gap-2">
      <button
        class="px-3 py-1.5 text-sm rounded-md border border-border hover:bg-accent transition-colors"
        onclick={cancelRenameConversation}
      >
        {t("common_cancel")}
      </button>
      <button
        class="px-3 py-1.5 text-sm rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50"
        disabled={renameConversationSaving}
        onclick={() => void confirmRenameConversation()}
      >
        {renameConversationSaving ? t("common_loading") : t("common_save")}
      </button>
    </div>
  </div>
</Modal>

<FolderPicker
  bind:open={folderPickerOpen}
  initialHost={folderPickerInitialHost}
  initialPath={folderPickerInitialPath}
  onConfirm={onFolderPicked}
/>

{#if showCliBrowser}
  <CliSessionBrowser
    cwd="/"
    onclose={() => (showCliBrowser = false)}
    onimported={(runId) => {
      showCliBrowser = false;
      loadRuns();
      goto(`/chat?run=${runId}`);
    }}
  />
{/if}

<Modal bind:open={deleteConfirmOpen} title={t("sidebar_deleteConfirm")}>
  <p class="text-sm text-muted-foreground mb-4">{t("sidebar_deleteDesc")}</p>
  <div class="flex justify-end gap-2">
    <button
      class="px-3 py-1.5 text-sm rounded-md border border-border hover:bg-accent transition-colors"
      onclick={cancelDeleteConversation}
    >
      {t("sidebar_deleteCancel")}
    </button>
    <button
      class="px-3 py-1.5 text-sm rounded-md bg-destructive text-destructive-foreground hover:bg-destructive/90 transition-colors"
      onclick={confirmDeleteConversation}
    >
      {t("sidebar_deleteOk")}
    </button>
  </div>
</Modal>

<Modal bind:open={removeProjectConfirmOpen} title={t("sidebar_removeProjectConfirm")}>
  <p class="text-sm text-muted-foreground mb-4">{t("sidebar_removeProjectDesc")}</p>
  <div class="flex justify-end gap-2">
    <button
      class="px-3 py-1.5 text-sm rounded-md border border-border hover:bg-accent transition-colors"
      onclick={cancelRemoveProject}
    >
      {t("sidebar_deleteCancel")}
    </button>
    <button
      class="px-3 py-1.5 text-sm rounded-md bg-destructive text-destructive-foreground hover:bg-destructive/90 transition-colors"
      onclick={confirmRemoveProject}
    >
      {t("sidebar_deleteOk")}
    </button>
  </div>
</Modal>
