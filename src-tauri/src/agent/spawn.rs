use crate::agent::adapter::{self, AdapterSettings};
use crate::agent::claude_stream::{HELION_AGENT_ID, HELION_CLI_NAME};

/// Build the command + args for a given agent (pipe-exec mode, not stream session)
pub fn build_agent_command(
    agent: &str,
    prompt: &str,
    settings: &AdapterSettings,
    print: bool,
) -> Result<(String, Vec<String>), String> {
    log::debug!(
        "[spawn] build_agent_command: agent={}, print={}, model={:?}, perm={:?}, allowed={}, disallowed={}",
        agent, print, settings.model, settings.permission_mode, settings.allowed_tools.len(), settings.disallowed_tools.len()
    );
    match agent {
        HELION_AGENT_ID | "claude" => {
            let mut args: Vec<String> = vec![];
            if print {
                args.push("--print".to_string());
            }

            // Use shared helper for all settings flags
            args.extend(adapter::build_settings_args(settings, print));

            if !prompt.is_empty() {
                args.push(prompt.to_string());
            }
            log::debug!(
                "[spawn] HelionCoder command: {} {}",
                HELION_CLI_NAME,
                args.join(" ")
            );
            Ok((crate::agent::claude_stream::resolve_claude_path(), args))
        }
        _ => Err(format!(
            "Unsupported agent: {}. Supported: {}",
            agent, HELION_AGENT_ID
        )),
    }
}
