<script lang="ts">
  import {
    checkAgentCli,
    checkAuthStatus,
    detectInstallMethods,
    getCliDistTags,
    installHelioncoderCli,
    listApiModels,
    runClaudeLogin,
    setCliApiConfig,
    updateUserSettings,
  } from "$lib/api";
  import { loadCliInfo } from "$lib/stores";
  import type { InstallMethod, PlatformCredential, PlatformPreset } from "$lib/types";
  import { PLATFORM_PRESETS, PRESET_CATEGORIES } from "$lib/utils/platform-presets";
  import { dbg, dbgWarn } from "$lib/utils/debug";
  import { IS_WINDOWS } from "$lib/utils/platform";
  import { getTransport } from "$lib/transport";
  import { t } from "$lib/i18n/index.svelte";

  let { onComplete }: { onComplete: () => void } = $props();

  type WizardStep =
    | "checking"
    | "cli_not_found"
    | "auth_choice"
    | "oauth_login"
    | "api_key_setup"
    | "done";

  let step = $state<WizardStep>("checking");
  let error = $state("");

  // CLI install state
  let installMethods = $state<InstallMethod[]>([]);
  let cliInstalling = $state(false);
  let cliInstallAttempted = $state(false);
  let cliInstallStatus = $state<"idle" | "running" | "success" | "failed">("idle");
  let cliInstallError = $state("");
  let cliLatestVersion = $state<string | null>(null);

  // Copy button state: method id → "copy" | "copied"
  let copyStates = $state<Record<string, string>>({});

  // Recheck state
  let rechecking = $state(false);
  let locatingCli = $state(false);
  let cliLocateError = $state("");

  // OAuth state
  let oauthLoading = $state(false);
  let installProgress = $state<string[]>([]);

  // API key state
  let selectedPlatform = $state<PlatformPreset | null>(null);
  let apiKey = $state("");
  let customBaseUrl = $state("");
  let showKey = $state(false);
  let saving = $state(false);

  // Done state
  let doneTimer = $state<ReturnType<typeof setTimeout> | null>(null);

  // Start checking on mount
  $effect(() => {
    if (step === "checking") {
      runInitialCheck();
    }
  });

  async function runInitialCheck() {
    dbg("wizard", "starting initial check");
    try {
      const [cliResult, authResult] = await Promise.all([
        checkAgentCli("helioncoder"),
        checkAuthStatus(),
      ]);

      dbg("wizard", "check results", {
        cliFound: cliResult.found,
        hasOAuth: authResult.has_oauth,
        hasApiKey: authResult.has_api_key,
      });

      if (cliResult.found && (authResult.has_oauth || authResult.has_api_key)) {
        // Fully configured — mark onboarding done and skip
        await completeOnboarding();
        return;
      }

      if (cliResult.found && !authResult.has_oauth && !authResult.has_api_key) {
        // CLI found but no API info — go straight to API configuration.
        step = "api_key_setup";
        return;
      }

      // CLI not found — show install commands
      step = "cli_not_found";
      await loadInstallMethods();
    } catch (e) {
      dbgWarn("wizard", "initial check error", e);
      // If check fails, assume CLI not installed
      step = "cli_not_found";
      await loadInstallMethods();
    }
  }

  async function loadInstallMethods() {
    const [methodsResult, tagsResult] = await Promise.allSettled([
      detectInstallMethods(),
      getCliDistTags(),
    ]);
    if (methodsResult.status === "fulfilled") {
      installMethods = methodsResult.value;
      dbg("wizard", "install methods", installMethods);
    } else {
      dbgWarn("wizard", "detect methods error", methodsResult.reason);
      installMethods = [];
    }
    if (tagsResult.status === "fulfilled") {
      cliLatestVersion = tagsResult.value.latest ?? tagsResult.value.stable ?? null;
      dbg("wizard", "CLI dist tags", tagsResult.value);
    } else {
      dbgWarn("wizard", "CLI dist tags error", tagsResult.reason);
      cliLatestVersion = null;
    }
  }

  async function copyText(id: string, text: string) {
    try {
      await navigator.clipboard.writeText(text);
      copyStates = { ...copyStates, [id]: "copied" };
      setTimeout(() => {
        copyStates = { ...copyStates, [id]: "copy" };
      }, 1500);
    } catch (e) {
      dbgWarn("wizard", "copy failed", e);
    }
  }

  async function startCliInstall(auto = false) {
    if (cliInstalling || (auto && cliInstallAttempted)) return;
    cliInstallAttempted = true;
    cliInstalling = true;
    cliInstallStatus = "running";
    cliInstallError = "";
    installProgress = [];

    if (!getTransport().isDesktop()) {
      cliInstalling = false;
      cliInstallStatus = "failed";
      cliInstallError = t("setup_autoInstallDesktopOnly");
      return;
    }

    const wizTransport = getTransport();
    let unlisten = () => {};
    try {
      const unlistenAppend = await wizTransport.listen<string>("setup-progress", (payload) => {
        installProgress = [...installProgress, payload];
      });
      const unlistenReplace = await wizTransport.listen<string>(
        "setup-progress-replace",
        (payload) => {
          if (installProgress.length > 0) {
            installProgress = [...installProgress.slice(0, -1), payload];
          } else {
            installProgress = [payload];
          }
        },
      );
      unlisten = () => {
        unlistenAppend();
        unlistenReplace();
      };

      const success = await installHelioncoderCli(cliLatestVersion ?? undefined);
      if (!success) {
        cliInstallStatus = "failed";
        cliInstallError = t("setup_installFailedDesc");
        return;
      }

      installProgress = [...installProgress, t("setup_installRechecking")];
      const [cliResult, authResult] = await Promise.all([
        checkAgentCli("helioncoder"),
        checkAuthStatus(),
      ]);
      if (!cliResult.found) {
        cliInstallStatus = "failed";
        cliInstallError = t("setup_installSucceededButNotFound");
        return;
      }

      cliInstallStatus = "success";
      if (authResult.has_oauth || authResult.has_api_key) {
        await completeOnboarding();
      } else {
        step = "api_key_setup";
      }
    } catch (e) {
      dbgWarn("wizard", "CLI install error", e);
      cliInstallStatus = "failed";
      cliInstallError = String((e as Error)?.message ?? e);
    } finally {
      unlisten();
      cliInstalling = false;
    }
  }

  async function recheckCli() {
    rechecking = true;
    cliLocateError = "";
    try {
      const result = await checkAgentCli("helioncoder");
      dbg("wizard", "recheck result", result);
      if (result.found) {
        step = "api_key_setup";
      }
    } catch (e) {
      dbgWarn("wizard", "recheck error", e);
    } finally {
      rechecking = false;
    }
  }

  async function locateCliBinary() {
    locatingCli = true;
    cliLocateError = "";
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        directory: false,
        multiple: false,
        title: t("setup_selectCliBinaryTitle"),
      });
      if (!selected || typeof selected !== "string") return;
      await updateUserSettings({ helioncoder_cli_path: selected });
      await recheckCli();
    } catch (e) {
      cliLocateError = String((e as Error)?.message ?? e);
      dbgWarn("wizard", "locate CLI binary error", e);
    } finally {
      locatingCli = false;
    }
  }

  async function startOAuthLogin() {
    step = "oauth_login";
    oauthLoading = true;
    error = "";
    installProgress = [];

    const wizTransport = getTransport();
    const unlistenAppend = await wizTransport.listen<string>("setup-progress", (payload) => {
      installProgress = [...installProgress, payload];
    });
    const unlistenReplace = await wizTransport.listen<string>(
      "setup-progress-replace",
      (payload) => {
        if (installProgress.length > 0) {
          installProgress = [...installProgress.slice(0, -1), payload];
        } else {
          installProgress = [payload];
        }
      },
    );
    const unlisten = () => {
      unlistenAppend();
      unlistenReplace();
    };

    try {
      const success = await runClaudeLogin();
      unlisten();

      if (success) {
        dbg("wizard", "oauth login success");
        await completeOnboarding();
      } else {
        error = t("setup_loginFailed");
      }
    } catch (e) {
      unlisten();
      dbgWarn("wizard", "oauth login error", e);
      error = String(e);
    } finally {
      oauthLoading = false;
    }
  }

  function selectPlatform(preset: PlatformPreset) {
    selectedPlatform = preset;
    apiKey = "";
    customBaseUrl = preset.base_url;
    showKey = false;
  }

  function cleanModelOptions(models: string[]): string[] {
    const seen = new Set<string>();
    const result: string[] = [];
    for (const model of models) {
      const trimmed = model.trim();
      if (!trimmed) continue;
      const key = trimmed.toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      result.push(trimmed);
    }
    return result;
  }

  async function resolveModelOptions(effectiveBaseUrl: string): Promise<string[]> {
    if (!selectedPlatform) return [];
    const presetModels = cleanModelOptions(selectedPlatform.models ?? []);
    const shouldFetch = !!effectiveBaseUrl || selectedPlatform.id !== "anthropic";
    if (!shouldFetch) return presetModels;
    try {
      const result = await listApiModels(apiKey, effectiveBaseUrl || "");
      const models = cleanModelOptions(result.models);
      if (models.length > 0) {
        return models;
      }
      if (result.error) {
        dbgWarn("wizard", "model list returned no models", result.error);
      }
    } catch (e) {
      dbgWarn("wizard", "model list fetch failed", e);
    }
    return presetModels;
  }

  async function saveApiKey() {
    if (!selectedPlatform) return;
    saving = true;
    error = "";

    try {
      const effectiveBaseUrl =
        selectedPlatform.id === "custom" ? customBaseUrl : selectedPlatform.base_url;
      const modelOptions = await resolveModelOptions(effectiveBaseUrl || "");
      const defaultModel = modelOptions[0] ?? "";
      const smallModel = modelOptions[1] ?? modelOptions[0] ?? "";
      const platformId =
        selectedPlatform.id === "custom" ? `custom-${Date.now()}` : selectedPlatform.id;
      const platformCredential: PlatformCredential = {
        platform_id: platformId,
        name: selectedPlatform.id === "custom" ? "Custom" : selectedPlatform.name,
        api_key: apiKey || undefined,
        base_url: effectiveBaseUrl || undefined,
        auth_env_var: selectedPlatform.auth_env_var,
        models: modelOptions.length ? modelOptions : undefined,
      };

      await setCliApiConfig(apiKey, effectiveBaseUrl || "", defaultModel, smallModel, modelOptions);
      await loadCliInfo(true);

      await updateUserSettings({
        auth_mode: "cli",
        anthropic_api_key: null,
        anthropic_base_url: null,
        auth_env_var: null,
        active_platform_id: platformId,
        platform_credentials: [platformCredential],
        onboarding_completed: true,
      } as Partial<import("$lib/types").UserSettings> & {
        anthropic_api_key: null;
        anthropic_base_url: null;
        auth_env_var: null;
      });

      dbg("wizard", "api key saved", {
        platform: selectedPlatform.id,
        hasKey: !!apiKey,
        hasBaseUrl: !!effectiveBaseUrl,
        modelCount: modelOptions.length,
      });
      window.dispatchEvent(
        new CustomEvent("helion:models-updated", {
          detail: { platformId, models: modelOptions },
        }),
      );

      await completeOnboarding();
    } catch (e) {
      dbgWarn("wizard", "save api key error", e);
      error = String(e);
    } finally {
      saving = false;
    }
  }

  async function completeOnboarding() {
    try {
      await updateUserSettings({ onboarding_completed: true });
    } catch {
      // Non-critical — continue anyway
    }
    step = "done";
    doneTimer = setTimeout(() => {
      onComplete();
    }, 2000);
  }

  function finishNow() {
    if (doneTimer) clearTimeout(doneTimer);
    onComplete();
  }

  const UNIX_INSTALL_LATEST =
    "curl -fsSL https://raw.githubusercontent.com/gabrielpondc/HelionCoder/main/scripts/install.sh | sh";
  const UNIX_UNINSTALL =
    "curl -fsSL https://raw.githubusercontent.com/gabrielpondc/HelionCoder/main/scripts/uninstall.sh | sh";
  const WINDOWS_INSTALL_LATEST =
    "iwr https://raw.githubusercontent.com/gabrielpondc/HelionCoder/main/scripts/install.ps1 -UseB | iex";
  const WINDOWS_UNINSTALL =
    "iwr https://raw.githubusercontent.com/gabrielpondc/HelionCoder/main/scripts/uninstall.ps1 -UseB | iex";

  let availableMethods = $derived(installMethods.filter((m) => m.available));
  let unavailableMethods = $derived(installMethods.filter((m) => !m.available));
  let fallbackInstallCommand = $derived(
    availableMethods[0]?.command ??
      installMethods[0]?.command ??
      (IS_WINDOWS ? WINDOWS_INSTALL_LATEST : UNIX_INSTALL_LATEST),
  );
  let cliLatestLabel = $derived(
    cliLatestVersion ? `v${cliLatestVersion}` : t("setup_latestCliVersion"),
  );
  let detectedVersionInstallCommand = $derived(
    cliLatestVersion
      ? IS_WINDOWS
        ? `$env:HELION_VERSION="${cliLatestVersion}"; iwr https://raw.githubusercontent.com/gabrielpondc/HelionCoder/main/scripts/install.ps1 -UseB | iex`
        : `curl -fsSL https://raw.githubusercontent.com/gabrielpondc/HelionCoder/main/scripts/install.sh | sh -s -- ${cliLatestVersion}`
      : fallbackInstallCommand,
  );
  let manualCommands = $derived([
    {
      id: "install-latest",
      label: t("setup_installLatest"),
      command: fallbackInstallCommand,
    },
    ...(cliLatestVersion
      ? [
          {
            id: "install-version",
            label: t("setup_installVersion", { version: cliLatestLabel }),
            command: detectedVersionInstallCommand,
          },
        ]
      : []),
    {
      id: "verify-install",
      label: t("setup_verifyInstall"),
      command: "helion-coder --version",
    },
    {
      id: "uninstall",
      label: t("setup_uninstallCli"),
      command: IS_WINDOWS ? WINDOWS_UNINSTALL : UNIX_UNINSTALL,
    },
  ]);
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center bg-background">
  <div class="w-full mx-auto px-6 {step === 'cli_not_found' ? 'max-w-2xl' : 'max-w-xl'}">
    {#if step === "checking"}
      <!-- Checking step -->
      <div class="flex flex-col items-center gap-4 py-16">
        <div
          class="h-8 w-8 border-2 border-primary/30 border-t-primary rounded-full animate-spin"
        ></div>
        <p class="text-sm text-muted-foreground">{t("setup_checking")}</p>
      </div>
    {:else if step === "cli_not_found"}
      <!-- CLI not found — offer locating an existing binary or running the official installer. -->
      <div class="flex flex-col gap-5">
        <div class="text-center">
          <h2 class="text-xl font-semibold">{t("setup_cliNotFound")}</h2>
          <p class="text-sm text-muted-foreground mt-2">{t("setup_cliNotFoundDesc")}</p>
        </div>

        <div class="rounded-lg border border-border/80 bg-muted/25 p-4">
          <div class="flex items-start gap-3">
            <div
              class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md border {cliInstallStatus ===
              'failed'
                ? 'border-red-500/30 bg-red-500/10 text-red-500'
                : cliInstallStatus === 'success'
                  ? 'border-green-500/30 bg-green-500/10 text-green-600'
                  : 'border-primary/25 bg-primary/10 text-primary'}"
            >
              {#if cliInstallStatus === "failed"}
                <svg
                  class="h-4 w-4"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  aria-hidden="true"><path d="M18 6 6 18" /><path d="m6 6 12 12" /></svg
                >
              {:else if cliInstallStatus === "success"}
                <svg
                  class="h-4 w-4"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  aria-hidden="true"><path d="M20 6 9 17l-5-5" /></svg
                >
              {:else}
                <span
                  class="h-4 w-4 rounded-full border-2 border-current/25 border-t-current {cliInstalling
                    ? 'animate-spin'
                    : ''}"
                ></span>
              {/if}
            </div>

            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-2">
                <p class="text-sm font-medium">
                  {#if cliInstallStatus === "failed"}
                    {t("setup_autoInstallFailed")}
                  {:else if cliInstallStatus === "success"}
                    {t("setup_autoInstallDone")}
                  {:else}
                    {t("setup_autoInstallTitle")}
                  {/if}
                </p>
                {#if cliInstalling}
                  <span
                    class="rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary"
                    >{t("setup_installing")}</span
                  >
                {/if}
              </div>
              <p class="mt-1 text-xs text-muted-foreground">
                {t("setup_autoInstallDesc", { version: cliLatestLabel })}
              </p>

              {#if installProgress.length > 0}
                <div
                  class="mt-3 max-h-40 overflow-auto rounded-md bg-zinc-950 px-3 py-2 text-left font-mono text-[11px] leading-5 text-zinc-100 shadow-inner"
                >
                  {#each installProgress.slice(-12) as line}
                    <div class="whitespace-pre-wrap break-words">{line}</div>
                  {/each}
                </div>
              {/if}

              {#if cliInstallError}
                <p class="mt-2 text-xs text-red-500">{cliInstallError}</p>
              {/if}
            </div>
          </div>
        </div>

        <div class="space-y-2">
          <div class="flex items-center justify-between gap-3">
            <p class="text-xs font-medium text-muted-foreground">{t("setup_manualFallback")}</p>
            {#if unavailableMethods.length > 0}
              <p class="text-[11px] text-muted-foreground/70">
                {unavailableMethods[0].name}: {unavailableMethods[0].unavailable_reason}
              </p>
            {/if}
          </div>
          <div class="grid gap-2">
            {#each manualCommands as item}
              <div
                class="flex items-center gap-2 rounded-md border border-border/70 bg-background/80 p-2"
              >
                <div class="min-w-0 flex-1">
                  <p class="mb-1 text-[11px] font-medium text-muted-foreground">{item.label}</p>
                  <code class="block select-all truncate font-mono text-xs text-foreground/90"
                    >{item.command}</code
                  >
                </div>
                <button
                  class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border text-muted-foreground transition-colors hover:bg-accent hover:text-foreground {copyStates[
                    item.id
                  ] === 'copied'
                    ? 'border-green-500/30 text-green-600'
                    : ''}"
                  onclick={() => copyText(item.id, item.command)}
                  title={copyStates[item.id] === "copied"
                    ? t("setup_copied")
                    : t("setup_copyCommand")}
                  aria-label={copyStates[item.id] === "copied"
                    ? t("setup_copied")
                    : t("setup_copyCommand")}
                >
                  {#if copyStates[item.id] === "copied"}
                    <svg
                      class="h-3.5 w-3.5"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      aria-hidden="true"><path d="M20 6 9 17l-5-5" /></svg
                    >
                  {:else}
                    <svg
                      class="h-3.5 w-3.5"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.8"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      aria-hidden="true"
                      ><rect x="9" y="9" width="13" height="13" rx="2" /><rect
                        x="2"
                        y="2"
                        width="13"
                        height="13"
                        rx="2"
                      /></svg
                    >
                  {/if}
                </button>
              </div>
            {/each}
          </div>
        </div>

        <div class="flex flex-wrap items-center justify-center gap-3">
          <button
            class="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50"
            disabled={cliInstalling}
            onclick={() => startCliInstall(false)}
          >
            {#if cliInstalling}
              <span class="flex items-center gap-2">
                <span
                  class="h-3 w-3 rounded-full border border-primary-foreground/30 border-t-primary-foreground animate-spin"
                ></span>
                {t("setup_installing")}
              </span>
            {:else}
              {cliInstallStatus === "failed" ? t("setup_retryInstall") : t("setup_autoInstall")}
            {/if}
          </button>
          <button
            class="rounded-md border border-border px-4 py-2 text-sm transition-colors hover:bg-accent disabled:opacity-50"
            disabled={rechecking || cliInstalling}
            onclick={recheckCli}
          >
            {#if rechecking}
              <span class="flex items-center gap-2">
                <span
                  class="h-3 w-3 border border-foreground/30 border-t-foreground rounded-full animate-spin"
                ></span>
                {t("setup_recheck")}
              </span>
            {:else}
              {t("setup_recheck")}
            {/if}
          </button>
          <button
            class="rounded-md border border-border px-4 py-2 text-sm transition-colors hover:bg-accent disabled:opacity-50"
            disabled={locatingCli || cliInstalling || !getTransport().isDesktop()}
            onclick={locateCliBinary}
          >
            {#if locatingCli}
              <span class="flex items-center gap-2">
                <span
                  class="h-3 w-3 border border-foreground/30 border-t-foreground rounded-full animate-spin"
                ></span>
                {t("setup_locateCli")}
              </span>
            {:else}
              {t("setup_locateCli")}
            {/if}
          </button>
        </div>

        <p class="text-xs text-muted-foreground text-center">
          {IS_WINDOWS ? t("setup_winRecheckHint") : t("setup_setupHint")}
        </p>
        {#if cliLocateError}
          <p class="text-xs text-red-500 text-center">{cliLocateError}</p>
        {/if}
      </div>
    {:else if step === "auth_choice"}
      <!-- Auth method choice -->
      <div class="flex flex-col gap-6">
        <div class="text-center">
          <h2 class="text-xl font-semibold">{t("setup_authTitle")}</h2>
          <p class="text-sm text-muted-foreground mt-2">{t("setup_authDesc")}</p>
        </div>

        <div class="grid grid-cols-2 gap-4">
          <!-- OAuth -->
          <button
            class="flex flex-col items-center gap-3 rounded-lg border border-border p-6 text-center transition-colors hover:border-primary/50 hover:bg-accent/50"
            onclick={() => {
              step = "api_key_setup";
            }}
          >
            <svg
              class="h-8 w-8 text-primary"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="1.5"
              stroke-linecap="round"
              stroke-linejoin="round"
              ><path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4" /><polyline
                points="10 17 15 12 10 7"
              /><line x1="15" x2="3" y1="12" y2="12" /></svg
            >
            <div>
              <p class="font-medium text-sm">{t("setup_oauthTitle")}</p>
              <p class="text-xs text-muted-foreground mt-1">{t("setup_oauthDesc")}</p>
            </div>
            <span
              class="rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary"
              >{t("setup_recommended")}</span
            >
          </button>

          <!-- API Key -->
          <button
            class="flex flex-col items-center gap-3 rounded-lg border border-border p-6 text-center transition-colors hover:border-primary/50 hover:bg-accent/50"
            onclick={() => {
              step = "api_key_setup";
            }}
          >
            <svg
              class="h-8 w-8 text-muted-foreground"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="1.5"
              stroke-linecap="round"
              stroke-linejoin="round"
              ><path
                d="m21 2-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0 3 3L22 7l-3-3m-3.5 3.5L19 4"
              /></svg
            >
            <div>
              <p class="font-medium text-sm">{t("setup_apiKeyTitle")}</p>
              <p class="text-xs text-muted-foreground mt-1">{t("setup_apiKeyDesc")}</p>
            </div>
          </button>
        </div>
      </div>
    {:else if step === "oauth_login"}
      <!-- OAuth login in progress -->
      <div class="flex flex-col items-center gap-4 py-8">
        {#if oauthLoading}
          <div
            class="h-8 w-8 border-2 border-primary/30 border-t-primary rounded-full animate-spin"
          ></div>
          <p class="text-sm font-medium">{t("setup_openingBrowser")}</p>
          <p class="text-xs text-muted-foreground text-center">{t("setup_completeBrowser")}</p>
        {/if}

        {#if error}
          <div class="rounded-lg border border-red-500/30 bg-red-500/5 p-3 w-full max-w-sm">
            <p class="text-sm text-red-500">{error}</p>
          </div>
        {/if}

        <button
          class="rounded-md border border-border px-4 py-2 text-xs hover:bg-accent transition-colors mt-4"
          onclick={() => {
            step = "auth_choice";
            error = "";
          }}>{t("setup_back")}</button
        >
      </div>
    {:else if step === "api_key_setup"}
      <!-- API key setup with platform selection -->
      <div class="flex flex-col gap-5">
        <div class="flex items-center gap-2">
          <button
            class="rounded-md p-1 hover:bg-accent transition-colors"
            onclick={() => {
              step = "auth_choice";
              selectedPlatform = null;
              error = "";
            }}
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
          <h2 class="text-lg font-semibold">{t("setup_selectPlatform")}</h2>
        </div>

        {#if !selectedPlatform}
          <!-- Platform grid -->
          <div class="flex flex-col gap-4 max-h-[60vh] overflow-y-auto pr-1">
            {#each PRESET_CATEGORIES as category}
              {@const presets = PLATFORM_PRESETS.filter((p) => p.category === category.id)}
              {#if presets.length > 0}
                <div>
                  <p
                    class="text-[11px] font-medium text-muted-foreground uppercase tracking-wider mb-2"
                  >
                    {category.label}
                  </p>
                  <div class="grid grid-cols-3 gap-2">
                    {#each presets as preset}
                      <button
                        class="flex flex-col gap-0.5 rounded-lg border border-border p-3 text-left transition-colors hover:border-primary/50 hover:bg-accent/50"
                        onclick={() => selectPlatform(preset)}
                      >
                        <span class="text-sm font-medium truncate">{preset.name}</span>
                        <span class="text-[10px] text-muted-foreground truncate"
                          >{preset.description}</span
                        >
                      </button>
                    {/each}
                  </div>
                </div>
              {/if}
            {/each}
          </div>
        {:else}
          <!-- Platform config form -->
          <div class="flex flex-col gap-4">
            <div
              class="flex items-center gap-2 rounded-lg border border-primary/30 bg-primary/5 p-3"
            >
              <span class="font-medium text-sm">{selectedPlatform.name}</span>
              <span class="text-xs text-muted-foreground">{selectedPlatform.description}</span>
              <button
                class="ml-auto text-xs text-muted-foreground hover:text-foreground transition-colors"
                onclick={() => {
                  selectedPlatform = null;
                }}>{t("setup_change")}</button
              >
            </div>

            <!-- Custom: extra Base URL input -->
            {#if selectedPlatform.id === "custom"}
              <div class="flex flex-col gap-1.5">
                <label class="text-xs font-medium text-muted-foreground">{t("setup_baseUrl")}</label
                >
                <input
                  type="text"
                  bind:value={customBaseUrl}
                  placeholder="https://api.example.com"
                  class="w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:border-ring"
                />
              </div>
            {/if}

            <!-- API Key input -->
            <div class="flex flex-col gap-1.5">
              <label class="text-xs font-medium text-muted-foreground"
                >{t("setup_apiKeyLabel")}</label
              >
              <div class="relative">
                <input
                  type={showKey ? "text" : "password"}
                  bind:value={apiKey}
                  placeholder={selectedPlatform.key_placeholder}
                  class="w-full rounded-md border border-border bg-background px-3 py-2 pr-16 text-sm font-mono focus:outline-none focus:border-ring"
                />
                <button
                  class="absolute right-2 top-1/2 -translate-y-1/2 rounded px-2 py-0.5 text-xs text-muted-foreground hover:text-foreground transition-colors"
                  onclick={() => {
                    showKey = !showKey;
                  }}>{showKey ? t("setup_hide") : t("setup_show")}</button
                >
              </div>
              {#if selectedPlatform.id === "ollama"}
                <p class="text-xs text-muted-foreground">{t("setup_noKeyNeeded")}</p>
              {/if}
            </div>

            <!-- Auth type info -->
            <p class="text-xs text-muted-foreground">
              {selectedPlatform.auth_env_var === "ANTHROPIC_API_KEY"
                ? t("setup_authTypeApiKey")
                : t("setup_authTypeBearer")}
            </p>

            {#if error}
              <div class="rounded-lg border border-red-500/30 bg-red-500/5 p-2">
                <p class="text-xs text-red-500">{error}</p>
              </div>
            {/if}

            <button
              class="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50"
              disabled={saving || (selectedPlatform.category !== "local" && !apiKey)}
              onclick={saveApiKey}
            >
              {#if saving}
                <span class="flex items-center gap-2 justify-center">
                  <span
                    class="h-3 w-3 border border-primary-foreground/30 border-t-primary-foreground rounded-full animate-spin"
                  ></span>
                  {t("setup_saving")}
                </span>
              {:else}
                {t("setup_saveAndContinue")}
              {/if}
            </button>
          </div>
        {/if}
      </div>
    {:else if step === "done"}
      <!-- Done! -->
      <div class="flex flex-col items-center gap-4 py-16">
        <div class="flex h-16 w-16 items-center justify-center rounded-full bg-green-500/10">
          <svg
            class="h-8 w-8 text-green-500"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"><path d="M20 6 9 17l-5-5" /></svg
          >
        </div>
        <h2 class="text-xl font-semibold">{t("setup_allSet")}</h2>
        <p class="text-sm text-muted-foreground">{t("setup_allSetDesc")}</p>
        <button
          class="rounded-md bg-primary px-6 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 transition-colors mt-2"
          onclick={finishNow}>{t("setup_start")}</button
        >
      </div>
    {/if}
  </div>
</div>
