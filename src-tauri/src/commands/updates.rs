use crate::process_ext::{DetachForUpdate, HideConsole};
use reqwest::Client;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ── Constants ──

const GITHUB_API_URL: &str =
    "https://api.github.com/repos/gabrielpondc/HelionCoder-APP/releases/latest";
const CLI_UPDATE_TIMEOUT_SECS: u64 = 600;

// ── HTTP client (reuse across requests) ──

static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(10))
        .user_agent(format!("HelionCoder/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
});

// ── Types ──

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub has_update: bool,
    pub latest_version: String,
    pub current_version: String,
    pub download_url: String,
    pub asset_name: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCommandResult {
    pub success: bool,
    pub output: String,
}

// ── Version comparison ──

/// Compare two semver-like version strings. Returns true if `latest` is newer than `current`.
/// Strips leading 'v' prefix. Pre-release versions (e.g. "1.0.0-beta.1") are considered
/// older than the same version without pre-release suffix.
/// Returns false on any parse failure (safe degradation).
fn parse_version(s: &str) -> Option<([u64; 3], bool)> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let (main, has_pre) = if let Some(idx) = s.find('-') {
        (&s[..idx], true)
    } else {
        (s, false)
    };
    let parts: Vec<&str> = main.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let major = parts[0].parse::<u64>().ok()?;
    let minor = parts[1].parse::<u64>().ok()?;
    let patch = parts[2].parse::<u64>().ok()?;
    Some(([major, minor, patch], has_pre))
}

fn is_newer(current: &str, latest: &str) -> bool {
    let (cur_ver, cur_pre) = match parse_version(current) {
        Some(v) => v,
        None => return false,
    };
    let (lat_ver, lat_pre) = match parse_version(latest) {
        Some(v) => v,
        None => return false,
    };

    if lat_ver > cur_ver {
        return true;
    }
    if lat_ver < cur_ver {
        return false;
    }

    matches!((cur_pre, lat_pre), (true, false))
}

// ── Asset selection ──

fn platform_tokens() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        &["macos", "darwin", "apple"]
    }
    #[cfg(target_os = "windows")]
    {
        &["windows", "win32", "win"]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        &["linux", "appimage", "deb"]
    }
}

fn arch_tokens() -> &'static [&'static str] {
    match std::env::consts::ARCH {
        "aarch64" => &["aarch64", "arm64", "universal"],
        "x86_64" => &["x86_64", "x64", "amd64", "universal"],
        "x86" => &["x86", "i686", "ia32"],
        _ => &["universal"],
    }
}

fn asset_score(name: &str) -> i32 {
    let lower = name.to_ascii_lowercase();
    let mut score = 0;
    if platform_tokens().iter().any(|token| lower.contains(token)) {
        score += 20;
    }
    if arch_tokens().iter().any(|token| lower.contains(token)) {
        score += 30;
    }
    if lower.contains("universal") {
        score += 5;
    }
    if lower.contains("helioncoder") || lower.contains("helion-coder") {
        score += 5;
    }
    score
}

fn select_asset_for_exts(
    body: &serde_json::Value,
    preferred_exts: &[&str],
) -> Option<(String, String)> {
    let assets = body["assets"].as_array()?;

    for ext in preferred_exts {
        let ext_lower = ext.to_ascii_lowercase();
        let mut best: Option<(&serde_json::Value, i32)> = None;
        for asset in assets {
            let name = asset["name"].as_str().unwrap_or("");
            let lower = name.to_ascii_lowercase();
            if !lower.ends_with(&ext_lower) {
                continue;
            }
            let Some(url) = asset["browser_download_url"].as_str() else {
                continue;
            };
            if url.is_empty() {
                continue;
            }
            let score = asset_score(name);
            if best
                .as_ref()
                .map(|(_, best_score)| score > *best_score)
                .unwrap_or(true)
            {
                best = Some((asset, score));
            }
        }
        if let Some((asset, _)) = best {
            return Some((
                asset["browser_download_url"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                asset["name"].as_str().unwrap_or("").to_string(),
            ));
        }
    }

    for asset in assets {
        if let Some(url) = asset["browser_download_url"].as_str() {
            return Some((
                url.to_string(),
                asset["name"].as_str().unwrap_or("").to_string(),
            ));
        }
    }

    None
}

/// Platform-independent: select download URL given preferred extensions.
#[cfg(test)]
fn select_download_url_for_exts(body: &serde_json::Value, preferred_exts: &[&str]) -> String {
    select_asset_for_exts(body, preferred_exts)
        .map(|(url, _)| url)
        .unwrap_or_else(|| body["html_url"].as_str().unwrap_or("").to_string())
}

fn select_asset(body: &serde_json::Value) -> (String, String) {
    #[cfg(target_os = "macos")]
    let exts: &[&str] = &[".app.tar.gz", ".dmg", ".zip"];
    #[cfg(target_os = "windows")]
    let exts: &[&str] = &[".zip", ".exe", ".msi"];
    #[cfg(all(unix, not(target_os = "macos")))]
    let exts: &[&str] = &[".appimage", ".tar.gz", ".deb"];

    select_asset_for_exts(body, exts).unwrap_or_else(|| {
        (
            body["html_url"].as_str().unwrap_or("").to_string(),
            String::new(),
        )
    })
}

// ── Tauri commands ──

#[tauri::command]
pub async fn check_for_updates(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    let current_version = app.package_info().version.to_string();
    log::debug!(
        "[updates] checking for app updates, current={}",
        current_version
    );

    let resp = match CLIENT
        .get(GITHUB_API_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[updates] network error (offline/timeout): {}", e);
            return Ok(empty_update(current_version));
        }
    };

    let status = resp.status();
    if !status.is_success() {
        log::warn!("[updates] GitHub API returned HTTP {}", status);
        return Ok(empty_update(current_version));
    }

    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[updates] failed to parse response: {}", e);
            return Ok(empty_update(current_version));
        }
    };

    let tag = body["tag_name"].as_str().unwrap_or("");
    if tag.is_empty() {
        log::warn!("[updates] empty tag_name in response");
        return Ok(empty_update(current_version));
    }

    let (download_url, asset_name) = select_asset(&body);
    let latest_version = tag.strip_prefix('v').unwrap_or(tag).to_string();
    let has_update = is_newer(&current_version, tag);

    log::debug!(
        "[updates] current={} latest={} has_update={} asset={}",
        current_version,
        latest_version,
        has_update,
        asset_name
    );

    Ok(UpdateInfo {
        has_update,
        latest_version,
        current_version,
        download_url,
        asset_name,
    })
}

#[tauri::command]
pub async fn run_cli_update() -> Result<UpdateCommandResult, String> {
    let cli_bin = crate::agent::claude_stream::resolve_helioncoder_path();
    let path_env = crate::agent::claude_stream::augmented_path();
    log::debug!("[updates] running CLI update: {}", cli_bin);

    let child = tokio::process::Command::new(&cli_bin)
        .arg("update")
        .env("PATH", path_env)
        // The desktop button uses the CLI updater's external helper to replace the binary.
        // Do not relaunch a standalone CLI process into this captured stdio pipe; it can
        // keep the command open and make the app look frozen.
        .env("HELION_CLI_UPDATE_RELAUNCH", "0")
        .env("HELION_CLI_UPDATE_DETACHED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .hide_console()
        .spawn()
        .map_err(|e| format!("Failed to start CLI updater: {}", e))?;

    let output = tokio::time::timeout(
        Duration::from_secs(CLI_UPDATE_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| "CLI update timed out after 10 minutes".to_string())?
    .map_err(|e| format!("CLI updater process error: {}", e))?;

    crate::agent::claude_stream::invalidate_claude_path_cache();

    let mut text = String::new();
    text.push_str(String::from_utf8_lossy(&output.stdout).trim());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(stderr);
    }

    Ok(UpdateCommandResult {
        success: output.status.success(),
        output: text,
    })
}

#[tauri::command]
pub async fn install_app_update(
    app: tauri::AppHandle,
    download_url: Option<String>,
    asset_name: Option<String>,
) -> Result<UpdateCommandResult, String> {
    let (url, asset) = match download_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(url) => (
            url.to_string(),
            asset_name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| asset_name_from_url(url)),
        ),
        None => {
            let info = check_for_updates(app.clone()).await?;
            if !info.has_update {
                return Ok(UpdateCommandResult {
                    success: false,
                    output: "No app update is available.".to_string(),
                });
            }
            (info.download_url, info.asset_name)
        }
    };

    if url.is_empty() {
        return Err("No app update download URL is available.".to_string());
    }

    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to locate current app executable: {}", e))?;
    let script = create_app_update_script(&url, &asset, &current_exe)?;
    spawn_app_update_script(&script, &url, &asset, &current_exe)?;

    let app_to_exit = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(350)).await;
        log::debug!("[updates] exiting app for self-update");
        app_to_exit.exit(0);
    });

    Ok(UpdateCommandResult {
        success: true,
        output: "App update scheduled. HelionCoder will close and relaunch.".to_string(),
    })
}

fn empty_update(current_version: String) -> UpdateInfo {
    UpdateInfo {
        has_update: false,
        latest_version: String::new(),
        current_version,
        download_url: String::new(),
        asset_name: String::new(),
    }
}

// ── App self-update helper scripts ──

fn create_app_update_script(
    _download_url: &str,
    asset_name: &str,
    _current_exe: &Path,
) -> Result<PathBuf, String> {
    let work_dir = app_update_work_dir()?;
    fs::create_dir_all(&work_dir).map_err(|e| format!("Failed to create update dir: {}", e))?;
    let safe_asset = sanitize_asset_name(asset_name);

    #[cfg(target_os = "macos")]
    let (script_name, script_body) = ("update-app.sh", macos_update_script(&work_dir, &safe_asset));
    #[cfg(target_os = "windows")]
    let (script_name, script_body) = (
        "update-app.ps1",
        windows_update_script(&work_dir, &safe_asset),
    );
    #[cfg(all(unix, not(target_os = "macos")))]
    let (script_name, script_body) = ("update-app.sh", linux_update_script(&work_dir, &safe_asset));

    let script_path = work_dir.join(script_name);
    fs::write(&script_path, script_body)
        .map_err(|e| format!("Failed to write update script: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path)
            .map_err(|e| format!("Failed to read update script metadata: {}", e))?
            .permissions();
        perms.set_mode(0o700);
        fs::set_permissions(&script_path, perms)
            .map_err(|e| format!("Failed to mark update script executable: {}", e))?;
    }

    Ok(script_path)
}

fn spawn_app_update_script(
    script_path: &Path,
    download_url: &str,
    asset_name: &str,
    current_exe: &Path,
) -> Result<(), String> {
    let pid = std::process::id().to_string();
    let exe = current_exe.to_string_lossy().to_string();
    let asset = sanitize_asset_name(asset_name);

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut cmd = std::process::Command::new("powershell");
        cmd.arg("-NoProfile")
            .arg("-WindowStyle")
            .arg("Hidden")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(script_path)
            .arg(&pid)
            .arg(download_url)
            .arg(&asset)
            .arg(&exe);
        cmd
    };

    #[cfg(not(target_os = "windows"))]
    let mut command = {
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg(script_path)
            .arg(&pid)
            .arg(download_url)
            .arg(&asset)
            .arg(&exe);
        cmd
    };

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .detach_for_update();

    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to start app updater helper: {}", e))
}

fn app_update_work_dir() -> Result<PathBuf, String> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("System clock error: {}", e))?
        .as_millis();
    Ok(std::env::temp_dir().join(format!(
        "helioncoder-app-update-{}-{}",
        std::process::id(),
        ts
    )))
}

fn sanitize_asset_name(name: &str) -> String {
    let trimmed = name.trim();
    let base = Path::new(trimmed)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(trimmed);
    let clean: String = base
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '+') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() {
        "HelionCoder-update.bin".to_string()
    } else {
        clean
    }
}

fn asset_name_from_url(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut segments| segments.next_back())
                .map(ToOwned::to_owned)
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "HelionCoder-update.bin".to_string())
}

#[cfg(target_os = "macos")]
fn macos_update_script(work_dir: &Path, safe_asset: &str) -> String {
    format!(
        r#"#!/bin/sh
set -eu

TARGET_PID="$1"
DOWNLOAD_URL="$2"
ASSET_NAME="$3"
CURRENT_EXE="$4"
WORK_DIR={work_dir}
ASSET_SAFE="{safe_asset}"
DOWNLOAD_PATH="$WORK_DIR/$ASSET_SAFE"

mkdir -p "$WORK_DIR"
/usr/bin/curl -L --fail --retry 2 --connect-timeout 20 -o "$DOWNLOAD_PATH" "$DOWNLOAD_URL"

APP_PATH=""
CUR="$CURRENT_EXE"
while [ "$CUR" != "/" ]; do
  case "$CUR" in
    *.app) APP_PATH="$CUR"; break ;;
  esac
  CUR="$(/usr/bin/dirname "$CUR")"
done

if [ -z "$APP_PATH" ] || [ ! -d "$APP_PATH" ]; then
  echo "Could not locate running .app bundle for $CURRENT_EXE" >&2
  exit 1
fi

wait_for_exit() {{
  i=0
  while /bin/kill -0 "$TARGET_PID" 2>/dev/null && [ "$i" -lt 40 ]; do
    /bin/sleep 0.25
    i=$((i + 1))
  done
  if /bin/kill -0 "$TARGET_PID" 2>/dev/null; then
    /bin/kill "$TARGET_PID" 2>/dev/null || true
    /bin/sleep 1
  fi
  if /bin/kill -0 "$TARGET_PID" 2>/dev/null; then
    /bin/kill -9 "$TARGET_PID" 2>/dev/null || true
  fi
}}

install_app() {{
  SRC_APP="$1"
  if [ -z "$SRC_APP" ] || [ ! -d "$SRC_APP" ]; then
    echo "Update archive did not contain an app bundle" >&2
    exit 1
  fi
  BACKUP="$APP_PATH.update-backup"
  /bin/rm -rf "$BACKUP"
  if [ -d "$APP_PATH" ]; then
    /bin/mv "$APP_PATH" "$BACKUP"
  fi
  if /usr/bin/ditto "$SRC_APP" "$APP_PATH"; then
    /bin/rm -rf "$BACKUP"
  else
    /bin/rm -rf "$APP_PATH"
    if [ -d "$BACKUP" ]; then /bin/mv "$BACKUP" "$APP_PATH"; fi
    exit 1
  fi
}}

LOWER="$(printf '%s' "$ASSET_NAME" | /usr/bin/tr '[:upper:]' '[:lower:]')"
UNPACK_DIR="$WORK_DIR/unpack"
/bin/rm -rf "$UNPACK_DIR"
/bin/mkdir -p "$UNPACK_DIR"

wait_for_exit

case "$LOWER" in
  *.dmg)
    MOUNT_DIR="$WORK_DIR/mount"
    /bin/rm -rf "$MOUNT_DIR"
    /bin/mkdir -p "$MOUNT_DIR"
    /usr/bin/hdiutil attach "$DOWNLOAD_PATH" -nobrowse -quiet -mountpoint "$MOUNT_DIR"
    SRC_APP="$(/usr/bin/find "$MOUNT_DIR" -maxdepth 3 -name '*.app' -type d | /usr/bin/head -n 1)"
    install_app "$SRC_APP"
    /usr/bin/hdiutil detach "$MOUNT_DIR" -quiet || true
    ;;
  *.zip)
    /usr/bin/unzip -q "$DOWNLOAD_PATH" -d "$UNPACK_DIR"
    SRC_APP="$(/usr/bin/find "$UNPACK_DIR" -maxdepth 4 -name '*.app' -type d | /usr/bin/head -n 1)"
    install_app "$SRC_APP"
    ;;
  *.app.tar.gz|*.tar.gz|*.tgz)
    /usr/bin/tar -xzf "$DOWNLOAD_PATH" -C "$UNPACK_DIR"
    SRC_APP="$(/usr/bin/find "$UNPACK_DIR" -maxdepth 4 -name '*.app' -type d | /usr/bin/head -n 1)"
    install_app "$SRC_APP"
    ;;
  *)
    echo "Unsupported macOS update asset: $ASSET_NAME" >&2
    exit 1
    ;;
esac

/usr/bin/open "$APP_PATH"
"#,
        work_dir = shell_single_quote(&work_dir.to_string_lossy()),
        safe_asset = safe_asset
    )
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_update_script(work_dir: &Path, safe_asset: &str) -> String {
    format!(
        r#"#!/bin/sh
set -eu

TARGET_PID="$1"
DOWNLOAD_URL="$2"
ASSET_NAME="$3"
CURRENT_EXE="$4"
WORK_DIR={work_dir}
ASSET_SAFE="{safe_asset}"
DOWNLOAD_PATH="$WORK_DIR/$ASSET_SAFE"

mkdir -p "$WORK_DIR"
if command -v curl >/dev/null 2>&1; then
  curl -L --fail --retry 2 --connect-timeout 20 -o "$DOWNLOAD_PATH" "$DOWNLOAD_URL"
else
  wget -O "$DOWNLOAD_PATH" "$DOWNLOAD_URL"
fi

i=0
while kill -0 "$TARGET_PID" 2>/dev/null && [ "$i" -lt 40 ]; do
  sleep 0.25
  i=$((i + 1))
done
if kill -0 "$TARGET_PID" 2>/dev/null; then
  kill "$TARGET_PID" 2>/dev/null || true
  sleep 1
fi
if kill -0 "$TARGET_PID" 2>/dev/null; then
  kill -9 "$TARGET_PID" 2>/dev/null || true
fi

LOWER="$(printf '%s' "$ASSET_NAME" | tr '[:upper:]' '[:lower:]')"
BACKUP="$CURRENT_EXE.update-backup"

install_binary() {{
  SRC="$1"
  rm -f "$BACKUP"
  if [ -f "$CURRENT_EXE" ]; then mv "$CURRENT_EXE" "$BACKUP"; fi
  if cp "$SRC" "$CURRENT_EXE"; then
    chmod +x "$CURRENT_EXE"
    rm -f "$BACKUP"
  else
    rm -f "$CURRENT_EXE"
    if [ -f "$BACKUP" ]; then mv "$BACKUP" "$CURRENT_EXE"; fi
    exit 1
  fi
}}

case "$LOWER" in
  *.appimage)
    chmod +x "$DOWNLOAD_PATH"
    install_binary "$DOWNLOAD_PATH"
    ;;
  *.tar.gz|*.tgz)
    UNPACK_DIR="$WORK_DIR/unpack"
    rm -rf "$UNPACK_DIR"
    mkdir -p "$UNPACK_DIR"
    tar -xzf "$DOWNLOAD_PATH" -C "$UNPACK_DIR"
    SRC_BIN="$(find "$UNPACK_DIR" -type f -perm -111 -name 'HelionCoder*' | head -n 1)"
    install_binary "$SRC_BIN"
    ;;
  *.deb)
    if command -v xdg-open >/dev/null 2>&1; then xdg-open "$DOWNLOAD_PATH" >/dev/null 2>&1 || true; fi
    exit 0
    ;;
  *)
    echo "Unsupported Linux update asset: $ASSET_NAME" >&2
    exit 1
    ;;
esac

nohup "$CURRENT_EXE" >/dev/null 2>&1 &
"#,
        work_dir = shell_single_quote(&work_dir.to_string_lossy()),
        safe_asset = safe_asset
    )
}

#[cfg(target_os = "windows")]
fn windows_update_script(work_dir: &Path, safe_asset: &str) -> String {
    format!(
        r#"$ErrorActionPreference = "Stop"

$TargetPid = [int]$args[0]
$DownloadUrl = [string]$args[1]
$AssetName = [string]$args[2]
$CurrentExe = [string]$args[3]
$WorkDir = {work_dir}
$AssetSafe = "{safe_asset}"
$DownloadPath = Join-Path $WorkDir $AssetSafe

New-Item -ItemType Directory -Force -Path $WorkDir | Out-Null
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Invoke-WebRequest -Uri $DownloadUrl -OutFile $DownloadPath -UseBasicParsing

$proc = Get-Process -Id $TargetPid -ErrorAction SilentlyContinue
if ($proc -and -not $proc.WaitForExit(10000)) {{
  Stop-Process -Id $TargetPid -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 1
}}

function Install-Binary([string]$Source) {{
  if (-not (Test-Path $Source)) {{ throw "Update archive did not contain an executable." }}
  $backup = "$CurrentExe.update-backup"
  Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
  if (Test-Path $CurrentExe) {{
    Move-Item -LiteralPath $CurrentExe -Destination $backup -Force
  }}
  try {{
    Copy-Item -LiteralPath $Source -Destination $CurrentExe -Force
    Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
  }} catch {{
    Remove-Item -LiteralPath $CurrentExe -Force -ErrorAction SilentlyContinue
    if (Test-Path $backup) {{ Move-Item -LiteralPath $backup -Destination $CurrentExe -Force }}
    throw
  }}
}}

$lower = $AssetName.ToLowerInvariant()
if ($lower.EndsWith(".zip")) {{
  $unpack = Join-Path $WorkDir "unpack"
  Remove-Item -Recurse -Force -Path $unpack -ErrorAction SilentlyContinue
  Expand-Archive -LiteralPath $DownloadPath -DestinationPath $unpack -Force
  $src = Get-ChildItem -Path $unpack -Recurse -File -Include "HelionCoder*.exe" | Select-Object -First 1
  Install-Binary $src.FullName
  Start-Process -FilePath $CurrentExe
}} elseif ($lower.EndsWith(".msi")) {{
  Start-Process -FilePath "msiexec.exe" -ArgumentList @("/i", "`"$DownloadPath`"", "/passive", "/norestart") -Wait
  if (Test-Path $CurrentExe) {{ Start-Process -FilePath $CurrentExe }}
}} elseif ($lower.EndsWith(".exe") -and ($lower.Contains("setup") -or $lower.Contains("installer"))) {{
  Start-Process -FilePath $DownloadPath -ArgumentList "/S" -Wait
  if (Test-Path $CurrentExe) {{ Start-Process -FilePath $CurrentExe }}
}} elseif ($lower.EndsWith(".exe")) {{
  Install-Binary $DownloadPath
  Start-Process -FilePath $CurrentExe
}} else {{
  throw "Unsupported Windows update asset: $AssetName"
}}
"#,
        work_dir = powershell_single_quote(&work_dir.to_string_lossy()),
        safe_asset = safe_asset
    )
}

#[cfg(not(target_os = "windows"))]
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "windows")]
fn powershell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_normal_upgrade() {
        assert!(is_newer("0.1.2", "v0.1.3"));
    }

    #[test]
    fn test_equal_versions() {
        assert!(!is_newer("0.1.3", "0.1.3"));
    }

    #[test]
    fn test_latest_is_prerelease() {
        assert!(!is_newer("0.1.3", "0.1.3-beta.1"));
    }

    #[test]
    fn test_current_is_prerelease() {
        assert!(is_newer("0.1.3-beta.1", "0.1.3"));
    }

    #[test]
    fn test_downgrade() {
        assert!(!is_newer("0.2.0", "0.1.9"));
    }

    #[test]
    fn test_v_prefix() {
        assert!(is_newer("v0.1.0", "v0.1.1"));
    }

    #[test]
    fn test_invalid_semver() {
        assert!(!is_newer("abc", "0.1.0"));
    }

    #[test]
    fn test_select_download_url_prefers_app_archive_on_macos() {
        let body = json!({
            "html_url": "https://example.com/HelionCoder-APP/releases/tag/v0.5.0",
            "assets": [
                { "name": "HelionCoder_0.5.0_macos_aarch64.dmg", "browser_download_url": "https://example.com/a.dmg" },
                { "name": "HelionCoder_0.5.0_macos_aarch64.app.tar.gz", "browser_download_url": "https://example.com/a.app.tar.gz" }
            ]
        });
        assert_eq!(
            select_download_url_for_exts(&body, &[".app.tar.gz", ".dmg"]),
            "https://example.com/a.app.tar.gz"
        );
    }

    #[test]
    fn test_select_download_url_prefers_matching_arch() {
        let body = json!({
            "html_url": "https://example.com/HelionCoder-APP/releases/tag/v0.5.0",
            "assets": [
                { "name": "HelionCoder_0.5.0_macos_x64.dmg", "browser_download_url": "https://example.com/x64.dmg" },
                { "name": "HelionCoder_0.5.0_macos_aarch64.dmg", "browser_download_url": "https://example.com/arm64.dmg" }
            ]
        });
        let selected = select_download_url_for_exts(&body, &[".dmg"]);
        if std::env::consts::ARCH == "aarch64" {
            assert_eq!(selected, "https://example.com/arm64.dmg");
        } else {
            assert_eq!(selected, "https://example.com/x64.dmg");
        }
    }

    #[test]
    fn test_select_download_url_prefers_zip() {
        let body = json!({
            "html_url": "https://example.com/HelionCoder-APP/releases/tag/v0.5.0",
            "assets": [
                { "name": "HelionCoder_0.5.0_windows_x64-setup.exe", "browser_download_url": "https://example.com/setup.exe" },
                { "name": "HelionCoder_0.5.0_windows_x64.zip", "browser_download_url": "https://example.com/a.zip" }
            ]
        });
        assert_eq!(
            select_download_url_for_exts(&body, &[".zip", ".exe"]),
            "https://example.com/a.zip"
        );
    }

    #[test]
    fn test_select_download_url_prefers_appimage() {
        let body = json!({
            "html_url": "https://example.com/HelionCoder-APP/releases/tag/v0.5.0",
            "assets": [
                { "name": "HelionCoder_0.5.0_linux_x64.AppImage", "browser_download_url": "https://example.com/a.AppImage" }
            ]
        });
        assert_eq!(
            select_download_url_for_exts(&body, &[".appimage", ".deb"]),
            "https://example.com/a.AppImage"
        );
    }

    #[test]
    fn test_select_download_url_falls_back_to_html() {
        let body = json!({
            "html_url": "https://example.com/HelionCoder-APP/releases/tag/v0.5.0",
            "assets": []
        });
        assert_eq!(
            select_download_url_for_exts(&body, &[".dmg"]),
            "https://example.com/HelionCoder-APP/releases/tag/v0.5.0"
        );
    }
}
