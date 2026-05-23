use crate::agent::claude_stream::{augmented_path, try_resolve_claude_path};
use crate::models::{
    now_iso, CliAccount, CliCommand, CliInfo, CliInfoError, CliModelInfo, RemoteHost,
};
use crate::process_ext::HideConsole;
use serde_json::Value;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

/// Cached CLI info with TTL
#[derive(Clone)]
pub struct CliInfoCache {
    inner: Arc<RwLock<Option<(CliInfo, std::time::Instant)>>>,
}

impl Default for CliInfoCache {
    fn default() -> Self {
        Self::new()
    }
}

impl CliInfoCache {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(None)),
        }
    }
}

const CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes
const PROCESS_TIMEOUT: Duration = Duration::from_secs(10);

/// Get CLI info, using cache if available and not expired.
pub async fn get_cli_info(cache: &CliInfoCache, force: bool) -> Result<CliInfo, CliInfoError> {
    // Check cache
    if !force {
        let guard = cache.inner.read().await;
        if let Some((ref info, ref instant)) = *guard {
            if instant.elapsed() < CACHE_TTL {
                log::debug!(
                    "[control] returning cached CLI info ({} models)",
                    info.models.len()
                );
                return Ok(info.clone());
            }
        }
    }

    // Resolve binary
    let claude_bin = try_resolve_claude_path().ok_or_else(|| CliInfoError {
        code: "cli_not_found".to_string(),
        message: "HelionCoder CLI binary not found".to_string(),
    })?;
    log::debug!("[control] resolved HelionCoder binary: {}", claude_bin);

    // Spawn process
    let path_env = augmented_path();
    let mut cmd = tokio::process::Command::new(&claude_bin);
    cmd.arg("-p")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--input-format")
        .arg("stream-json")
        .arg("--verbose")
        .env("PATH", &path_env)
        .env_remove("CLAUDECODE") // Allow running inside a CLI session
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .hide_console()
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| {
        log::error!("[control] failed to spawn HelionCoder: {}", e);
        CliInfoError {
            code: "cli_not_found".to_string(),
            message: format!("Failed to spawn HelionCoder: {}", e),
        }
    })?;

    log::debug!("[control] spawned HelionCoder process pid={:?}", child.id());

    // Send initialize request
    let init_request = serde_json::json!({
        "type": "control_request",
        "request_id": "ocv_init_1",
        "request": { "subtype": "initialize" }
    });

    let mut stdin = child.stdin.take().ok_or_else(|| CliInfoError {
        code: "protocol_error".to_string(),
        message: "Failed to capture stdin".to_string(),
    })?;

    let mut line = serde_json::to_string(&init_request).map_err(|e| CliInfoError {
        code: "protocol_error".to_string(),
        message: format!("Failed to serialize request: {}", e),
    })?;
    line.push('\n');

    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| CliInfoError {
            code: "protocol_error".to_string(),
            message: format!("Failed to write to stdin: {}", e),
        })?;
    if let Err(e) = stdin.flush().await {
        log::warn!("[control] stdin flush failed: {}", e);
    }
    drop(stdin); // Close stdin to signal we're done

    log::debug!("[control] sent initialize request, reading stdout...");

    // Read stdout with timeout
    let stdout = child.stdout.take().ok_or_else(|| CliInfoError {
        code: "protocol_error".to_string(),
        message: "Failed to capture stdout".to_string(),
    })?;

    let result = timeout(PROCESS_TIMEOUT, read_control_response(stdout)).await;

    // Kill process regardless
    let _ = child.kill().await;
    let _ = child.wait().await;

    let cli_info = match result {
        Ok(Ok(info)) => info,
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            return Err(CliInfoError {
                code: "timeout".to_string(),
                message: format!(
                    "Timed out after {}s waiting for CLI response",
                    PROCESS_TIMEOUT.as_secs()
                ),
            });
        }
    };

    // Read current model from HelionCoder config files.
    let current_model = read_helioncoder_configured_model();
    let cli_info = CliInfo {
        current_model,
        ..cli_info
    };

    log::debug!(
        "[control] got {} models, {} commands, current_model={:?}",
        cli_info.models.len(),
        cli_info.commands.len(),
        &cli_info.current_model
    );

    // Update cache
    let mut guard = cache.inner.write().await;
    *guard = Some((cli_info.clone(), std::time::Instant::now()));

    Ok(cli_info)
}

/// Get CLI info by launching HelionCoder on a configured remote host over SSH.
///
/// Remote info is intentionally not stored in the local CLI cache; different
/// hosts can expose different model lists and active settings.
pub async fn get_remote_cli_info(remote: &RemoteHost) -> Result<CliInfo, CliInfoError> {
    let init_request = serde_json::json!({
        "type": "control_request",
        "request_id": "ocv_remote_init_1",
        "request": { "subtype": "initialize" }
    });
    let init_line = serde_json::to_string(&init_request).map_err(|e| CliInfoError {
        code: "protocol_error".to_string(),
        message: format!("Failed to serialize request: {}", e),
    })?;

    let cli_expr = remote
        .remote_claude_path
        .as_deref()
        .map(crate::agent::ssh::shell_escape_path)
        .unwrap_or_else(|| "\"$(command -v helion-coder || command -v helioncoder)\"".to_string());
    let args = [
        "-p",
        "--output-format",
        "stream-json",
        "--input-format",
        "stream-json",
        "--verbose",
    ]
    .iter()
    .map(|arg| crate::agent::ssh::shell_escape(arg))
    .collect::<Vec<_>>()
    .join(" ");
    let remote_cmd = format!(
        "cd ~ && printf '%s\\n' {} | {} {}",
        crate::agent::ssh::shell_escape(&init_line),
        cli_expr,
        args
    );

    let mut cmd = crate::agent::ssh::build_ssh_command(remote, &remote_cmd);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .hide_console()
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| CliInfoError {
        code: "ssh_spawn_failed".to_string(),
        message: format!("Failed to spawn ssh: {}", e),
    })?;
    let stdout = child.stdout.take().ok_or_else(|| CliInfoError {
        code: "protocol_error".to_string(),
        message: "Failed to capture remote stdout".to_string(),
    })?;

    let result = timeout(PROCESS_TIMEOUT, read_control_response(stdout)).await;
    let _ = child.kill().await;
    let _ = child.wait().await;

    let mut cli_info = match result {
        Ok(Ok(info)) => info,
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            return Err(CliInfoError {
                code: "timeout".to_string(),
                message: format!(
                    "Timed out after {}s waiting for remote CLI response",
                    PROCESS_TIMEOUT.as_secs()
                ),
            });
        }
    };

    cli_info.current_model = read_remote_helioncoder_configured_model(remote).await;
    log::debug!(
        "[control] remote {} got {} models, current_model={:?}",
        remote.name,
        cli_info.models.len(),
        cli_info.current_model
    );

    Ok(cli_info)
}

/// Read stdout lines looking for a control_response event.
async fn read_control_response(
    stdout: tokio::process::ChildStdout,
) -> Result<CliInfo, CliInfoError> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut reader = BufReader::new(stdout).lines();
    let mut line_count = 0u32;

    while let Ok(Some(text)) = reader.next_line().await {
        line_count += 1;
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        log::trace!(
            "[control] stdout line #{}: {}",
            line_count,
            &text[..text.len().min(200)]
        );

        let parsed: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let event_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");

        if event_type == "control_response" {
            // Extract the response body
            let response = parsed.get("response").ok_or_else(|| CliInfoError {
                code: "protocol_error".to_string(),
                message: "control_response missing 'response' field".to_string(),
            })?;

            // Check for auth errors
            if let Some(error) = response.get("error").and_then(|v| v.as_str()) {
                if error.contains("auth") || error.contains("token") || error.contains("login") {
                    return Err(CliInfoError {
                        code: "not_authenticated".to_string(),
                        message: error.to_string(),
                    });
                }
                return Err(CliInfoError {
                    code: "protocol_error".to_string(),
                    message: format!("Control response error: {}", error),
                });
            }

            // The response may be nested: response.response contains the actual data
            // (CLI returns { subtype, request_id, response: { models, commands, ... } })
            let data = response.get("response").unwrap_or(response);

            // Parse models
            let models: Vec<CliModelInfo> = data
                .get("models")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            let commands: Vec<CliCommand> = data
                .get("commands")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            let available_output_styles: Vec<String> = data
                .get("available_output_styles")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            let account: Option<CliAccount> = data
                .get("account")
                .and_then(|v| serde_json::from_value(v.clone()).ok());

            return Ok(CliInfo {
                models,
                commands,
                available_output_styles,
                account,
                current_model: None, // populated by caller from ~/.claude/settings.json
                fetched_at: now_iso(),
            });
        }

        // Safety: don't read forever
        if line_count > 50 {
            return Err(CliInfoError {
                code: "protocol_error".to_string(),
                message: "No control_response found in first 50 lines".to_string(),
            });
        }
    }

    Err(CliInfoError {
        code: "protocol_error".to_string(),
        message: format!("EOF after {} lines without control_response", line_count),
    })
}

fn read_json_string_field(path: &std::path::Path, key: &str) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&contents).ok()?;
    let value = parsed.get(key)?.as_str()?.trim();
    if value.is_empty() {
        None
    } else {
        log::debug!("[control] read {} from {:?}: {:?}", key, path, value);
        Some(value.to_string())
    }
}

/// Read HelionCoder's configured model.
///
/// Newer configs use ~/.helioncoder/settings.json:model; older/imported configs
/// may still keep the selected OpenAI-compatible model in config.json:openaiModel.
fn read_helioncoder_configured_model() -> Option<String> {
    let home = crate::storage::home_dir()?;
    let root = std::path::Path::new(&home).join(".helioncoder");
    read_json_string_field(&root.join("settings.json"), "model")
        .or_else(|| read_json_string_field(&root.join("config.json"), "openaiModel"))
}

async fn read_remote_helioncoder_configured_model(remote: &RemoteHost) -> Option<String> {
    let python = r#"import json, os
for path, key in [
    ("~/.helioncoder/settings.json", "model"),
    ("~/.helioncoder/config.json", "openaiModel"),
]:
    try:
        with open(os.path.expanduser(path), "r", encoding="utf-8") as f:
            value = json.load(f).get(key)
        if isinstance(value, str) and value.strip():
            print(value.strip())
            break
    except Exception:
        pass
"#;
    let remote_cmd = format!(
        r#"if command -v python3 >/dev/null 2>&1; then python3 -c {}; else model="$(sed -n 's/.*"model"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$HOME/.helioncoder/settings.json" 2>/dev/null | head -n1)"; if [ -z "$model" ]; then model="$(sed -n 's/.*"openaiModel"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$HOME/.helioncoder/config.json" 2>/dev/null | head -n1)"; fi; printf '%s\n' "$model"; fi"#,
        crate::agent::ssh::shell_escape(python)
    );

    let mut cmd = crate::agent::ssh::build_ssh_command(remote, &remote_cmd);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .hide_console()
        .kill_on_drop(true);

    let output = tokio::time::timeout(std::time::Duration::from_secs(5), cmd.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let model = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if model.is_empty() {
        None
    } else {
        Some(model)
    }
}

/// Fallback model list when CLI is unavailable.
pub fn fallback_cli_info() -> CliInfo {
    CliInfo {
        models: vec![
            CliModelInfo {
                value: "default".to_string(),
                display_name: "Default (recommended)".to_string(),
                description: "Sonnet 4.5".to_string(),
                supports_effort: Some(true),
                supported_effort_levels: Some(vec![
                    "low".into(),
                    "medium".into(),
                    "high".into(),
                    "max".into(),
                ]),
                supports_adaptive_thinking: Some(true),
            },
            CliModelInfo {
                value: "opus".to_string(),
                display_name: "Opus".to_string(),
                description: "Opus 4.7".to_string(),
                supports_effort: Some(true),
                supported_effort_levels: Some(vec![
                    "low".into(),
                    "medium".into(),
                    "high".into(),
                    "xhigh".into(),
                    "max".into(),
                ]),
                supports_adaptive_thinking: Some(true),
            },
            CliModelInfo {
                value: "haiku".to_string(),
                display_name: "Haiku".to_string(),
                description: "Haiku 4.5".to_string(),
                supports_effort: Some(false),
                supported_effort_levels: None,
                supports_adaptive_thinking: Some(false),
            },
        ],
        commands: vec![],
        available_output_styles: vec!["default".to_string()],
        account: None,
        current_model: read_helioncoder_configured_model(),
        fetched_at: now_iso(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_cli_info_effort_metadata() {
        let info = fallback_cli_info();
        let find = |v: &str| info.models.iter().find(|m| m.value == v).unwrap();

        assert_eq!(find("default").supports_effort, Some(true));
        assert!(find("default")
            .supported_effort_levels
            .as_ref()
            .unwrap()
            .contains(&"medium".to_string()));
        assert_eq!(find("opus").supports_effort, Some(true));
        assert!(find("opus")
            .supported_effort_levels
            .as_ref()
            .unwrap()
            .contains(&"xhigh".to_string()));
        assert_eq!(find("haiku").supports_effort, Some(false));
        assert!(find("haiku").supported_effort_levels.is_none());
    }
}
