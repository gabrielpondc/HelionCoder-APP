use crate::agent::control::{self, CliInfoCache};
use crate::models::CliInfo;
use tauri::State;

#[tauri::command]
pub async fn get_cli_info(
    cache: State<'_, CliInfoCache>,
    force_refresh: Option<bool>,
    remote_host_name: Option<String>,
) -> Result<CliInfo, String> {
    log::debug!(
        "[control] get_cli_info IPC, force={}, remote={:?}",
        force_refresh.unwrap_or(false),
        remote_host_name
    );
    if let Some(remote_name) = remote_host_name.as_deref().filter(|s| !s.is_empty()) {
        let settings = crate::storage::settings::get_user_settings();
        let Some(remote) = settings
            .remote_hosts
            .iter()
            .find(|host| host.name == remote_name)
            .cloned()
        else {
            return Err(format!("Remote host '{}' not found", remote_name));
        };
        return match control::get_remote_cli_info(&remote).await {
            Ok(info) => Ok(info),
            Err(e) => {
                log::warn!(
                    "[control] remote CLI info failed for {} ({}): {}, using remote-safe fallback",
                    remote_name,
                    e.code,
                    e.message
                );
                let mut fallback = control::fallback_cli_info();
                fallback.current_model = None;
                Ok(fallback)
            }
        };
    }

    match control::get_cli_info(&cache, force_refresh.unwrap_or(false)).await {
        Ok(info) => Ok(info),
        Err(e) => {
            log::warn!(
                "[control] CLI info failed ({}): {}, using fallback",
                e.code,
                e.message
            );
            Ok(control::fallback_cli_info())
        }
    }
}
