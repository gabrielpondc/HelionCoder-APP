/**
 * Auto-name gating logic for chat sessions.
 *
 * Extracted as pure functions so the latch/guard semantics can be
 * unit-tested without DOM or Svelte runtime.
 */

/** Derive an auto-name from the user's first submitted instruction. */
export function deriveAutoName(prompt: string): string {
  return prompt.trim();
}

export interface AutoNameState {
  phase: string;
  runId: string | undefined;
  runName: string | undefined;
  prompt: string | undefined;
  autoNameDone: boolean;
}

/**
 * Determine whether auto-name should fire for the current state.
 *
 * Returns `{ fire: true, autoName }` when all conditions are met:
 * - phase is "idle"
 * - run exists with a prompt but no name yet
 * - auto-name has not already been attempted for this run (`autoNameDone` is false)
 */
export function shouldAutoName(state: AutoNameState): { fire: boolean; autoName?: string } {
  if (state.phase !== "idle") return { fire: false };
  if (!state.runId) return { fire: false };
  if (state.runName) return { fire: false };
  if (state.autoNameDone) return { fire: false };
  if (!state.prompt) return { fire: false };

  const autoName = deriveAutoName(state.prompt);
  if (!autoName) return { fire: false };

  return { fire: true, autoName };
}
