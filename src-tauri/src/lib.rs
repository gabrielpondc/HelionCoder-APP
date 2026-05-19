pub mod agent;
pub mod commands;
pub mod hooks;
pub mod models;
pub mod pricing;
pub mod process_ext;
pub mod storage;
pub mod web_server;

use agent::adapter::new_actor_session_map;
use agent::control::CliInfoCache;
use agent::spawn_locks::SpawnLocks;
use agent::stream::new_process_map;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::Arc;
use storage::events::EventWriter;
use tauri::{Emitter, Manager};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

/// Effective web server port (may differ from configured port if busy)
pub type EffectiveWebPort = Arc<AtomicU16>;
/// Web-server-specific cancel token for restart support
pub type WebServerCancel = Arc<tokio::sync::Mutex<CancellationToken>>;
/// Token version — shared between IPC and web server for rotation detection
pub type SharedTokenVersion = Arc<AtomicU64>;
/// WS shutdown broadcast — token rotation triggers disconnect of all WS clients
pub type WsShutdownSender = Arc<broadcast::Sender<()>>;
/// Live token — hot-swappable via RwLock for immediate login/logout on rotation
pub type SharedLiveToken = Arc<tokio::sync::RwLock<String>>;
/// Mutex to serialize web server start/stop operations
pub type WebServerLock = Arc<tokio::sync::Mutex<()>>;
/// JoinHandle for the serve task — await during stop to ensure port release
pub type WebServerHandle = Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>;
/// Generation counter — each spawn_server increments; stale tasks check before cleanup.
/// Newtype to avoid Tauri manage() collision with SharedTokenVersion (both Arc<AtomicU64>).
#[derive(Clone)]
pub struct WebServerGeneration(pub Arc<AtomicU64>);
/// Effective bind address — reflects actual running state (not settings).
/// Newtype to avoid Tauri manage() collision with SharedLiveToken (both Arc<RwLock<String>>).
#[derive(Clone)]
pub struct EffectiveWebBind(pub Arc<tokio::sync::RwLock<String>>);
/// Startup warning — populated when origins are degraded or other non-fatal startup issues.
#[derive(Clone)]
pub struct WebServerWarning(pub Arc<tokio::sync::RwLock<Option<String>>>);

/// One-shot gate to prevent concurrent shutdown tasks.
/// CAS ensures only the first caller proceeds; subsequent quit/close events are no-ops.
pub struct ShutdownGate(AtomicBool);

impl Default for ShutdownGate {
    fn default() -> Self {
        Self::new()
    }
}

impl ShutdownGate {
    pub fn new() -> Self {
        Self(AtomicBool::new(false))
    }
    /// Returns `true` if this call entered the gate (first caller wins).
    pub fn try_enter(&self) -> bool {
        self.0
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
}

pub fn run() {
    // Initialize logging — our crate at debug level by default
    // Override with RUST_LOG env var, e.g. RUST_LOG=warn cargo tauri dev
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("helioncoder_desktop_lib=debug,warn"),
    )
    .format_timestamp_millis()
    .init();

    log::info!("HelionCoder Desktop starting");

    // Set up Windows Job Object so child processes are killed on crash/force-quit.
    // No-op on non-Windows.
    process_ext::setup_job_kill_on_close();

    // Reconcile orphaned runs on startup
    storage::runs::reconcile_orphaned_runs();

    // Clean up legacy hook-bridge (removed: was redundant with stream-json mode)
    hooks::setup::cleanup_hook_bridge();

    // Global cancellation token — shared with all session actors for graceful shutdown
    let cancel_token = CancellationToken::new();
    let cancel_for_exit = cancel_token.clone();

    // Web server shared state
    let ws_shutdown_sender: WsShutdownSender = Arc::new(broadcast::channel::<()>(1).0);
    let shared_token_version: SharedTokenVersion = Arc::new(AtomicU64::new(0));
    let shared_live_token: SharedLiveToken = {
        use rand::Rng;
        let token: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();
        log::debug!("[app] ephemeral web token generated (masked)");
        Arc::new(tokio::sync::RwLock::new(token))
    };
    let effective_web_port: EffectiveWebPort = Arc::new(AtomicU16::new(0));
    let ws_cancel: WebServerCancel = Arc::new(tokio::sync::Mutex::new(CancellationToken::new()));
    let ws_lock: WebServerLock = Arc::new(tokio::sync::Mutex::new(()));
    let ws_handle: WebServerHandle = Arc::new(tokio::sync::Mutex::new(None));
    let ws_generation = WebServerGeneration(Arc::new(AtomicU64::new(0)));
    let ws_effective_bind = EffectiveWebBind(Arc::new(tokio::sync::RwLock::new(String::new())));
    let ws_warning = WebServerWarning(Arc::new(tokio::sync::RwLock::new(None)));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .manage(new_process_map())
        .manage(new_actor_session_map())
        .manage(CliInfoCache::new())
        .manage(Arc::new(EventWriter::new()))
        .manage(SpawnLocks::new())
        .manage(ShutdownGate::new())
        .manage(cancel_token)
        .manage(ws_shutdown_sender)
        .manage(shared_token_version)
        .manage(shared_live_token)
        .manage(effective_web_port)
        .manage(ws_cancel)
        .manage(ws_lock)
        .manage(ws_handle)
        .manage(ws_generation)
        .manage(ws_effective_bind)
        .manage(ws_warning)
        // NOTE: Currently ~60 IPC commands. If approaching 80+, consider grouping
        // into Tauri command modules or using a single dispatch command with typed payloads.
        .invoke_handler(tauri::generate_handler![
            commands::runs::list_runs,
            commands::runs::get_run,
            commands::runs::start_run,
            commands::runs::stop_run,
            commands::runs::update_run_model,
            commands::runs::rename_run,
            commands::runs::soft_delete_runs,
            commands::runs::search_prompts,
            commands::history::search_runs,
            commands::history::get_run_files,
            commands::runs::add_prompt_favorite,
            commands::runs::remove_prompt_favorite,
            commands::runs::update_prompt_favorite_tags,
            commands::runs::update_prompt_favorite_note,
            commands::runs::list_prompt_favorites,
            commands::runs::list_prompt_tags,
            commands::chat::send_chat_message,
            commands::events::get_run_events,
            commands::artifacts::get_run_artifacts,
            commands::settings::get_user_settings,
            commands::settings::update_user_settings,
            commands::settings::get_agent_settings,
            commands::settings::update_agent_settings,
            commands::fs::list_directory,
            commands::fs::check_is_directory,
            commands::fs::read_file_base64,
            commands::remote_fs::list_remote_directory,
            commands::remote_fs::resolve_remote_home,
            commands::git::get_git_summary,
            commands::git::get_git_branch,
            commands::git::get_git_diff,
            commands::git::get_git_status,
            commands::export::export_conversation,
            commands::export::write_html_export,
            commands::files::read_text_file,
            commands::files::stat_text_file,
            commands::files::write_text_file,
            commands::files::read_task_output,
            commands::files::list_memory_files,
            commands::stats::get_usage_overview,
            commands::stats::get_global_usage_overview,
            commands::stats::get_helion_usage_stats,
            commands::stats::clear_usage_cache,
            commands::stats::get_heatmap_daily,
            commands::stats::get_changelog,
            commands::diagnostics::check_agent_cli,
            commands::diagnostics::test_remote_host,
            commands::diagnostics::get_cli_dist_tags,
            commands::diagnostics::check_project_init,
            commands::diagnostics::check_ssh_key,
            commands::diagnostics::generate_ssh_key,
            commands::diagnostics::run_diagnostics,
            commands::diagnostics::detect_local_proxy,
            commands::diagnostics::test_api_connectivity,
            commands::diagnostics::list_api_models,
            commands::session::start_session,
            commands::session::send_session_message,
            commands::session::stop_session,
            commands::session::send_session_control,
            commands::session::broadcast_mcp_toggle,
            commands::session::get_bus_events,
            commands::session::fork_session,
            commands::session::side_question,
            commands::session::start_ralph_loop,
            commands::session::cancel_ralph_loop,
            commands::session::approve_session_tool,
            commands::session::cancel_control_request,
            commands::session::respond_permission,
            commands::session::respond_hook_callback,
            commands::session::respond_elicitation,
            commands::control::get_cli_info,
            commands::teams::list_teams,
            commands::teams::get_team_config,
            commands::teams::list_team_tasks,
            commands::teams::get_team_task,
            commands::teams::get_team_inbox,
            commands::teams::get_all_team_inboxes,
            commands::teams::delete_team,
            commands::plugins::list_marketplaces,
            commands::plugins::list_marketplace_plugins,
            commands::plugins::list_standalone_skills,
            commands::plugins::list_project_commands,
            commands::plugins::get_skill_content,
            commands::plugins::list_installed_plugins,
            commands::plugins::install_plugin,
            commands::plugins::uninstall_plugin,
            commands::plugins::enable_plugin,
            commands::plugins::disable_plugin,
            commands::plugins::update_plugin,
            commands::plugins::add_marketplace,
            commands::plugins::remove_marketplace,
            commands::plugins::update_marketplace,
            commands::plugins::create_skill,
            commands::plugins::update_skill,
            commands::plugins::delete_skill,
            commands::plugins::check_community_health,
            commands::plugins::search_community_skills,
            commands::plugins::get_community_skill_detail,
            commands::plugins::install_community_skill,
            commands::agents::list_agents,
            commands::agents::read_agent_file,
            commands::agents::create_agent_file,
            commands::agents::update_agent_file,
            commands::agents::delete_agent_file,
            commands::clipboard::get_clipboard_files,
            commands::clipboard::read_clipboard_file,
            commands::clipboard::save_temp_attachment,
            commands::mcp::list_configured_mcp_servers,
            commands::mcp::add_mcp_server,
            commands::mcp::remove_mcp_server,
            commands::mcp::toggle_mcp_server_config,
            commands::mcp::get_disabled_mcp_servers,
            commands::mcp::check_mcp_registry_health,
            commands::mcp::search_mcp_registry,
            commands::cli_config::get_cli_config,
            commands::cli_config::get_project_cli_config,
            commands::cli_config::update_cli_config,
            commands::cli_settings::get_cli_permissions,
            commands::cli_settings::update_cli_permissions,
            commands::onboarding::check_auth_status,
            commands::onboarding::detect_install_methods,
            commands::onboarding::install_helioncoder_cli,
            commands::onboarding::run_claude_login,
            commands::onboarding::get_auth_overview,
            commands::onboarding::set_cli_api_key,
            commands::onboarding::set_cli_api_config,
            commands::onboarding::remove_cli_api_key,
            commands::screenshot::capture_screenshot,
            commands::screenshot::update_screenshot_hotkey,
            commands::cli_sync::discover_cli_sessions,
            commands::cli_sync::import_cli_session,
            commands::cli_sync::sync_cli_session,
            commands::updates::check_for_updates,
            commands::updates::install_app_update,
            commands::updates::run_cli_update,
            commands::web_server::get_web_server_status,
            commands::web_server::get_web_server_token,
            commands::web_server::regenerate_web_server_token,
            commands::web_server::restart_web_server,
            commands::web_server::get_local_ip,
            commands::preview::open_preview_window,
            commands::preview::close_preview_window,
            commands::workspace::detect_workspace_tools,
            commands::workspace::open_workspace_tool,
        ])
        .setup(move |app| {
            // Set up broadcast emitter (requires AppHandle, so must be in setup)
            let broadcaster = web_server::broadcaster::EventBroadcaster::new();
            let writer = app.state::<Arc<EventWriter>>().inner().clone();
            let emitter = Arc::new(web_server::broadcaster::BroadcastEmitter::new(
                writer,
                app.handle().clone(),
                broadcaster.clone(),
            ));
            app.manage(broadcaster);
            app.manage(emitter);

            // Start web server (non-blocking, spawns async task)
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match web_server::start_server(&app_handle).await {
                    Ok(true) => log::debug!("[app] web server started"),
                    Ok(false) => log::debug!("[app] web server disabled"),
                    Err(e) => log::error!("[app] web server failed to start: {}", e),
                }
            });

            // Start team file watcher for ~/.claude/teams/ and ~/.claude/tasks/
            let cancel = app.state::<CancellationToken>().inner().clone();
            hooks::team_watcher::start_team_watcher(app.handle().clone(), cancel);

            setup_app_menu(app)?;

            // Global shortcut plugin — must be registered inside setup() with a handler
            // so the event dispatch loop is properly initialized
            {
                use tauri_plugin_global_shortcut::ShortcutState;
                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_handler(|app, _shortcut, event| {
                            if event.state == ShortcutState::Pressed {
                                commands::screenshot::handle_global_shortcut(app);
                            }
                        })
                        .build(),
                )?;
            }

            // Register screenshot hotkey from settings (must come after plugin init)
            commands::screenshot::init_screenshot_hotkey(app.handle());

            Ok(())
        })
        .on_window_event(move |window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    // Only intercept close for the main window
                    if window.label() != "main" {
                        return;
                    }
                    api.prevent_close();
                    log::debug!("[app] close requested, starting graceful shutdown");
                    let app = window.app_handle().clone();
                    if let Some(gate) = app.try_state::<ShutdownGate>() {
                        if !gate.try_enter() {
                            return; // shutdown already in progress
                        }
                    }
                    if let Some(ct) = app.try_state::<CancellationToken>() {
                        ct.cancel();
                    }
                    tauri::async_runtime::spawn(async move {
                        graceful_shutdown_actors(&app).await;
                        app.exit(0);
                    });
                }
                tauri::WindowEvent::Destroyed if window.label() == "main" => {
                    // Safety fallback: cancel actors if main window is truly destroyed (e.g. app.exit()).
                    // Skip for secondary windows (e.g. preview) — destroying them must not shut down the app.
                    cancel_for_exit.cancel();
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        // macOS: clicking the dock icon when all windows are hidden should reopen the window
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } = event
        {
            if !has_visible_windows {
                show_main_window(app_handle);
                log::debug!("[app] reopened window from dock click");
            }
        }

        let _ = (app_handle, event); // suppress unused warnings on non-macOS
    });
}

fn setup_app_menu(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};

    let about_metadata = AboutMetadata {
        name: Some("HelionCoder".to_string()),
        version: Some(app.package_info().version.to_string()),
        authors: Some(vec![
            "杨凤伟 指导开发".to_string(),
            "顾家楷 完成开发".to_string(),
            "叶凌云 提供开发工具".to_string(),
        ]),
        copyright: Some(
            "由 杨凤伟 指导开发 · 由 顾家楷 完成开发 · 由 叶凌云 提供开发工具".to_string(),
        ),
        credits: Some("由 杨凤伟 指导开发\n由 顾家楷 完成开发\n由 叶凌云 提供开发工具".to_string()),
        ..Default::default()
    };

    let app_menu = Submenu::with_items(
        app,
        "HelionCoder",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("关于 HelionCoder"), Some(about_metadata))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, Some("服务"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some("隐藏 HelionCoder"))?,
            &PredefinedMenuItem::hide_others(app, Some("隐藏其他"))?,
            &PredefinedMenuItem::show_all(app, Some("显示全部"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                "app-quit",
                "退出 HelionCoder",
                true,
                Some("CmdOrCtrl+KeyQ"),
            )?,
        ],
    )?;

    let file_menu = Submenu::with_items(
        app,
        "文件",
        true,
        &[
            &MenuItem::with_id(
                app,
                "cmd-new-claude",
                "新建聊天",
                true,
                Some("CmdOrCtrl+KeyN"),
            )?,
            &MenuItem::with_id(
                app,
                "cmd-set-cwd",
                "打开文件夹...",
                true,
                Some("CmdOrCtrl+KeyO"),
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                "cmd-export-chat",
                "导出聊天为 Markdown",
                true,
                Some("CmdOrCtrl+Shift+KeyE"),
            )?,
            &MenuItem::with_id(
                app,
                "cmd-export-chat-html",
                "导出聊天为 HTML",
                true,
                Some("CmdOrCtrl+Shift+KeyH"),
            )?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, Some("关闭窗口"))?,
        ],
    )?;

    let edit_menu = Submenu::with_items(
        app,
        "编辑",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("撤销"))?,
            &PredefinedMenuItem::redo(app, Some("重做"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some("剪切"))?,
            &PredefinedMenuItem::copy(app, Some("复制"))?,
            &PredefinedMenuItem::paste(app, Some("粘贴"))?,
            &PredefinedMenuItem::select_all(app, Some("全选"))?,
        ],
    )?;

    let chat_menu = Submenu::with_items(
        app,
        "聊天",
        true,
        &[
            &MenuItem::with_id(
                app,
                "cmd-switch-model",
                "切换模型...",
                true,
                Some("CmdOrCtrl+KeyP"),
            )?,
            &MenuItem::with_id(app, "cmd-compact", "压缩对话", true, None::<&str>)?,
            &MenuItem::with_id(app, "cmd-toggle-plan", "切换计划模式", true, None::<&str>)?,
            &MenuItem::with_id(app, "cmd-review", "审查更改", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "cmd-stop-run", "停止运行", true, None::<&str>)?,
        ],
    )?;

    let tools_menu = Submenu::with_items(
        app,
        "工具",
        true,
        &[
            &MenuItem::with_id(
                app,
                "cmd-git-diff",
                "查看 Git Diff",
                true,
                Some("CmdOrCtrl+Shift+KeyD"),
            )?,
            &MenuItem::with_id(app, "cmd-git-status", "查看 Git 状态", true, None::<&str>)?,
            &MenuItem::with_id(app, "cmd-token-cost", "查看 Token 用量", true, None::<&str>)?,
        ],
    )?;

    let navigation_menu = Submenu::with_items(
        app,
        "导航",
        true,
        &[
            &MenuItem::with_id(app, "cmd-go-chat", "前往聊天", true, None::<&str>)?,
            &MenuItem::with_id(app, "cmd-go-memory", "前往记忆", true, None::<&str>)?,
            &MenuItem::with_id(app, "cmd-go-plugins", "前往插件与技能", true, None::<&str>)?,
            &MenuItem::with_id(app, "cmd-go-usage", "前往用量", true, None::<&str>)?,
            &MenuItem::with_id(
                app,
                "cmd-go-settings",
                "前往设置",
                true,
                Some("CmdOrCtrl+Comma"),
            )?,
        ],
    )?;

    let settings_menu = Submenu::with_items(
        app,
        "设置",
        true,
        &[
            &MenuItem::with_id(app, "cmd-set-model", "设置默认模型", true, None::<&str>)?,
            &MenuItem::with_id(app, "cmd-configure-tools", "配置工具", true, None::<&str>)?,
            &MenuItem::with_id(app, "cmd-permissions", "权限", true, None::<&str>)?,
        ],
    )?;

    let view_menu = Submenu::with_items(
        app,
        "视图",
        true,
        &[
            &MenuItem::with_id(
                app,
                "app-command-palette",
                "命令面板...",
                true,
                Some("CmdOrCtrl+KeyK"),
            )?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::fullscreen(app, Some("进入全屏"))?,
        ],
    )?;

    let window_menu = Submenu::with_items(
        app,
        "窗口",
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some("最小化"))?,
            &PredefinedMenuItem::maximize(app, Some("缩放"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, Some("关闭窗口"))?,
        ],
    )?;

    let help_menu = Submenu::with_items(
        app,
        "帮助",
        true,
        &[
            &MenuItem::with_id(app, "cmd-doctor", "运行 Doctor", true, None::<&str>)?,
            &MenuItem::with_id(app, "cmd-version", "版本信息", true, None::<&str>)?,
        ],
    )?;

    let menu = Menu::with_items(
        app,
        &[
            &app_menu,
            &file_menu,
            &edit_menu,
            &chat_menu,
            &tools_menu,
            &navigation_menu,
            &settings_menu,
            &view_menu,
            &window_menu,
            &help_menu,
        ],
    )?;

    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        let id = event.id().as_ref();
        if let Some(command_id) = id.strip_prefix("cmd-") {
            let _ = app.emit("ocv:native-menu-command", command_id);
            return;
        }

        match id {
            "app-command-palette" => {
                let _ = app.emit("ocv:native-menu-command", "command-palette");
            }
            "app-quit" => {
                if let Some(gate) = app.try_state::<ShutdownGate>() {
                    if !gate.try_enter() {
                        return;
                    }
                }
                if let Some(ct) = app.try_state::<CancellationToken>() {
                    ct.cancel();
                }
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    graceful_shutdown_actors(&app).await;
                    app.exit(0);
                });
            }
            _ => {}
        }
    });

    log::debug!("[app] application menu created");
    Ok(())
}

/// Restore the main window: unminimize if needed, then show and focus.
fn show_main_window(handle: &impl tauri::Manager<tauri::Wry>) {
    if let Some(w) = handle.get_webview_window("main") {
        if w.is_minimized().unwrap_or(false) {
            let _ = w.unminimize();
        }
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Graceful shutdown: wait for actors to self-clean, then force-kill remaining processes.
///
/// Two-phase approach:
/// - Phase 1: Wait up to 3s for actors to exit (cancel token already fired → handle_stop → kill+wait).
/// - Phase 2: Drain remaining actors, try_send Stop, join with 2s timeout, abort if stuck.
/// - Then drain ProcessMap (stream processes).
async fn graceful_shutdown_actors(app: &tauri::AppHandle) {
    use crate::agent::adapter::ActorSessionMap;
    use crate::agent::session_actor::ActorCommand;
    use crate::agent::stream::ProcessMap;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);

    // ── Phase 1: Wait for actors to self-cleanup (cancel already fired) ──
    if let Some(sessions) = app.try_state::<ActorSessionMap>() {
        loop {
            let count = sessions.lock().await.len();
            if count == 0 {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                log::warn!(
                    "[app] graceful shutdown: {} actors still alive, force stopping",
                    count
                );
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // ── Phase 2: Force-stop remaining actors ──
        let remaining: Vec<_> = {
            let mut map = sessions.lock().await;
            map.drain().collect()
        };
        for (run_id, handle) in remaining {
            log::debug!("[app] force stopping actor: {}", run_id);
            // try_send avoids blocking if mailbox is full (bounded channel, 64 slots)
            let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
            let _ = handle
                .cmd_tx
                .try_send(ActorCommand::Stop { reply: reply_tx });
            // Get AbortHandle before consuming JoinHandle in timeout
            let abort = handle.join_handle.abort_handle();
            match tokio::time::timeout(std::time::Duration::from_secs(2), handle.join_handle).await
            {
                Ok(Ok(())) => {
                    log::debug!("[app] actor {} exited cleanly", run_id);
                }
                Ok(Err(e)) => {
                    log::warn!("[app] actor {} join error: {}", run_id, e);
                }
                Err(_) => {
                    log::warn!("[app] actor {} did not exit in 2s, aborting task", run_id);
                    abort.abort();
                }
            }
        }
    }

    // ── Kill remaining stream processes ──
    // ProcessMap lock is only held briefly (run_agent/stop_process do remove-then-await),
    // but we keep a timeout as a defensive fallback.
    if let Some(process_map) = app.try_state::<ProcessMap>() {
        let to_kill = match tokio::time::timeout(std::time::Duration::from_secs(1), async {
            let mut map = process_map.lock().await;
            map.drain().collect::<Vec<_>>()
        })
        .await
        {
            Ok(vec) => vec,
            Err(_) => {
                log::warn!(
                    "[app] graceful shutdown: ProcessMap lock timeout, \
                     skipping (kill_on_drop / Job Object may handle)"
                );
                Vec::new()
            }
        };
        for (run_id, mut child) in to_kill {
            log::debug!("[app] graceful shutdown: killing stream process {}", run_id);
            let _ = child.kill().await;
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
        }
    }

    log::debug!("[app] graceful shutdown complete");
}
