use crate::models::{
    HelionDailyStats, HelionModelStats, HelionUsageStats, ModelUsageSummary, RawRunUsage, RunMeta,
};
use rusqlite::{params, Connection};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

#[derive(Default)]
struct RunEditStats {
    additions: u64,
    deletions: u64,
}

#[derive(Default)]
struct ModelBuilder {
    sessions: u32,
    messages: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    cost_usd: f64,
    additions: u64,
    deletions: u64,
}

#[derive(Default)]
struct DailyBuilder {
    sessions: u32,
    messages: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    cost_usd: f64,
    additions: u64,
    deletions: u64,
}

pub fn db_path() -> Result<PathBuf, String> {
    let home =
        super::dirs_next().ok_or_else(|| "Could not determine home directory".to_string())?;
    let dir = home.join(".helioncoder");
    super::ensure_dir(&dir).map_err(|e| format!("create {} failed: {e}", dir.display()))?;
    Ok(dir.join("helioncoder.sqlite"))
}

fn open_db() -> Result<Connection, String> {
    let path = db_path()?;
    let conn =
        Connection::open(&path).map_err(|e| format!("open {} failed: {e}", path.display()))?;
    init_schema(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS usage_sessions (
          run_id TEXT PRIMARY KEY,
          started_at TEXT NOT NULL,
          last_activity_at TEXT,
          model TEXT,
          messages INTEGER NOT NULL DEFAULT 0,
          input_tokens INTEGER NOT NULL DEFAULT 0,
          output_tokens INTEGER NOT NULL DEFAULT 0,
          cache_read_tokens INTEGER NOT NULL DEFAULT 0,
          cache_write_tokens INTEGER NOT NULL DEFAULT 0,
          total_tokens INTEGER NOT NULL DEFAULT 0,
          cost_usd REAL NOT NULL DEFAULT 0,
          additions INTEGER NOT NULL DEFAULT 0,
          deletions INTEGER NOT NULL DEFAULT 0,
          updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS usage_daily (
          day TEXT PRIMARY KEY,
          sessions INTEGER NOT NULL DEFAULT 0,
          messages INTEGER NOT NULL DEFAULT 0,
          input_tokens INTEGER NOT NULL DEFAULT 0,
          output_tokens INTEGER NOT NULL DEFAULT 0,
          cache_read_tokens INTEGER NOT NULL DEFAULT 0,
          cache_write_tokens INTEGER NOT NULL DEFAULT 0,
          total_tokens INTEGER NOT NULL DEFAULT 0,
          cost_usd REAL NOT NULL DEFAULT 0,
          additions INTEGER NOT NULL DEFAULT 0,
          deletions INTEGER NOT NULL DEFAULT 0,
          updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS usage_models (
          model TEXT PRIMARY KEY,
          sessions INTEGER NOT NULL DEFAULT 0,
          messages INTEGER NOT NULL DEFAULT 0,
          input_tokens INTEGER NOT NULL DEFAULT 0,
          output_tokens INTEGER NOT NULL DEFAULT 0,
          cache_read_tokens INTEGER NOT NULL DEFAULT 0,
          cache_write_tokens INTEGER NOT NULL DEFAULT 0,
          total_tokens INTEGER NOT NULL DEFAULT 0,
          cost_usd REAL NOT NULL DEFAULT 0,
          additions INTEGER NOT NULL DEFAULT 0,
          deletions INTEGER NOT NULL DEFAULT 0,
          updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS app_profile (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| format!("init helion usage schema failed: {e}"))?;
    Ok(())
}

fn run_events_path(run_id: &str) -> PathBuf {
    super::run_dir(run_id).join("events.jsonl")
}

fn parse_started_date_utc(started_at: &str) -> Option<chrono::NaiveDate> {
    chrono::DateTime::parse_from_rfc3339(started_at)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc).date_naive())
        .or_else(|| {
            started_at
                .get(..10)
                .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        })
}

fn parse_started_hour_local(started_at: &str) -> Option<u8> {
    use chrono::Timelike;
    chrono::DateTime::parse_from_rfc3339(started_at)
        .ok()
        .map(|dt| dt.hour() as u8)
}

fn value_u64(value: &serde_json::Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|v| v.as_u64())
}

fn count_lines(text: &str) -> u64 {
    if text.is_empty() {
        0
    } else if text.ends_with('\n') {
        text.split('\n').count().saturating_sub(1) as u64
    } else {
        text.split('\n').count() as u64
    }
}

fn count_patch_lines(value: &serde_json::Value) -> RunEditStats {
    let mut stats = RunEditStats::default();
    let Some(hunks) = value.as_array() else {
        return stats;
    };
    for hunk in hunks {
        let Some(lines) = hunk.get("lines").and_then(|v| v.as_array()) else {
            continue;
        };
        for line in lines {
            let Some(line) = line.as_str() else {
                continue;
            };
            if line.starts_with('+') {
                stats.additions += 1;
            } else if line.starts_with('-') {
                stats.deletions += 1;
            }
        }
    }
    stats
}

fn edit_stats_from_tool_event(event: &serde_json::Value) -> RunEditStats {
    let tool_name = event
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if !matches!(
        tool_name,
        "Edit" | "edit_file" | "Write" | "write_file" | "NotebookEdit"
    ) {
        return RunEditStats::default();
    }

    let input = event.get("input").unwrap_or(&serde_json::Value::Null);
    let result = event
        .get("tool_use_result")
        .unwrap_or(&serde_json::Value::Null);
    let mut stats = RunEditStats {
        additions: value_u64(result, "_patchAdded").unwrap_or(0),
        deletions: value_u64(result, "_patchRemoved").unwrap_or(0),
    };

    if stats.additions == 0 && stats.deletions == 0 {
        let patch_stats = count_patch_lines(
            result
                .get("structuredPatch")
                .unwrap_or(&serde_json::Value::Null),
        );
        stats.additions += patch_stats.additions;
        stats.deletions += patch_stats.deletions;
    }

    if stats.additions == 0 && stats.deletions == 0 {
        if let Some(content) = input.get("content").and_then(|v| v.as_str()) {
            stats.additions += count_lines(content);
        }
        if let Some(old_string) = input.get("old_string").and_then(|v| v.as_str()) {
            stats.deletions += count_lines(old_string);
        }
        if let Some(new_string) = input.get("new_string").and_then(|v| v.as_str()) {
            stats.additions += count_lines(new_string);
        }
    }

    stats
}

fn count_messages_and_edits(run_id: &str) -> (u64, RunEditStats) {
    let path = run_events_path(run_id);
    let Ok(file) = fs::File::open(path) else {
        return (0, RunEditStats::default());
    };
    let reader = BufReader::new(file);
    let mut messages = 0u64;
    let mut edits = RunEditStats::default();

    for line in reader.lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let is_message = line.contains("\"user_message\"")
            || line.contains("\"message_complete\"")
            || line.contains("\"type\":\"user\"")
            || line.contains("\"type\":\"assistant\"");
        if is_message {
            messages += 1;
        }
        if !line.contains("\"tool_end\"") {
            continue;
        }
        let Ok(envelope) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let event = if envelope.get("_bus").and_then(|v| v.as_bool()) == Some(true) {
            envelope.get("event").unwrap_or(&serde_json::Value::Null)
        } else {
            &envelope
        };
        if event.get("type").and_then(|v| v.as_str()) != Some("tool_end") {
            continue;
        }
        let tool_stats = edit_stats_from_tool_event(event);
        edits.additions += tool_stats.additions;
        edits.deletions += tool_stats.deletions;
    }

    (messages, edits)
}

fn fallback_model(meta: &RunMeta, usage: Option<&RawRunUsage>) -> String {
    meta.model
        .clone()
        .or_else(|| {
            usage.and_then(|u| {
                u.model_usage
                    .iter()
                    .max_by(|a, b| {
                        let at = a.1.input_tokens + a.1.output_tokens;
                        let bt = b.1.input_tokens + b.1.output_tokens;
                        at.cmp(&bt)
                    })
                    .map(|(model, _)| model.clone())
            })
        })
        .unwrap_or_else(|| "Unknown".to_string())
}

fn add_model_usage(
    map: &mut HashMap<String, ModelBuilder>,
    model: &str,
    usage: Option<&ModelUsageSummary>,
    fallback_usage: Option<&RawRunUsage>,
    messages: u64,
    edits: &RunEditStats,
    cost_usd: f64,
) {
    let builder = map.entry(model.to_string()).or_default();
    builder.sessions += 1;
    builder.messages += messages;
    builder.additions += edits.additions;
    builder.deletions += edits.deletions;
    builder.cost_usd += usage.map(|u| u.cost_usd).unwrap_or(cost_usd);
    if let Some(u) = usage {
        builder.input_tokens += u.input_tokens;
        builder.output_tokens += u.output_tokens;
        builder.cache_read_tokens += u.cache_read_tokens;
        builder.cache_write_tokens += u.cache_write_tokens;
    } else if let Some(u) = fallback_usage {
        builder.input_tokens += u.input_tokens;
        builder.output_tokens += u.output_tokens;
        builder.cache_read_tokens += u.cache_read_tokens;
        builder.cache_write_tokens += u.cache_write_tokens;
    }
}

pub fn refresh_and_read() -> Result<HelionUsageStats, String> {
    let db_path = db_path()?;
    let conn = open_db()?;
    let metas = super::runs::list_all_run_metas();
    let now = crate::models::now_iso();

    let mut total_sessions = 0u32;
    let mut total_messages = 0u64;
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut cache_read_tokens = 0u64;
    let mut cache_write_tokens = 0u64;
    let mut total_cost_usd = 0.0;
    let mut total_additions = 0u64;
    let mut total_deletions = 0u64;
    let mut daily_map: BTreeMap<String, DailyBuilder> = BTreeMap::new();
    let mut model_map: HashMap<String, ModelBuilder> = HashMap::new();
    let mut hour_counts: [u32; 24] = [0; 24];

    conn.execute("DELETE FROM usage_sessions", [])
        .map_err(|e| format!("clear usage_sessions failed: {e}"))?;

    for meta in metas {
        total_sessions += 1;
        let usage = super::events::extract_run_usage(&meta.id);
        let (messages, edits) = count_messages_and_edits(&meta.id);
        let run_input = usage.as_ref().map(|u| u.input_tokens).unwrap_or(0);
        let run_output = usage.as_ref().map(|u| u.output_tokens).unwrap_or(0);
        let run_cache_read = usage.as_ref().map(|u| u.cache_read_tokens).unwrap_or(0);
        let run_cache_write = usage.as_ref().map(|u| u.cache_write_tokens).unwrap_or(0);
        let run_cost = usage.as_ref().map(|u| u.total_cost_usd).unwrap_or(0.0);
        let model = fallback_model(&meta, usage.as_ref());
        let total_tokens = run_input + run_output + run_cache_read + run_cache_write;

        total_messages += messages;
        input_tokens += run_input;
        output_tokens += run_output;
        cache_read_tokens += run_cache_read;
        cache_write_tokens += run_cache_write;
        total_cost_usd += run_cost;
        total_additions += edits.additions;
        total_deletions += edits.deletions;

        if let Some(hour) = parse_started_hour_local(&meta.started_at) {
            hour_counts[hour as usize] += 1;
        }

        if let Some(date) = parse_started_date_utc(&meta.started_at) {
            let day = daily_map
                .entry(date.format("%Y-%m-%d").to_string())
                .or_default();
            day.sessions += 1;
            day.messages += messages;
            day.input_tokens += run_input;
            day.output_tokens += run_output;
            day.cache_read_tokens += run_cache_read;
            day.cache_write_tokens += run_cache_write;
            day.cost_usd += run_cost;
            day.additions += edits.additions;
            day.deletions += edits.deletions;
        }

        if let Some(ref usage) = usage {
            if usage.model_usage.is_empty() {
                add_model_usage(
                    &mut model_map,
                    &model,
                    None,
                    Some(usage),
                    messages,
                    &edits,
                    run_cost,
                );
            } else {
                for (model_name, model_usage) in &usage.model_usage {
                    add_model_usage(
                        &mut model_map,
                        model_name,
                        Some(model_usage),
                        None,
                        messages,
                        &edits,
                        0.0,
                    );
                }
            }
        } else {
            add_model_usage(
                &mut model_map,
                &model,
                None,
                None,
                messages,
                &edits,
                run_cost,
            );
        }

        conn.execute(
            r#"
            INSERT OR REPLACE INTO usage_sessions
              (run_id, started_at, last_activity_at, model, messages, input_tokens,
               output_tokens, cache_read_tokens, cache_write_tokens, total_tokens,
               cost_usd, additions, deletions, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
            params![
                meta.id,
                meta.started_at,
                meta.ended_at,
                model,
                messages as i64,
                run_input as i64,
                run_output as i64,
                run_cache_read as i64,
                run_cache_write as i64,
                total_tokens as i64,
                run_cost,
                edits.additions as i64,
                edits.deletions as i64,
                now,
            ],
        )
        .map_err(|e| format!("upsert usage_session failed: {e}"))?;
    }

    conn.execute("DELETE FROM usage_daily", [])
        .map_err(|e| format!("clear usage_daily failed: {e}"))?;
    for (day, stats) in &daily_map {
        conn.execute(
            r#"
            INSERT OR REPLACE INTO usage_daily
              (day, sessions, messages, input_tokens, output_tokens, cache_read_tokens,
               cache_write_tokens, total_tokens, cost_usd, additions, deletions, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                day,
                stats.sessions as i64,
                stats.messages as i64,
                stats.input_tokens as i64,
                stats.output_tokens as i64,
                stats.cache_read_tokens as i64,
                stats.cache_write_tokens as i64,
                (stats.input_tokens
                    + stats.output_tokens
                    + stats.cache_read_tokens
                    + stats.cache_write_tokens) as i64,
                stats.cost_usd,
                stats.additions as i64,
                stats.deletions as i64,
                now,
            ],
        )
        .map_err(|e| format!("upsert usage_daily failed: {e}"))?;
    }

    conn.execute("DELETE FROM usage_models", [])
        .map_err(|e| format!("clear usage_models failed: {e}"))?;
    let mut by_model: Vec<HelionModelStats> = model_map
        .into_iter()
        .map(|(model, stats)| HelionModelStats {
            model,
            sessions: stats.sessions,
            messages: stats.messages,
            input_tokens: stats.input_tokens,
            output_tokens: stats.output_tokens,
            cache_read_tokens: stats.cache_read_tokens,
            cache_write_tokens: stats.cache_write_tokens,
            total_tokens: stats.input_tokens
                + stats.output_tokens
                + stats.cache_read_tokens
                + stats.cache_write_tokens,
            cost_usd: stats.cost_usd,
            additions: stats.additions,
            deletions: stats.deletions,
        })
        .collect();
    by_model.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
    for stats in &by_model {
        conn.execute(
            r#"
            INSERT OR REPLACE INTO usage_models
              (model, sessions, messages, input_tokens, output_tokens, cache_read_tokens,
               cache_write_tokens, total_tokens, cost_usd, additions, deletions, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                stats.model,
                stats.sessions as i64,
                stats.messages as i64,
                stats.input_tokens as i64,
                stats.output_tokens as i64,
                stats.cache_read_tokens as i64,
                stats.cache_write_tokens as i64,
                stats.total_tokens as i64,
                stats.cost_usd,
                stats.additions as i64,
                stats.deletions as i64,
                now,
            ],
        )
        .map_err(|e| format!("upsert usage_models failed: {e}"))?;
    }

    let daily: Vec<HelionDailyStats> = daily_map
        .into_iter()
        .map(|(date, stats)| HelionDailyStats {
            date,
            sessions: stats.sessions,
            messages: stats.messages,
            input_tokens: stats.input_tokens,
            output_tokens: stats.output_tokens,
            cache_read_tokens: stats.cache_read_tokens,
            cache_write_tokens: stats.cache_write_tokens,
            total_tokens: stats.input_tokens
                + stats.output_tokens
                + stats.cache_read_tokens
                + stats.cache_write_tokens,
            cost_usd: stats.cost_usd,
            additions: stats.additions,
            deletions: stats.deletions,
        })
        .collect();

    let (active_days, current_streak, longest_streak) = compute_streaks(&daily);
    let peak_hour = hour_counts
        .iter()
        .enumerate()
        .max_by_key(|(_, count)| *count)
        .and_then(|(hour, count)| if *count > 0 { Some(hour as u8) } else { None });
    let favorite_model = by_model.first().map(|m| m.model.clone());

    Ok(HelionUsageStats {
        total_sessions,
        total_messages,
        total_tokens: input_tokens + output_tokens + cache_read_tokens + cache_write_tokens,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        total_cost_usd,
        active_days,
        current_streak,
        longest_streak,
        peak_hour,
        favorite_model,
        total_additions,
        total_deletions,
        by_model,
        daily,
        sqlite_path: db_path.display().to_string(),
    })
}

fn compute_streaks(daily: &[HelionDailyStats]) -> (u32, u32, u32) {
    let active: std::collections::BTreeSet<chrono::NaiveDate> = daily
        .iter()
        .filter(|d| d.sessions > 0 || d.messages > 0 || d.total_tokens > 0)
        .filter_map(|d| chrono::NaiveDate::parse_from_str(&d.date, "%Y-%m-%d").ok())
        .collect();
    if active.is_empty() {
        return (0, 0, 0);
    }

    let mut longest = 0u32;
    let mut current_run = 0u32;
    let mut prev: Option<chrono::NaiveDate> = None;
    for day in &active {
        if prev.map(|p| *day == p + chrono::Duration::days(1)) == Some(true) {
            current_run += 1;
        } else {
            current_run = 1;
        }
        longest = longest.max(current_run);
        prev = Some(*day);
    }

    let today = chrono::Utc::now().date_naive();
    let mut cursor = today;
    let mut current = 0u32;
    while active.contains(&cursor) {
        current += 1;
        cursor -= chrono::Duration::days(1);
    }

    (active.len() as u32, current, longest)
}
