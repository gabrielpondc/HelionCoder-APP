//! SSH command builder for remote Claude Code execution.
//!
//! Uses the system `ssh` binary — no new crate dependencies.
//! All remote commands are shell-escaped to prevent injection.

use crate::models::RemoteHost;
use crate::process_ext::HideConsole;
use std::path::PathBuf;
use tokio::process::Command;

/// Shell-escape a string using single quotes (POSIX-safe).
/// Any embedded single quote is replaced with `'\''` (end quote, escaped quote, start quote).
pub fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Shell-escape a path, preserving a leading tilde so the remote shell expands it.
/// `~/projects/my app` → `~/'projects/my app'`, bare `~` → `~`.
/// Without the bare-`~` carve-out, `shell_escape("~")` would emit `'~'` and POSIX
/// shells skip tilde expansion inside quotes, so `cd '~'` would look for a literal
/// directory named `~`.
pub fn shell_escape_path(s: &str) -> String {
    if s == "~" {
        return "~".to_string();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        format!("~/{}", shell_escape(rest))
    } else {
        shell_escape(s)
    }
}

/// Expand `~` to `$HOME` for local filesystem paths (e.g. SSH key paths).
/// Unlike shell_escape_path, this does actual expansion since `Command::arg()` doesn't go through a shell.
pub fn expand_local_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = crate::storage::home_dir() {
            let mut p = std::path::PathBuf::from(&home);
            p.push(rest);
            return p.to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

fn is_password_auth(remote: &RemoteHost) -> bool {
    remote.auth_method == "password" || (remote.key_path.is_none() && remote.password.is_some())
}

fn write_askpass_helper() -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(format!(
        "helioncoder-ssh-askpass-{}{}",
        uuid::Uuid::new_v4(),
        if cfg!(windows) { ".cmd" } else { ".sh" }
    ));

    #[cfg(windows)]
    let script = "@echo off\r\n<nul set /p dummy=%HELION_SSH_PASSWORD%\r\n";

    #[cfg(not(windows))]
    let script = "#!/bin/sh\nprintf '%s' \"$HELION_SSH_PASSWORD\"\n";

    std::fs::write(&path, script)
        .map_err(|e| format!("failed to write SSH askpass helper: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("failed to chmod SSH askpass helper: {}", e))?;
    }

    Ok(path)
}

fn configure_password_auth(cmd: &mut Command, password: &str) {
    match write_askpass_helper() {
        Ok(helper) => {
            cmd.env("SSH_ASKPASS", helper);
            cmd.env("SSH_ASKPASS_REQUIRE", "force");
            cmd.env("HELION_SSH_PASSWORD", password);
            // Some OpenSSH builds still require DISPLAY to consider askpass.
            cmd.env(
                "DISPLAY",
                std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string()),
            );
        }
        Err(e) => {
            log::warn!(
                "[ssh] password auth requested but askpass setup failed: {}",
                e
            );
        }
    }
}

/// Build an SSH `Command` that runs `remote_shell_command` on the remote host.
pub fn build_ssh_command(remote: &RemoteHost, remote_shell_command: &str) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.hide_console();
    if is_password_auth(remote) {
        cmd.arg("-o").arg("NumberOfPasswordPrompts=1");
        cmd.arg("-o")
            .arg("PreferredAuthentications=password,keyboard-interactive");
        cmd.arg("-o").arg("PubkeyAuthentication=no");
        if let Some(password) = remote.password.as_deref() {
            configure_password_auth(&mut cmd, password);
        }
    } else {
        cmd.arg("-o").arg("BatchMode=yes");
    }
    cmd.arg("-o").arg("ConnectTimeout=10");
    cmd.arg("-o").arg("ServerAliveInterval=30");
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");

    if remote.port != 22 {
        cmd.arg("-p").arg(remote.port.to_string());
    }
    if !is_password_auth(remote) {
        if let Some(ref key) = remote.key_path {
            // Expand ~/... for local key path (Command::arg doesn't go through shell)
            cmd.arg("-i").arg(expand_local_tilde(key));
        }
    }

    let target = format!("{}@{}", remote.user, remote.host);
    cmd.arg(&target);
    cmd.arg(remote_shell_command);

    log::debug!(
        "[ssh] build_ssh_command: target={}, port={}, auth={}, key={:?}, cmd_len={}",
        target,
        remote.port,
        if is_password_auth(remote) {
            "password"
        } else {
            "key"
        },
        remote.key_path,
        remote_shell_command.len()
    );

    cmd
}

/// Build the shell command string to run Claude CLI on the remote host.
///
/// - `cwd`: Already-snapshotted remote_cwd from RunMeta (audit #4).
/// - `claude_args`: CLI arguments (e.g. `["--output-format", "stream-json", ...]`).
/// - `api_key`: Anthropic official API key (`x-api-key` header).
/// - `auth_token`: Third-party platform token (`Authorization: Bearer` header).
/// - `base_url`: Custom API endpoint URL.
#[allow(clippy::too_many_arguments)]
pub fn build_remote_claude_command(
    remote: &RemoteHost,
    cwd: &str,
    claude_args: &[String],
    api_key: Option<&str>,
    auth_token: Option<&str>,
    base_url: Option<&str>,
    models: Option<&[String]>,
    extra_env: Option<&std::collections::HashMap<String, String>>,
) -> String {
    let cli_expr = remote
        .remote_claude_path
        .as_deref()
        .map(shell_escape_path)
        .unwrap_or_else(|| "\"$(command -v helion-coder || command -v helioncoder)\"".to_string());

    let mut parts = Vec::new();

    // cd to remote working directory (preserves ~/... expansion)
    parts.push(format!("cd {}", shell_escape_path(cwd)));

    // Build the HelionCoder command with optional env var prefixes.
    let mut claude_parts = Vec::new();
    if let Some(key) = api_key {
        claude_parts.push(format!("ANTHROPIC_API_KEY={}", shell_escape(key)));
        // Clear AUTH_TOKEN to avoid remote shell env vars interfering
        claude_parts.push("ANTHROPIC_AUTH_TOKEN=".to_string());
    }
    if let Some(token) = auth_token {
        claude_parts.push(format!("ANTHROPIC_AUTH_TOKEN={}", shell_escape(token)));
        // Clear API_KEY to avoid conflict
        claude_parts.push("ANTHROPIC_API_KEY=".to_string());
    }
    if let Some(url) = base_url {
        claude_parts.push(format!("ANTHROPIC_BASE_URL={}", shell_escape(url)));
    }
    // Inject model tier env vars for third-party platforms
    if let Some(m) = models {
        for (k, v) in crate::commands::session::resolve_model_tiers(m) {
            claude_parts.push(format!("{}={}", k, shell_escape(&v)));
        }
    }
    // Inject extra env vars (only allow safe key names: [A-Z0-9_])
    if let Some(extra) = extra_env {
        for (k, v) in extra {
            if k.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                claude_parts.push(format!("{}={}", k, shell_escape(v)));
            } else {
                log::warn!("[ssh] skipping extra_env key with invalid chars: {}", k);
            }
        }
    }
    // Enable file checkpointing in SDK/non-interactive mode
    claude_parts.push("CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=1".to_string());
    // Escape custom binary paths; default to PATH lookup with helion-coder preferred.
    let cli_label = cli_expr.clone();
    claude_parts.push(cli_expr);
    for arg in claude_args {
        claude_parts.push(shell_escape(arg));
    }

    parts.push(claude_parts.join(" "));

    let full_cmd = parts.join(" && ");
    log::debug!(
        "[ssh] build_remote_claude_command: cwd={}, bin={}, args={}, has_key={}, has_token={}, has_url={}, has_model={}, extra_env_count={}",
        cwd,
        cli_label,
        claude_args.len(),
        api_key.is_some(),
        auth_token.is_some(),
        base_url.is_some(),
        models.is_some(),
        extra_env.map_or(0, |e| e.len())
    );

    full_cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_escape_path_bare_tilde_stays_unquoted() {
        // Quoting `~` would block the remote shell's tilde expansion.
        assert_eq!(shell_escape_path("~"), "~");
    }

    #[test]
    fn shell_escape_path_tilde_prefix_preserved() {
        assert_eq!(shell_escape_path("~/projects"), "~/'projects'");
        assert_eq!(shell_escape_path("~/my app"), "~/'my app'");
    }

    #[test]
    fn shell_escape_path_absolute_quoted() {
        assert_eq!(shell_escape_path("/home/user"), "'/home/user'");
        assert_eq!(shell_escape_path("/path with space"), "'/path with space'");
    }

    #[test]
    fn shell_escape_path_handles_embedded_quote() {
        // POSIX-safe: end quote, escaped quote, start quote.
        assert_eq!(shell_escape_path("/it's"), "'/it'\\''s'");
    }
}
