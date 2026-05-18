use reqwest::Client;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

// ── Constants ──

const RELEASES_API_URL: &str =
    "https://api.github.com/repos/gabrielpondc/HelionCoder/releases?per_page=50";
const CACHE_TTL: Duration = Duration::from_secs(600); // 10 minutes

// ── HTTP client (reuse across requests) ──

static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(10))
        .user_agent("HelionCoder/0.5.0")
        .build()
        .unwrap_or_default()
});

// ── Cache ──

struct CacheEntry {
    entries: Vec<ChangelogEntry>,
    fetched_at: Instant,
}

static CACHE: LazyLock<Mutex<Option<CacheEntry>>> = LazyLock::new(|| Mutex::new(None));

// ── Types ──

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangelogEntry {
    pub version: String,
    pub date: String,
    pub changes: Vec<String>,
}

// ── Public API ──

/// Fetch and parse HelionCoder releases from GitHub.
/// Results are cached for 10 minutes.
pub async fn get_changelog() -> Result<Vec<ChangelogEntry>, String> {
    // Check cache
    {
        let cache = CACHE.lock().await;
        if let Some(ref entry) = *cache {
            if entry.fetched_at.elapsed() < CACHE_TTL {
                log::debug!(
                    "[changelog] cache hit: {} entries, age={:.0}s",
                    entry.entries.len(),
                    entry.fetched_at.elapsed().as_secs_f64()
                );
                return Ok(entry.entries.clone());
            }
        }
    }

    // Fetch from GitHub
    log::debug!("[changelog] fetching from {}", RELEASES_API_URL);
    let resp = CLIENT
        .get(RELEASES_API_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch changelog: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Changelog fetch failed: HTTP {}", resp.status()));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read changelog body: {}", e))?;

    log::debug!("[changelog] fetched {} bytes, parsing", text.len());
    let releases: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Failed to parse releases: {}", e))?;
    let entries = parse_releases(&releases);
    log::debug!("[changelog] parsed {} version entries", entries.len());

    // Update cache
    {
        let mut cache = CACHE.lock().await;
        *cache = Some(CacheEntry {
            entries: entries.clone(),
            fetched_at: Instant::now(),
        });
    }

    Ok(entries)
}

// ── Parser ──

fn release_version(release: &serde_json::Value) -> Option<String> {
    release
        .get("tag_name")
        .and_then(|v| v.as_str())
        .map(|tag| tag.strip_prefix('v').unwrap_or(tag).trim().to_string())
        .filter(|tag| !tag.is_empty())
}

fn release_date(release: &serde_json::Value) -> String {
    release
        .get("published_at")
        .or_else(|| release.get("created_at"))
        .and_then(|v| v.as_str())
        .and_then(|s| s.split('T').next())
        .unwrap_or("")
        .to_string()
}

fn parse_release_body(body: &str) -> Vec<String> {
    let mut changes = Vec::new();
    for line in body.lines() {
        let trimmed = line
            .trim()
            .trim_start_matches(['-', '*'])
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')
            .trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.eq_ignore_ascii_case("changes")
            || trimmed.eq_ignore_ascii_case("changelog")
        {
            continue;
        }
        changes.push(trimmed.to_string());
    }
    changes
}

fn parse_releases(body: &serde_json::Value) -> Vec<ChangelogEntry> {
    let Some(releases) = body.as_array() else {
        return Vec::new();
    };

    releases
        .iter()
        .filter(|release| !release["draft"].as_bool().unwrap_or(false))
        .filter_map(|release| {
            let version = release_version(release)?;
            let date = release_date(release);
            let body = release.get("body").and_then(|v| v.as_str()).unwrap_or("");
            let mut changes = parse_release_body(body);
            if changes.is_empty() {
                let name = release
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|name| !name.is_empty() && *name != version);
                changes.push(name.unwrap_or("Release published").to_string());
            }
            Some(ChangelogEntry {
                version,
                date,
                changes,
            })
        })
        .collect()
}

#[cfg(test)]
/// Parse CHANGELOG.md format: `## X.Y.Z - Date\n\n- Change 1\n- Change 2`
fn parse_changelog(text: &str) -> Vec<ChangelogEntry> {
    let mut entries: Vec<ChangelogEntry> = Vec::new();
    let mut current_version = String::new();
    let mut current_date = String::new();
    let mut current_changes: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();

        // Version header: "## X.Y.Z" or "## X.Y.Z - Date"
        if trimmed.starts_with("## ") {
            // Save previous entry
            if !current_version.is_empty() && !current_changes.is_empty() {
                entries.push(ChangelogEntry {
                    version: current_version.clone(),
                    date: current_date.clone(),
                    changes: current_changes.clone(),
                });
            }

            let header = trimmed.trim_start_matches("## ").trim();
            if let Some(dash_pos) = header.find(" - ") {
                current_version = header[..dash_pos].trim().to_string();
                current_date = header[dash_pos + 3..].trim().to_string();
            } else {
                current_version = header.to_string();
                current_date = String::new();
            }
            current_changes = Vec::new();
        } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            // Change item
            let change = trimmed[2..].trim().to_string();
            if !change.is_empty() {
                current_changes.push(change);
            }
        }
        // Skip blank lines, title ("# Changelog"), etc.
    }

    // Don't forget the last entry
    if !current_version.is_empty() && !current_changes.is_empty() {
        entries.push(ChangelogEntry {
            version: current_version,
            date: current_date,
            changes: current_changes,
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_changelog() {
        let md = r#"# Changelog

## 2.1.42 - 2026-02-13

- Fix bug in session handling
- Add new feature X

## 2.1.41 - 2026-02-10

- Improve performance
- Fix crash on startup
"#;
        let entries = parse_changelog(md);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].version, "2.1.42");
        assert_eq!(entries[0].date, "2026-02-13");
        assert_eq!(entries[0].changes.len(), 2);
        assert_eq!(entries[1].version, "2.1.41");
    }

    #[test]
    fn test_parse_no_date() {
        let md = "## 1.0.0\n\n- Initial release\n";
        let entries = parse_changelog(md);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].version, "1.0.0");
        assert_eq!(entries[0].date, "");
    }
}
