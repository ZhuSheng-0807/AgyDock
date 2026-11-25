use chrono::{DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, TimeZone, Timelike, Utc};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

const TRANSCRIPT_CANDIDATES: [&str; 2] = [
    ".system_generated/logs/transcript.jsonl",
    ".system_generated/logs/transcript_full.jsonl",
];

fn re_user_request() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<USER_REQUEST>([\s\S]*?)</USER_REQUEST>").unwrap())
}

fn re_html_tags() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<[^>]+>").unwrap())
}

fn re_whitespace() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s+").unwrap())
}

fn re_sys_content() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)content=([\s\S]*?)(?:</SYSTEM_MESSAGE>|$)").unwrap())
}

fn re_sys_sender() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)sender=([a-z0-9_-]{8,})").unwrap())
}

fn re_setting_model() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"changed setting `Model Selection` from .*? to ([^.\r\n]+)").unwrap()
    })
}

fn re_conversation_id() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?i)"conversationId"\s*:\s*"([a-z0-9_-]+)""#).unwrap())
}

fn re_manage_subagents() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)\{"role":"([^"]+)","type":"([^"]+)","conversationId":"([a-z0-9_-]+)""#)
            .unwrap()
    })
}

fn re_wrapped_system_blocks() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?is)<t3_code_instructions>[\s\S]*?(?:</t3_code_instructions>|$)|<runtime_info>[\s\S]*?(?:</runtime_info>|$)",
        )
        .unwrap()
    })
}

#[derive(Debug, Clone)]
pub struct BrainSourceSpec {
    pub id: String,
    pub label: String,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSourceInfo {
    pub id: String,
    pub label: String,
    pub dirs: Vec<String>,
    pub exists: bool,
    pub session_count: usize,
}

pub fn infer_source_from_dir(dir: &Path) -> (String, String) {
    let norm = dir
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase();
    if norm.contains("/antigravity-acp/") || norm.ends_with("/antigravity-acp/brain") {
        ("t3".to_string(), "T3 Code".to_string())
    } else if norm.contains("/antigravity-cli/") || norm.ends_with("/antigravity-cli/brain") {
        ("cli".to_string(), "CLI".to_string())
    } else if norm.contains("/.gemini/antigravity/") || norm.ends_with("/antigravity/brain") {
        ("app".to_string(), "Antigravity".to_string())
    } else {
        ("custom".to_string(), "自定义目录".to_string())
    }
}

pub fn is_antigravity_installed() -> bool {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(local_app) = dirs::data_local_dir() {
        candidates.push(local_app.join("Programs").join("antigravity").join("Antigravity.exe"));
        candidates.push(local_app.join("Programs").join("Antigravity").join("Antigravity.exe"));
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        candidates.push(PathBuf::from(&pf).join("Antigravity").join("Antigravity.exe"));
    }
    #[cfg(target_os = "macos")]
    candidates.push(PathBuf::from("/Applications/Antigravity.app"));

    candidates.into_iter().any(|p| p.exists())
}

pub fn is_t3_code_installed() -> bool {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(local_app) = dirs::data_local_dir() {
        candidates.push(
            local_app
                .join("Programs")
                .join("t3code")
                .join("T3 Code (Beta).exe"),
        );
        candidates.push(local_app.join("Programs").join("t3code").join("T3 Code.exe"));
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        candidates.push(PathBuf::from(&pf).join("T3 Code").join("T3 Code.exe"));
        candidates.push(PathBuf::from(&pf).join("T3 Code").join("T3 Code (Beta).exe"));
    }
    candidates.into_iter().any(|p| p.is_file())
}

pub fn is_agy_cli_installed() -> bool {
    let shim_dir_lower = dirs::data_local_dir()
        .or_else(dirs::config_dir)
        .map(|d| d.join("AgyDock").join("cli-shim").to_string_lossy().to_lowercase());

    if let Some(path_env) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_env) {
            let dir_lower = dir.to_string_lossy().to_lowercase();
            if shim_dir_lower
                .as_deref()
                .map(|s| dir_lower.trim_end_matches(['\\', '/']) == s.trim_end_matches(['\\', '/']))
                .unwrap_or(false)
            {
                continue;
            }
            if dir.join("agy.exe").is_file()
                || dir.join("antigravity-cli.exe").is_file()
                || dir.join("agy").is_file()
            {
                return true;
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        if home.join(".local").join("bin").join("agy.exe").is_file()
            || home.join(".local").join("bin").join("agy").is_file()
            || home.join(".antigravity").join("bin").join("agy.exe").is_file()
            || home.join(".antigravity").join("bin").join("agy").is_file()
        {
            return true;
        }
    }

    if let Some(local_app) = dirs::data_local_dir() {
        if local_app
            .join("Programs")
            .join("antigravity-cli")
            .join("agy.exe")
            .is_file()
            || local_app.join("antigravity").join("agy.exe").is_file()
        {
            return true;
        }
    }

    false
}

pub fn discover_brain_sources_from_home_filtered(
    home: Option<&Path>,
    include_app: bool,
    include_cli: bool,
    include_t3: bool,
) -> Vec<BrainSourceSpec> {
    let mut sources = Vec::new();
    let Some(h) = home else {
        if include_app {
            sources.push(BrainSourceSpec {
                id: "app".to_string(),
                label: "Antigravity".to_string(),
                dir: PathBuf::from(".gemini").join("antigravity").join("brain"),
            });
        }
        if include_cli {
            sources.push(BrainSourceSpec {
                id: "cli".to_string(),
                label: "CLI".to_string(),
                dir: PathBuf::from(".gemini").join("antigravity-cli").join("brain"),
            });
        }
        return sources;
    };

    if include_app {
        sources.push(BrainSourceSpec {
            id: "app".to_string(),
            label: "Antigravity".to_string(),
            dir: h.join(".gemini").join("antigravity").join("brain"),
        });
    }
    if include_cli {
        sources.push(BrainSourceSpec {
            id: "cli".to_string(),
            label: "CLI".to_string(),
            dir: h.join(".gemini").join("antigravity-cli").join("brain"),
        });
    }

    if include_t3 {
        let t3_root = h
            .join(".t3")
            .join("userdata")
            .join("providers")
            .join("antigravity");
        if let Ok(entries) = fs::read_dir(&t3_root) {
            let mut subdirs: Vec<PathBuf> = entries
                .flatten()
                .filter_map(|e| {
                    let p = e.path();
                    if p.is_dir() {
                        Some(p)
                    } else {
                        None
                    }
                })
                .collect();
            subdirs.sort();
            for sub in subdirs {
                let acp_brain = sub.join("antigravity-acp").join("brain");
                if acp_brain.is_dir() {
                    sources.push(BrainSourceSpec {
                        id: "t3".to_string(),
                        label: "T3 Code".to_string(),
                        dir: acp_brain,
                    });
                }
            }
        }
    }

    sources
}

#[cfg(test)]
pub fn discover_brain_sources_from_home(home: Option<&Path>) -> Vec<BrainSourceSpec> {
    discover_brain_sources_from_home_filtered(home, true, true, true)
}

pub fn discover_default_brain_sources() -> Vec<BrainSourceSpec> {
    if let Ok(env_dir) = std::env::var("ANTIGRAVITY_BRAIN_DIR") {
        let trimmed = env_dir.trim();
        if !trimmed.is_empty() {
            let dir = PathBuf::from(trimmed);
            let (id, label) = infer_source_from_dir(&dir);
            return vec![BrainSourceSpec { id, label, dir }];
        }
    }
    let home = dirs::home_dir();
    let app_installed = is_antigravity_installed();
    let cli_installed = is_agy_cli_installed();
    let t3_installed = is_t3_code_installed();

    let mut sources = discover_brain_sources_from_home_filtered(
        home.as_deref(),
        app_installed,
        cli_installed,
        t3_installed,
    );

    // Fallback: if no known install path matched (e.g., portable Antigravity install),
    // still scan Antigravity's default brain dir if present.
    if sources.is_empty() {
        let fallback = discover_brain_sources_from_home_filtered(home.as_deref(), true, false, false);
        if fallback.iter().any(|s| s.dir.is_dir()) {
            sources = fallback;
        }
    }

    sources
}

pub fn get_default_brain_dir() -> PathBuf {
    if let Ok(env_dir) = std::env::var("ANTIGRAVITY_BRAIN_DIR") {
        if !env_dir.trim().is_empty() {
            return PathBuf::from(env_dir);
        }
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".gemini").join("antigravity").join("brain");
    }
    PathBuf::from(".gemini").join("antigravity").join("brain")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn system_time_to_ms(st: SystemTime) -> i64 {
    st.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut iter = s.chars();
    let collected: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        let shortened: String = collected.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{shortened}...")
    } else {
        collected
    }
}

fn extract_user_title(content: &str) -> String {
    if content.trim().is_empty() {
        return String::new();
    }
    let inner = re_user_request()
        .captures(content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
        .unwrap_or(content);
    let unwrapped = re_wrapped_system_blocks().replace_all(inner, " ");
    let no_tags = re_html_tags().replace_all(&unwrapped, " ");
    let collapsed = re_whitespace().replace_all(&no_tags, " ");
    truncate_chars(collapsed.trim(), 84)
}

fn extract_system_prompt_preview(content: &str) -> String {
    if content.trim().is_empty() {
        return String::new();
    }
    let inner = re_sys_content()
        .captures(content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
        .unwrap_or(content);
    let no_tags = re_html_tags().replace_all(inner, " ");
    let collapsed = re_whitespace().replace_all(&no_tags, " ");
    truncate_chars(collapsed.trim(), 84)
}

fn normalize_setting_model(raw: &str) -> Option<String> {
    let cleaned = raw.trim().trim_end_matches('.');
    if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("none") {
        return None;
    }
    let mapped = match cleaned {
        "Gemini 4 Carbon (High)" => "gemini-4-carbon-high",
        "Gemini 4 Neon (High)" => "gemini-4-neon-high",
        "Gemini 4 Hydrogen (High)" => "gemini-4-hydrogen-tiered-high",
        other => other,
    };
    Some(mapped.to_string())
}

fn unquote_arg(val: Option<&Value>) -> Option<String> {
    let s = val?.as_str()?.trim();
    let unquoted = if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        s[1..s.len() - 1].trim()
    } else {
        s
    };
    if unquoted.is_empty() {
        None
    } else {
        Some(unquoted.to_string())
    }
}

fn value_to_u64(val: Option<&Value>) -> u64 {
    match val {
        Some(Value::Number(n)) => n
            .as_u64()
            .or_else(|| n.as_i64().filter(|&v| v > 0).map(|v| v as u64))
            .or_else(|| n.as_f64().filter(|&v| v.is_finite() && v > 0.0).map(|v| v as u64))
            .unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse::<u64>().unwrap_or(0),
        _ => 0,
    }
}

fn parse_iso_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub step_index: i64,
    pub ts: i64,
    #[serde(skip_serializing)]
    pub raw_model: Option<String>,
    pub model: String,
    pub status: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub tool_count: u64,
    pub gen_duration_ms: Option<i64>,
    pub tokens_per_sec: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct SpawnedChild {
    pub child_id: String,
    pub parent_id: String,
    pub parent_model: Option<String>,
    pub requested_model: Option<String>,
    pub role: Option<String>,
    pub subagent_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ParsedSession {
    pub id: String,
    pub source: String,
    pub source_label: String,
    pub title: String,
    pub has_user_input: bool,
    pub system_sender_id: Option<String>,
    pub system_prompt_preview: String,
    pub started_at: Option<i64>,
    pub last_at: Option<i64>,
    pub setting_model: Option<String>,
    pub spawned_children: Vec<SpawnedChild>,
    pub is_subagent: bool,
    pub parent_id: Option<String>,
    pub root_parent_id: Option<String>,
    pub parent_title: Option<String>,
    pub subagent_role: Option<String>,
    pub subagent_type: Option<String>,
    pub events: Vec<SessionEvent>,
}

#[derive(Debug, Clone)]
struct PendingInvoke {
    requested_model: Option<String>,
    role: Option<String>,
    subagent_type: Option<String>,
}

struct PendingSpeed {
    event_idx: usize,
    start_ts: i64,
    output_tokens: u64,
    remaining_tools: usize,
    last_tool_ts: Option<i64>,
}

fn finalize_pending_speed(pending: Option<PendingSpeed>, events: &mut [SessionEvent]) {
    let Some(p) = pending else {
        return;
    };
    let Some(end_ts) = p.last_tool_ts else {
        return;
    };
    let duration_ms = end_ts - p.start_ts;
    if !(1_000..=600_000).contains(&duration_ms) || p.output_tokens == 0 {
        return;
    }
    let tps = (p.output_tokens as f64) / (duration_ms as f64 / 1000.0);
    if !(1.0..=1200.0).contains(&tps) {
        return;
    }
    if let Some(ev) = events.get_mut(p.event_idx) {
        ev.gen_duration_ms = Some(duration_ms);
        ev.tokens_per_sec = Some((tps * 10.0).round() / 10.0);
    }
}

pub fn parse_transcript_text(text: &str, session_id: &str) -> ParsedSession {
    let mut title = String::new();
    let mut has_user_input = false;
    let mut system_sender_id: Option<String> = None;
    let mut system_prompt_preview = String::new();
    let mut events = Vec::new();
    let mut spawned_children = Vec::new();
    let mut first_ts: Option<i64> = None;
    let mut last_ts: Option<i64> = None;
    let mut current_setting_model: Option<String> = None;
    let mut last_explicit_model: Option<String> = None;
    let mut pending_invokes: VecDeque<PendingInvoke> = VecDeque::new();
    let mut pending_speed: Option<PendingSpeed> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let entry: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let ts_opt = entry
            .get("created_at")
            .and_then(|v| v.as_str())
            .and_then(parse_iso_ms);
        if let Some(ts) = ts_opt {
            first_ts = Some(first_ts.map_or(ts, |cur| cur.min(ts)));
            last_ts = Some(last_ts.map_or(ts, |cur| cur.max(ts)));
        }

        let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(mut p) = pending_speed.take() {
            if matches!(entry_type, "USER_INPUT" | "SYSTEM_MESSAGE" | "PLANNER_RESPONSE") {
                finalize_pending_speed(Some(p), &mut events);
            } else {
                if let Some(tk) = ts_opt {
                    p.last_tool_ts = Some(p.last_tool_ts.map_or(tk, |cur| cur.max(tk)));
                }
                p.remaining_tools = p.remaining_tools.saturating_sub(1);
                if p.remaining_tools == 0 {
                    finalize_pending_speed(Some(p), &mut events);
                } else {
                    pending_speed = Some(p);
                }
            }
        }
        if entry_type == "USER_INPUT" {
            has_user_input = true;
        }

        if let Some(content) = entry.get("content").and_then(|v| v.as_str()) {
            if title.is_empty() && entry_type == "USER_INPUT" {
                title = extract_user_title(content);
            }

            if system_sender_id.is_none() && entry_type == "SYSTEM_MESSAGE" {
                if let Some(cap) = re_sys_sender().captures(content) {
                    let sender = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                    if !sender.is_empty() && sender != "system" && sender != session_id {
                        system_sender_id = Some(sender.to_string());
                    }
                }
                if system_prompt_preview.is_empty() {
                    system_prompt_preview = extract_system_prompt_preview(content);
                }
            }

            for cap in re_setting_model().captures_iter(content) {
                if let Some(m) = cap.get(1) {
                    if let Some(norm) = normalize_setting_model(m.as_str()) {
                        current_setting_model = Some(norm);
                    }
                }
            }

            for cap in re_conversation_id().captures_iter(content) {
                if let Some(cm) = cap.get(1) {
                    let child_id = cm.as_str();
                    if !child_id.is_empty() && child_id != session_id {
                        let matched = pending_invokes.pop_front();
                        spawned_children.push(SpawnedChild {
                            child_id: child_id.to_string(),
                            parent_id: session_id.to_string(),
                            parent_model: last_explicit_model
                                .clone()
                                .or_else(|| current_setting_model.clone()),
                            requested_model: matched.as_ref().and_then(|m| m.requested_model.clone()),
                            role: matched.as_ref().and_then(|m| m.role.clone()),
                            subagent_type: matched.as_ref().and_then(|m| m.subagent_type.clone()),
                        });
                    }
                }
            }

            for cap in re_manage_subagents().captures_iter(content) {
                let role = cap.get(1).map(|m| m.as_str().to_string());
                let stype = cap.get(2).map(|m| m.as_str().to_string());
                let child_id = cap.get(3).map(|m| m.as_str()).unwrap_or("");
                if !child_id.is_empty() && child_id != session_id {
                    spawned_children.push(SpawnedChild {
                        child_id: child_id.to_string(),
                        parent_id: session_id.to_string(),
                        parent_model: last_explicit_model
                            .clone()
                            .or_else(|| current_setting_model.clone()),
                        requested_model: None,
                        role,
                        subagent_type: stype,
                    });
                }
            }
        }

        let usage = entry.get("usage").and_then(|v| v.as_object());
        let input_tokens = value_to_u64(
            entry
                .get("input_tokens")
                .or_else(|| entry.get("prompt_tokens"))
                .or_else(|| usage.and_then(|u| u.get("input_tokens")))
                .or_else(|| usage.and_then(|u| u.get("prompt_tokens"))),
        );
        let output_tokens = value_to_u64(
            entry
                .get("output_tokens")
                .or_else(|| entry.get("completion_tokens"))
                .or_else(|| usage.and_then(|u| u.get("output_tokens")))
                .or_else(|| usage.and_then(|u| u.get("completion_tokens"))),
        );
        let cache_read_tokens = value_to_u64(
            entry
                .get("cache_read_tokens")
                .or_else(|| entry.get("cached_tokens"))
                .or_else(|| entry.get("cache_read_input_tokens"))
                .or_else(|| usage.and_then(|u| u.get("cache_read_tokens")))
                .or_else(|| usage.and_then(|u| u.get("cached_tokens"))),
        );
        let cache_write_tokens = value_to_u64(
            entry
                .get("cache_creation_tokens")
                .or_else(|| entry.get("cache_write_tokens"))
                .or_else(|| usage.and_then(|u| u.get("cache_creation_tokens"))),
        );

        let mut tool_count: u64 = 0;
        let mut has_ask_question = false;
        if let Some(tool_calls) = entry.get("tool_calls").and_then(|v| v.as_array()) {
            tool_count = tool_calls.len() as u64;
            for tc in tool_calls {
                let name = tc.get("name").and_then(|v| v.as_str());
                if name == Some("ask_question") {
                    has_ask_question = true;
                }
                if name == Some("invoke_subagent") {
                    let args = tc.get("args");
                    pending_invokes.push_back(PendingInvoke {
                        requested_model: unquote_arg(args.and_then(|a| a.get("Model"))),
                        role: unquote_arg(args.and_then(|a| a.get("Role"))),
                        subagent_type: unquote_arg(args.and_then(|a| a.get("TypeName"))),
                    });
                }
            }
        }

        let has_tokens =
            input_tokens > 0 || output_tokens > 0 || cache_read_tokens > 0 || cache_write_tokens > 0;
        if !has_tokens && entry_type != "PLANNER_RESPONSE" {
            continue;
        }

        let raw_model_str = entry
            .get("model")
            .or_else(|| entry.get("model_name"))
            .or_else(|| entry.get("model_id"))
            .or_else(|| usage.and_then(|u| u.get("model")))
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        if let Some(ref em) = raw_model_str {
            last_explicit_model = Some(em.clone());
        }

        let resolved_local = raw_model_str.or_else(|| current_setting_model.clone());
        let total_tokens = input_tokens + output_tokens + cache_read_tokens + cache_write_tokens;
        let step_index = entry
            .get("step_index")
            .and_then(|v| v.as_i64())
            .unwrap_or(events.len() as i64);
        let status = entry
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("DONE")
            .to_string();

        events.push(SessionEvent {
            step_index,
            ts: ts_opt.or(last_ts).unwrap_or_else(now_ms),
            raw_model: resolved_local.clone(),
            model: resolved_local.unwrap_or_else(|| "unknown".to_string()),
            status,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            total_tokens,
            tool_count,
            gen_duration_ms: None,
            tokens_per_sec: None,
        });

        if entry_type == "PLANNER_RESPONSE" && tool_count > 0 && output_tokens > 0 && !has_ask_question {
            if let Some(start_ts) = ts_opt {
                pending_speed = Some(PendingSpeed {
                    event_idx: events.len() - 1,
                    start_ts,
                    output_tokens,
                    remaining_tools: tool_count as usize,
                    last_tool_ts: None,
                });
            }
        }
    }

    finalize_pending_speed(pending_speed, &mut events);

    let fallback_title = if !title.is_empty() {
        title
    } else if !system_prompt_preview.is_empty() {
        system_prompt_preview.clone()
    } else {
        let short_id: String = session_id.chars().take(8).collect();
        format!("Session {short_id}")
    };

    ParsedSession {
        id: session_id.to_string(),
        source: "app".to_string(),
        source_label: "Antigravity".to_string(),
        title: fallback_title,
        has_user_input,
        system_sender_id,
        system_prompt_preview,
        started_at: first_ts,
        last_at: last_ts,
        setting_model: current_setting_model.or(last_explicit_model),
        spawned_children,
        is_subagent: false,
        parent_id: None,
        root_parent_id: None,
        parent_title: None,
        subagent_role: None,
        subagent_type: None,
        events,
    }
}

pub fn resolve_session_graph(sessions: Vec<ParsedSession>) -> Vec<ParsedSession> {
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for (idx, s) in sessions.iter().enumerate() {
        by_id.insert(s.id.clone(), idx);
    }

    let mut child_to_hint: HashMap<String, SpawnedChild> = HashMap::new();
    for s in &sessions {
        for child in &s.spawned_children {
            child_to_hint
                .entry(child.child_id.clone())
                .and_modify(|existing| {
                    if existing.role.is_none() && child.role.is_some() {
                        existing.role = child.role.clone();
                    }
                    if existing.subagent_type.is_none() && child.subagent_type.is_some() {
                        existing.subagent_type = child.subagent_type.clone();
                    }
                    if existing.requested_model.is_none() && child.requested_model.is_some() {
                        existing.requested_model = child.requested_model.clone();
                    }
                    if existing.parent_model.is_none() && child.parent_model.is_some() {
                        existing.parent_model = child.parent_model.clone();
                    }
                })
                .or_insert_with(|| child.clone());
        }
    }

    for s in &sessions {
        if !s.has_user_input {
            if let Some(ref sender) = s.system_sender_id {
                if !child_to_hint.contains_key(&s.id) {
                    let parent_model = by_id
                        .get(sender)
                        .and_then(|&idx| sessions[idx].setting_model.clone());
                    child_to_hint.insert(
                        s.id.clone(),
                        SpawnedChild {
                            child_id: s.id.clone(),
                            parent_id: sender.clone(),
                            parent_model,
                            requested_model: None,
                            role: None,
                            subagent_type: None,
                        },
                    );
                }
            }
        }
    }

    fn resolve_root_parent_id(
        session_id: &str,
        sessions: &[ParsedSession],
        by_id: &HashMap<String, usize>,
        hints: &HashMap<String, SpawnedChild>,
        depth: usize,
    ) -> Option<String> {
        if depth > 8 {
            return None;
        }
        let hint = hints.get(session_id)?;
        if let Some(&p_idx) = by_id.get(&hint.parent_id) {
            if sessions[p_idx].has_user_input {
                return Some(sessions[p_idx].id.clone());
            }
        }
        resolve_root_parent_id(&hint.parent_id, sessions, by_id, hints, depth + 1)
            .or_else(|| Some(hint.parent_id.clone()))
    }

    fn resolve_model(
        session_id: &str,
        sessions: &[ParsedSession],
        by_id: &HashMap<String, usize>,
        hints: &HashMap<String, SpawnedChild>,
        depth: usize,
    ) -> Option<String> {
        if depth > 6 {
            return None;
        }
        if let Some(&idx) = by_id.get(session_id) {
            if let Some(ref m) = sessions[idx].setting_model {
                return Some(m.clone());
            }
        }
        let hint = hints.get(session_id)?;
        if let Some(ref req) = hint.requested_model {
            if req != "inherit" {
                return Some(req.clone());
            }
        }
        if let Some(ref pm) = hint.parent_model {
            return Some(pm.clone());
        }
        resolve_model(&hint.parent_id, sessions, by_id, hints, depth + 1)
    }

    let mut output = Vec::with_capacity(sessions.len());
    for s in &sessions {
        let hint = child_to_hint.get(&s.id);
        let is_subagent = !s.has_user_input && hint.is_some();
        let parent_id = if is_subagent {
            hint.map(|h| h.parent_id.clone())
        } else {
            None
        };
        let root_parent_id = if is_subagent {
            resolve_root_parent_id(&s.id, &sessions, &by_id, &child_to_hint, 0)
        } else {
            None
        };
        let parent_title = root_parent_id
            .as_ref()
            .and_then(|pid| by_id.get(pid))
            .map(|&idx| sessions[idx].title.clone());
        let subagent_role = if is_subagent {
            hint.and_then(|h| h.role.clone())
        } else {
            None
        };
        let subagent_type = if is_subagent {
            hint.and_then(|h| h.subagent_type.clone())
        } else {
            None
        };

        let mut display_title = s.title.clone();
        if is_subagent {
            if let Some(ref role) = subagent_role {
                display_title = role.clone();
            } else if !s.system_prompt_preview.is_empty() {
                display_title = s.system_prompt_preview.clone();
            }
        }

        let needs_model = s.events.iter().any(|ev| ev.raw_model.is_none());
        let inherited_model = if needs_model {
            resolve_model(&s.id, &sessions, &by_id, &child_to_hint, 0)
        } else {
            None
        };

        let mut cloned = s.clone();
        cloned.title = display_title;
        cloned.is_subagent = is_subagent;
        cloned.parent_id = parent_id;
        cloned.root_parent_id = root_parent_id;
        cloned.parent_title = parent_title;
        cloned.subagent_role = subagent_role;
        cloned.subagent_type = subagent_type;
        if let Some(ref im) = inherited_model {
            for ev in &mut cloned.events {
                if ev.raw_model.is_none() {
                    ev.model = im.clone();
                }
            }
        }
        output.push(cloned);
    }

    output
}

struct CacheEntry {
    mtime_ms: i64,
    size: u64,
    session: ParsedSession,
}

pub struct ScanResult {
    pub brain_dir: String,
    pub is_custom_dir: bool,
    pub sources: Vec<UsageSourceInfo>,
    pub scanned_at: i64,
    pub error: Option<String>,
    pub total_dirs: usize,
    pub parsed_sessions: usize,
    pub skipped_dirs: usize,
    pub sessions: Vec<ParsedSession>,
}

#[derive(Debug, Clone)]
enum ScannerMode {
    Auto,
    Custom(PathBuf),
    #[cfg_attr(not(test), allow(dead_code))]
    Explicit(Vec<BrainSourceSpec>),
}

pub struct BrainScanner {
    mode: ScannerMode,
    cache: HashMap<PathBuf, CacheEntry>,
}

fn canonical_source_order(id: &str) -> u8 {
    match id {
        "app" => 0,
        "cli" => 1,
        "t3" => 2,
        "custom" => 3,
        _ => 4,
    }
}

fn path_dedup_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

impl BrainScanner {
    pub fn new(brain_dir: PathBuf) -> Self {
        Self {
            mode: ScannerMode::Custom(brain_dir),
            cache: HashMap::new(),
        }
    }

    pub fn new_auto() -> Self {
        Self {
            mode: ScannerMode::Auto,
            cache: HashMap::new(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new_sources(sources: Vec<BrainSourceSpec>) -> Self {
        Self {
            mode: ScannerMode::Explicit(sources),
            cache: HashMap::new(),
        }
    }

    pub fn set_dir(&mut self, brain_dir: PathBuf) {
        self.mode = ScannerMode::Custom(brain_dir);
        self.cache.clear();
    }

    pub fn reset_to_auto(&mut self) {
        self.mode = ScannerMode::Auto;
        self.cache.clear();
    }

    pub fn is_custom_dir(&self) -> bool {
        matches!(self.mode, ScannerMode::Custom(_))
    }

    pub fn primary_dir(&self) -> PathBuf {
        match &self.mode {
            ScannerMode::Custom(dir) => dir.clone(),
            ScannerMode::Explicit(list) => list
                .first()
                .map(|s| s.dir.clone())
                .unwrap_or_else(get_default_brain_dir),
            ScannerMode::Auto => get_default_brain_dir(),
        }
    }

    pub fn scan(&mut self) -> ScanResult {
        let is_custom_dir = self.is_custom_dir();
        let raw_specs = match &self.mode {
            ScannerMode::Auto => discover_default_brain_sources(),
            ScannerMode::Custom(dir) => {
                let (id, label) = infer_source_from_dir(dir);
                vec![BrainSourceSpec {
                    id,
                    label,
                    dir: dir.clone(),
                }]
            }
            ScannerMode::Explicit(list) => list.clone(),
        };

        let mut seen_dirs: HashSet<String> = HashSet::new();
        let mut specs: Vec<BrainSourceSpec> = Vec::new();
        for spec in raw_specs {
            let key = path_dedup_key(&spec.dir);
            if !key.is_empty() && seen_dirs.insert(key) {
                specs.push(spec);
            }
        }

        let mut source_map: HashMap<String, UsageSourceInfo> = HashMap::new();
        let mut active_paths: HashSet<PathBuf> = HashSet::new();
        let mut all_sessions: Vec<ParsedSession> = Vec::new();
        let mut total_dirs = 0usize;
        let mut skipped_dirs = 0usize;
        let mut existing_dirs: Vec<String> = Vec::new();

        for spec in &specs {
            let dir_str = spec.dir.to_string_lossy().to_string();
            let info = source_map
                .entry(spec.id.clone())
                .or_insert_with(|| UsageSourceInfo {
                    id: spec.id.clone(),
                    label: spec.label.clone(),
                    dirs: Vec::new(),
                    exists: false,
                    session_count: 0,
                });
            if !info.dirs.contains(&dir_str) {
                info.dirs.push(dir_str.clone());
            }

            let read_dir = match fs::read_dir(&spec.dir) {
                Ok(rd) => rd,
                Err(err) => {
                    if is_custom_dir {
                        let msg = if err.kind() == ErrorKind::NotFound {
                            format!("目录不存在: {dir_str}")
                        } else {
                            format!("无法读取目录: {err}")
                        };
                        let mut sources: Vec<UsageSourceInfo> = source_map.into_values().collect();
                        sources.sort_by_key(|s| canonical_source_order(&s.id));
                        return ScanResult {
                            brain_dir: dir_str,
                            is_custom_dir,
                            sources,
                            scanned_at: now_ms(),
                            error: Some(msg),
                            total_dirs: 0,
                            parsed_sessions: 0,
                            skipped_dirs: 0,
                            sessions: Vec::new(),
                        };
                    }
                    if err.kind() != ErrorKind::NotFound {
                        skipped_dirs += 1;
                    }
                    continue;
                }
            };

            info.exists = true;
            existing_dirs.push(dir_str.clone());
            let mut group_sessions: Vec<ParsedSession> = Vec::new();

            for entry_res in read_dir {
                let entry = match entry_res {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let ft = match entry.file_type() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if !ft.is_dir() {
                    continue;
                }
                total_dirs += 1;

                let session_id = entry.file_name().to_string_lossy().to_string();
                let session_dir = entry.path();
                let mut found_path: Option<PathBuf> = None;
                let mut found_meta: Option<fs::Metadata> = None;
                let mut perm_denied = false;

                for rel in TRANSCRIPT_CANDIDATES {
                    let candidate = session_dir.join(rel);
                    match fs::metadata(&candidate) {
                        Ok(meta) if meta.is_file() => {
                            found_path = Some(candidate);
                            found_meta = Some(meta);
                            break;
                        }
                        Ok(_) => {}
                        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                            perm_denied = true;
                            break;
                        }
                        Err(_) => {}
                    }
                }

                if perm_denied {
                    skipped_dirs += 1;
                    continue;
                }

                let (file_path, meta) = match (found_path, found_meta) {
                    (Some(p), Some(m)) => (p, m),
                    _ => continue,
                };

                let mtime_ms = meta.modified().map(system_time_to_ms).unwrap_or(0);
                let size = meta.len();
                active_paths.insert(file_path.clone());

                if let Some(cached) = self.cache.get(&file_path) {
                    if cached.mtime_ms == mtime_ms && cached.size == size {
                        let mut sess = cached.session.clone();
                        sess.source = spec.id.clone();
                        sess.source_label = spec.label.clone();
                        group_sessions.push(sess);
                        continue;
                    }
                }

                match fs::read_to_string(&file_path) {
                    Ok(text) => {
                        let mut parsed = parse_transcript_text(&text, &session_id);
                        parsed.source = spec.id.clone();
                        parsed.source_label = spec.label.clone();
                        let fallback_ts = meta
                            .created()
                            .ok()
                            .map(system_time_to_ms)
                            .unwrap_or(mtime_ms);
                        if parsed.started_at.is_none() {
                            parsed.started_at = Some(fallback_ts);
                        }
                        if parsed.last_at.is_none() {
                            parsed.last_at = Some(mtime_ms);
                        }
                        self.cache.insert(
                            file_path,
                            CacheEntry {
                                mtime_ms,
                                size,
                                session: parsed.clone(),
                            },
                        );
                        group_sessions.push(parsed);
                    }
                    Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                        skipped_dirs += 1;
                    }
                    Err(_) => {}
                }
            }

            let mut resolved_group = resolve_session_graph(group_sessions);
            for s in &mut resolved_group {
                s.source = spec.id.clone();
                s.source_label = spec.label.clone();
            }
            info.session_count += resolved_group.len();
            all_sessions.extend(resolved_group);
        }

        self.cache.retain(|k, _| active_paths.contains(k));

        let mut sources: Vec<UsageSourceInfo> = source_map.into_values().collect();
        sources.sort_by_key(|s| canonical_source_order(&s.id));

        let brain_dir_display = if !existing_dirs.is_empty() {
            existing_dirs.join(" ; ")
        } else {
            self.primary_dir().to_string_lossy().to_string()
        };

        if !is_custom_dir && existing_dirs.is_empty() {
            return ScanResult {
                brain_dir: brain_dir_display,
                is_custom_dir,
                sources,
                scanned_at: now_ms(),
                error: Some("未找到任何可用的 Antigravity / CLI 日志目录".to_string()),
                total_dirs: 0,
                parsed_sessions: 0,
                skipped_dirs,
                sessions: Vec::new(),
            };
        }

        ScanResult {
            brain_dir: brain_dir_display,
            is_custom_dir,
            sources,
            scanned_at: now_ms(),
            error: None,
            total_dirs,
            parsed_sessions: all_sessions.len(),
            skipped_dirs,
            sessions: all_sessions,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageMeta {
    pub brain_dir: String,
    pub is_custom_dir: bool,
    pub sources: Vec<UsageSourceInfo>,
    pub source_filter: String,
    pub scanned_at: i64,
    pub error: Option<String>,
    pub total_dirs: usize,
    pub parsed_sessions: usize,
    pub skipped_dirs: usize,
    pub all_time_events_count: usize,
    pub available_models: Vec<String>,
    pub available_days: Vec<String>,
    pub selected_day: Option<String>,
    pub selected_start_day: Option<String>,
    pub selected_end_day: Option<String>,
    pub selected_start_ts: Option<i64>,
    pub selected_end_ts: Option<i64>,
    pub range: String,
    pub granularity: String,
    pub model_filter: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub net_tokens: u64,
    pub cache_hit_rate: f64,
    pub call_count: u64,
    pub session_count: usize,
    pub user_session_count: usize,
    pub subagent_session_count: usize,
    pub tool_call_count: u64,
    pub avg_tokens_per_call: u64,
    pub avg_tokens_per_session: u64,
    pub avg_output_tokens_per_sec: f64,
    pub peak_output_tokens_per_sec: f64,
    pub speed_sample_count: u64,
    pub last_activity_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesBucket {
    pub ts: i64,
    pub key: String,
    pub label: String,
    pub short_label: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub net_tokens: u64,
    pub call_count: u64,
    pub cache_hit_rate: f64,
    pub avg_output_tokens_per_sec: f64,
    pub speed_sample_count: u64,
    #[serde(skip_serializing)]
    pub speed_output_tokens: u64,
    #[serde(skip_serializing)]
    pub speed_duration_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model: String,
    pub call_count: u64,
    pub session_count: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub net_tokens: u64,
    pub cache_hit_rate: f64,
    pub avg_output_tokens_per_sec: f64,
    pub peak_output_tokens_per_sec: f64,
    pub speed_sample_count: u64,
    pub share: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub source: String,
    pub source_label: String,
    pub title: String,
    pub is_subagent: bool,
    pub parent_id: Option<String>,
    pub root_parent_id: Option<String>,
    pub parent_title: Option<String>,
    pub subagent_role: Option<String>,
    pub subagent_type: Option<String>,
    pub started_at: Option<i64>,
    pub last_at: Option<i64>,
    pub duration_ms: i64,
    pub primary_model: String,
    pub models: Vec<String>,
    pub call_count: u64,
    pub tool_call_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub net_tokens: u64,
    pub total_tokens: u64,
    pub cache_hit_rate: f64,
    pub avg_output_tokens_per_sec: f64,
    pub speed_output_tokens: u64,
    pub speed_duration_ms: i64,
    pub speed_sample_count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarDay {
    pub day: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub net_tokens: u64,
    pub total_tokens: u64,
    pub call_count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageResponse {
    pub meta: UsageMeta,
    pub summary: UsageSummary,
    pub series: Vec<SeriesBucket>,
    pub calendar: Vec<CalendarDay>,
    pub models: Vec<ModelUsage>,
    pub sessions: Vec<SessionSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetailResponse {
    pub id: String,
    pub source: String,
    pub source_label: String,
    pub title: String,
    pub started_at: Option<i64>,
    pub last_at: Option<i64>,
    pub events: Vec<SessionEvent>,
}

fn calc_cache_hit_rate(input_tokens: u64, cache_read_tokens: u64) -> f64 {
    let denom = input_tokens + cache_read_tokens;
    if denom == 0 {
        0.0
    } else {
        cache_read_tokens as f64 / denom as f64
    }
}

fn calc_weighted_tps(output_tokens: u64, duration_ms: i64) -> f64 {
    if duration_ms <= 0 || output_tokens == 0 {
        0.0
    } else {
        let raw = (output_tokens as f64) * 1000.0 / (duration_ms as f64);
        (raw * 10.0).round() / 10.0
    }
}

fn local_dt_from_ms(ms: i64) -> DateTime<Local> {
    let utc = DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_else(Utc::now);
    utc.with_timezone(&Local)
}

fn bucket_start_local(ms: i64, granularity: &str) -> DateTime<Local> {
    let dt = local_dt_from_ms(ms);
    let target = if granularity == "hour" {
        Local.with_ymd_and_hms(dt.year(), dt.month(), dt.day(), dt.hour(), 0, 0)
    } else {
        Local.with_ymd_and_hms(dt.year(), dt.month(), dt.day(), 0, 0, 0)
    };
    match target {
        LocalResult::Single(t) => t,
        LocalResult::Ambiguous(t, _) => t,
        LocalResult::None => dt,
    }
}

fn format_bucket_labels(dt: &DateTime<Local>, granularity: &str) -> (String, String, String) {
    let y = dt.year();
    let m = dt.month();
    let d = dt.day();
    let hh = dt.hour();
    if granularity == "hour" {
        (
            format!("{y:04}-{m:02}-{d:02}T{hh:02}"),
            format!("{m:02}-{d:02} {hh:02}:00"),
            format!("{hh:02}:00"),
        )
    } else {
        (
            format!("{y:04}-{m:02}-{d:02}"),
            format!("{y:04}-{m:02}-{d:02}"),
            format!("{m:02}/{d:02}"),
        )
    }
}

fn local_midnight(date: NaiveDate) -> Option<DateTime<Local>> {
    match Local.with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0) {
        LocalResult::Single(t) => Some(t),
        LocalResult::Ambiguous(t, _) => Some(t),
        LocalResult::None => None,
    }
}

fn parse_local_day_bounds(day_opt: Option<&str>) -> Option<(String, DateTime<Local>, DateTime<Local>)> {
    let raw = day_opt?.trim();
    if raw.len() != 10 {
        return None;
    }
    let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()?;
    let next_date = date.succ_opt()?;
    let start = local_midnight(date)?;
    let end = local_midnight(next_date)?;
    Some((date.format("%Y-%m-%d").to_string(), start, end))
}

fn format_local_day_key(ms: i64) -> String {
    let dt = local_dt_from_ms(ms);
    format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day())
}

fn parse_local_day_range(
    start_opt: Option<&str>,
    end_opt: Option<&str>,
) -> Option<(String, String, DateTime<Local>, DateTime<Local>)> {
    let s_raw = start_opt?.trim();
    let e_raw = end_opt?.trim();
    if s_raw.len() != 10 || e_raw.len() != 10 {
        return None;
    }
    let d1 = NaiveDate::parse_from_str(s_raw, "%Y-%m-%d").ok()?;
    let d2 = NaiveDate::parse_from_str(e_raw, "%Y-%m-%d").ok()?;
    let (start_date, end_date) = if d1 <= d2 { (d1, d2) } else { (d2, d1) };
    let next_after_end = end_date.succ_opt()?;
    let start_dt = local_midnight(start_date)?;
    let end_dt = local_midnight(next_after_end)?;
    Some((
        start_date.format("%Y-%m-%d").to_string(),
        end_date.format("%Y-%m-%d").to_string(),
        start_dt,
        end_dt,
    ))
}

#[derive(Debug, Default, Clone)]
pub struct AggregateRangeFilter<'a> {
    pub start_day: Option<&'a str>,
    pub end_day: Option<&'a str>,
    pub start_ts: Option<i64>,
    pub end_ts: Option<i64>,
}

#[cfg(test)]
pub fn aggregate_usage(
    scan_result: &ScanResult,
    range_opt: Option<&str>,
    granularity_opt: Option<&str>,
    model_opt: Option<&str>,
    source_opt: Option<&str>,
    day_opt: Option<&str>,
    now_override: Option<i64>,
) -> UsageResponse {
    aggregate_usage_with_filter(
        scan_result,
        range_opt,
        granularity_opt,
        model_opt,
        source_opt,
        day_opt,
        AggregateRangeFilter::default(),
        now_override,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn aggregate_usage_with_filter(
    scan_result: &ScanResult,
    range_opt: Option<&str>,
    granularity_opt: Option<&str>,
    model_opt: Option<&str>,
    source_opt: Option<&str>,
    day_opt: Option<&str>,
    custom_filter: AggregateRangeFilter<'_>,
    now_override: Option<i64>,
) -> UsageResponse {
    let now = now_override.unwrap_or_else(now_ms);
    let ts_window = match (custom_filter.start_ts, custom_filter.end_ts) {
        (Some(a), Some(b)) if b > a => Some((a, b)),
        _ => None,
    };
    let day_range_bounds = if ts_window.is_none() {
        parse_local_day_range(custom_filter.start_day, custom_filter.end_day)
    } else {
        None
    };
    let day_bounds = if ts_window.is_none() && day_range_bounds.is_none() {
        parse_local_day_bounds(day_opt)
    } else {
        None
    };

    let range = if ts_window.is_some() || day_range_bounds.is_some() {
        "custom"
    } else if day_bounds.is_some() {
        "day"
    } else {
        match range_opt.unwrap_or("7d") {
            "24h" => "24h",
            "7d" => "7d",
            "30d" => "30d",
            "all" => "all",
            _ => "7d",
        }
    };
    let granularity = if ts_window.is_some() {
        match granularity_opt {
            Some("day") => "day",
            _ => "hour",
        }
    } else if let Some((ref sd, ref ed, _, _)) = day_range_bounds {
        match granularity_opt {
            Some("hour") => "hour",
            Some("day") => "day",
            _ => {
                if sd == ed {
                    "hour"
                } else {
                    "day"
                }
            }
        }
    } else if day_bounds.is_some() {
        "hour"
    } else {
        match granularity_opt {
            Some("hour") => "hour",
            Some("day") => "day",
            _ => {
                if range == "24h" {
                    "hour"
                } else {
                    "day"
                }
            }
        }
    };
    let model_filter = model_opt.filter(|m| !m.is_empty() && *m != "all");
    let source_filter = source_opt.filter(|s| !s.is_empty() && *s != "all");

    let (range_start, range_end): (Option<i64>, Option<i64>) =
        if let Some((st, et)) = ts_window {
            (Some(st), Some(et))
        } else if let Some((_, _, ref start_dt, ref end_dt)) = day_range_bounds {
            (Some(start_dt.timestamp_millis()), Some(end_dt.timestamp_millis()))
        } else if let Some((_, ref start_dt, ref end_dt)) = day_bounds {
            (Some(start_dt.timestamp_millis()), Some(end_dt.timestamp_millis()))
        } else {
            let rs = match range {
                "24h" => Some(now - 24 * 60 * 60 * 1000),
                "7d" => Some(now - 7 * 24 * 60 * 60 * 1000),
                "30d" => Some(now - 30 * 24 * 60 * 60 * 1000),
                _ => None,
            };
            (rs, None)
        };

    let mut all_models: BTreeSet<String> = BTreeSet::new();
    let mut available_days: BTreeSet<String> = BTreeSet::new();
    let mut calendar_map: HashMap<String, CalendarDay> = HashMap::new();
    let mut all_time_events_count = 0usize;

    let mut summary = UsageSummary {
        total_tokens: 0,
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        net_tokens: 0,
        cache_hit_rate: 0.0,
        call_count: 0,
        session_count: 0,
        user_session_count: 0,
        subagent_session_count: 0,
        tool_call_count: 0,
        avg_tokens_per_call: 0,
        avg_tokens_per_session: 0,
        avg_output_tokens_per_sec: 0.0,
        peak_output_tokens_per_sec: 0.0,
        speed_sample_count: 0,
        last_activity_at: None,
    };

    let mut summary_speed_out = 0u64;
    let mut summary_speed_ms = 0i64;
    let mut summary_peak_strict = 0.0f64;
    let mut summary_peak_any = 0.0f64;

    struct ModelAgg {
        call_count: u64,
        sessions: HashSet<(String, String)>,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
        total_tokens: u64,
        net_tokens: u64,
        speed_output_tokens: u64,
        speed_duration_ms: i64,
        speed_sample_count: u64,
        peak_strict: f64,
        peak_any: f64,
    }

    let mut bucket_map: HashMap<i64, SeriesBucket> = HashMap::new();
    let mut model_map: HashMap<String, ModelAgg> = HashMap::new();
    let mut session_summaries: Vec<SessionSummary> = Vec::new();
    let mut min_event_ts: Option<i64> = None;
    let mut max_event_ts: Option<i64> = None;

    for session in &scan_result.sessions {
        if let Some(sf) = source_filter {
            if session.source != sf {
                continue;
            }
        }
        let mut sess_input = 0u64;
        let mut sess_output = 0u64;
        let mut sess_cache_read = 0u64;
        let mut sess_cache_write = 0u64;
        let mut sess_calls = 0u64;
        let mut sess_tools = 0u64;
        let mut sess_speed_out = 0u64;
        let mut sess_speed_ms = 0i64;
        let mut sess_speed_samples = 0u64;
        let mut sess_first_ts: Option<i64> = None;
        let mut sess_last_ts: Option<i64> = None;
        let mut sess_model_tokens: HashMap<String, u64> = HashMap::new();

        for ev in &session.events {
            all_models.insert(ev.model.clone());
            let ev_day_key = format_local_day_key(ev.ts);
            available_days.insert(ev_day_key.clone());
            all_time_events_count += 1;

            if model_filter.is_none_or(|mf| ev.model == mf) {
                let cal = calendar_map
                    .entry(ev_day_key.clone())
                    .or_insert_with(|| CalendarDay {
                        day: ev_day_key,
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_read_tokens: 0,
                        net_tokens: 0,
                        total_tokens: 0,
                        call_count: 0,
                    });
                cal.input_tokens += ev.input_tokens;
                cal.output_tokens += ev.output_tokens;
                cal.cache_read_tokens += ev.cache_read_tokens;
                cal.net_tokens += ev.input_tokens + ev.output_tokens;
                cal.total_tokens += ev.total_tokens;
                cal.call_count += 1;
            }

            if let Some(rs) = range_start {
                if ev.ts < rs {
                    continue;
                }
            }
            if let Some(re) = range_end {
                if ev.ts >= re {
                    continue;
                }
            }
            if let Some(mf) = model_filter {
                if ev.model != mf {
                    continue;
                }
            }

            sess_calls += 1;
            sess_input += ev.input_tokens;
            sess_output += ev.output_tokens;
            sess_cache_read += ev.cache_read_tokens;
            sess_cache_write += ev.cache_write_tokens;
            sess_tools += ev.tool_count;

            sess_first_ts = Some(sess_first_ts.map_or(ev.ts, |cur| cur.min(ev.ts)));
            sess_last_ts = Some(sess_last_ts.map_or(ev.ts, |cur| cur.max(ev.ts)));
            min_event_ts = Some(min_event_ts.map_or(ev.ts, |cur| cur.min(ev.ts)));
            max_event_ts = Some(max_event_ts.map_or(ev.ts, |cur| cur.max(ev.ts)));

            *sess_model_tokens.entry(ev.model.clone()).or_insert(0) += ev.total_tokens;

            let b_dt = bucket_start_local(ev.ts, granularity);
            let b_ms = b_dt.timestamp_millis();
            let bucket = bucket_map.entry(b_ms).or_insert_with(|| {
                let (key, label, short_label) = format_bucket_labels(&b_dt, granularity);
                SeriesBucket {
                    ts: b_ms,
                    key,
                    label,
                    short_label,
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    total_tokens: 0,
                    net_tokens: 0,
                    call_count: 0,
                    cache_hit_rate: 0.0,
                    avg_output_tokens_per_sec: 0.0,
                    speed_sample_count: 0,
                    speed_output_tokens: 0,
                    speed_duration_ms: 0,
                }
            });
            bucket.input_tokens += ev.input_tokens;
            bucket.output_tokens += ev.output_tokens;
            bucket.cache_read_tokens += ev.cache_read_tokens;
            bucket.cache_write_tokens += ev.cache_write_tokens;
            bucket.total_tokens += ev.total_tokens;
            bucket.net_tokens += ev.input_tokens + ev.output_tokens;
            bucket.call_count += 1;

            let m_agg = model_map.entry(ev.model.clone()).or_insert_with(|| ModelAgg {
                call_count: 0,
                sessions: HashSet::new(),
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                total_tokens: 0,
                net_tokens: 0,
                speed_output_tokens: 0,
                speed_duration_ms: 0,
                speed_sample_count: 0,
                peak_strict: 0.0,
                peak_any: 0.0,
            });
            m_agg.call_count += 1;
            m_agg.sessions.insert((session.source.clone(), session.id.clone()));
            m_agg.input_tokens += ev.input_tokens;
            m_agg.output_tokens += ev.output_tokens;
            m_agg.cache_read_tokens += ev.cache_read_tokens;
            m_agg.cache_write_tokens += ev.cache_write_tokens;
            m_agg.total_tokens += ev.total_tokens;
            m_agg.net_tokens += ev.input_tokens + ev.output_tokens;

            if let (Some(dur_ms), Some(tps)) = (ev.gen_duration_ms, ev.tokens_per_sec) {
                sess_speed_out += ev.output_tokens;
                sess_speed_ms += dur_ms;
                sess_speed_samples += 1;

                summary_speed_out += ev.output_tokens;
                summary_speed_ms += dur_ms;
                summary.speed_sample_count += 1;
                if tps > summary_peak_any {
                    summary_peak_any = tps;
                }
                if dur_ms >= 3_000 && ev.output_tokens >= 100 && tps > summary_peak_strict {
                    summary_peak_strict = tps;
                }

                bucket.speed_output_tokens += ev.output_tokens;
                bucket.speed_duration_ms += dur_ms;
                bucket.speed_sample_count += 1;

                m_agg.speed_output_tokens += ev.output_tokens;
                m_agg.speed_duration_ms += dur_ms;
                m_agg.speed_sample_count += 1;
                if tps > m_agg.peak_any {
                    m_agg.peak_any = tps;
                }
                if dur_ms >= 3_000 && ev.output_tokens >= 100 && tps > m_agg.peak_strict {
                    m_agg.peak_strict = tps;
                }
            }
        }

        if sess_calls > 0 {
            let sess_total = sess_input + sess_output + sess_cache_read + sess_cache_write;
            let sess_net = sess_input + sess_output;
            let primary_model = sess_model_tokens
                .iter()
                .max_by_key(|(_, &tok)| tok)
                .map(|(m, _)| m.clone())
                .unwrap_or_else(|| "unknown".to_string());

            summary.total_tokens += sess_total;
            summary.input_tokens += sess_input;
            summary.output_tokens += sess_output;
            summary.cache_read_tokens += sess_cache_read;
            summary.cache_write_tokens += sess_cache_write;
            summary.net_tokens += sess_net;
            summary.call_count += sess_calls;
            summary.session_count += 1;
            if session.is_subagent {
                summary.subagent_session_count += 1;
            } else {
                summary.user_session_count += 1;
            }
            summary.tool_call_count += sess_tools;
            if let Some(lt) = sess_last_ts {
                summary.last_activity_at =
                    Some(summary.last_activity_at.map_or(lt, |cur| cur.max(lt)));
            }

            let duration_ms = (sess_last_ts.unwrap_or(0) - sess_first_ts.unwrap_or(0)).max(0);
            session_summaries.push(SessionSummary {
                id: session.id.clone(),
                source: session.source.clone(),
                source_label: session.source_label.clone(),
                title: session.title.clone(),
                is_subagent: session.is_subagent,
                parent_id: session.parent_id.clone(),
                root_parent_id: session.root_parent_id.clone(),
                parent_title: session.parent_title.clone(),
                subagent_role: session.subagent_role.clone(),
                subagent_type: session.subagent_type.clone(),
                started_at: sess_first_ts,
                last_at: sess_last_ts,
                duration_ms,
                primary_model,
                models: sess_model_tokens.into_keys().collect(),
                call_count: sess_calls,
                tool_call_count: sess_tools,
                input_tokens: sess_input,
                output_tokens: sess_output,
                cache_read_tokens: sess_cache_read,
                cache_write_tokens: sess_cache_write,
                net_tokens: sess_net,
                total_tokens: sess_total,
                cache_hit_rate: calc_cache_hit_rate(sess_input, sess_cache_read),
                avg_output_tokens_per_sec: calc_weighted_tps(sess_speed_out, sess_speed_ms),
                speed_output_tokens: sess_speed_out,
                speed_duration_ms: sess_speed_ms,
                speed_sample_count: sess_speed_samples,
            });
        }
    }

    summary.cache_hit_rate = calc_cache_hit_rate(summary.input_tokens, summary.cache_read_tokens);
    summary.avg_output_tokens_per_sec = calc_weighted_tps(summary_speed_out, summary_speed_ms);
    summary.peak_output_tokens_per_sec = if summary_peak_strict > 0.0 {
        summary_peak_strict
    } else {
        summary_peak_any
    };
    summary.avg_tokens_per_call = if summary.call_count > 0 {
        summary.total_tokens / summary.call_count
    } else {
        0
    };
    summary.avg_tokens_per_session = if summary.session_count > 0 {
        summary.total_tokens / summary.session_count as u64
    } else {
        0
    };

    // Build continuous local-time series
    let mut timeline_end = bucket_start_local(now, granularity);
    let mut timeline_start = if let Some((st, et)) = ts_window {
        timeline_end = bucket_start_local(et.saturating_sub(1).max(st), granularity);
        bucket_start_local(st, granularity)
    } else if let Some((_, _, ref start_dt, ref end_dt)) = day_range_bounds {
        let last_ms = end_dt.timestamp_millis().saturating_sub(1);
        timeline_end = bucket_start_local(last_ms, granularity);
        let mut s = bucket_start_local(start_dt.timestamp_millis(), granularity);
        if granularity == "hour" {
            let min_allowed = timeline_end - Duration::hours(167);
            if s < min_allowed {
                s = min_allowed;
            }
        }
        s
    } else if let Some((_, ref start_dt, _)) = day_bounds {
        timeline_end = *start_dt + Duration::hours(23);
        *start_dt
    } else if let Some(rs) = range_start {
        if granularity == "hour" && range == "30d" {
            bucket_start_local(now - 72 * 60 * 60 * 1000, "hour")
        } else if granularity == "hour" && range == "7d" {
            bucket_start_local(now - 7 * 24 * 60 * 60 * 1000, "hour")
        } else {
            bucket_start_local(rs, granularity)
        }
    } else if let Some(min_ts) = min_event_ts {
        if let Some(max_ts) = max_event_ts {
            let max_b = bucket_start_local(max_ts, granularity);
            if max_b > timeline_end {
                timeline_end = max_b;
            }
        }
        let mut s = bucket_start_local(min_ts, granularity);
        if granularity == "hour" {
            let min_allowed = timeline_end - Duration::hours(72);
            if s < min_allowed {
                s = min_allowed;
            }
        }
        s
    } else if granularity == "hour" {
        timeline_end - Duration::hours(23)
    } else {
        timeline_end - Duration::days(6)
    };

    let max_buckets = if granularity == "hour" { 168 } else { 365 };
    let mut series = Vec::new();
    let mut count = 0usize;
    while timeline_start <= timeline_end && count < max_buckets {
        let ms = timeline_start.timestamp_millis();
        if let Some(mut existing) = bucket_map.remove(&ms) {
            existing.cache_hit_rate =
                calc_cache_hit_rate(existing.input_tokens, existing.cache_read_tokens);
            existing.avg_output_tokens_per_sec =
                calc_weighted_tps(existing.speed_output_tokens, existing.speed_duration_ms);
            series.push(existing);
        } else {
            let (key, label, short_label) = format_bucket_labels(&timeline_start, granularity);
            series.push(SeriesBucket {
                ts: ms,
                key,
                label,
                short_label,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                total_tokens: 0,
                net_tokens: 0,
                call_count: 0,
                cache_hit_rate: 0.0,
                avg_output_tokens_per_sec: 0.0,
                speed_sample_count: 0,
                speed_output_tokens: 0,
                speed_duration_ms: 0,
            });
        }
        timeline_start = if granularity == "hour" {
            timeline_start + Duration::hours(1)
        } else {
            timeline_start + Duration::days(1)
        };
        count += 1;
    }

    let mut models: Vec<ModelUsage> = model_map
        .into_iter()
        .map(|(model, m)| ModelUsage {
            model,
            call_count: m.call_count,
            session_count: m.sessions.len(),
            input_tokens: m.input_tokens,
            output_tokens: m.output_tokens,
            cache_read_tokens: m.cache_read_tokens,
            cache_write_tokens: m.cache_write_tokens,
            total_tokens: m.total_tokens,
            net_tokens: m.net_tokens,
            cache_hit_rate: calc_cache_hit_rate(m.input_tokens, m.cache_read_tokens),
            avg_output_tokens_per_sec: calc_weighted_tps(m.speed_output_tokens, m.speed_duration_ms),
            peak_output_tokens_per_sec: if m.peak_strict > 0.0 {
                m.peak_strict
            } else {
                m.peak_any
            },
            speed_sample_count: m.speed_sample_count,
            share: if summary.total_tokens > 0 {
                m.total_tokens as f64 / summary.total_tokens as f64
            } else {
                0.0
            },
        })
        .collect();
    models.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));

    session_summaries.sort_by(|a, b| b.last_at.unwrap_or(0).cmp(&a.last_at.unwrap_or(0)));

    let mut calendar: Vec<CalendarDay> = calendar_map.into_values().collect();
    calendar.sort_by(|a, b| a.day.cmp(&b.day));

    UsageResponse {
        meta: UsageMeta {
            brain_dir: scan_result.brain_dir.clone(),
            is_custom_dir: scan_result.is_custom_dir,
            sources: scan_result.sources.clone(),
            source_filter: source_filter.unwrap_or("all").to_string(),
            scanned_at: scan_result.scanned_at,
            error: scan_result.error.clone(),
            total_dirs: scan_result.total_dirs,
            parsed_sessions: scan_result.parsed_sessions,
            skipped_dirs: scan_result.skipped_dirs,
            all_time_events_count,
            available_models: all_models.into_iter().collect(),
            available_days: available_days.into_iter().collect(),
            selected_day: day_bounds.map(|(k, _, _)| k),
            selected_start_day: day_range_bounds.as_ref().map(|(s, _, _, _)| s.clone()),
            selected_end_day: day_range_bounds.as_ref().map(|(_, e, _, _)| e.clone()),
            selected_start_ts: ts_window.map(|(s, _)| s),
            selected_end_ts: ts_window.map(|(_, e)| e),
            range: range.to_string(),
            granularity: granularity.to_string(),
            model_filter: model_filter.unwrap_or("all").to_string(),
        },
        summary,
        series,
        calendar,
        models,
        sessions: session_summaries,
    }
}

fn escape_csv_cell(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn iso_from_ms(ms: Option<i64>) -> String {
    ms.and_then(DateTime::<Utc>::from_timestamp_millis)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

pub fn build_sessions_csv(sessions: &[SessionSummary]) -> String {
    let mut lines = Vec::with_capacity(sessions.len() + 1);
    lines.push(
        "session_id,source,kind,parent_id,title,started_at,last_at,primary_model,calls,tool_calls,input_tokens,output_tokens,cache_read_tokens,net_tokens,total_tokens,cache_hit_rate"
            .to_string(),
    );
    for s in sessions {
        let row = [
            s.id.clone(),
            s.source_label.clone(),
            if s.is_subagent {
                "subagent".to_string()
            } else {
                "main".to_string()
            },
            s.root_parent_id
                .clone()
                .or_else(|| s.parent_id.clone())
                .unwrap_or_default(),
            s.title.clone(),
            iso_from_ms(s.started_at),
            iso_from_ms(s.last_at),
            s.primary_model.clone(),
            s.call_count.to_string(),
            s.tool_call_count.to_string(),
            s.input_tokens.to_string(),
            s.output_tokens.to_string(),
            s.cache_read_tokens.to_string(),
            s.net_tokens.to_string(),
            s.total_tokens.to_string(),
            format!("{:.2}%", s.cache_hit_rate * 100.0),
            if s.avg_output_tokens_per_sec > 0.0 {
                format!("{:.1}", s.avg_output_tokens_per_sec)
            } else {
                String::new()
            },
        ];
        lines.push(
            row.iter()
                .map(|c| escape_csv_cell(c))
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    format!("\u{FEFF}{}", lines.join("\r\n"))
}

pub fn load_persisted_brain_dir() -> Option<PathBuf> {
    let cfg = config_file_path()?;
    let raw = fs::read_to_string(cfg).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let p = v.get("brainDir")?.as_str()?.trim();
    if p.is_empty() {
        None
    } else {
        Some(PathBuf::from(p))
    }
}

pub fn save_persisted_brain_dir(dir: Option<&Path>) {
    let Some(cfg) = config_file_path() else {
        return;
    };
    if let Some(d) = dir {
        if let Some(parent) = cfg.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let body = serde_json::json!({ "brainDir": d.to_string_lossy() });
        let _ = fs::write(cfg, body.to_string());
    } else {
        let _ = fs::remove_file(cfg);
    }
}

fn config_file_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("antigravity-usage-dashboard").join("config.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_legacy_models_and_subagent_graph() {
        let parent_txt = [
            r#"{"step_index":0,"type":"USER_INPUT","created_at":"2026-11-18T02:00:00Z","content":"<USER_REQUEST>重构 VerifyCodeList</USER_REQUEST><USER_SETTINGS_CHANGE>The user changed setting `Model Selection` from None to Orchestrator (Carbon).</USER_SETTINGS_CHANGE>"}"#,
            r#"{"step_index":1,"type":"PLANNER_RESPONSE","created_at":"2026-11-18T02:00:05Z","input_tokens":5000,"output_tokens":300,"tool_calls":[{"name":"invoke_subagent","args":{"Model":"\"inherit\"","TypeName":"\"research\"","Role":"\"Code Auditor\""}}]}"#,
            r#"{"step_index":2,"type":"GENERIC","created_at":"2026-11-18T02:00:06Z","content":"Created subagent: {\"conversationId\": \"child-1111-aaaa\"}"}"#,
        ]
        .join("\n");

        let child_txt = r#"{"step_index":1,"type":"PLANNER_RESPONSE","created_at":"2026-11-18T02:00:10Z","input_tokens":8000,"cache_read_tokens":2000,"output_tokens":400}"#;

        let p = parse_transcript_text(&parent_txt, "parent-0000");
        let c = parse_transcript_text(child_txt, "child-1111-aaaa");
        let resolved = resolve_session_graph(vec![p, c]);

        let child = resolved.iter().find(|s| s.id == "child-1111-aaaa").unwrap();
        assert!(child.is_subagent);
        assert_eq!(child.root_parent_id.as_deref(), Some("parent-0000"));
        assert_eq!(child.parent_title.as_deref(), Some("重构 VerifyCodeList"));
        assert_eq!(child.title, "Code Auditor");
        assert_eq!(child.subagent_type.as_deref(), Some("research"));
        assert_eq!(child.events[0].model, "Orchestrator (Carbon)");

        // Single-day hourly drill-down returns exact 24 hourly buckets
        let scan = ScanResult {
            brain_dir: "test".to_string(),
            is_custom_dir: false,
            sources: vec![],
            scanned_at: 0,
            error: None,
            total_dirs: 2,
            parsed_sessions: 2,
            skipped_dirs: 0,
            sessions: resolved,
        };
        let agg = aggregate_usage(&scan, Some("7d"), None, None, None, Some("2026-11-18"), None);
        assert_eq!(agg.meta.range, "day");
        assert_eq!(agg.meta.granularity, "hour");
        assert_eq!(agg.meta.selected_day.as_deref(), Some("2026-11-18"));
        assert_eq!(agg.series.len(), 24);
        assert_eq!(agg.summary.call_count, 2);
        assert_eq!(agg.summary.total_tokens, 15700);
        assert_eq!(agg.summary.speed_sample_count, 1);
        assert!((agg.summary.avg_output_tokens_per_sec - 300.0).abs() < 0.1);
    }

    #[test]
    fn computes_multi_tool_output_speed_accurately() {
        let txt = [
            r#"{"step_index":0,"type":"PLANNER_RESPONSE","created_at":"2026-11-24T10:00:00Z","model":"gemini-4-neon-high","input_tokens":2000,"output_tokens":1000,"tool_calls":[{"name":"write_file"},{"name":"write_file"}]}"#,
            r#"{"step_index":1,"type":"GENERIC","created_at":"2026-11-24T10:00:04Z","content":"ok"}"#,
            r#"{"step_index":2,"type":"GENERIC","created_at":"2026-11-24T10:00:10Z","content":"ok"}"#,
            r#"{"step_index":3,"type":"PLANNER_RESPONSE","created_at":"2026-11-24T10:00:11Z","model":"gemini-4-neon-high","input_tokens":2500,"output_tokens":200}"#,
            r#"{"step_index":4,"type":"USER_INPUT","created_at":"2026-11-24T10:15:00Z","content":"next"}"#,
        ]
        .join("\n");

        let parsed = parse_transcript_text(&txt, "speed-test");
        assert_eq!(parsed.events.len(), 2);
        // Step 0 spans 10s (to last parallel tool call) -> 1000 / 10 = 100.0 tok/s
        assert_eq!(parsed.events[0].gen_duration_ms, Some(10_000));
        assert_eq!(parsed.events[0].tokens_per_sec, Some(100.0));
        // Step 3 has no tool_calls (followed by USER_INPUT 15m later) -> must NOT count user wait time
        assert_eq!(parsed.events[1].gen_duration_ms, None);
        assert_eq!(parsed.events[1].tokens_per_sec, None);
    }

    #[test]
    fn strips_t3_bootstrap_tags_for_session_title() {
        let t3_txt = [
            r#"{"step_index":0,"type":"USER_INPUT","created_at":"2026-11-24T11:08:22Z","content":"<USER_REQUEST>\n<t3_code_instructions>\n## T3 Code interaction mode\n</t3_code_instructions>\n<runtime_info>Some runtime info</runtime_info>\n</USER_REQUEST>\n<ADDITIONAL_METADATA>meta</ADDITIONAL_METADATA>"}"#,
            r#"{"step_index":1,"type":"PLANNER_RESPONSE","created_at":"2026-11-24T11:08:22Z","model":"gemini-pro-orchestrator-neon","input_tokens":8000,"output_tokens":60,"cache_read_tokens":0}"#,
            r#"{"step_index":2,"type":"USER_INPUT","created_at":"2026-11-24T11:08:42Z","content":"<USER_REQUEST>\n在当前目录下写一个 py 输出 hello world<runtime_info>In case you're asked: running in T3 Code</runtime_info>\n</USER_REQUEST>"}"#,
            r#"{"step_index":3,"type":"PLANNER_RESPONSE","created_at":"2026-11-24T11:08:45Z","model":"gemini-4-hydrogen-medium","input_tokens":3400,"output_tokens":200,"cache_read_tokens":1000}"#,
        ]
        .join("\n");

        let parsed = parse_transcript_text(&t3_txt, "e3604c40-t3");
        assert!(parsed.has_user_input);
        assert_eq!(parsed.title, "在当前目录下写一个 py 输出 hello world");
        assert_eq!(parsed.events.len(), 2);
    }

    #[test]
    fn scans_and_filters_multiple_sources_independently() {
        let tmp = std::env::temp_dir().join(format!("agydock_multi_src_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let app_dir = tmp.join(".gemini").join("antigravity").join("brain");
        let cli_dir = tmp.join(".gemini").join("antigravity-cli").join("brain");
        let t3_dir = tmp
            .join(".t3")
            .join("userdata")
            .join("providers")
            .join("antigravity")
            .join("abc123")
            .join("antigravity-acp")
            .join("brain");

        fs::create_dir_all(app_dir.join("s-app").join(".system_generated").join("logs")).unwrap();
        fs::create_dir_all(cli_dir.join("s-cli").join(".system_generated").join("logs")).unwrap();
        fs::create_dir_all(t3_dir.join("s-t3").join(".system_generated").join("logs")).unwrap();

        fs::write(
            app_dir.join("s-app/.system_generated/logs/transcript.jsonl"),
            r#"{"step_index":0,"type":"USER_INPUT","created_at":"2026-11-24T10:00:00Z","content":"<USER_REQUEST>App 任务</USER_REQUEST>"}
{"step_index":1,"type":"PLANNER_RESPONSE","created_at":"2026-11-24T10:00:05Z","model":"gemini-4-neon-high","input_tokens":1000,"output_tokens":100,"cache_read_tokens":0}"#,
        )
        .unwrap();
        fs::write(
            cli_dir.join("s-cli/.system_generated/logs/transcript.jsonl"),
            r#"{"step_index":0,"type":"USER_INPUT","created_at":"2026-11-24T11:00:00Z","content":"<USER_REQUEST>CLI 任务</USER_REQUEST>"}
{"step_index":1,"type":"PLANNER_RESPONSE","created_at":"2026-11-24T11:00:05Z","model":"gemini-4-neon-high","input_tokens":2000,"output_tokens":200,"cache_read_tokens":0}"#,
        )
        .unwrap();
        fs::write(
            t3_dir.join("s-t3/.system_generated/logs/transcript.jsonl"),
            r#"{"step_index":0,"type":"USER_INPUT","created_at":"2026-11-24T12:00:00Z","content":"<USER_REQUEST><t3_code_instructions>boot</t3_code_instructions></USER_REQUEST>"}
{"step_index":1,"type":"USER_INPUT","created_at":"2026-11-24T12:00:02Z","content":"<USER_REQUEST>T3 任务</USER_REQUEST>"}
{"step_index":2,"type":"PLANNER_RESPONSE","created_at":"2026-11-24T12:00:05Z","model":"gemini-4-hydrogen-medium","input_tokens":3000,"output_tokens":300,"cache_read_tokens":0}"#,
        )
        .unwrap();

        let discovered = discover_brain_sources_from_home(Some(&tmp));
        assert_eq!(discovered.len(), 3);

        let mut scanner = BrainScanner::new_sources(discovered);
        let scan_res = scanner.scan();
        assert!(scan_res.error.is_none());
        assert!(!scan_res.is_custom_dir);
        assert_eq!(scan_res.parsed_sessions, 3);
        assert_eq!(scan_res.sources.len(), 3);

        let agg_all = aggregate_usage(&scan_res, Some("all"), None, None, None, None, None);
        assert_eq!(agg_all.summary.session_count, 3);
        assert_eq!(agg_all.summary.total_tokens, 6600);
        assert_eq!(agg_all.meta.source_filter, "all");

        let agg_t3 = aggregate_usage(&scan_res, Some("all"), None, None, Some("t3"), None, None);
        assert_eq!(agg_t3.summary.session_count, 1);
        assert_eq!(agg_t3.summary.total_tokens, 3300);
        assert_eq!(agg_t3.sessions[0].source, "t3");
        assert_eq!(agg_t3.sessions[0].source_label, "T3 Code");
        assert_eq!(agg_t3.sessions[0].title, "T3 任务");
        assert_eq!(agg_t3.meta.sources.len(), 3);

        let csv = build_sessions_csv(&agg_t3.sessions);
        assert!(csv.starts_with("\u{FEFF}session_id,source,"));
        assert!(csv.contains("T3 Code"));

        // Filtered discovery excludes uninstalled sources
        let only_app = discover_brain_sources_from_home_filtered(Some(&tmp), true, false, false);
        assert_eq!(only_app.len(), 1);
        assert_eq!(only_app[0].id, "app");
        assert_eq!(only_app[0].label, "Antigravity");

        // Custom date range and timestamp window filtering
        let agg_custom = aggregate_usage_with_filter(
            &scan_res,
            Some("custom"),
            Some("day"),
            None,
            None,
            None,
            AggregateRangeFilter {
                start_day: Some("2026-11-23"),
                end_day: Some("2026-11-25"),
                start_ts: None,
                end_ts: None,
            },
            None,
        );
        assert_eq!(agg_custom.meta.range, "custom");
        assert_eq!(agg_custom.meta.selected_start_day.as_deref(), Some("2026-11-23"));
        assert_eq!(agg_custom.meta.selected_end_day.as_deref(), Some("2026-11-25"));
        assert_eq!(agg_custom.series.len(), 3);
        assert_eq!(agg_custom.summary.session_count, 3);

        let _ = fs::remove_dir_all(&tmp);
    }
}
