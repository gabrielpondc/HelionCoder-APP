<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import type {
    Attachment,
    AuthOverview,
    CliCommand,
    CliModelInfo,
    DirEntry,
    GitSummary,
    PlatformCredential,
  } from "$lib/types";
  import * as api from "$lib/api";
  import { createGitBranchPoller } from "$lib/utils/git-branch";
  import AgentSelector from "./AgentSelector.svelte";
  import AuthSourceBadge from "./AuthSourceBadge.svelte";
  import SkillSelector from "./SkillSelector.svelte";
  import FileAttachment from "./FileAttachment.svelte";
  import SlashMenu from "./SlashMenu.svelte";
  import AtMentionMenu from "./AtMentionMenu.svelte";
  import {
    filterSlashCommands,
    mergeWithVirtual,
    parseVirtualAction,
    getCommandInteraction,
    getArgumentHint,
    shouldBackFromSubView,
    isSubViewInputValid,
    classifyCloseReason,
    groupSlashCommands,
    VIRTUAL_COMMANDS,
  } from "$lib/utils/slash-commands";
  import type { SlashCommandGroups } from "$lib/utils/slash-commands";
  import { dbg, dbgWarn } from "$lib/utils/debug";
  import { IS_MAC } from "$lib/utils/platform";
  import { currentLocale, t } from "$lib/i18n/index.svelte";
  import { formatPasteSize } from "$lib/utils/format";
  import {
    BINARY_ATTACHMENT_TYPES,
    MAX_ATTACHMENTS,
    MAX_PASTE_BLOCKS,
    PDF_MAX_BINARY_SIZE,
    PDF_MAX_PATH_SIZE,
    isTextFile,
    isPdf,
    isConvertibleFile,
    isConvertibleByExt,
    isSpreadsheetExt,
    getFileExtension,
    classifyByMime,
    getFileSizeLimit,
    getSizeLimitByMime,
  } from "$lib/utils/file-types";
  import { convertFile } from "$lib/utils/file-convert";
  import { uuid } from "$lib/utils/uuid";
  import type { ClipboardFileInfo } from "$lib/api";
  import type { PromptInputSnapshot } from "$lib/types";
  import type { EditedFilesSummary } from "$lib/utils/edit-summary";
  import {
    type HistoryState,
    type HistoryAction,
    createHistoryState,
    checkAndReset,
    resetHistory,
    shouldIntercept,
    getHistoryAction,
    hasMultipleVisualLines,
  } from "$lib/utils/input-history";

  let {
    agent = "helioncoder",
    disabled = false,
    hasRun = false,
    running = false,
    sessionAlive = false,
    canResume = false,
    useStreamSession = false,
    isRemote = false,
    cliCommands = [],
    models = [],
    currentModel = "",
    effort = "",
    permissionMode = "",
    onSend,
    onAgentChange,
    onInterrupt,
    onModelSwitch,
    onEffortChange,
    onPermissionModeChange,
    onVirtualCommand,
    fastModeState = "",
    onFastModeSwitch,
    cwd = "/",
    authMode = "cli",
    platformId = "anthropic",
    platformCredentials = [],
    onPlatformChange,
    authOverview = null,
    authSourceLabel = "",
    authSourceCategory = "unknown",
    apiKeySource = "",
    onAuthModeChange,
    localProxyStatuses = {} as Record<string, { running: boolean; needsAuth: boolean }>,
    availableSkills = [],
    skillItems = [],
    agents = [],
    showAuthBadge = true,
    pendingPermission = false,
    hasStash = false,
    onBtwSend,
    onRestoreStash,
    onShortcutHelp,
    userHistory = [] as string[],
    runId = "",
    appMode = "code" as "chat" | "cowork" | "code",
    variant = "dock" as "dock" | "hero",
    editedSummary = null as EditedFilesSummary | null,
    workspaceSummary = null as GitSummary | null,
    editorContextPath = "",
    onOpenEditedFiles,
    onReviewChanges,
  }: {
    agent?: string;
    disabled?: boolean;
    hasRun?: boolean;
    running?: boolean;
    sessionAlive?: boolean;
    canResume?: boolean;
    useStreamSession?: boolean;
    isRemote?: boolean;
    cliCommands?: CliCommand[];
    models?: CliModelInfo[];
    currentModel?: string;
    effort?: string;
    permissionMode?: string;
    onSend: (text: string, attachments: Attachment[]) => void;
    onAgentChange?: (agent: string) => void;
    onInterrupt?: () => void;
    onModelSwitch?: (model: string) => void;
    onEffortChange?: (effort: string) => void;
    onPermissionModeChange?: (mode: string) => void;
    onVirtualCommand?: (action: string, args: string) => void;
    fastModeState?: string;
    onFastModeSwitch?: (mode: "on" | "off") => void;
    cwd?: string;
    authMode?: string;
    platformId?: string;
    platformCredentials?: PlatformCredential[];
    onPlatformChange?: (platformId: string) => void;
    authOverview?: AuthOverview | null;
    authSourceLabel?: string;
    authSourceCategory?: string;
    apiKeySource?: string;
    onAuthModeChange?: (mode: string) => void;
    localProxyStatuses?: Record<string, { running: boolean; needsAuth: boolean }>;
    availableSkills?: string[];
    skillItems?: { name: string; description: string }[];
    agents?: { name: string; description: string }[];
    showAuthBadge?: boolean; // TODO: remove unused auth props after hero migration
    pendingPermission?: boolean;
    hasStash?: boolean;
    onBtwSend?: (question: string, attachments: Attachment[]) => void;
    onRestoreStash?: () => void;
    onShortcutHelp?: () => void;
    userHistory?: string[];
    runId?: string;
    appMode?: "chat" | "cowork" | "code";
    variant?: "dock" | "hero";
    editedSummary?: EditedFilesSummary | null;
    workspaceSummary?: GitSummary | null;
    editorContextPath?: string;
    onOpenEditedFiles?: () => void;
    onReviewChanges?: () => void;
  } = $props();

  // ── BTW mode (side question) ──
  let btwMode = $state(false);
  let contextOnlyChat = $derived(appMode === "chat");

  let effectivePlaceholder = $derived(
    btwMode
      ? t("prompt_sideChatPlaceholder")
      : pendingPermission
        ? t("prompt_pendingPermission")
        : appMode === "cowork"
          ? t("prompt_coworkPlaceholder")
          : appMode === "chat"
            ? t("prompt_chatPlaceholder")
            : hasRun
              ? t("prompt_hasRunPlaceholder")
              : t("prompt_newPlaceholder"),
  );

  // ── Git branch (fetched from cwd) ──
  const branchPoller = createGitBranchPoller(api.getGitBranch);
  let gitBranch = $state("");

  // Fetch on cwd / isRemote change
  $effect(() => {
    void cwd;
    void isRemote;
    if (contextOnlyChat) {
      gitBranch = "";
      return;
    }
    const effectiveCwd = isRemote ? "" : cwd;
    branchPoller.refresh(effectiveCwd).then((b) => {
      gitBranch = b;
    });
  });

  // Poll every 10s to catch branch changes made by CLI commands
  $effect(() => {
    if (isRemote || contextOnlyChat) return;
    const interval = setInterval(() => {
      branchPoller.refresh(cwd).then((b) => {
        gitBranch = b;
      });
    }, 10_000);
    return () => clearInterval(interval);
  });

  // ── Branch color (7 rainbow colors based on name hash) ──
  const BRANCH_COLORS = [
    { bg: "bg-red-500/15", text: "text-red-400" },
    { bg: "bg-orange-500/15", text: "text-orange-400" },
    { bg: "bg-yellow-500/15", text: "text-yellow-400" },
    { bg: "bg-green-500/15", text: "text-green-400" },
    { bg: "bg-blue-500/15", text: "text-blue-400" },
    { bg: "bg-indigo-500/15", text: "text-indigo-400" },
    { bg: "bg-purple-500/15", text: "text-purple-400" },
  ];

  function branchColor(name: string) {
    let hash = 0;
    for (let i = 0; i < name.length; i++) {
      hash = (hash * 31 + name.charCodeAt(i)) | 0;
    }
    return BRANCH_COLORS[Math.abs(hash) % BRANCH_COLORS.length];
  }

  let currentBranchColor = $derived(branchColor(gitBranch));

  // ── Permission mode selector ──
  const PERMISSION_MODES = [
    {
      value: "default",
      icon: "hand",
      label: () => t("prompt_permAskLabel"),
      shortLabel: () => t("prompt_permAskShort"),
      description: () => t("prompt_permAskDesc"),
      cls: "text-foreground/70",
      dotCls: "bg-foreground/40",
      borderCls: "",
    },
    {
      value: "acceptEdits",
      icon: "edit",
      label: () => t("prompt_permAutoReadLabel"),
      shortLabel: () => t("prompt_permAutoReadShort"),
      description: () => t("prompt_permAutoReadDesc"),
      cls: "text-blue-400",
      dotCls: "bg-blue-400",
      borderCls:
        "border-blue-400/40 focus-within:border-blue-400/60 focus-within:shadow-[0_0_0_1px_rgba(96,165,250,0.15)]",
    },
    {
      value: "bypassPermissions",
      icon: "warning",
      label: () => t("prompt_permAutoAllLabel"),
      shortLabel: () => t("prompt_permAutoAllShort"),
      description: () => t("prompt_permAutoAllDesc"),
      cls: "text-amber-500",
      dotCls: "bg-amber-500",
      borderCls:
        "border-amber-500/40 focus-within:border-amber-500/60 focus-within:shadow-[0_0_0_1px_rgba(245,158,11,0.15)]",
    },
    {
      value: "plan",
      icon: "list",
      label: () => t("prompt_permPlanLabel"),
      shortLabel: () => t("prompt_permPlanShort"),
      description: () => t("prompt_permPlanDesc"),
      cls: "text-purple-400",
      dotCls: "bg-purple-400",
      borderCls:
        "border-purple-400/40 focus-within:border-purple-400/60 focus-within:shadow-[0_0_0_1px_rgba(192,132,252,0.15)]",
    },
    {
      value: "auto",
      icon: "spark",
      label: () => t("prompt_permAutoLabel"),
      shortLabel: () => t("prompt_permAutoShort"),
      description: () => t("prompt_permAutoDesc"),
      cls: "text-teal-400",
      dotCls: "bg-teal-400",
      borderCls:
        "border-teal-400/40 focus-within:border-teal-400/60 focus-within:shadow-[0_0_0_1px_rgba(45,212,191,0.15)]",
    },
    {
      value: "dontAsk",
      icon: "ban",
      label: () => t("prompt_permDontAskLabel"),
      shortLabel: () => t("prompt_permDontAskShort"),
      description: () => t("prompt_permDontAskDesc"),
      cls: "text-red-400",
      dotCls: "bg-red-400",
      borderCls:
        "border-red-400/40 focus-within:border-red-400/60 focus-within:shadow-[0_0_0_1px_rgba(248,113,113,0.15)]",
    },
  ];

  let modeDropdownOpen = $state(false);
  let modeBtnEl: HTMLButtonElement | undefined = $state();
  let modeDropdownEl: HTMLDivElement | undefined = $state();
  let modeDropdownStyle = $state("");

  let currentMode = $derived(
    PERMISSION_MODES.find((m) => m.value === permissionMode) ?? PERMISSION_MODES[0],
  );

  function toggleModeDropdown() {
    if (modeDropdownOpen) {
      modeDropdownOpen = false;
      return;
    }
    // Close other menus
    if (slashMenuOpen) closeSlashMenu("mode-open");
    if (atMenuOpen) closeAtMenu("mode-open");
    if (modelDropdownOpen) modelDropdownOpen = false;
    if (plusMenuOpen) plusMenuOpen = false;

    modeDropdownOpen = true;
    if (variant === "hero") {
      modeDropdownStyle = "position:absolute; bottom:46px; left:48px; z-index:70;";
    } else if (modeBtnEl) {
      const rect = modeBtnEl.getBoundingClientRect();
      // Open upward (input is at bottom of screen)
      modeDropdownStyle = `position:fixed; bottom:${window.innerHeight - rect.top + 4}px; left:${rect.left}px; z-index:50;`;
    }
  }

  function selectMode(mode: string) {
    modeDropdownOpen = false;
    onPermissionModeChange?.(mode);
  }

  let plusMenuOpen = $state(false);
  let plusBtnEl: HTMLButtonElement | undefined = $state();
  let plusMenuEl: HTMLDivElement | undefined = $state();
  let plusMenuStyle = $state("");
  let includeEditorContext = $state(true);
  const INCLUDE_EDITOR_CONTEXT_KEY = "helioncoder:include-editor-context";

  function togglePlusMenu() {
    if (plusMenuOpen) {
      plusMenuOpen = false;
      return;
    }
    if (slashMenuOpen) closeSlashMenu("plus-open");
    if (atMenuOpen) closeAtMenu("plus-open");
    modeDropdownOpen = false;
    modelDropdownOpen = false;

    plusMenuOpen = true;
    if (variant === "hero") {
      plusMenuStyle = "position:absolute; bottom:46px; left:12px; z-index:70;";
    } else if (plusBtnEl) {
      const rect = plusBtnEl.getBoundingClientRect();
      plusMenuStyle = `position:fixed; bottom:${window.innerHeight - rect.top + 4}px; left:${rect.left}px; z-index:70;`;
    }
  }

  function choosePlusAttachment() {
    plusMenuOpen = false;
    if (pendingAttachments.length >= MAX_ATTACHMENTS) {
      showFileToast(t("prompt_maxAttachments", { count: String(MAX_ATTACHMENTS) }));
      return;
    }
    fileInput?.click();
  }

  function toggleEditorContext() {
    includeEditorContext = !includeEditorContext;
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(INCLUDE_EDITOR_CONTEXT_KEY, includeEditorContext ? "true" : "false");
    }
  }

  function togglePlanFromPlusMenu() {
    togglePlanModeChip();
  }

  function createPlanFromPlusMenu() {
    plusMenuOpen = false;
    createPlanDraft();
  }

  function chooseDocumentSkillFromPlusMenu(label: string) {
    plusMenuOpen = false;
    insertDocumentSkillPrompt(label);
  }

  function openPluginsFromPlusMenu() {
    plusMenuOpen = false;
    goto("/plugins");
  }

  const EFFORT_OPTIONS = [
    { value: "low", zh: "低", en: "Low" },
    { value: "medium", zh: "中", en: "Medium" },
    { value: "high", zh: "高", en: "High" },
    { value: "max", zh: "超高", en: "Max" },
  ];

  let modelDropdownOpen = $state(false);
  let modelBtnEl: HTMLButtonElement | undefined = $state();
  let modelDropdownEl: HTMLDivElement | undefined = $state();
  let modelDropdownStyle = $state("");

  let selectedModelInfo = $derived(
    models.find((m) => m.value === currentModel) ??
      models.find((m) => currentModel && currentModel.includes(m.value) && m.value !== "default"),
  );
  let modelDisplayLabel = $derived.by(() => {
    if (selectedModelInfo?.displayName) return selectedModelInfo.displayName;
    if (currentModel) return currentModel;
    return currentLocale().startsWith("zh") ? "默认" : "Default";
  });
  let availableEfforts = $derived.by(() => {
    const levels = selectedModelInfo?.supportedEffortLevels;
    if (levels?.length) return EFFORT_OPTIONS.filter((opt) => levels.includes(opt.value));
    return EFFORT_OPTIONS;
  });
  let normalizedEffort = $derived(effort || "medium");
  let effortDisplayLabel = $derived.by(() => {
    const zh = currentLocale().startsWith("zh");
    const found = EFFORT_OPTIONS.find((opt) => opt.value === normalizedEffort);
    return found ? (zh ? found.zh : found.en) : zh ? "中" : "Medium";
  });

  function toggleModelDropdown() {
    if (!onModelSwitch && !onEffortChange) return;
    if (modelDropdownOpen) {
      modelDropdownOpen = false;
      return;
    }
    if (slashMenuOpen) closeSlashMenu("model-open");
    if (atMenuOpen) closeAtMenu("model-open");
    if (modeDropdownOpen) modeDropdownOpen = false;
    if (plusMenuOpen) plusMenuOpen = false;

    modelDropdownOpen = true;
    if (variant === "hero") {
      modelDropdownStyle = "position:absolute; bottom:48px; right:58px; z-index:70;";
    } else if (modelBtnEl) {
      const rect = modelBtnEl.getBoundingClientRect();
      modelDropdownStyle = `position:fixed; bottom:${window.innerHeight - rect.top + 4}px; right:${Math.max(12, window.innerWidth - rect.right)}px; z-index:50;`;
    }
  }

  function selectModelFromDropdown(model: string) {
    modelDropdownOpen = false;
    onModelSwitch?.(model);
  }

  function selectEffortFromDropdown(nextEffort: string) {
    modelDropdownOpen = false;
    onEffortChange?.(nextEffort);
  }

  let workspaceChangeLabel = $derived.by(() => {
    if (!workspaceSummary || workspaceSummary.total_files <= 0) return "";
    const zh = currentLocale().startsWith("zh");
    return zh
      ? `${workspaceSummary.total_files} 个文件已更改`
      : `${workspaceSummary.total_files} file${workspaceSummary.total_files === 1 ? "" : "s"} changed`;
  });
  let showEditedChangeSummary = $derived(variant !== "hero" && !!editedSummary?.hasChanges);
  let showWorkspaceChangeSummary = $derived(
    !!workspaceSummary && workspaceSummary.total_files > 0 && !showEditedChangeSummary,
  );

  const DOCUMENT_SKILLS = [
    { id: "word", label: () => t("prompt_docSkillWord"), ext: "DOCX" },
    { id: "excel", label: () => t("prompt_docSkillExcel"), ext: "XLSX" },
    { id: "ppt", label: () => t("prompt_docSkillPpt"), ext: "PPTX" },
    { id: "pdf", label: () => t("prompt_docSkillPdf"), ext: "PDF" },
  ];

  function fileNameFromPath(path: string): string {
    const normalized = path.replaceAll("\\", "/");
    return normalized.split("/").filter(Boolean).pop() ?? path;
  }

  function focusAndResize() {
    requestAnimationFrame(() => {
      autoResize();
      textareaEl?.focus();
    });
  }

  function replaceOrAppendPrompt(text: string) {
    inputText = inputText.trim() ? `${inputText.trim()}\n\n${text}` : text;
    focusAndResize();
  }

  function createPlanDraft() {
    if (onPermissionModeChange && permissionMode !== "plan") selectMode("plan");
    const current = inputText.trim();
    inputText = current ? t("prompt_planPrefix", { instruction: current }) : t("prompt_planSeed");
    focusAndResize();
  }

  function togglePlanModeChip() {
    if (!onPermissionModeChange) return;
    selectMode(permissionMode === "plan" ? "default" : "plan");
  }

  function insertDocumentSkillPrompt(label: string) {
    replaceOrAppendPrompt(t("prompt_docSkillSeed", { kind: label }));
  }

  let editedStatusFile = $derived(editedSummary?.activeFile ?? editedSummary?.files[0] ?? null);
  let editedStatusLabel = $derived.by(() => {
    if (!editedSummary?.hasChanges) return "";
    if (editedSummary.isEditing && editedStatusFile) {
      return t("prompt_editingFile", { file: fileNameFromPath(editedStatusFile.path) });
    }
    return t("prompt_changedFiles", { count: String(editedSummary.totalFiles) });
  });

  interface PastedBlock {
    id: string;
    text: string;
    lineCount: number;
    charCount: number;
    preview: string;
    ext?: string;
  }

  interface PathRef {
    id: string;
    name: string;
    path: string;
    isDir: boolean;
  }

  let inputText = $state("");
  let pendingAttachments = $state<
    Array<{
      id: string;
      name: string;
      type: string;
      size: number;
      contentBase64?: string;
      /** Filesystem path for >20MB clipboard PDFs (path-reference mode). */
      filePath?: string;
    }>
  >([]);
  let pastedBlocks = $state<PastedBlock[]>([]);
  let pendingPathRefs = $state<PathRef[]>([]);

  let fileInput: HTMLInputElement | undefined = $state();
  let textareaEl: HTMLTextAreaElement | undefined = $state();
  let lastEscTime = 0;
  let histState: HistoryState = createHistoryState();

  $effect(() => {
    if (checkAndReset(histState, userHistory.length, runId)) {
      dbg("prompt-history", "reset", { runId, len: userHistory.length });
    }
  });

  /** Chunked ArrayBuffer→base64 (32KB chunks — safe for large files, avoids stack overflow). */
  function arrayBufferToBase64(buffer: ArrayBuffer): string {
    const bytes = new Uint8Array(buffer);
    const CHUNK = 0x8000;
    let binary = "";
    for (let i = 0; i < bytes.length; i += CHUNK) {
      const slice = bytes.subarray(i, Math.min(i + CHUNK, bytes.length));
      binary += String.fromCharCode.apply(null, slice as unknown as number[]);
    }
    return btoa(binary);
  }

  // ── File toast ──
  let toastMessage = $state<string | null>(null);
  let toastVariant = $state<"error" | "info">("error");
  let toastTimeout: ReturnType<typeof setTimeout> | null = null;
  function showFileToast(msg: string, variant: "error" | "info" = "error") {
    toastMessage = msg;
    toastVariant = variant;
    if (toastTimeout) clearTimeout(toastTimeout);
    toastTimeout = setTimeout(() => {
      toastMessage = null;
    }, 3500);
  }

  // ── Slash menu state ──
  let slashMenuOpen = $state(false);
  let slashSelectedIndex = $state(0);
  let slashPhase: "commands" | "sub-model" | "sub-fast" = $state("commands");
  let slashSubSelectedIndex = $state(0);
  let activeSlashCmd: CliCommand | null = $state(null);

  let slashEnabled = $derived(
    !contextOnlyChat && (agent === "helioncoder" || agent === "claude") && !!useStreamSession,
  );
  let slashBtnEl: HTMLButtonElement | undefined = $state();
  let savedInputForSlash = $state("");

  let allCommands = $derived(mergeWithVirtual(cliCommands ?? []));
  let skillNameSet = $derived(new Set(availableSkills));

  let slashQuery = $derived.by(() => {
    if (!slashMenuOpen || slashPhase !== "commands") return null;
    const m = inputText.match(/^\/([a-zA-Z0-9_-]*)$/);
    return m?.[1] ?? "";
  });

  let filteredCommands = $derived.by(() => {
    if (slashQuery === null) return [];
    return filterSlashCommands(allCommands, slashQuery);
  });

  let slashGroups = $derived.by((): SlashCommandGroups | null => {
    if (slashQuery !== "") return null; // non-empty query or menu closed → flat mode
    if (filteredCommands.length === 0) return null;
    return groupSlashCommands(filteredCommands, skillNameSet);
  });

  let effectiveCommands = $derived(slashGroups ? slashGroups.flatOrder : filteredCommands);

  let hintText = $derived.by(() => {
    if (slashPhase !== "commands" || effectiveCommands.length === 0) return "";
    const idx = Math.min(slashSelectedIndex, effectiveCommands.length - 1);
    return getArgumentHint(effectiveCommands[idx]);
  });

  $effect(() => {
    if (slashMenuOpen)
      dbg("slash", slashGroups ? "grouped" : "flat", { count: effectiveCommands.length });
  });

  // ── @-mention state ──
  let atMenuOpen = $state(false);
  let atQuery = $state("");
  let atResults = $state<DirEntry[]>([]);
  let atSelectedIndex = $state(0);
  let atLoading = $state(false);
  /** Position in inputText where the `@` trigger starts. */
  let atStartPos = $state(-1);
  let atDebounceTimer: ReturnType<typeof setTimeout> | null = null;

  function closeAtMenu(reason: string) {
    if (!atMenuOpen) return;
    dbg("at-mention", `close:${reason}`);
    atMenuOpen = false;
    atQuery = "";
    atResults = [];
    atSelectedIndex = 0;
    atStartPos = -1;
    atLoading = false;
    if (atDebounceTimer) {
      clearTimeout(atDebounceTimer);
      atDebounceTimer = null;
    }
  }

  function openAtMenu(pos: number) {
    // Audit #6: disable @ completion in remote mode (local listDirectory not applicable)
    if (isRemote || contextOnlyChat) return;
    if (slashMenuOpen) closeSlashMenu("at-open");
    if (modeDropdownOpen) modeDropdownOpen = false;
    if (plusMenuOpen) plusMenuOpen = false;
    atMenuOpen = true;
    atStartPos = pos;
    atQuery = "";
    atResults = [];
    atSelectedIndex = 0;
    dbg("at-mention", "open", { pos });
  }

  function resolveAtPath(query: string): string {
    // Resolve relative query against cwd to get absolute path for listDirectory
    if (!query) return cwd;
    if (query.startsWith("/")) return query;
    const base = cwd.endsWith("/") ? cwd : cwd + "/";
    return base + query;
  }

  async function fetchAtResults(query: string) {
    atLoading = true;
    try {
      // Split into directory path + filename prefix
      const lastSlash = query.lastIndexOf("/");
      let dirQuery: string;
      let prefix: string;
      if (lastSlash >= 0) {
        dirQuery = query.slice(0, lastSlash + 1);
        prefix = query.slice(lastSlash + 1).toLowerCase();
      } else {
        dirQuery = "";
        prefix = query.toLowerCase();
      }
      const absPath = resolveAtPath(dirQuery);
      dbg("at-mention", "fetch", { absPath, prefix });
      const listing = await api.listDirectory(absPath, true);
      // Filter by prefix and limit to 10
      const filtered = listing.entries
        .filter((e) => e.name.toLowerCase().startsWith(prefix))
        .slice(0, 10);
      atResults = filtered;
      atSelectedIndex = 0;
    } catch (e) {
      dbg("at-mention", "fetch error", e);
      atResults = [];
    } finally {
      atLoading = false;
    }
  }

  function handleAtInput(cursorPos: number) {
    if (contextOnlyChat) {
      if (atMenuOpen) closeAtMenu("chat-mode");
      return;
    }
    // Scan backwards from cursor for nearest @ preceded by whitespace or at position 0
    let atPos = -1;
    for (let i = cursorPos - 1; i >= 0; i--) {
      const ch = inputText[i];
      if (ch === "@") {
        // Valid if at start or preceded by whitespace
        if (i === 0 || /\s/.test(inputText[i - 1])) {
          atPos = i;
        }
        break;
      }
      if (/\s/.test(ch)) break; // whitespace before finding @ means no active @-mention
    }

    if (atPos >= 0) {
      const query = inputText.slice(atPos + 1, cursorPos);
      if (!atMenuOpen) openAtMenu(atPos);
      atQuery = query;

      // Debounce directory listing
      if (atDebounceTimer) clearTimeout(atDebounceTimer);
      atDebounceTimer = setTimeout(() => {
        fetchAtResults(query);
      }, 150);
    } else if (atMenuOpen) {
      closeAtMenu("no-at");
    }
  }

  function selectAtEntry(entry: DirEntry) {
    if (atStartPos < 0 || !textareaEl) return;
    const cursorPos = textareaEl.selectionStart ?? inputText.length;
    const prefix = inputText.slice(0, atStartPos + 1); // keeps the @
    const suffix = inputText.slice(cursorPos);

    // Build the path relative to what was already typed
    const lastSlash = atQuery.lastIndexOf("/");
    const dirPrefix = lastSlash >= 0 ? atQuery.slice(0, lastSlash + 1) : "";
    const relativePath = dirPrefix + entry.name;

    if (entry.is_dir) {
      // Append / and keep menu open for deeper navigation
      inputText = prefix + relativePath + "/" + suffix;
      requestAnimationFrame(() => {
        if (textareaEl) {
          const newPos = atStartPos + 1 + relativePath.length + 1;
          textareaEl.selectionStart = textareaEl.selectionEnd = newPos;
          textareaEl.focus();
        }
        // Trigger new fetch for subdirectory contents
        handleAtInput(atStartPos + 1 + relativePath.length + 1);
      });
    } else {
      // Insert file path and close menu
      inputText = prefix + relativePath + suffix;
      closeAtMenu("select");
      requestAnimationFrame(() => {
        if (textareaEl) {
          const newPos = atStartPos + 1 + relativePath.length;
          textareaEl.selectionStart = textareaEl.selectionEnd = newPos;
          textareaEl.focus();
        }
      });
    }
    dbg("at-mention", "select", { name: entry.name, isDir: entry.is_dir });
  }

  // Force close when conditions no longer met
  $effect(() => {
    if (!slashEnabled && slashMenuOpen) {
      closeSlashMenu("disabled");
    }
  });

  /** Restore saved input text and clear the saved value to prevent stale restores. */
  function restoreSavedInput() {
    if (savedInputForSlash !== "") {
      inputText = savedInputForSlash;
      savedInputForSlash = "";
    }
  }

  function clearSavedInput() {
    savedInputForSlash = "";
  }

  function closeSlashMenu(reason: string) {
    if (!slashMenuOpen) return;
    dbg("slash", `close:${reason}`);
    slashMenuOpen = false;
    slashPhase = "commands";
    activeSlashCmd = null;
    slashSelectedIndex = 0;
    slashSubSelectedIndex = 0;

    if (classifyCloseReason(reason) === "clear") {
      clearSavedInput();
    } else {
      restoreSavedInput();
    }
  }

  $effect(() => {
    if (contextOnlyChat) {
      if (plusMenuOpen) plusMenuOpen = false;
      if (modeDropdownOpen) modeDropdownOpen = false;
      if (slashMenuOpen) closeSlashMenu("chat-mode");
      if (atMenuOpen) closeAtMenu("chat-mode");
      includeEditorContext = false;
    }
  });

  function selectSlashCommand(cmd: CliCommand, trigger: "enter" | "tab") {
    const interaction = getCommandInteraction(cmd);
    dbg("slash", `select:${interaction}:${trigger}`, { name: cmd.name });

    switch (interaction) {
      case "immediate":
        if (trigger === "enter") {
          inputText = `/${cmd.name}`;
          closeSlashMenu("execute");
          handleSend();
        } else {
          // Tab: fill only, don't execute
          closeSlashMenu("fill");
          inputText = `/${cmd.name} `;
          moveCursorToEnd();
        }
        break;
      case "free-text":
        closeSlashMenu("fill");
        inputText = `/${cmd.name} `;
        moveCursorToEnd();
        break;
      case "enum":
        activeSlashCmd = cmd;
        inputText = `/${cmd.name} `;
        if (cmd.name === "fast") {
          slashPhase = "sub-fast";
          slashSubSelectedIndex = fastModeState === "on" ? 1 : 0;
        } else {
          slashPhase = "sub-model";
          slashSubSelectedIndex = 0;
        }
        moveCursorToEnd();
        break;
    }
  }

  function goBackToCommands() {
    const cmdName = activeSlashCmd?.name;
    dbg("slash", "back-to-commands", { from: cmdName });
    activeSlashCmd = null;
    slashPhase = "commands";
    slashSubSelectedIndex = 0;
    if (cmdName) inputText = `/${cmdName}`;
    slashSelectedIndex = 0;
    moveCursorToEnd();
  }

  function handleSubModelSelect(model: CliModelInfo) {
    dbg("slash", "sub-model-select", { value: model.value });
    const restoreText = savedInputForSlash;
    closeSlashMenu("sub-select"); // clears savedInputForSlash
    inputText = restoreText; // restore user draft
    if (textareaEl) textareaEl.style.height = "auto";
    onModelSwitch?.(model.value);
  }

  function handleFastModeSelect(mode: "on" | "off") {
    dbg("slash", "fast-select", { mode });
    const restoreText = savedInputForSlash;
    closeSlashMenu("sub-select");
    inputText = restoreText;
    if (textareaEl) textareaEl.style.height = "auto";
    onFastModeSwitch?.(mode);
  }

  function moveCursorToEnd() {
    requestAnimationFrame(() => {
      if (textareaEl) {
        textareaEl.selectionStart = textareaEl.selectionEnd = inputText.length;
        textareaEl.focus();
      }
    });
  }

  /** Open slash menu from the L2 button or "More..." pill. */
  function openSlashMenuFromButton() {
    if (!slashEnabled) return;
    if (slashMenuOpen) {
      closeSlashMenu("button-toggle");
      return;
    }
    if (atMenuOpen) closeAtMenu("slash-button");
    if (modeDropdownOpen) modeDropdownOpen = false;
    if (plusMenuOpen) plusMenuOpen = false;

    savedInputForSlash = inputText;
    inputText = "/";
    slashMenuOpen = true;
    slashPhase = "commands";
    slashSelectedIndex = 0;
    moveCursorToEnd();
    dbg("slash", "open:button", { saved: savedInputForSlash.length });
  }

  /** Handle L3 quick-action pill click. Three branches: enum, free-text, immediate. */
  function handleQuickAction(cmd: CliCommand) {
    if (!slashEnabled) return;
    dbg("slash", "quick-action", { name: cmd.name });
    const interaction = getCommandInteraction(cmd);

    if (interaction === "enum") {
      // e.g., model/fast: close other menus → save input → open sub-view
      if (atMenuOpen) closeAtMenu("quick-action");
      if (modeDropdownOpen) modeDropdownOpen = false;
      if (plusMenuOpen) plusMenuOpen = false;
      savedInputForSlash = inputText;
      inputText = `/${cmd.name} `;
      activeSlashCmd = cmd;
      if (cmd.name === "fast") {
        slashPhase = "sub-fast";
        slashSubSelectedIndex = fastModeState === "on" ? 1 : 0;
      } else {
        slashPhase = "sub-model";
        slashSubSelectedIndex = 0;
      }
      slashMenuOpen = true;
      moveCursorToEnd();
      return;
    }

    if (interaction === "free-text") {
      // Fill "/cmd " and focus — don't send, don't clear draft
      if (atMenuOpen) closeAtMenu("quick-action");
      if (modeDropdownOpen) modeDropdownOpen = false;
      if (plusMenuOpen) plusMenuOpen = false;
      inputText = `/${cmd.name} `;
      moveCursorToEnd();
      return;
    }

    // immediate: execute directly without touching inputText/pastedBlocks/attachments
    const vDef = VIRTUAL_COMMANDS.find((v) => v.name === cmd.name);
    if (vDef) {
      if (typeof vDef["_action"] === "string" && onVirtualCommand) {
        onVirtualCommand(vDef["_action"] as string, "");
        return;
      }
      if (typeof vDef["_navigate"] === "string") {
        goto(vDef["_navigate"] as string);
        return;
      }
    }
    // Regular CLI command: send "/cmd" directly without draft/attachments
    onSend(`/${cmd.name}`, []);
  }

  function handleInput() {
    autoResize();

    // Exit history mode if user edits the recalled text
    if (histState.index >= 0 && inputText !== userHistory[histState.index]) {
      dbg("prompt-history", "exit: user edited", { index: histState.index });
      resetHistory(histState);
    }

    // @-mention detection: runs BEFORE slashEnabled guard so it works pre-session
    const cursorPos = textareaEl?.selectionStart ?? inputText.length;
    handleAtInput(cursorPos);

    if (!slashEnabled) {
      dbg("slash", "disabled", { agent, useStreamSession, sessionAlive, canResume, inputText });
      return;
    }

    if (slashPhase === "sub-model" || slashPhase === "sub-fast") {
      // Close sub-view if input no longer matches /activeCmdName
      if (activeSlashCmd && !isSubViewInputValid(inputText, activeSlashCmd.name)) {
        closeSlashMenu("sub-invalid-input");
      }
      return;
    }

    // Commands phase
    const match = inputText.match(/^\/([a-zA-Z0-9_-]*)$/);
    if (match) {
      slashSelectedIndex = 0;
      if (!slashMenuOpen) {
        dbg("slash", "open", { query: match[1] });
        if (modeDropdownOpen) modeDropdownOpen = false;
        slashMenuOpen = true;
        slashPhase = "commands";
      }
    } else if (slashMenuOpen) {
      closeSlashMenu("no-match");
    }
  }

  /** Apply a history action (shared by immediate and deferred paths). */
  function applyHistoryAction(action: NonNullable<HistoryAction>) {
    if (action.type === "boundary") {
      dbg("prompt-history", "boundary", { index: histState.index });
      return;
    }

    if (action.type === "enter") {
      histState.draft = getInputSnapshot();
      histState.index = action.index;
      dbg("prompt-history", "up: enter history", { index: 0, total: userHistory.length });
    } else if (action.type === "up") {
      histState.index = action.index;
      dbg("prompt-history", "up", { index: action.index });
    } else if (action.type === "down") {
      histState.index = action.index;
      dbg("prompt-history", "down", { index: action.index });
    } else if (action.type === "restore-draft") {
      histState.index = -1;
      if (histState.draft) {
        dbg("prompt-history", "restore-draft", {
          textLen: histState.draft.text.length,
          atts: histState.draft.attachments.length,
          pastes: histState.draft.pastedBlocks.length,
        });
        restoreSnapshot(histState.draft);
        histState.draft = null;
        return; // restoreSnapshot handles autoResize + focus
      }
      inputText = "";
      pendingAttachments = [];
      pastedBlocks = [];
    }

    if (action.type !== "restore-draft") {
      // Bounds guard: if index is stale (timeline changed between events), bail out
      if (histState.index >= userHistory.length) {
        dbg("prompt-history", "stale index, resetting", {
          index: histState.index,
          len: userHistory.length,
        });
        resetHistory(histState);
        return;
      }
      inputText = userHistory[histState.index];
      pendingAttachments = [];
      pastedBlocks = [];
    }

    requestAnimationFrame(() => {
      autoResize();
      if (textareaEl) {
        textareaEl.selectionStart = textareaEl.selectionEnd = textareaEl.value.length;
      }
    });
  }

  function handleKeydown(e: KeyboardEvent) {
    // Skip during IME composition (e.g., Chinese input confirming with Enter)
    if (e.isComposing || e.keyCode === 229) return;

    // ── @-mention menu ──
    if (atMenuOpen) {
      if (e.key === "Escape") {
        e.preventDefault();
        closeAtMenu("escape");
        return;
      }
      if (atResults.length > 0) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          atSelectedIndex = Math.min(atSelectedIndex + 1, atResults.length - 1);
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          atSelectedIndex = Math.max(atSelectedIndex - 1, 0);
          return;
        }
        if (e.key === "Enter" || e.key === "Tab") {
          e.preventDefault();
          selectAtEntry(atResults[atSelectedIndex]);
          return;
        }
      }
      // Let other keys through for typing
    }

    // ── Sub-model phase ──
    if (slashMenuOpen && slashPhase === "sub-model") {
      if (e.key === "Escape") {
        e.preventDefault();
        goBackToCommands();
        return;
      }
      if (e.key === "Backspace") {
        if (
          shouldBackFromSubView(inputText, textareaEl?.selectionStart ?? 0, activeSlashCmd?.name)
        ) {
          e.preventDefault();
          goBackToCommands();
          return;
        }
        // else: normal backspace (let it through)
        return;
      }
      if (models.length > 0) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          slashSubSelectedIndex = Math.min(slashSubSelectedIndex + 1, models.length - 1);
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          slashSubSelectedIndex = Math.max(slashSubSelectedIndex - 1, 0);
          return;
        }
        if (e.key === "Enter" || e.key === "Tab") {
          e.preventDefault();
          handleSubModelSelect(models[slashSubSelectedIndex]);
          return;
        }
      }
      // Let other keys through for typing in sub-view
      return;
    }

    // ── Sub-fast phase ──
    if (slashMenuOpen && slashPhase === "sub-fast") {
      if (e.key === "Escape") {
        e.preventDefault();
        goBackToCommands();
        return;
      }
      if (e.key === "Backspace") {
        if (
          shouldBackFromSubView(inputText, textareaEl?.selectionStart ?? 0, activeSlashCmd?.name)
        ) {
          e.preventDefault();
          goBackToCommands();
          return;
        }
        return;
      }
      const FAST_OPTIONS = 2;
      if (e.key === "ArrowDown") {
        e.preventDefault();
        slashSubSelectedIndex = Math.min(slashSubSelectedIndex + 1, FAST_OPTIONS - 1);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        slashSubSelectedIndex = Math.max(slashSubSelectedIndex - 1, 0);
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        handleFastModeSelect(slashSubSelectedIndex === 0 ? "off" : "on");
        return;
      }
      return;
    }

    // ── Commands phase ──
    if (slashMenuOpen && slashPhase === "commands") {
      if (e.key === "Escape") {
        e.preventDefault();
        closeSlashMenu("escape");
        return;
      }
      if (effectiveCommands.length > 0) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          slashSelectedIndex = Math.min(slashSelectedIndex + 1, effectiveCommands.length - 1);
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          slashSelectedIndex = Math.max(slashSelectedIndex - 1, 0);
          return;
        }
        if (e.key === "Enter") {
          e.preventDefault();
          selectSlashCommand(effectiveCommands[slashSelectedIndex], "enter");
          return;
        }
        if (e.key === "Tab") {
          e.preventDefault();
          selectSlashCommand(effectiveCommands[slashSelectedIndex], "tab");
          return;
        }
      }
      // No filteredCommands but menu open (empty state) — only Esc handled above
      return;
    }

    // ── Input history (Up/Down arrow) ──
    if (
      shouldIntercept(
        e.key,
        e,
        {
          atMenuOpen,
          slashMenuOpen,
          modeDropdownOpen: modeDropdownOpen || plusMenuOpen || modelDropdownOpen,
        },
        textareaEl?.selectionStart ?? 0,
        textareaEl?.selectionEnd ?? 0,
        userHistory.length,
      ) &&
      textareaEl
    ) {
      // Multi-line or visually wrapped text: defer to next frame to let the
      // browser move the cursor first. Only trigger history if cursor didn't
      // move (meaning we're at the visual top/bottom edge).
      if (hasMultipleVisualLines(textareaEl)) {
        const posBefore = textareaEl.selectionStart;
        const key = e.key;
        // Immediate path: cursor at absolute start (Up) or end (Down)
        // — guaranteed to be at the visual edge, no need to defer.
        const atAbsoluteEdge =
          (key === "ArrowUp" && posBefore === 0) ||
          (key === "ArrowDown" && posBefore === textareaEl.value.length);
        if (atAbsoluteEdge) {
          const action = getHistoryAction(
            key,
            histState,
            userHistory.length,
            textareaEl.value,
            posBefore,
          );
          if (action) {
            e.preventDefault();
            applyHistoryAction(action);
            return;
          }
        }
        // Let browser handle cursor movement, check on next frame
        requestAnimationFrame(() => {
          if (!textareaEl) return;
          if (textareaEl.selectionStart !== posBefore) return; // cursor moved — normal nav
          const action = getHistoryAction(
            key,
            histState,
            userHistory.length,
            textareaEl.value,
            textareaEl.selectionStart,
          );
          if (action) {
            applyHistoryAction(action);
          }
        });
        return;
      }

      // Single visual line: handle immediately
      const action = getHistoryAction(
        e.key,
        histState,
        userHistory.length,
        textareaEl.value,
        textareaEl.selectionStart,
      );
      if (action) {
        e.preventDefault();
        applyHistoryAction(action);
        return;
      }
    }

    // ── Double Esc: clear all input ──
    if (e.key === "Escape") {
      const now = Date.now();
      if (now - lastEscTime < 400 && hasContent()) {
        e.preventDefault();
        clearAll();
        lastEscTime = 0;
        return;
      }
      lastEscTime = now;
      return;
    }

    // ── ? shortcut help: when input is empty, forward to parent instead of typing "?" ──
    if (e.key === "?" && !hasContent() && onShortcutHelp) {
      e.preventDefault();
      onShortcutHelp();
      return;
    }

    // ── Normal input ──
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }

  /** Wrap path in backtick fence that won't conflict with path content. */
  function wrapPathInBackticks(p: string): string {
    let maxRun = 0;
    let currentRun = 0;
    for (const ch of p) {
      if (ch === "`") {
        currentRun++;
        maxRun = Math.max(maxRun, currentRun);
      } else {
        currentRun = 0;
      }
    }
    const fence = "`".repeat(maxRun + 1);
    const needsPadding = p.startsWith("`") || p.endsWith("`");
    return needsPadding ? `${fence} ${p} ${fence}` : `${fence}${p}${fence}`;
  }

  function handleSend() {
    const typed = inputText.trim();

    // Virtual slash command check — based on raw textarea, not paste blocks
    if (typed) {
      const virtual = parseVirtualAction(typed);
      if (virtual) {
        dbg("slash", `virtual:${virtual.name}`, { args: virtual.args });
        if (virtual.name === "model" && virtual.args && onModelSwitch) {
          inputText = "";
          if (textareaEl) textareaEl.style.height = "auto";
          onModelSwitch(virtual.args);
          return; // pastedBlocks preserved
        }
        // Navigation virtual commands (e.g. /config → /settings?tab=cli-config)
        const vDef = VIRTUAL_COMMANDS.find((v) => v.name === virtual.name);
        if (vDef && typeof vDef["_navigate"] === "string") {
          inputText = "";
          if (textareaEl) textareaEl.style.height = "auto";
          goto(vDef["_navigate"] as string);
          return;
        }
        // Side question virtual command (/btw <question>)
        if (vDef && vDef["_action"] === "side-question" && onBtwSend) {
          if (virtual.args) {
            inputText = "";
            if (textareaEl) textareaEl.style.height = "auto";
            onBtwSend(virtual.args, []);
          }
          return;
        }
        // Action virtual commands (e.g. /copy → copy-last)
        if (vDef && typeof vDef["_action"] === "string" && onVirtualCommand) {
          inputText = "";
          if (textareaEl) textareaEl.style.height = "auto";
          onVirtualCommand(vDef["_action"] as string, virtual.args);
          return;
        }
      }
    }

    // Separate regular (binary) and path-reference attachments
    const regularAtts = pendingAttachments.filter((a) => a.contentBase64);
    const pathRefAtts = contextOnlyChat
      ? []
      : pendingAttachments.filter((a) => a.filePath && !a.contentBase64);

    // Combine paste blocks + typed text + path-reference file paths
    const parts: string[] = pastedBlocks.map((b) => b.text);
    if (pathRefAtts.length > 0) {
      const refs = pathRefAtts.map((a) => `[PDF: ${a.filePath}]`).join("\n");
      parts.push(refs);
    }
    // pendingPathRefs (directories, large files from drag-drop)
    if (!contextOnlyChat && pendingPathRefs.length > 0) {
      parts.push(pendingPathRefs.map((r) => wrapPathInBackticks(r.path)).join("\n"));
    }
    if (!contextOnlyChat && includeEditorContext && editorContextPath.trim()) {
      parts.push(
        t("prompt_editorContextSeed", {
          file: wrapPathInBackticks(editorContextPath.trim()),
        }),
      );
    }

    if (typed) parts.push(typed);
    const text = parts.join("\n\n");
    if (!text || disabled) return;

    dbg("prompt", "send", {
      len: text.length,
      pasteBlocks: pastedBlocks.length,
      attachments: regularAtts.length,
      pathRefs: pathRefAtts.length,
      dragPathRefs: pendingPathRefs.length,
      agent,
    });

    const attachments: Attachment[] = regularAtts.map((a) => ({
      name: a.name,
      type: a.type,
      size: a.size,
      contentBase64: a.contentBase64!,
    }));

    inputText = "";
    pendingAttachments = [];
    pastedBlocks = [];
    pendingPathRefs = [];
    resetHistory(histState);
    onSend(text, attachments);

    // Reset textarea height
    if (textareaEl) textareaEl.style.height = "auto";
  }

  function handleBtwSend() {
    if (!onBtwSend) return;
    const regularAtts = pendingAttachments.filter((a) => a.contentBase64);
    const parts = pastedBlocks.map((b) => b.text);
    const typed = inputText.trim();
    if (typed) parts.push(typed);
    const question = parts.join("\n\n").trim();
    if (!question) return;
    const attachments: Attachment[] = regularAtts.map((a) => ({
      name: a.name,
      type: a.type,
      size: a.size,
      contentBase64: a.contentBase64!,
    }));
    dbg("prompt", "btwSend", { len: question.length, attachments: attachments.length });
    inputText = "";
    pendingAttachments = [];
    pastedBlocks = [];
    pendingPathRefs = [];
    resetHistory(histState);
    if (textareaEl) textareaEl.style.height = "auto";
    onBtwSend(question, attachments);
  }

  async function processFiles(files: FileList | File[]) {
    let binaryRemaining = MAX_ATTACHMENTS - pendingAttachments.length;
    let textRemaining = MAX_PASTE_BLOCKS - pastedBlocks.length;
    const rejected: string[] = [];

    for (const file of Array.from(files)) {
      // MIME normalization: force application/pdf when detected by extension
      // (backend silently skips attachments with unrecognized MIME types)
      const detectedPdf = !isPdf(file.type) && getFileExtension(file.name) === "pdf";
      const effectivePdf = isPdf(file.type) || detectedPdf;

      // PDF >20MB ≤100MB: save to temp, use path-reference (CLI handles via pdftoppm)
      if (effectivePdf && file.size > PDF_MAX_BINARY_SIZE) {
        if (contextOnlyChat) {
          showFileToast(t("prompt_fileTooLarge", { limit: "20", name: file.name }));
          continue;
        }
        if (file.size > PDF_MAX_PATH_SIZE) {
          showFileToast(t("prompt_fileTooLarge", { limit: "100", name: file.name }));
          continue;
        }
        if (binaryRemaining <= 0) {
          showFileToast(t("prompt_maxAttachments", { count: String(MAX_ATTACHMENTS) }));
          break;
        }
        binaryRemaining--;
        try {
          const buffer = await file.arrayBuffer();
          const base64 = arrayBufferToBase64(buffer);
          const tempPath = await api.saveTempAttachment(file.name, base64);
          pendingAttachments = [
            ...pendingAttachments,
            {
              id: uuid().slice(0, 8),
              name: file.name,
              type: "application/pdf",
              size: file.size,
              filePath: tempPath,
            },
          ];
          dbg("prompt", "pdf-temp-path-ref", { name: file.name, size: file.size, path: tempPath });
        } catch (e) {
          binaryRemaining++;
          dbgWarn("prompt", "pdf-temp-save-failed", { name: file.name, error: e });
          showFileToast(t("prompt_fileTooLarge", { limit: "20", name: file.name }));
        }
        continue;
      }

      // 1) Size check — per type (images: no limit, PDF: 20MB, text: 10MB)
      const sizeLimit = getFileSizeLimit(file);
      if (file.size > sizeLimit) {
        const limitMB = sizeLimit / (1024 * 1024);
        showFileToast(t("prompt_fileTooLarge", { limit: String(limitMB), name: file.name }));
        continue;
      }

      // 2) Binary attachment: images + PDF (≤20MB)
      if (BINARY_ATTACHMENT_TYPES.includes(file.type) || detectedPdf) {
        if (binaryRemaining <= 0) {
          showFileToast(t("prompt_maxAttachments", { count: String(MAX_ATTACHMENTS) }));
          break;
        }
        binaryRemaining--;
        const reader = new FileReader();
        reader.onload = () => {
          const dataUrl = reader.result as string;
          const base64 = dataUrl.split(",")[1] ?? "";
          pendingAttachments = [
            ...pendingAttachments,
            {
              id: uuid().slice(0, 8),
              name: file.name || `attachment.${file.type.split("/")[1] || "bin"}`,
              type: detectedPdf ? "application/pdf" : file.type,
              size: file.size,
              contentBase64: base64,
            },
          ];
          dbg("prompt", "add-binary-file", { name: file.name, type: file.type, size: file.size });
        };
        reader.readAsDataURL(file);
        continue;
      }
      // 3) Text file → pastedBlock
      if (isTextFile(file)) {
        if (textRemaining <= 0) {
          showFileToast(t("prompt_maxTextFiles", { count: String(MAX_PASTE_BLOCKS) }));
          break;
        }
        textRemaining--; // Pre-decrement before async read to prevent race
        const reader = new FileReader();
        reader.onload = () => {
          const text = reader.result as string;
          const lines = text.split("\n");
          const lineCount = lines.length;
          const charCount = text.length;
          const ext = getFileExtension(file.name);
          const preview = file.name || `file.${ext}`;

          pastedBlocks = [
            ...pastedBlocks,
            {
              id: uuid().slice(0, 8),
              text,
              lineCount,
              charCount,
              preview,
              ext,
            },
          ];
          dbg("prompt", "add-text-file", {
            name: file.name,
            lines: lineCount,
            chars: charCount,
          });
        };
        reader.readAsText(file);
        continue;
      }
      // 3.5) Convertible → await conversion, then add as pastedBlock
      if (isConvertibleFile(file)) {
        if (textRemaining <= 0) {
          showFileToast(t("prompt_maxTextFiles", { count: String(MAX_PASTE_BLOCKS) }));
          break;
        }
        textRemaining--;
        try {
          const { text } = await convertFile(file);
          const lineCount = text.split("\n").length;
          pastedBlocks = [
            ...pastedBlocks,
            {
              id: uuid().slice(0, 8),
              text,
              lineCount,
              charCount: text.length,
              preview: file.name,
              ext: getFileExtension(file.name),
            },
          ];
          dbg("prompt", "converted-file", { name: file.name, lines: lineCount });
        } catch (e) {
          textRemaining++; // Roll back quota on failure
          showFileToast(t("prompt_conversionFailed", { name: file.name }));
          dbgWarn("prompt", "conversion-failed", { name: file.name, error: e });
        }
        continue;
      }
      // 4) Unsupported
      rejected.push(getFileExtension(file.name) || file.type || "unknown");
    }
    if (rejected.length > 0) {
      showFileToast(t("prompt_unsupportedFile", { ext: rejected[0] }));
    }
  }

  function handleFileSelect(e: Event) {
    const input = e.target as HTMLInputElement;
    const files = input.files;
    if (!files) return;
    processFiles(files);
    input.value = "";
  }

  function removeAttachment(id: string) {
    pendingAttachments = pendingAttachments.filter((a) => a.id !== id);
  }

  function handlePaste(e: ClipboardEvent) {
    // Step 1: Check for clipboard binary files (images, PDF) BEFORE text
    const items = e.clipboardData?.items;
    if (items) {
      const binaryItems: DataTransferItem[] = [];
      for (let i = 0; i < items.length; i++) {
        if (BINARY_ATTACHMENT_TYPES.includes(items[i].type)) {
          binaryItems.push(items[i]);
        } else if (items[i].kind === "file") {
          // Extension fallback: browser may give wrong/empty MIME for PDF
          const file = items[i].getAsFile();
          if (file && getFileExtension(file.name) === "pdf") {
            binaryItems.push(items[i]);
          }
        }
      }
      if (binaryItems.length > 0) {
        e.preventDefault();
        const filesToProcess: File[] = [];
        for (const item of binaryItems) {
          const file = item.getAsFile();
          if (file) filesToProcess.push(file);
        }
        if (filesToProcess.length > 0) processFiles(filesToProcess);
        return;
      }
    }

    // Step 2: Text paste handling
    const text = e.clipboardData?.getData("text/plain");

    if (!text) {
      // Empty text — likely Finder file paste (macOS puts file URLs, not text)
      e.preventDefault();
      tryNativeClipboardPaste();
      return;
    }

    const lines = text.split("\n");
    const lineCount = lines.length;
    const charCount = text.length;

    if (lineCount < 5 && charCount < 500) {
      // Short text — could be Finder filename or normal short text
      // Don't preventDefault → let browser insert text normally
      const snapshot = inputText;
      const cursorPos = textareaEl?.selectionStart ?? inputText.length;
      // Async check: if native clipboard has files, roll back the inserted text
      tryNativeClipboardPaste(snapshot, cursorPos);
      return;
    }

    // Long text → intercept, compress into chip
    e.preventDefault();
    if (pastedBlocks.length >= MAX_PASTE_BLOCKS) {
      showFileToast(t("prompt_maxPasteBlocks", { count: String(MAX_PASTE_BLOCKS) }));
      return;
    }

    const firstLine = lines[0].trim();
    const preview = firstLine.length > 40 ? firstLine.slice(0, 40) + "..." : firstLine;

    pastedBlocks = [
      ...pastedBlocks,
      {
        id: uuid().slice(0, 8),
        text,
        lineCount,
        charCount,
        preview,
      },
    ];

    dbg("prompt", "paste-compressed", { lineCount, charCount, blocks: pastedBlocks.length });
  }

  function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
    return Promise.race([
      promise,
      new Promise<never>((_, reject) => setTimeout(() => reject(new Error("timeout")), ms)),
    ]);
  }

  async function tryNativeClipboardPaste(snapshot?: string, cursorPos?: number) {
    try {
      const files = await withTimeout(api.getClipboardFiles(), 250);
      if (files.length === 0) return; // No files — text already inserted (or empty paste)

      dbg("prompt", "native-clipboard-files", { count: files.length });

      // Roll back browser-inserted text if we have a snapshot
      if (snapshot !== undefined) {
        inputText = snapshot;
        if (textareaEl && cursorPos !== undefined) {
          requestAnimationFrame(() => {
            textareaEl!.selectionStart = textareaEl!.selectionEnd = cursorPos;
          });
        }
      }
      await processClipboardPaths(files);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      // Only show toast when user explicitly pasted files (no text in clipboard)
      if (snapshot === undefined && msg.includes("not yet supported")) {
        showFileToast(t("prompt_clipboardUnsupported"));
      }
      dbg("prompt", "native clipboard failed/timeout", e);
    }
  }

  async function processClipboardPaths(files: ClipboardFileInfo[]) {
    let binaryRemaining = MAX_ATTACHMENTS - pendingAttachments.length;
    let textRemaining = MAX_PASTE_BLOCKS - pastedBlocks.length;
    const rejected: string[] = [];

    for (const file of files) {
      // MIME normalization: force application/pdf for extension-detected PDFs
      // (backend silently skips attachments with unrecognized MIME types)
      const clipboardPdf =
        file.mime_type !== "application/pdf" && getFileExtension(file.name).toLowerCase() === "pdf";
      const effectiveMime = clipboardPdf ? "application/pdf" : file.mime_type;

      // PDF path-reference: >20MB ≤100MB → store path only, CLI handles via pdftoppm
      if (isPdf(effectiveMime) && file.size > PDF_MAX_BINARY_SIZE) {
        if (contextOnlyChat) {
          showFileToast(t("prompt_fileTooLarge", { limit: "20", name: file.name }));
          continue;
        }
        if (file.size > PDF_MAX_PATH_SIZE) {
          showFileToast(t("prompt_fileTooLarge", { limit: "100", name: file.name }));
          continue;
        }
        if (binaryRemaining <= 0) {
          showFileToast(t("prompt_maxAttachments", { count: String(MAX_ATTACHMENTS) }));
          break;
        }
        binaryRemaining--;
        pendingAttachments = [
          ...pendingAttachments,
          {
            id: uuid().slice(0, 8),
            name: file.name,
            type: effectiveMime,
            size: file.size,
            filePath: file.path,
          },
        ];
        dbg("prompt", "clipboard-pdf-path-ref", {
          name: file.name,
          size: file.size,
          path: file.path,
        });
        continue;
      }

      const sizeLimit = getSizeLimitByMime(effectiveMime);
      if (file.size > sizeLimit) {
        const limitMB = sizeLimit / (1024 * 1024);
        showFileToast(t("prompt_fileTooLarge", { limit: String(limitMB), name: file.name }));
        continue;
      }
      const cls = classifyByMime(effectiveMime);

      if (cls === "binary") {
        if (binaryRemaining <= 0) {
          showFileToast(t("prompt_maxAttachments", { count: String(MAX_ATTACHMENTS) }));
          break;
        }
        binaryRemaining--;
        try {
          const content = await api.readClipboardFile(file.path, false);
          pendingAttachments = [
            ...pendingAttachments,
            {
              id: uuid().slice(0, 8),
              name: file.name,
              type: effectiveMime,
              size: file.size,
              contentBase64: content.content_base64,
            },
          ];
          dbg("prompt", "clipboard-binary", { name: file.name, type: effectiveMime });
        } catch (e) {
          dbg("prompt", "clipboard-read-error", { name: file.name, error: e });
        }
      } else if (cls === "text") {
        if (textRemaining <= 0) {
          showFileToast(t("prompt_maxTextFiles", { count: String(MAX_PASTE_BLOCKS) }));
          break;
        }
        textRemaining--;
        try {
          const content = await api.readClipboardFile(file.path, true);
          const text = content.content_text ?? "";
          const lineCount = text.split("\n").length;
          pastedBlocks = [
            ...pastedBlocks,
            {
              id: uuid().slice(0, 8),
              text,
              lineCount,
              charCount: text.length,
              preview: file.name,
              ext: getFileExtension(file.name),
            },
          ];
          dbg("prompt", "clipboard-text", { name: file.name, lines: lineCount });
        } catch (e) {
          dbg("prompt", "clipboard-read-error", { name: file.name, error: e });
        }
      } else if (cls === "convertible" || isConvertibleByExt(getFileExtension(file.name))) {
        if (textRemaining <= 0) {
          showFileToast(t("prompt_maxTextFiles", { count: String(MAX_PASTE_BLOCKS) }));
          break;
        }
        textRemaining--;
        try {
          const content = await api.readClipboardFile(file.path, false);
          const binary = atob(content.content_base64);
          const bytes = new Uint8Array(binary.length);
          for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
          const blob = new File([bytes], file.name, { type: file.mime_type });
          const { text } = await convertFile(blob);
          const lineCount = text.split("\n").length;
          pastedBlocks = [
            ...pastedBlocks,
            {
              id: uuid().slice(0, 8),
              text,
              lineCount,
              charCount: text.length,
              preview: file.name,
              ext: getFileExtension(file.name),
            },
          ];
          dbg("prompt", "clipboard-converted", { name: file.name, lines: lineCount });
        } catch (e) {
          textRemaining++;
          showFileToast(t("prompt_conversionFailed", { name: file.name }));
          dbgWarn("prompt", "clipboard-convert-error", { name: file.name, error: e });
        }
      } else {
        rejected.push(getFileExtension(file.name) || "unknown");
      }
    }
    if (rejected.length > 0) {
      showFileToast(t("prompt_unsupportedFile", { ext: rejected[0] }));
    }
  }

  function removePastedBlock(id: string) {
    pastedBlocks = pastedBlocks.filter((b) => b.id !== id);
  }

  function handleSkillSelect(skillName: string) {
    dbg("prompt", "skill-select fill", { skillName });
    inputText = `/${skillName} `;
    requestAnimationFrame(() => {
      autoResize();
      textareaEl?.focus();
    });
  }

  function autoResize() {
    if (!textareaEl) return;
    textareaEl.style.height = "auto";
    const maxHeight = 4 * 24; // ~4 lines
    textareaEl.style.height = Math.min(textareaEl.scrollHeight, maxHeight) + "px";
  }

  // ── Drag-drop state ──
  let dragCounter = $state(0);
  let dragActive = $derived(dragCounter > 0);

  function handleDragEnter(e: DragEvent) {
    e.preventDefault();
    dragCounter++;
  }

  function handleDragLeave(e: DragEvent) {
    e.preventDefault();
    dragCounter--;
  }

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
  }

  function handleDrop(e: DragEvent) {
    e.preventDefault();
    dragCounter = 0;
    const files = e.dataTransfer?.files;
    if (!files || files.length === 0) return;
    processFiles(files);
  }

  let canSend = $derived(
    !disabled &&
      (!!inputText.trim() ||
        pastedBlocks.length > 0 ||
        (!contextOnlyChat && pendingAttachments.some((a) => a.filePath)) ||
        (!contextOnlyChat && pendingPathRefs.length > 0)),
  );

  // ── Mode dropdown outside-click + Escape ──
  onMount(() => {
    const savedIncludeEditorContext =
      typeof localStorage !== "undefined" ? localStorage.getItem(INCLUDE_EDITOR_CONTEXT_KEY) : null;
    if (savedIncludeEditorContext !== null) {
      includeEditorContext = savedIncludeEditorContext === "true";
    }

    function onDocClick(e: MouseEvent) {
      if (
        plusMenuOpen &&
        plusBtnEl &&
        !plusBtnEl.contains(e.target as Node) &&
        plusMenuEl &&
        !plusMenuEl.contains(e.target as Node)
      ) {
        plusMenuOpen = false;
      }
      if (
        modeDropdownOpen &&
        modeBtnEl &&
        !modeBtnEl.contains(e.target as Node) &&
        modeDropdownEl &&
        !modeDropdownEl.contains(e.target as Node)
      ) {
        modeDropdownOpen = false;
      }
      if (
        modelDropdownOpen &&
        modelBtnEl &&
        !modelBtnEl.contains(e.target as Node) &&
        modelDropdownEl &&
        !modelDropdownEl.contains(e.target as Node)
      ) {
        modelDropdownOpen = false;
      }
    }
    function onDocKeydown(e: KeyboardEvent) {
      if (modeDropdownOpen && e.key === "Escape") {
        modeDropdownOpen = false;
      }
      if (modelDropdownOpen && e.key === "Escape") {
        modelDropdownOpen = false;
      }
      if (plusMenuOpen && e.key === "Escape") {
        plusMenuOpen = false;
      }
    }
    document.addEventListener("mousedown", onDocClick, true);
    document.addEventListener("keydown", onDocKeydown);
    return () => {
      document.removeEventListener("mousedown", onDocClick, true);
      document.removeEventListener("keydown", onDocKeydown);
    };
  });

  export function focus() {
    textareaEl?.focus();
  }

  export function setValue(text: string) {
    inputText = text;
    requestAnimationFrame(() => {
      autoResize();
      textareaEl?.focus();
    });
  }

  export function appendText(text: string) {
    inputText = inputText ? inputText + "\n" + text : text;
    requestAnimationFrame(() => {
      autoResize();
      textareaEl?.focus();
    });
  }

  export function triggerSend() {
    handleSend();
  }

  export function addFiles(files: FileList | File[]) {
    return processFiles(files);
  }

  export function addPathRefs(refs: Array<{ path: string; name: string; isDir: boolean }>) {
    if (contextOnlyChat) return;
    const newRefs = refs.map((ref) => ({
      id: uuid().slice(0, 8),
      name: ref.name,
      path: ref.path,
      isDir: ref.isDir,
    }));
    pendingPathRefs = [...pendingPathRefs, ...newRefs];
    dbg("prompt", "add-path-refs", { count: refs.length });
  }

  function removePathRef(id: string) {
    pendingPathRefs = pendingPathRefs.filter((r) => r.id !== id);
  }

  export function showToast(message: string, variant: "error" | "info" = "info") {
    showFileToast(message, variant);
  }

  export function getInputSnapshot(): PromptInputSnapshot {
    return {
      text: inputText,
      attachments: [...pendingAttachments],
      pastedBlocks: [...pastedBlocks],
      pathRefs: contextOnlyChat ? [] : [...pendingPathRefs],
    };
  }

  export function restoreSnapshot(snapshot: PromptInputSnapshot): void {
    inputText = snapshot.text;
    pendingAttachments = snapshot.attachments;
    pastedBlocks = snapshot.pastedBlocks as PastedBlock[];
    pendingPathRefs = contextOnlyChat ? [] : (snapshot.pathRefs ?? []);
    resetHistory(histState);
    requestAnimationFrame(() => {
      autoResize();
      textareaEl?.focus();
    });
  }

  export function clearAll(): void {
    inputText = "";
    pendingAttachments = [];
    pendingPathRefs = [];
    pastedBlocks = [];
    resetHistory(histState);
    requestAnimationFrame(() => autoResize());
  }

  function hasContent(): boolean {
    return !!(
      inputText.trim() ||
      pendingAttachments.length ||
      pastedBlocks.length ||
      pendingPathRefs.length
    );
  }
</script>

<!-- Web drag handlers — only fire when Tauri dragDropEnabled is false (non-Tauri builds).
     When dragDropEnabled: true, Tauri intercepts OS drag events and Web drag events do not fire. -->
<div
  class="relative w-full bg-transparent {variant === 'hero'
    ? 'px-0 py-0 max-w-[730px] mx-auto'
    : `px-4 pb-3 pt-2 ${
        appMode === 'cowork'
          ? 'max-w-[940px] mx-auto'
          : appMode === 'code'
            ? 'max-w-[760px] mx-auto'
            : 'max-w-[760px] mx-auto'
      }`}"
  ondragenter={handleDragEnter}
  ondragleave={handleDragLeave}
  ondragover={handleDragOver}
  ondrop={handleDrop}
>
  <!-- Drag overlay -->
  {#if dragActive}
    <div
      class="absolute inset-0 z-10 flex items-center justify-center rounded-lg border-2 border-dashed border-primary/50 bg-primary/5 backdrop-blur-[1px]"
    >
      <span class="text-sm font-medium text-primary/70">{t("prompt_dropFiles")}</span>
    </div>
  {/if}

  <!-- File toast -->
  {#if toastMessage}
    <div
      class="absolute -top-10 left-4 right-4 z-20 flex items-center gap-2 rounded-md px-3 py-1.5 text-xs shadow-lg animate-fade-in {toastVariant ===
      'error'
        ? 'bg-destructive/90 text-destructive-foreground'
        : 'bg-muted text-foreground'}"
    >
      <svg
        class="h-3.5 w-3.5 shrink-0"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <circle cx="12" cy="12" r="10" /><path d="M12 8v4" /><path d="M12 16h.01" />
      </svg>
      <span>{toastMessage}</span>
    </div>
  {/if}

  {#if variant === "hero"}
    <img
      src="/helion-mascot.png"
      alt=""
      aria-hidden="true"
      class="pointer-events-none absolute right-1 -top-[101px] z-20 hidden w-[330px] max-w-[42vw] select-none drop-shadow-[0_18px_36px_rgba(15,23,42,0.16)] sm:block"
    />
  {/if}

  <!-- Attachment & paste block previews -->
  {#if pendingAttachments.length > 0 || pastedBlocks.length > 0 || pendingPathRefs.length > 0}
    <div class="mb-2 flex flex-wrap gap-1.5">
      {#each pendingAttachments as att (att.id)}
        <FileAttachment
          name={att.name}
          size={att.size}
          mimeType={att.type}
          isPathRef={!!att.filePath && !att.contentBase64}
          onremove={() => removeAttachment(att.id)}
        />
      {/each}
      {#each pendingPathRefs as ref (ref.id)}
        <FileAttachment
          name={ref.name}
          size={0}
          mimeType={ref.isDir ? "inode/directory" : "application/octet-stream"}
          isPathRef={true}
          onremove={() => removePathRef(ref.id)}
        />
      {/each}
      {#each pastedBlocks as block (block.id)}
        {@const isSpreadsheet = block.ext ? isSpreadsheetExt(block.ext) : false}
        <span
          class="inline-flex items-center gap-1.5 rounded-md border border-blue-200 dark:border-blue-800 bg-blue-50 dark:bg-blue-950/50 text-blue-700 dark:text-blue-300 px-2 py-1 text-xs"
        >
          {#if isSpreadsheet}
            <!-- Table/spreadsheet icon -->
            <svg
              class="h-3.5 w-3.5 shrink-0"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
              <path d="M3 9h18" /><path d="M3 15h18" /><path d="M9 3v18" />
            </svg>
          {:else}
            <!-- Clipboard icon for text -->
            <svg
              class="h-3.5 w-3.5 shrink-0"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <rect width="8" height="4" x="8" y="2" rx="1" ry="1" /><path
                d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"
              />
            </svg>
          {/if}
          <span class="truncate max-w-[200px]">{block.preview}</span>
          <span class="text-blue-400 dark:text-blue-500"
            >{formatPasteSize(block.lineCount, block.charCount)}</span
          >
          <button
            onclick={() => removePastedBlock(block.id)}
            class="ml-0.5 rounded p-0.5 transition-colors hover:bg-blue-200/50 dark:hover:bg-blue-800/50"
            title={t("prompt_removePaste")}
          >
            <svg
              class="h-3 w-3"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/each}
    </div>
  {/if}

  {#if variant !== "hero" && showWorkspaceChangeSummary && workspaceSummary}
    <div class="pb-2">
      <button
        type="button"
        class="flex w-full items-center gap-2 rounded-2xl border border-border/65 bg-background/95 px-4 py-2 text-sm font-medium text-muted-foreground shadow-sm transition-colors hover:border-border hover:bg-accent/70 hover:text-foreground"
        onclick={() => (onReviewChanges ?? onOpenEditedFiles)?.()}
        title={t("prompt_viewChanges")}
      >
        <span class="h-2 w-2 shrink-0 rounded-full bg-emerald-500"></span>
        <span class="min-w-0 flex-1 truncate text-left tabular-nums">{workspaceChangeLabel}</span>
        {#if workspaceSummary.total_insertions > 0}
          {#key workspaceSummary.total_insertions}
            <span class="count-pop tabular-nums text-emerald-500"
              >+{workspaceSummary.total_insertions}</span
            >
          {/key}
        {/if}
        {#if workspaceSummary.total_deletions > 0}
          {#key workspaceSummary.total_deletions}
            <span class="count-pop tabular-nums text-red-400"
              >-{workspaceSummary.total_deletions}</span
            >
          {/key}
        {/if}
        <span class="shrink-0 border-l border-border/70 pl-2 text-primary"
          >{t("prompt_viewChanges")}</span
        >
      </button>
    </div>
  {/if}

  {#if showEditedChangeSummary && editedSummary}
    <div class="pb-2">
      <button
        type="button"
        class="flex w-full items-center gap-2 rounded-2xl border border-border/65 bg-background/95 px-4 py-2 text-sm font-medium text-muted-foreground shadow-sm transition-colors hover:border-border hover:bg-accent/70 hover:text-foreground"
        onclick={() => (onOpenEditedFiles ?? onReviewChanges)?.()}
        title={t("prompt_editedFiles")}
      >
        {#if editedSummary.isEditing}
          <span class="h-2 w-2 shrink-0 rounded-full bg-amber-400 animate-pulse"></span>
        {:else}
          <span class="h-2 w-2 shrink-0 rounded-full bg-emerald-500"></span>
        {/if}
        <span class="min-w-0 flex-1 truncate text-left">{editedStatusLabel}</span>
        {#if editedSummary.totalAdditions > 0}
          {#key editedSummary.totalAdditions}
            <span class="count-pop tabular-nums text-emerald-500"
              >+{editedSummary.totalAdditions}</span
            >
          {/key}
        {/if}
        {#if editedSummary.totalDeletions > 0}
          {#key editedSummary.totalDeletions}
            <span class="count-pop tabular-nums text-red-400">-{editedSummary.totalDeletions}</span>
          {/key}
        {/if}
        <span class="shrink-0 border-l border-border/70 pl-2 text-primary"
          >{t("prompt_viewChanges")}</span
        >
      </button>
    </div>
  {/if}

  <!-- Unified input container -->
  <div
    class="{variant === 'hero'
      ? 'relative z-10 overflow-visible rounded-[20px] border bg-background/86 shadow-[0_18px_50px_rgba(15,23,42,0.12)] backdrop-blur-xl'
      : appMode === 'cowork'
        ? 'rounded-xl border bg-background shadow-[0_1px_3px_rgba(0,0,0,0.14)]'
        : 'rounded-lg border bg-muted/30 shadow-sm'} transition-colors {btwMode
      ? 'border-blue-500/50 shadow-[0_0_0_1px_rgba(59,130,246,0.15)]'
      : currentMode.borderCls ||
        'border-border focus-within:border-ring/50 focus-within:shadow-[0_0_0_1px_hsl(var(--ring)/0.15)]'}"
  >
    {#if variant === "hero" && showWorkspaceChangeSummary && workspaceSummary}
      <div
        class="absolute -top-px left-0 right-0 z-20 flex items-center gap-2 rounded-t-[20px] border-b border-border/55 bg-background/58 px-4 py-2 text-xs text-muted-foreground shadow-[0_8px_26px_rgba(15,23,42,0.06)] backdrop-blur-xl"
      >
        <span class="min-w-0 truncate tabular-nums">{workspaceChangeLabel}</span>
        {#if workspaceSummary.total_insertions > 0}
          {#key workspaceSummary.total_insertions}
            <span class="count-pop tabular-nums text-emerald-500"
              >+{workspaceSummary.total_insertions}</span
            >
          {/key}
        {/if}
        {#if workspaceSummary.total_deletions > 0}
          {#key workspaceSummary.total_deletions}
            <span class="count-pop tabular-nums text-red-400"
              >-{workspaceSummary.total_deletions}</span
            >
          {/key}
        {/if}
        <button
          type="button"
          class="ml-auto shrink-0 rounded-md px-2 py-0.5 font-medium text-foreground/80 transition-colors hover:bg-accent hover:text-foreground"
          onclick={() => (onReviewChanges ?? onOpenEditedFiles)?.()}
        >
          {t("prompt_viewChanges")}
        </button>
      </div>
    {/if}

    {#if pendingPermission}
      <div
        role="status"
        aria-live="polite"
        class="flex items-center gap-2 px-4 pt-2.5 pb-0.5 text-xs text-amber-500"
      >
        <svg
          class="h-3.5 w-3.5 shrink-0"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <circle cx="12" cy="12" r="10" />
          <line x1="12" y1="8" x2="12" y2="12" />
          <line x1="12" y1="16" x2="12.01" y2="16" />
        </svg>
        <span>{t("prompt_pendingPermission")}</span>
      </div>
    {/if}

    <!-- Textarea -->
    <textarea
      bind:this={textareaEl}
      bind:value={inputText}
      onkeydown={handleKeydown}
      oninput={handleInput}
      onpaste={handlePaste}
      placeholder={effectivePlaceholder}
      rows={1}
      {disabled}
      class="relative z-10 w-full resize-none bg-transparent {variant === 'hero'
        ? `px-5 ${showWorkspaceChangeSummary ? 'pt-12' : 'pt-4'} pb-4 text-[15px]`
        : appMode === 'cowork'
          ? 'px-5 pt-4 pb-5 text-lg'
          : 'px-4 pt-3 pb-2 text-sm'} text-foreground placeholder:text-muted-foreground/60 focus:outline-none disabled:opacity-50"
      style={variant === "hero"
        ? "min-height: 78px;"
        : appMode === "cowork"
          ? "min-height: 68px;"
          : "min-height: 36px;"}
    ></textarea>

    {#if atMenuOpen}
      <AtMentionMenu
        entries={atResults}
        selectedIndex={atSelectedIndex}
        loading={atLoading}
        query={atQuery}
        anchorEl={textareaEl}
        onSelect={selectAtEntry}
        onHover={(i) => (atSelectedIndex = i)}
        onDismiss={() => closeAtMenu("click-outside")}
      />
    {/if}

    {#if slashMenuOpen}
      <SlashMenu
        commands={filteredCommands}
        {slashGroups}
        selectedIndex={slashSelectedIndex}
        anchorEl={textareaEl}
        triggerEl={slashBtnEl}
        phase={slashPhase}
        {models}
        {currentModel}
        subSelectedIndex={slashSubSelectedIndex}
        {hintText}
        inputDisplay={inputText}
        {fastModeState}
        onSelect={(cmd) => selectSlashCommand(cmd, "enter")}
        onHover={(i) => (slashSelectedIndex = i)}
        onSubHover={(i) => (slashSubSelectedIndex = i)}
        onSubSelect={handleSubModelSelect}
        onFastSelect={handleFastModeSelect}
        onBack={goBackToCommands}
        onDismiss={() => closeSlashMenu("click-outside")}
      />
    {/if}

    <!-- Bottom action bar -->
    <div
      class="flex items-center justify-between px-2 pb-2 {appMode === 'cowork' ? 'min-h-10' : ''}"
    >
      <!-- Left: agent selector + permission mode -->
      <div class="flex items-center gap-1">
        {#if !hasRun && onAgentChange}
          <AgentSelector value={agent} onchange={(a) => onAgentChange?.(a)} />
        {/if}
        {#if !contextOnlyChat}
          <button
            bind:this={plusBtnEl}
            type="button"
            class="flex h-7 w-7 items-center justify-center rounded-full transition-colors disabled:opacity-30 {plusMenuOpen
              ? 'bg-accent text-foreground'
              : 'text-muted-foreground/70 hover:bg-accent hover:text-foreground'}"
            onclick={togglePlusMenu}
            {disabled}
            title={t("prompt_moreInputOptions")}
            aria-label={t("prompt_moreInputOptions")}
            aria-expanded={plusMenuOpen}
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
              <path d="M12 5v14" /><path d="M5 12h14" />
            </svg>
          </button>
        {/if}
        {#if !contextOnlyChat && onPermissionModeChange}
          <button
            bind:this={modeBtnEl}
            class="flex items-center gap-1 rounded-md px-1.5 py-0.5 {appMode === 'cowork'
              ? 'text-sm'
              : 'text-[11px]'} font-medium transition-colors {currentMode.cls} hover:bg-accent border border-transparent hover:border-border"
            onclick={toggleModeDropdown}
            title={t("prompt_permissionModeTitle", { mode: currentMode.label() })}
          >
            <svg
              class="h-3 w-3"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path
                d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"
              />
            </svg>
            {currentMode.shortLabel()}
            <svg
              class="h-2.5 w-2.5 text-foreground/30"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"><path d="m6 9 6 6 6-6" /></svg
            >
          </button>
        {:else if !hasRun}
          <div class="w-1"></div>
        {/if}
        {#if !contextOnlyChat}
          <button
            class="flex items-center gap-1 rounded-md px-1.5 py-0.5 {appMode === 'cowork'
              ? 'text-sm'
              : 'text-[11px]'} font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground border border-transparent hover:border-border"
            onclick={() => window.dispatchEvent(new CustomEvent("ocv:open-permissions"))}
            title={t("permissions_title")}
          >
            <svg
              class="h-3 w-3"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <rect width="18" height="11" x="3" y="11" rx="2" ry="2" />
              <path d="M7 11V7a5 5 0 0 1 10 0v4" />
            </svg>
            {t("permissions_rules")}
          </button>
        {/if}
        {#if showAuthBadge && !hasRun}
          <AuthSourceBadge
            {authOverview}
            {authSourceLabel}
            {authSourceCategory}
            {apiKeySource}
            {hasRun}
            {authMode}
            {platformCredentials}
            {platformId}
            {onAuthModeChange}
            {onPlatformChange}
            {localProxyStatuses}
          />
        {/if}
        {#if !contextOnlyChat}
          <SkillSelector
            skills={skillItems}
            {agents}
            disabled={disabled || running}
            onSelect={handleSkillSelect}
          />
        {/if}
        {#if hasStash && onRestoreStash}
          <button
            class="flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium bg-violet-500/15 text-violet-400 hover:bg-violet-500/25 transition-colors"
            title={t("prompt_stashRestore")}
            onclick={onRestoreStash}
          >
            <svg
              class="h-3 w-3"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" /><path d="M3 3v5h5" />
            </svg>
            {t("prompt_stashBadge")}
          </button>
        {/if}
      </div>

      <!-- Right: actions -->
      <div class="flex items-center gap-0.5">
        {#if slashEnabled}
          <button
            bind:this={slashBtnEl}
            class="flex h-7 w-7 items-center justify-center rounded-lg
              text-muted-foreground/60 hover:text-foreground hover:bg-accent transition-colors"
            onclick={openSlashMenuFromButton}
            title={t("prompt_slashCommands")}
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
              <path d="M17 3 7 21" />
            </svg>
          </button>
        {/if}
        <input
          bind:this={fileInput}
          type="file"
          multiple
          accept="image/png,image/jpeg,image/webp,image/gif,application/pdf,.txt,.md,.json,.ts,.tsx,.js,.jsx,.py,.rs,.svelte,.html,.css,.yaml,.yml,.toml,.xml,.sh,.sql,.go,.java,.c,.cpp,.h,.rb,.php,.swift,.csv,.log,.docx,.xlsx,.pptx"
          class="hidden"
          onchange={handleFileSelect}
        />
        <button
          class="flex h-7 w-7 items-center justify-center rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-accent transition-colors disabled:opacity-30"
          onclick={() => {
            plusMenuOpen = false;
            fileInput?.click();
          }}
          disabled={pendingAttachments.length >= 8}
          title={t("prompt_attachFiles")}
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
            <path
              d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"
            />
          </svg>
        </button>
        {#if IS_MAC}
          <!-- Screenshot capture button (macOS only) -->
          <button
            class="flex h-7 w-7 items-center justify-center rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-accent transition-colors disabled:opacity-30"
            onclick={() => api.captureScreenshot()}
            disabled={pendingAttachments.length >= 8}
            title={t("prompt_screenshot")}
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
              <path
                d="M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3l-2.5-3z"
              />
              <circle cx="12" cy="13" r="3" />
            </svg>
          </button>
        {/if}
        {#if onModelSwitch || onEffortChange}
          <button
            bind:this={modelBtnEl}
            type="button"
            class="ml-1 flex h-8 max-w-[220px] items-center gap-2 rounded-xl bg-muted/70 px-3 text-xs font-medium text-foreground transition-colors hover:bg-accent"
            onclick={toggleModelDropdown}
            title={currentModel || modelDisplayLabel}
          >
            <span class="min-w-0 max-w-[128px] truncate">{modelDisplayLabel}</span>
            <span class="shrink-0 text-muted-foreground">{effortDisplayLabel}</span>
            <svg
              class="h-3 w-3 shrink-0 text-muted-foreground"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
            >
              <path d="m6 9 6 6 6-6" />
            </svg>
          </button>
        {/if}

        {#if onBtwSend}
          <button
            onclick={() => (btwMode = !btwMode)}
            title={t("prompt_sideChat")}
            aria-pressed={btwMode}
            class="flex h-7 w-7 items-center justify-center rounded-lg transition-colors {btwMode
              ? 'bg-blue-500/10 text-blue-500'
              : 'text-muted-foreground/60 hover:bg-accent hover:text-foreground'}"
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
              <path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z" />
            </svg>
          </button>
        {/if}

        {#if running && onInterrupt}
          {#if canSend}
            {#if btwMode}
              <!-- BTW send: blue theme -->
              <button
                class="flex h-7 w-7 items-center justify-center rounded-lg transition-colors bg-blue-500 text-white hover:bg-blue-600"
                onclick={handleBtwSend}
                title={t("prompt_sideChatSend")}
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
                  <path d="M5 12h14" /><path d="m12 5 7 7-7 7" />
                </svg>
              </button>
            {:else}
              <!-- Mid-turn send: allow injecting a message while agent is running -->
              <button
                class="flex h-7 w-7 items-center justify-center rounded-lg transition-colors bg-primary text-primary-foreground hover:bg-primary/90"
                onclick={handleSend}
                title={t("prompt_send")}
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
                  <path d="M5 12h14" /><path d="m12 5 7 7-7 7" />
                </svg>
              </button>
            {/if}
          {/if}
          <button
            class="flex h-7 w-7 items-center justify-center rounded-lg text-destructive hover:bg-destructive/10 transition-colors"
            onclick={onInterrupt}
            title={t("prompt_stop")}
          >
            <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="currentColor">
              <rect x="6" y="6" width="12" height="12" rx="2" />
            </svg>
          </button>
        {:else}
          <button
            class="flex h-7 w-7 items-center justify-center rounded-lg transition-colors {canSend
              ? btwMode
                ? 'bg-blue-500 text-white hover:bg-blue-600'
                : 'bg-primary text-primary-foreground hover:bg-primary/90'
              : 'text-muted-foreground/40'}"
            onclick={btwMode ? handleBtwSend : handleSend}
            disabled={!canSend}
            title={btwMode ? t("prompt_sideChatSend") : t("prompt_send")}
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
              <path d="M5 12h14" /><path d="m12 5 7 7-7 7" />
            </svg>
          </button>
        {/if}
      </div>
    </div>
  </div>

  {#if plusMenuOpen}
    <div
      bind:this={plusMenuEl}
      class="z-[70] w-[276px] overflow-hidden rounded-2xl border border-border/70 bg-popover/95 p-1.5 shadow-2xl backdrop-blur-xl animate-fade-in"
      style={plusMenuStyle}
    >
      <button
        type="button"
        class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left text-sm text-foreground transition-colors hover:bg-accent/80 disabled:cursor-not-allowed disabled:opacity-45"
        onclick={choosePlusAttachment}
        disabled={pendingAttachments.length >= MAX_ATTACHMENTS}
      >
        <span
          class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"
        >
          <svg
            class="h-4 w-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.9"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path
              d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"
            />
          </svg>
        </span>
        <span class="min-w-0 flex-1 truncate">{t("prompt_addImagesAndFiles")}</span>
      </button>

      <button
        type="button"
        class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left text-sm text-foreground transition-colors hover:bg-accent/80"
        onclick={toggleEditorContext}
        aria-pressed={includeEditorContext}
      >
        <span
          class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"
        >
          <svg
            class="h-4 w-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.9"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8Z" />
            <path d="M19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8Z" />
          </svg>
        </span>
        <span class="min-w-0 flex-1 truncate">{t("prompt_includeEditorContext")}</span>
        <span
          class="relative h-5 w-9 shrink-0 rounded-full transition-colors {includeEditorContext
            ? 'bg-primary'
            : 'bg-muted-foreground/25'}"
        >
          <span
            class="absolute top-0.5 h-4 w-4 rounded-full bg-background shadow-sm transition-transform {includeEditorContext
              ? 'translate-x-[18px]'
              : 'translate-x-0.5'}"
          ></span>
        </span>
      </button>

      <button
        type="button"
        class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left text-sm text-foreground transition-colors hover:bg-accent/80 disabled:cursor-not-allowed disabled:opacity-45"
        onclick={togglePlanFromPlusMenu}
        disabled={!onPermissionModeChange}
        aria-pressed={permissionMode === "plan"}
      >
        <span
          class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"
        >
          <svg
            class="h-4 w-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.9"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M8 6h13" /><path d="M8 12h13" /><path d="M8 18h13" />
            <path d="M3 6h.01" /><path d="M3 12h.01" /><path d="M3 18h.01" />
          </svg>
        </span>
        <span class="min-w-0 flex-1 truncate">{t("prompt_planMode")}</span>
        <span
          class="relative h-5 w-9 shrink-0 rounded-full transition-colors {permissionMode === 'plan'
            ? 'bg-purple-500'
            : 'bg-muted-foreground/25'}"
        >
          <span
            class="absolute top-0.5 h-4 w-4 rounded-full bg-background shadow-sm transition-transform {permissionMode ===
            'plan'
              ? 'translate-x-[18px]'
              : 'translate-x-0.5'}"
          ></span>
        </span>
      </button>

      <button
        type="button"
        class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left text-sm text-foreground transition-colors hover:bg-accent/80"
        onclick={openPluginsFromPlusMenu}
      >
        <span
          class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"
        >
          <svg
            class="h-4 w-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.9"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <rect x="3" y="3" width="7" height="7" rx="1.5" />
            <rect x="14" y="3" width="7" height="7" rx="1.5" />
            <rect x="3" y="14" width="7" height="7" rx="1.5" />
            <rect x="14" y="14" width="7" height="7" rx="1.5" />
          </svg>
        </span>
        <span class="min-w-0 flex-1 truncate">{t("sidebar_plugins")}</span>
        <svg
          class="h-4 w-4 shrink-0 text-muted-foreground"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.9"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="m9 18 6-6-6-6" />
        </svg>
      </button>
    </div>
  {/if}

  {#if modeDropdownOpen}
    <div
      bind:this={modeDropdownEl}
      class="z-[70] w-[380px] overflow-hidden rounded-xl border border-border/70 bg-popover/95 p-1.5 shadow-2xl backdrop-blur-xl animate-fade-in"
      style={modeDropdownStyle}
    >
      {#each PERMISSION_MODES as mode}
        <button
          class="flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left transition-colors hover:bg-accent/80
            {permissionMode === mode.value ? 'bg-accent text-foreground' : 'text-muted-foreground'}"
          onclick={() => selectMode(mode.value)}
        >
          <span class="flex h-5 w-5 shrink-0 items-center justify-center">
            {#if permissionMode === mode.value}
              <svg
                class="h-4 w-4 text-foreground"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M20 6 9 17l-5-5" />
              </svg>
            {:else if mode.icon === "hand"}
              <svg
                class="h-4 w-4"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.8"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M18 11V6a2 2 0 0 0-4 0v5" />
                <path d="M14 10V5a2 2 0 0 0-4 0v6" />
                <path d="M10 10.5V7a2 2 0 0 0-4 0v6" />
                <path d="M18 11a2 2 0 1 1 4 0v2a9 9 0 0 1-18 0v-1" />
              </svg>
            {:else if mode.icon === "edit"}
              <svg
                class="h-4 w-4"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.8"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M12 20h9" />
                <path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z" />
              </svg>
            {:else if mode.icon === "warning"}
              <svg
                class="h-4 w-4"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.8"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path
                  d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"
                />
                <path d="M12 9v4" /><path d="M12 17h.01" />
              </svg>
            {:else if mode.icon === "list"}
              <svg
                class="h-4 w-4"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.8"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M8 6h13" /><path d="M8 12h13" /><path d="M8 18h13" />
                <path d="M3 6h.01" /><path d="M3 12h.01" /><path d="M3 18h.01" />
              </svg>
            {:else if mode.icon === "spark"}
              <svg
                class="h-4 w-4"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.8"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8Z" />
                <path d="M19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8Z" />
              </svg>
            {:else}
              <svg
                class="h-4 w-4"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.8"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <circle cx="12" cy="12" r="9" /><path d="m5 5 14 14" />
              </svg>
            {/if}
          </span>
          <span class="w-24 shrink-0 text-[15px] font-semibold {mode.cls}">{mode.label()}</span>
          <span class="min-w-0 flex-1 truncate text-sm text-muted-foreground">
            {mode.description()}
          </span>
        </button>
      {/each}
    </div>
  {/if}

  {#if modelDropdownOpen}
    <div
      bind:this={modelDropdownEl}
      class="z-[70] w-[300px] overflow-hidden rounded-xl border bg-background shadow-xl animate-fade-in"
      style={modelDropdownStyle}
    >
      <div
        class="border-b border-border/70 px-3 py-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground"
      >
        {currentLocale().startsWith("zh") ? "模型" : "Model"}
      </div>
      <div class="max-h-56 overflow-y-auto p-1">
        {#if models.length > 0}
          {#each models as model, i (model.value + ":" + i)}
            <button
              type="button"
              class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-xs transition-colors hover:bg-accent {currentModel ===
              model.value
                ? 'bg-accent text-foreground'
                : 'text-muted-foreground'}"
              onclick={() => selectModelFromDropdown(model.value)}
            >
              {#if currentModel === model.value}
                <svg
                  class="h-3.5 w-3.5 shrink-0 text-primary"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                >
                  <path d="M20 6 9 17l-5-5" />
                </svg>
              {:else}
                <span class="h-3.5 w-3.5 shrink-0"></span>
              {/if}
              <span class="min-w-0 flex-1 truncate">{model.displayName || model.value}</span>
            </button>
          {/each}
        {:else}
          <div class="px-3 py-3 text-xs text-muted-foreground">
            {currentLocale().startsWith("zh") ? "暂无可选模型" : "No models loaded"}
          </div>
        {/if}
      </div>
      {#if onEffortChange}
        <div
          class="border-t border-border/70 px-3 py-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground"
        >
          {currentLocale().startsWith("zh") ? "推理强度" : "Reasoning"}
        </div>
        <div class="grid grid-cols-4 gap-1 px-2 pb-2">
          {#each availableEfforts as option (option.value)}
            <button
              type="button"
              class="rounded-lg px-2 py-1.5 text-xs font-medium transition-colors {normalizedEffort ===
              option.value
                ? 'bg-primary text-primary-foreground'
                : 'text-muted-foreground hover:bg-accent hover:text-foreground'}"
              onclick={() => selectEffortFromDropdown(option.value)}
            >
              {currentLocale().startsWith("zh") ? option.zh : option.en}
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>
