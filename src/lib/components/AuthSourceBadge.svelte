<script lang="ts">
  import { goto } from "$app/navigation";
  import type { AuthOverview, PlatformCredential } from "$lib/types";
  import { t } from "$lib/i18n/index.svelte";
  import { onMount } from "svelte";

  let {
    authOverview = null,
    authSourceLabel = "",
    authSourceCategory = "unknown",
    apiKeySource = "",
    hasRun = false,
    authMode = "cli",
    platformCredentials: _platformCredentials = [],
    platformId: _platformId = "anthropic",
    onAuthModeChange: _onAuthModeChange,
    onPlatformChange: _onPlatformChange,
    variant = "default",
    localProxyStatuses: _localProxyStatuses = {},
  }: {
    authOverview?: AuthOverview | null;
    authSourceLabel?: string;
    authSourceCategory?: string;
    apiKeySource?: string;
    hasRun?: boolean;
    authMode?: string;
    platformCredentials?: PlatformCredential[];
    platformId?: string;
    onAuthModeChange?: (mode: string) => void;
    onPlatformChange?: (platformId: string) => void;
    variant?: "default" | "hero";
    localProxyStatuses?: Record<string, { running: boolean; needsAuth: boolean }>;
  } = $props();

  let dropdownOpen = $state(false);
  let wrapperEl: HTMLDivElement | undefined = $state();
  let buttonEl: HTMLButtonElement | undefined = $state();
  let dropdownStyle = $state("");

  const BADGE_COLORS: Record<string, string> = {
    login: "bg-emerald-500/15 text-emerald-500",
    env_key: "bg-blue-500/15 text-blue-400",
    none: "bg-amber-500/15 text-amber-500",
    other: "bg-foreground/10 text-foreground/60",
  };

  let badgeColor = $derived(BADGE_COLORS[authSourceCategory] ?? BADGE_COLORS.other);
  let preSessionLabel = $derived(authOverview ? t("auth_cliAuth") : "");
  let preSessionDotColor = $derived.by(() => {
    if (!authOverview) return "bg-muted-foreground/30";
    return authOverview.cli_login_available || authOverview.cli_has_api_key
      ? "bg-emerald-500"
      : "bg-amber-500";
  });
  let loadingLabel = $derived(authMode === "cli" ? t("auth_cliAuth") : t("auth_cliAuth"));

  function toggleDropdown() {
    if (hasRun) return;
    dropdownOpen = !dropdownOpen;
    if (dropdownOpen && buttonEl) updateDropdownPosition();
  }

  function updateDropdownPosition() {
    if (!buttonEl) return;
    const rect = buttonEl.getBoundingClientRect();
    const spaceBelow = window.innerHeight - rect.bottom;
    if (spaceBelow < 220) {
      dropdownStyle = `position:fixed; bottom:${window.innerHeight - rect.top + 4}px; left:${rect.left}px; z-index:50;`;
    } else {
      dropdownStyle = `position:fixed; top:${rect.bottom + 4}px; left:${rect.left}px; z-index:50;`;
    }
  }

  onMount(() => {
    function onDocClick(e: MouseEvent) {
      if (dropdownOpen && wrapperEl && !wrapperEl.contains(e.target as Node)) {
        dropdownOpen = false;
      }
    }
    function onDocKeydown(e: KeyboardEvent) {
      if (dropdownOpen && e.key === "Escape") {
        dropdownOpen = false;
      }
    }
    document.addEventListener("mousedown", onDocClick, true);
    document.addEventListener("keydown", onDocKeydown);
    return () => {
      document.removeEventListener("mousedown", onDocClick, true);
      document.removeEventListener("keydown", onDocKeydown);
    };
  });
</script>

{#if hasRun && authSourceLabel}
  <span
    class="shrink-0 rounded-md px-2 py-0.5 text-[11px] font-medium {badgeColor}"
    title={t("statusbar_authTitle", { source: apiKeySource })}
  >
    {authSourceLabel}
  </span>
{:else if !hasRun && authOverview}
  <div bind:this={wrapperEl} class="inline-flex items-center">
    <button
      bind:this={buttonEl}
      class="flex items-center gap-1.5 rounded-md transition-colors cursor-pointer
        {variant === 'hero'
        ? 'px-2.5 py-1 text-xs text-muted-foreground hover:text-foreground'
        : 'border px-2 py-1 text-xs font-medium hover:bg-accent'}"
      onclick={toggleDropdown}
      title={t("auth_sourceLabel")}
    >
      <span class="inline-block h-1.5 w-1.5 rounded-full {preSessionDotColor}"></span>
      {preSessionLabel}
      <svg
        class="h-2.5 w-2.5 text-muted-foreground"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"><path d="m6 9 6 6 6-6" /></svg
      >
    </button>

    {#if dropdownOpen}
      <div
        class="w-72 rounded-md border bg-background shadow-lg animate-fade-in"
        style={dropdownStyle}
      >
        <div class="space-y-1 p-2">
          <p
            class="px-2 pt-1 pb-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground/60"
          >
            {t("settings_auth_cliConfigTitle")}
          </p>
          <div class="rounded-sm bg-accent px-2.5 py-2">
            <div class="flex items-start gap-2.5">
              <span class="mt-1 inline-block h-2 w-2 shrink-0 rounded-full {preSessionDotColor}"
              ></span>
              <div class="min-w-0 flex-1">
                <p class="text-xs font-medium">{t("auth_cliAuth")}</p>
                {#if authOverview.cli_login_available}
                  <p class="mt-0.5 text-[10px] text-emerald-500">
                    {t("auth_loggedIn")}{authOverview.cli_login_account
                      ? `: ${authOverview.cli_login_account}`
                      : ""}
                  </p>
                {/if}
                {#if authOverview.cli_has_api_key}
                  <p class="mt-0.5 text-[10px] text-emerald-500">
                    {t("auth_cliKeyHint", { hint: authOverview.cli_api_key_hint ?? "" })}
                  </p>
                {:else}
                  <p class="mt-0.5 text-[10px] text-amber-500">
                    {t("settings_auth_cliNeedsApiKey")}
                  </p>
                {/if}
              </div>
            </div>
          </div>

          <button
            class="flex w-full items-center gap-1.5 rounded-sm px-2.5 py-1.5 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            onclick={() => {
              dropdownOpen = false;
              goto("/settings?tab=connection");
            }}
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
                d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"
              />
              <circle cx="12" cy="12" r="3" />
            </svg>
            {t("auth_configureInSettings")}
          </button>
        </div>
      </div>
    {/if}
  </div>
{:else if !hasRun && loadingLabel}
  <span
    class="inline-flex items-center gap-1.5 rounded-md border border-transparent px-2 py-1 text-xs font-medium text-muted-foreground/70"
  >
    <span class="inline-block h-1.5 w-1.5 rounded-full bg-muted-foreground/30"></span>
    {loadingLabel}
  </span>
{/if}
