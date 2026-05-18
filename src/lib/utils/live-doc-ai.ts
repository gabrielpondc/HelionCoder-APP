import * as api from "$lib/api";
import type {
  AiClient,
  AiGlobalConfig,
  AiMessageListener,
  AiModelFactory,
} from "$lib/vendor/aieditor";
import type { BusEvent, PlatformCredential, UserSettings } from "$lib/types";
import { PLATFORM_PRESETS } from "$lib/utils/platform-presets";
import { getTransport } from "$lib/transport";
import { getCliCurrentModel, loadCliInfo } from "$lib/stores/cli-info.svelte";
import { dbg, dbgWarn } from "$lib/utils/debug";

const DOC_AI_MODEL = "helion-docs";
const DOC_AI_TIMEOUT_MS = 180_000;

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object";
}

function modelStorageKey(platformId?: string | null): string {
  return `ocv:selected-model:${platformId || "anthropic"}`;
}

function storedModelForPlatform(platformId: string | null | undefined): string | undefined {
  if (typeof localStorage === "undefined") return undefined;
  return localStorage.getItem(modelStorageKey(platformId))?.trim() || undefined;
}

function modelsForPlatform(
  settings: UserSettings | null,
  platformId: string,
): readonly string[] | undefined {
  const credentials = settings?.platform_credentials ?? [];
  const credential = credentials.find(
    (item: PlatformCredential) => item.platform_id === platformId,
  );
  if (credential?.models?.length) return credential.models;
  return PLATFORM_PRESETS.find((preset) => preset.id === platformId)?.models;
}

async function resolveDocAiRuntime() {
  const [settings] = await Promise.all([
    api.getUserSettings().catch((e) => {
      dbgWarn("live-doc-ai", "settings unavailable", e);
      return null;
    }),
    loadCliInfo().catch(() => null),
  ]);

  const platformId =
    settings?.auth_mode === "api" ? (settings.active_platform_id ?? "anthropic") : "anthropic";
  const platformModels = modelsForPlatform(settings, platformId);
  const stored = storedModelForPlatform(platformId);
  const model =
    (stored && (!platformModels?.length || platformModels.includes(stored)) ? stored : undefined) ??
    (platformId === "anthropic" ? getCliCurrentModel() || settings?.default_model : undefined) ??
    platformModels?.[0] ??
    undefined;
  const cwd =
    (typeof localStorage !== "undefined" && localStorage.getItem("ocv:project-cwd")) ||
    settings?.working_directory ||
    "/";

  return { cwd, model, platformId };
}

function composeDocumentPrompt(selectedText: string, prompt: string): string {
  const operation = prompt.includes("{content}")
    ? prompt.split("{content}").join(selectedText)
    : `${selectedText ? `<content>\n${selectedText}\n</content>\n\n` : ""}${prompt}`;

  return `You are Helion's inline document editor AI.
Return only the final text that should be inserted into the document.
Do not mention that you are an AI. Do not explain your reasoning.
Do not include hidden thinking, analysis, tool logs, or surrounding commentary.
Do not use tools or inspect files. Work only from the provided document text and instruction.

${operation}`;
}

function eventRunId(event: BusEvent): string | undefined {
  return isRecord(event) && typeof event.run_id === "string" ? event.run_id : undefined;
}

class HelionDocAiClient implements AiClient {
  private stopped = false;
  private runId = "";
  private unlisten: (() => void) | null = null;
  private timeout: ReturnType<typeof setTimeout> | null = null;
  private streamedText = "";
  private stoppedNotified = false;

  constructor(
    private readonly listener: AiMessageListener,
    private readonly isZh: boolean,
  ) {}

  start(payload: string) {
    this.listener.onStart(this);
    void this.run(payload);
  }

  stop() {
    this.stopped = true;
    this.cleanup();
    if (this.runId) {
      api.stopSession(this.runId).catch((e) => dbgWarn("live-doc-ai", "stop failed", e));
    }
    this.notifyStop();
  }

  private async run(prompt: string) {
    try {
      const runtime = await resolveDocAiRuntime();
      if (this.stopped) return;

      const transport = getTransport();
      this.unlisten = await transport.listen<BusEvent>("bus-event", (event) => {
        this.handleBusEvent(event);
      });

      const run = await api.startRun(
        prompt,
        runtime.cwd,
        "helioncoder",
        runtime.model,
        undefined,
        runtime.platformId,
        "session_actor",
        true,
      );
      this.runId = run.id;
      transport.subscribeRun(run.id, 0);
      this.timeout = setTimeout(() => {
        this.fail(this.isZh ? "文档 AI 响应超时" : "Document AI timed out");
      }, DOC_AI_TIMEOUT_MS);

      await api.startSession(
        run.id,
        undefined,
        undefined,
        undefined,
        undefined,
        runtime.platformId,
        "default",
      );
    } catch (e) {
      dbgWarn("live-doc-ai", "run failed", e);
      this.fail(
        this.isZh
          ? `文档 AI 暂时不可用：${e instanceof Error ? e.message : String(e)}`
          : `Document AI is unavailable: ${e instanceof Error ? e.message : String(e)}`,
      );
    }
  }

  private handleBusEvent(event: BusEvent) {
    if (this.stopped || eventRunId(event) !== this.runId) return;

    if (event.type === "message_delta" && !event.parent_tool_use_id) {
      this.streamedText += event.text;
      this.listener.onMessage({ role: "assistant", content: event.text, status: 1 });
      return;
    }

    if (event.type === "thinking_delta" && !event.parent_tool_use_id) {
      this.listener.onMessage({
        role: "assistant",
        content: "",
        thinking: event.text,
        type: "thinking",
        status: 1,
      });
      return;
    }

    if (event.type === "message_complete" && !event.parent_tool_use_id) {
      const finalText = event.text || "";
      if (!this.streamedText && finalText) {
        this.listener.onMessage({ role: "assistant", content: finalText, status: 2 });
      } else {
        this.listener.onMessage({ role: "assistant", content: "", status: 2 });
      }
      this.cleanup();
      api.stopSession(this.runId).catch(() => {});
      this.notifyStop();
      return;
    }

    if (event.type === "permission_prompt" || event.type === "elicitation_prompt") {
      this.fail(this.isZh ? "文档 AI 需要人工确认，已停止" : "Document AI asked for confirmation");
      return;
    }

    if (event.type === "run_state" && (event.state === "failed" || event.state === "stopped")) {
      this.fail(event.error || (this.isZh ? "文档 AI 已停止" : "Document AI stopped"));
    }
  }

  private fail(message: string) {
    if (this.stopped) return;
    this.listener.onMessage({ role: "assistant", content: message, status: 2 });
    this.stop();
  }

  private cleanup() {
    if (this.timeout) {
      clearTimeout(this.timeout);
      this.timeout = null;
    }
    this.unlisten?.();
    this.unlisten = null;
    if (this.runId) getTransport().unsubscribeRun(this.runId);
  }

  private notifyStop() {
    if (this.stoppedNotified) return;
    this.stoppedNotified = true;
    this.listener.onStop();
  }
}

export function createLiveDocAiConfig(isZh: boolean): AiGlobalConfig {
  const modelFactory: AiModelFactory = {
    create(name: string) {
      if (name !== DOC_AI_MODEL) return undefined as never;
      return {
        chat(selectedText: string, prompt: string, listener: AiMessageListener) {
          const client = new HelionDocAiClient(listener, isZh);
          client.start(composeDocumentPrompt(selectedText, prompt));
        },
        chatWithPayload(payload: unknown, listener: AiMessageListener) {
          const client = new HelionDocAiClient(listener, isZh);
          client.start(typeof payload === "string" ? payload : JSON.stringify(payload));
        },
        createAiClientUrl() {
          return "";
        },
        createAiClient() {
          return new HelionDocAiClient(
            {
              onStart() {},
              onStop() {},
              onMessage() {},
            },
            isZh,
          );
        },
        wrapPayload(prompt: string) {
          return prompt;
        },
      } as never;
    },
  };

  return {
    models: { [DOC_AI_MODEL]: {} } as unknown as NonNullable<AiGlobalConfig["models"]>,
    modelFactory,
    bubblePanelEnable: true,
    bubblePanelModel: DOC_AI_MODEL,
    commandsEnable: true,
    codeBlock: {
      codeExplain: {
        model: DOC_AI_MODEL,
        prompt: isZh
          ? "解释这段代码的用途和关键逻辑，只返回适合插入文档的说明。"
          : "Explain what this code does and its key logic. Return only document-ready prose.",
      },
      codeComments: {
        model: DOC_AI_MODEL,
        prompt: isZh
          ? "为这段代码补充必要注释，只返回带注释的代码。"
          : "Add helpful comments to this code. Return only the commented code.",
      },
    },
  };
}
