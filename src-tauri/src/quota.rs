//! 账号套餐等级、剩余额度与刷新时间查询模块。
//!
//! 通过读取 Windows 凭据管理器 / 本地 DPAPI 备份中的 OAuth 令牌，
//! 经由 AgyDock 当前探测到的隔离代理（若监听）或系统网络访问 Google Cloud Code Assist 接口：
//! - `v1internal:loadCodeAssist`：获取套餐等级（Free / Pro / Ultra / Standard）、AI Credits 与项目 ID
//! - `v1internal:retrieveUserQuotaSummary`：获取汇总桶（如 5h 滚动窗口 / 周额度，best-effort）
//! - `v1internal:fetchAvailableModels`：获取各模型 `remainingFraction` 与 `resetTime`
//!
//! 令牌仅保留在内存缓存中，绝不序列化或写入 `quota-cache.json`。

use crate::credential::{self, CredentialTokens};
use crate::proxy::{self, ProxySettings};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const CLOUDCODE_HOSTS: [&str; 2] = [
    "daily-cloudcode-pa.googleapis.com",
    "cloudcode-pa.googleapis.com",
];
const OAUTH_HOST: &str = "oauth2.googleapis.com";
const OAUTH_PATH: &str = "/token";
const DEFAULT_USER_AGENT: &str = "antigravity/hub/2.24.0 windows/amd64";
const API_CLIENT_HEADER: &str = "google-cloud-sdk vscode_cloudshelleditor/0.1";
const CLIENT_METADATA_HEADER: &str =
    r#"{"ideType":"ANTIGRAVITY","platform":"WINDOWS","pluginType":"GEMINI"}"#;
const TOKEN_REFRESH_SKEW_MS: i64 = 60_000;
const DEFAULT_TOKEN_TTL_SEC: i64 = 3500;
const BATCH_CONCURRENCY: usize = 4;

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("tmp");
    let tmp_path = path.with_file_name(format!("{file_name}.tmp.{}.{seq}", std::process::id()));

    fs::write(&tmp_path, bytes)
        .map_err(|e| format!("写入临时文件 {} 失败: {e}", tmp_path.display()))?;

    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("替换目标文件 {} 失败: {err}", path.display()));
    }
    Ok(())
}

/// AI Credits 条目。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaCreditItem {
    pub credit_type: String,
    #[serde(default)]
    pub amount: Option<String>,
    #[serde(default)]
    pub min_for_usage: Option<String>,
}

/// 账号套餐等级信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanTierInfo {
    #[serde(default)]
    pub tier_id: Option<String>,
    pub tier_code: String,
    pub tier_label: String,
    #[serde(default)]
    pub current_tier_id: Option<String>,
    #[serde(default)]
    pub current_tier_name: Option<String>,
    #[serde(default)]
    pub paid_tier_id: Option<String>,
    #[serde(default)]
    pub paid_tier_name: Option<String>,
    #[serde(default)]
    pub entitlement_mismatch: bool,
    #[serde(default)]
    pub refresh_cadence_hint: String,
    #[serde(default)]
    pub ineligible_reason: Option<String>,
    #[serde(default)]
    pub credits: Vec<QuotaCreditItem>,
}

impl Default for PlanTierInfo {
    fn default() -> Self {
        Self {
            tier_id: None,
            tier_code: "unknown".to_string(),
            tier_label: "未知套餐".to_string(),
            current_tier_id: None,
            current_tier_name: None,
            paid_tier_id: None,
            paid_tier_name: None,
            entitlement_mismatch: false,
            refresh_cadence_hint: String::new(),
            ineligible_reason: None,
            credits: Vec::new(),
        }
    }
}

/// 来自 `retrieveUserQuotaSummary` 的原始窗口桶。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaBucketItem {
    pub group_id: String,
    pub group_label: String,
    pub bucket_id: String,
    pub label: String,
    #[serde(default)]
    pub window: Option<String>,
    #[serde(default)]
    pub remaining_fraction: Option<f64>,
    #[serde(default)]
    pub reset_at_ms: Option<i64>,
    #[serde(default)]
    pub reset_time_raw: Option<String>,
}

/// 来自 `fetchAvailableModels` 的单个模型额度项。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelQuotaItem {
    pub model_id: String,
    pub display_name: String,
    pub pool_key: String,
    pub pool_label: String,
    #[serde(default)]
    pub api_provider: Option<String>,
    #[serde(default)]
    pub recommended: bool,
    #[serde(default)]
    pub remaining_fraction: Option<f64>,
    #[serde(default)]
    pub reset_at_ms: Option<i64>,
    #[serde(default)]
    pub reset_time_raw: Option<String>,
}

/// 面向 UI 卡片展示的聚合额度池。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaPoolSummary {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub window: Option<String>,
    #[serde(default)]
    pub remaining_fraction: Option<f64>,
    #[serde(default)]
    pub reset_at_ms: Option<i64>,
    pub model_count: usize,
    pub source: String,
}

/// 单个账号的完整套餐与额度快照（不含任何令牌）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountQuotaSnapshot {
    pub email: String,
    #[serde(default)]
    pub account_id: Option<String>,
    pub fetched_at_ms: i64,
    #[serde(default)]
    pub host_used: Option<String>,
    #[serde(default)]
    pub via_proxy: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub plan: PlanTierInfo,
    #[serde(default)]
    pub pools: Vec<QuotaPoolSummary>,
    #[serde(default)]
    pub summary_buckets: Vec<QuotaBucketItem>,
    #[serde(default)]
    pub models: Vec<ModelQuotaItem>,
    #[serde(default)]
    pub min_remaining_fraction: Option<f64>,
    #[serde(default)]
    pub earliest_reset_at_ms: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// 仅存在于内存中的 OAuth Access Token 缓存。
#[derive(Default)]
pub struct TokenCache {
    entries: HashMap<String, CachedAccessToken>,
}

#[derive(Clone)]
struct CachedAccessToken {
    access_token: String,
    expires_at_ms: i64,
}

impl TokenCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn get_valid_with_expiry(&self, key: &str, now: i64) -> Option<(String, i64)> {
        let entry = self.entries.get(key)?;
        if entry.expires_at_ms > now + TOKEN_REFRESH_SKEW_MS && !entry.access_token.is_empty() {
            Some((entry.access_token.clone(), entry.expires_at_ms))
        } else {
            None
        }
    }

    fn set(&mut self, key: String, access_token: String, expires_at_ms: i64) {
        self.entries.insert(
            key,
            CachedAccessToken {
                access_token,
                expires_at_ms,
            },
        );
    }

    fn remove(&mut self, key: &str) {
        self.entries.remove(key);
    }
}

/// 本地非敏感额度快照持久化缓存（`%APPDATA%\antigravity-usage-dashboard\quota-cache.json`）。
pub struct QuotaCacheStore {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct QuotaCacheFile {
    #[serde(default = "default_cache_version")]
    version: u32,
    #[serde(default)]
    accounts: HashMap<String, AccountQuotaSnapshot>,
}

fn default_cache_version() -> u32 {
    1
}

impl QuotaCacheStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn default_store() -> Self {
        let base = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("antigravity-usage-dashboard");
        Self::new(base.join("quota-cache.json"))
    }

    pub fn load_all(&self) -> HashMap<String, AccountQuotaSnapshot> {
        let Ok(raw) = fs::read_to_string(&self.path) else {
            return HashMap::new();
        };
        let Ok(parsed) = serde_json::from_str::<QuotaCacheFile>(&raw) else {
            return HashMap::new();
        };
        parsed
            .accounts
            .into_iter()
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v))
            .collect()
    }

    fn save_map(&self, accounts: HashMap<String, AccountQuotaSnapshot>) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("无法创建额度缓存目录 {}: {e}", parent.display()))?;
        }
        let file = QuotaCacheFile {
            version: default_cache_version(),
            accounts,
        };
        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| format!("序列化额度缓存失败: {e}"))?;
        write_atomic(&self.path, json.as_bytes())
    }

    pub fn upsert(&self, snapshot: &AccountQuotaSnapshot) -> Result<(), String> {
        let key = snapshot.email.trim().to_ascii_lowercase();
        if key.is_empty() || key == "unknown" {
            return Ok(());
        }
        let mut map = self.load_all();
        map.insert(key, snapshot.clone());
        self.save_map(map)
    }

    pub fn upsert_many(&self, snapshots: &[AccountQuotaSnapshot]) -> Result<(), String> {
        let mut map = self.load_all();
        let mut changed = false;
        for snap in snapshots {
            if snap.error.is_some() {
                continue;
            }
            let key = snap.email.trim().to_ascii_lowercase();
            if key.is_empty() || key == "unknown" {
                continue;
            }
            map.insert(key, snap.clone());
            changed = true;
        }
        if changed {
            self.save_map(map)?;
        }
        Ok(())
    }
}

/// 构造 WinHTTP 所需的代理地址格式（不带 `http://` 前缀）。
pub fn format_winhttp_proxy(host: &str, port: u16) -> String {
    let trimmed = host.trim();
    let effective_host = if trimmed.is_empty() { "127.0.0.1" } else { trimmed };
    if effective_host.contains(':')
        && !(effective_host.starts_with('[') && effective_host.ends_with(']'))
    {
        format!("[{effective_host}]:{port}")
    } else {
        format!("{effective_host}:{port}")
    }
}

/// RFC 3986 表单参数编码。
pub fn url_encode_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    for &b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(Debug, Clone)]
struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

#[cfg(windows)]
mod winhttp_client {
    use super::HttpResponse;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Networking::WinHttp::{
        WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
        WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData,
        WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ACCESS_TYPE_NAMED_PROXY,
        WINHTTP_ADDREQ_FLAG_ADD, WINHTTP_ADDREQ_FLAG_REPLACE, WINHTTP_DECOMPRESSION_FLAG_DEFLATE,
        WINHTTP_DECOMPRESSION_FLAG_GZIP, WINHTTP_FLAG_SECURE, WINHTTP_OPTION_DECOMPRESSION,
        WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    };

    const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

    struct WinHttpHandle(*mut core::ffi::c_void);

    impl Drop for WinHttpHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }

    fn map_winhttp_error(context: &str, code: u32) -> String {
        let hint = match code {
            12002 => "请求超时（请检查代理或网络连通性）",
            12007 => "无法解析域名（DNS 失败或未走代理）",
            12029 => "无法连接到服务器或本地代理端口",
            12030 => "网络连接被对端重置或中断",
            12152 => "服务器或代理返回了无效响应",
            12157 | 12175 => "TLS 安全通道握手失败",
            _ => "WinHTTP 网络调用失败",
        };
        format!("{context}: {hint} (WinHTTP {code})")
    }

    pub fn post_https(
        host: &str,
        path: &str,
        user_agent: &str,
        extra_headers: &[(&str, &str)],
        body: &[u8],
        proxy: Option<&str>,
    ) -> Result<HttpResponse, String> {
        let ua_w = wide(user_agent);
        let proxy_w = proxy.map(wide);
        let (access_type, proxy_ptr) = match &proxy_w {
            Some(w) => (WINHTTP_ACCESS_TYPE_NAMED_PROXY, w.as_ptr()),
            None => (WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, ptr::null()),
        };

        let session = WinHttpHandle(unsafe {
            WinHttpOpen(ua_w.as_ptr(), access_type, proxy_ptr, ptr::null(), 0)
        });
        if session.0.is_null() {
            return Err(map_winhttp_error(
                "初始化 HTTP 会话失败",
                unsafe { GetLastError() },
            ));
        }

        unsafe {
            WinHttpSetTimeouts(session.0, 8_000, 8_000, 12_000, 15_000);
        }

        let host_w = wide(host);
        let connect = WinHttpHandle(unsafe { WinHttpConnect(session.0, host_w.as_ptr(), 443, 0) });
        if connect.0.is_null() {
            return Err(map_winhttp_error(
                &format!("连接 {host} 失败"),
                unsafe { GetLastError() },
            ));
        }

        let verb_w = wide("POST");
        let path_w = wide(path);
        let request = WinHttpHandle(unsafe {
            WinHttpOpenRequest(
                connect.0,
                verb_w.as_ptr(),
                path_w.as_ptr(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                WINHTTP_FLAG_SECURE,
            )
        });
        if request.0.is_null() {
            return Err(map_winhttp_error(
                "创建 HTTPS 请求失败",
                unsafe { GetLastError() },
            ));
        }

        let decomp: u32 = WINHTTP_DECOMPRESSION_FLAG_GZIP | WINHTTP_DECOMPRESSION_FLAG_DEFLATE;
        unsafe {
            WinHttpSetOption(
                request.0,
                WINHTTP_OPTION_DECOMPRESSION,
                &decomp as *const u32 as *const core::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
            );
        }

        for (k, v) in extra_headers {
            let line = wide(&format!("{k}: {v}\r\n"));
            let ok = unsafe {
                WinHttpAddRequestHeaders(
                    request.0,
                    line.as_ptr(),
                    u32::MAX,
                    WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE,
                )
            };
            if ok == 0 {
                return Err(map_winhttp_error(
                    "设置请求头失败",
                    unsafe { GetLastError() },
                ));
            }
        }

        let body_ptr = if body.is_empty() {
            ptr::null()
        } else {
            body.as_ptr() as *const core::ffi::c_void
        };
        let body_len = body.len() as u32;
        let sent = unsafe {
            WinHttpSendRequest(
                request.0,
                ptr::null(),
                0,
                body_ptr,
                body_len,
                body_len,
                0,
            )
        };
        if sent == 0 {
            return Err(map_winhttp_error(
                &format!("发送请求到 {host} 失败"),
                unsafe { GetLastError() },
            ));
        }

        let received = unsafe { WinHttpReceiveResponse(request.0, ptr::null_mut()) };
        if received == 0 {
            return Err(map_winhttp_error(
                &format!("等待 {host} 响应失败"),
                unsafe { GetLastError() },
            ));
        }

        let mut status_code: u32 = 0;
        let mut status_size: u32 = std::mem::size_of::<u32>() as u32;
        let status_ok = unsafe {
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                ptr::null(),
                &mut status_code as *mut u32 as *mut core::ffi::c_void,
                &mut status_size,
                ptr::null_mut(),
            )
        };
        if status_ok == 0 {
            return Err(map_winhttp_error(
                "读取 HTTP 状态码失败",
                unsafe { GetLastError() },
            ));
        }

        let mut out = Vec::new();
        loop {
            let mut avail: u32 = 0;
            let q_ok = unsafe { WinHttpQueryDataAvailable(request.0, &mut avail) };
            if q_ok == 0 {
                return Err(map_winhttp_error(
                    "读取响应长度失败",
                    unsafe { GetLastError() },
                ));
            }
            if avail == 0 {
                break;
            }
            if out.len().saturating_add(avail as usize) > MAX_RESPONSE_BYTES {
                return Err("服务器响应体超过 4MB 限制".to_string());
            }

            let mut buf = vec![0u8; avail as usize];
            let mut read: u32 = 0;
            let r_ok = unsafe {
                WinHttpReadData(
                    request.0,
                    buf.as_mut_ptr() as *mut core::ffi::c_void,
                    avail,
                    &mut read,
                )
            };
            if r_ok == 0 {
                return Err(map_winhttp_error(
                    "读取响应数据失败",
                    unsafe { GetLastError() },
                ));
            }
            if read == 0 {
                break;
            }
            out.extend_from_slice(&buf[..read as usize]);
        }

        Ok(HttpResponse {
            status: status_code as u16,
            body: out,
        })
    }
}

#[cfg(not(windows))]
mod winhttp_client {
    use super::HttpResponse;

    pub fn post_https(
        _host: &str,
        _path: &str,
        _user_agent: &str,
        _extra_headers: &[(&str, &str)],
        _body: &[u8],
        _proxy: Option<&str>,
    ) -> Result<HttpResponse, String> {
        Err("套餐额度查询仅支持 Windows".to_string())
    }
}

fn non_empty_str(val: Option<&Value>) -> Option<String> {
    val.and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn value_as_string(val: Option<&Value>) -> Option<String> {
    let v = val?;
    if let Some(s) = v.as_str() {
        let trimmed = s.trim();
        return (!trimmed.is_empty()).then(|| trimmed.to_string());
    }
    if let Some(i) = v.as_i64() {
        return Some(i.to_string());
    }
    if let Some(u) = v.as_u64() {
        return Some(u.to_string());
    }
    if let Some(f) = v.as_f64() {
        if f.is_finite() {
            return if f.fract() == 0.0 {
                Some(format!("{f:.0}"))
            } else {
                Some(format!("{f}"))
            };
        }
    }
    None
}

fn parse_rfc3339_ms(raw: Option<&str>) -> Option<i64> {
    let s = raw?.trim();
    if s.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn normalize_tier_code(tier_id: Option<&str>, tier_name: Option<&str>) -> String {
    let combined = format!(
        "{} {}",
        tier_id.unwrap_or_default(),
        tier_name.unwrap_or_default()
    )
    .to_ascii_lowercase();
    if combined.contains("ultra") {
        "ultra".to_string()
    } else if combined.contains("pro") {
        "pro".to_string()
    } else if combined.contains("standard")
        || combined.contains("enterprise")
        || combined.contains("business")
    {
        "standard".to_string()
    } else if combined.contains("free")
        || combined.contains("starter")
        || combined.contains("legacy")
    {
        "free".to_string()
    } else {
        "unknown".to_string()
    }
}

fn default_tier_label(tier_code: &str, raw_name: Option<&str>, raw_id: Option<&str>) -> String {
    match tier_code {
        "ultra" => raw_name
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "Google AI Ultra".to_string()),
        "pro" => raw_name
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "Google AI Pro".to_string()),
        "standard" => raw_name
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "Gemini Code Assist".to_string()),
        "free" => "Free".to_string(),
        _ => raw_name
            .filter(|s| !s.is_empty())
            .or(raw_id.filter(|s| !s.is_empty()))
            .unwrap_or("未知套餐")
            .to_string(),
    }
}

fn cadence_hint_for_tier(tier_code: &str) -> String {
    match tier_code {
        "pro" | "ultra" => "5h 滚动窗口 + 周额度".to_string(),
        "free" => "按周刷新".to_string(),
        _ => String::new(),
    }
}

fn push_credit_entries(out: &mut Vec<QuotaCreditItem>, val: Option<&Value>) {
    let Some(v) = val else {
        return;
    };
    match v {
        Value::Array(items) => {
            for item in items {
                let credit_type = non_empty_str(item.get("creditType"))
                    .unwrap_or_else(|| "GOOGLE_ONE_AI".to_string());
                let amount = value_as_string(item.get("creditAmount"))
                    .or_else(|| value_as_string(item.get("amount")));
                let min_for_usage = value_as_string(item.get("minimumCreditAmountForUsage"));
                out.push(QuotaCreditItem {
                    credit_type,
                    amount,
                    min_for_usage,
                });
            }
        }
        Value::Number(_) | Value::String(_) => {
            if let Some(amount) = value_as_string(Some(v)) {
                out.push(QuotaCreditItem {
                    credit_type: "GOOGLE_ONE_AI".to_string(),
                    amount: Some(amount),
                    min_for_usage: None,
                });
            }
        }
        _ => {}
    }
}

/// 解析 `/v1internal:loadCodeAssist` 响应为 `(project_id, PlanTierInfo)`。
pub fn parse_load_code_assist(val: &Value) -> (Option<String>, PlanTierInfo) {
    let project_id = non_empty_str(val.get("cloudaicompanionProject")).or_else(|| {
        val.get("cloudaicompanionProject")
            .and_then(|p| non_empty_str(p.get("id")))
    });

    let current = val.get("currentTier");
    let current_tier_id = current.and_then(|t| non_empty_str(t.get("id")));
    let current_tier_name = current.and_then(|t| non_empty_str(t.get("name")));

    let paid = val.get("paidTier");
    let paid_tier_id = paid.and_then(|t| non_empty_str(t.get("id")));
    let paid_tier_name = paid.and_then(|t| non_empty_str(t.get("name")));

    let current_code = normalize_tier_code(current_tier_id.as_deref(), current_tier_name.as_deref());
    let paid_code = normalize_tier_code(paid_tier_id.as_deref(), paid_tier_name.as_deref());

    // 个人 Google One 订阅（Pro / Ultra）在 loadCodeAssist 中会同时返回
    // currentTier = free-tier（基础消费级档位）与 paidTier = g1-pro-tier/g1-ultra-tier（订阅档位），
    // 属于服务端正常结构，不应判作异常。
    let entitlement_mismatch = false;

    // 优先展示 paidTier，否则回退 currentTier
    let (tier_id, tier_code, tier_label) = if paid_tier_id.is_some() || paid_tier_name.is_some() {
        let label = default_tier_label(
            &paid_code,
            paid_tier_name.as_deref(),
            paid_tier_id.as_deref(),
        );
        (paid_tier_id.clone(), paid_code.clone(), label)
    } else if current_tier_id.is_some() || current_tier_name.is_some() {
        let label = default_tier_label(
            &current_code,
            current_tier_name.as_deref(),
            current_tier_id.as_deref(),
        );
        (current_tier_id.clone(), current_code.clone(), label)
    } else {
        (None, "unknown".to_string(), "未知套餐".to_string())
    };

    let mut credits = Vec::new();
    push_credit_entries(&mut credits, paid.and_then(|p| p.get("availableCredits")));
    if credits.is_empty() {
        push_credit_entries(&mut credits, current.and_then(|c| c.get("availableCredits")));
    }
    if credits.is_empty() {
        push_credit_entries(
            &mut credits,
            val.get("availableCredits").or_else(|| val.get("credits")),
        );
    }

    let ineligible_reason = val
        .get("ineligibleTiers")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| {
            non_empty_str(item.get("reasonMessage")).or_else(|| non_empty_str(item.get("reasonCode")))
        });

    let refresh_cadence_hint = cadence_hint_for_tier(&tier_code);

    let plan = PlanTierInfo {
        tier_id,
        tier_code,
        tier_label,
        current_tier_id,
        current_tier_name,
        paid_tier_id,
        paid_tier_name,
        entitlement_mismatch,
        refresh_cadence_hint,
        ineligible_reason,
        credits,
    };

    (project_id, plan)
}

fn clamp_fraction(v: f64) -> Option<f64> {
    if v.is_finite() {
        Some(v.clamp(0.0, 1.0))
    } else {
        None
    }
}

fn extract_bucket_fraction(b: &Value) -> Option<f64> {
    if let Some(f) = b.get("remainingFraction").and_then(|v| v.as_f64()) {
        return clamp_fraction(f);
    }
    if let Some(used) = b.get("usedFraction").and_then(|v| v.as_f64()) {
        return clamp_fraction(1.0 - used);
    }
    if let Some(pct) = b.get("remainingPercentage").and_then(|v| v.as_f64()) {
        return clamp_fraction(pct / 100.0);
    }
    None
}

fn parse_bucket_entry(group_id: &str, group_label: &str, b: &Value) -> QuotaBucketItem {
    let bucket_id = non_empty_str(b.get("bucketId"))
        .or_else(|| non_empty_str(b.get("id")))
        .unwrap_or_else(|| group_id.to_string());
    let label = non_empty_str(b.get("displayName"))
        .or_else(|| non_empty_str(b.get("label")))
        .unwrap_or_else(|| bucket_id.clone());
    let window = non_empty_str(b.get("window"));
    let remaining_fraction = extract_bucket_fraction(b);
    let reset_time_raw = non_empty_str(b.get("resetTime"))
        .or_else(|| non_empty_str(b.get("resetTimestamp")));
    let reset_at_ms = parse_rfc3339_ms(reset_time_raw.as_deref());

    QuotaBucketItem {
        group_id: group_id.to_string(),
        group_label: group_label.to_string(),
        bucket_id,
        label,
        window,
        remaining_fraction,
        reset_at_ms,
        reset_time_raw,
    }
}

/// 解析 `/v1internal:retrieveUserQuotaSummary` 响应。
pub fn parse_quota_summary(val: &Value) -> Vec<QuotaBucketItem> {
    let root = val.get("response").unwrap_or(val);
    let mut out = Vec::new();

    if let Some(groups) = root
        .get("groups")
        .or_else(|| root.get("quotaGroups"))
        .and_then(|v| v.as_array())
    {
        for group in groups {
            let group_id = non_empty_str(group.get("id")).unwrap_or_else(|| "default".to_string());
            let group_label = non_empty_str(group.get("displayName"))
                .unwrap_or_else(|| group_id.clone());
            if let Some(buckets) = group
                .get("buckets")
                .or_else(|| group.get("quotaBuckets"))
                .and_then(|v| v.as_array())
            {
                for b in buckets {
                    out.push(parse_bucket_entry(&group_id, &group_label, b));
                }
            }
        }
    }

    if out.is_empty() {
        if let Some(buckets) = root
            .get("buckets")
            .or_else(|| root.get("quotaBuckets"))
            .and_then(|v| v.as_array())
        {
            for b in buckets {
                out.push(parse_bucket_entry("default", "默认额度池", b));
            }
        }
    }

    out
}

fn classify_model_pool(
    model_id: &str,
    display_name: &str,
    api_provider: Option<&str>,
) -> (&'static str, &'static str) {
    let lower_id = model_id.to_ascii_lowercase();
    let lower_name = display_name.to_ascii_lowercase();
    let lower_provider = api_provider.unwrap_or_default().to_ascii_lowercase();

    if lower_id.contains("claude")
        || lower_id.contains("gpt")
        || lower_id.contains("oss")
        || lower_name.contains("claude")
        || lower_name.contains("gpt")
        || lower_provider.contains("anthropic")
        || lower_provider.contains("openai")
        || lower_provider.contains("third_party")
    {
        return ("third_party", "Claude / GPT");
    }
    if lower_id.contains("image") || lower_id.contains("imagen") || lower_name.contains("image") {
        return ("image", "Image 模型");
    }
    if lower_id.contains("gemini") || lower_name.contains("gemini") {
        if lower_id.contains("flash")
            || lower_id.contains("lite")
            || lower_name.contains("flash")
            || lower_name.contains("lite")
        {
            return ("gemini_flash", "Gemini Flash");
        }
        return ("gemini_pro", "Gemini Pro");
    }
    ("other", "其他模型")
}

fn pool_sort_rank(pool_key: &str) -> u8 {
    match pool_key {
        "gemini_pro" => 0,
        "gemini_flash" => 1,
        "third_party" => 2,
        "image" => 3,
        _ => 4,
    }
}

/// 解析 `/v1internal:fetchAvailableModels` 响应中的模型额度列表。
pub fn parse_available_models(val: &Value) -> Vec<ModelQuotaItem> {
    let Some(models_obj) = val.get("models").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (model_id, raw) in models_obj {
        if raw
            .get("isInternal")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            continue;
        }
        let Some(quota_info) = raw.get("quotaInfo") else {
            continue;
        };

        let reset_time_raw = non_empty_str(quota_info.get("resetTime"));
        let reset_at_ms = parse_rfc3339_ms(reset_time_raw.as_deref());
        let remaining_fraction = if let Some(f) =
            quota_info.get("remainingFraction").and_then(|v| v.as_f64())
        {
            clamp_fraction(f)
        } else if reset_time_raw.is_some() {
            // 模型耗尽时服务端常省略 remainingFraction，仅保留 resetTime
            Some(0.0)
        } else {
            None
        };

        let display_name =
            non_empty_str(raw.get("displayName")).unwrap_or_else(|| model_id.clone());
        let api_provider = non_empty_str(raw.get("apiProvider"));
        let recommended = raw
            .get("recommended")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let (pool_key, pool_label) =
            classify_model_pool(model_id, &display_name, api_provider.as_deref());

        out.push(ModelQuotaItem {
            model_id: model_id.clone(),
            display_name,
            pool_key: pool_key.to_string(),
            pool_label: pool_label.to_string(),
            api_provider,
            recommended,
            remaining_fraction,
            reset_at_ms,
            reset_time_raw,
        });
    }

    out.sort_by(|a, b| {
        b.recommended
            .cmp(&a.recommended)
            .then_with(|| pool_sort_rank(&a.pool_key).cmp(&pool_sort_rank(&b.pool_key)))
            .then_with(|| {
                let fa = a.remaining_fraction.unwrap_or(2.0);
                let fb = b.remaining_fraction.unwrap_or(2.0);
                fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.display_name.cmp(&b.display_name))
    });

    out
}

fn clean_summary_bucket_label(label: &str) -> &str {
    let trimmed = label.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower == "five hour limit remaining" || lower == "5-hour limit remaining" {
        "5h 额度"
    } else if lower == "weekly limit remaining" {
        "周额度"
    } else {
        trimmed
    }
}

fn summary_group_rank(group_id: &str, group_label: &str) -> u8 {
    let combined = format!("{group_id} {group_label}").to_ascii_lowercase();
    if combined.contains("gemini") {
        0
    } else if combined.contains("claude") || combined.contains("gpt") || combined.contains("3p") {
        1
    } else {
        2
    }
}

fn summary_window_rank(window: Option<&str>, bucket_id: &str) -> u8 {
    let combined = format!("{} {bucket_id}", window.unwrap_or_default()).to_ascii_lowercase();
    if combined.contains("5h") {
        0
    } else if combined.contains("weekly") || combined.contains("7d") {
        1
    } else {
        2
    }
}

/// 从 `summary_buckets` 或 `models` 汇总出展示用的额度池列表。
pub fn build_quota_pools(
    buckets: &[QuotaBucketItem],
    models: &[ModelQuotaItem],
) -> Vec<QuotaPoolSummary> {
    if !buckets.is_empty() {
        let mut sorted: Vec<&QuotaBucketItem> = buckets.iter().collect();
        sorted.sort_by(|a, b| {
            summary_group_rank(&a.group_id, &a.group_label)
                .cmp(&summary_group_rank(&b.group_id, &b.group_label))
                .then_with(|| a.group_label.cmp(&b.group_label))
                .then_with(|| {
                    summary_window_rank(a.window.as_deref(), &a.bucket_id)
                        .cmp(&summary_window_rank(b.window.as_deref(), &b.bucket_id))
                })
                .then_with(|| a.bucket_id.cmp(&b.bucket_id))
        });

        return sorted
            .into_iter()
            .map(|b| {
                let cleaned = clean_summary_bucket_label(&b.label);
                let label = if b.group_label == cleaned || cleaned.is_empty() {
                    b.group_label.clone()
                } else {
                    format!("{} · {}", b.group_label, cleaned)
                };
                QuotaPoolSummary {
                    id: format!("{}:{}", b.group_id, b.bucket_id),
                    label,
                    window: b.window.clone(),
                    remaining_fraction: b.remaining_fraction,
                    reset_at_ms: b.reset_at_ms,
                    model_count: 0,
                    source: "summary".to_string(),
                }
            })
            .collect();
    }

    struct PoolAcc {
        label: String,
        min_fraction: Option<f64>,
        earliest_reset: Option<i64>,
        count: usize,
    }

    let mut map: HashMap<String, PoolAcc> = HashMap::new();
    for m in models {
        let acc = map.entry(m.pool_key.clone()).or_insert_with(|| PoolAcc {
            label: m.pool_label.clone(),
            min_fraction: None,
            earliest_reset: None,
            count: 0,
        });
        acc.count += 1;
        if let Some(f) = m.remaining_fraction {
            acc.min_fraction = Some(match acc.min_fraction {
                Some(cur) => cur.min(f),
                None => f,
            });
        }
        if let Some(rt) = m.reset_at_ms {
            acc.earliest_reset = Some(match acc.earliest_reset {
                Some(cur) => cur.min(rt),
                None => rt,
            });
        }
    }

    let mut pools: Vec<QuotaPoolSummary> = map
        .into_iter()
        .map(|(id, acc)| QuotaPoolSummary {
            id,
            label: acc.label,
            window: None,
            remaining_fraction: acc.min_fraction,
            reset_at_ms: acc.earliest_reset,
            model_count: acc.count,
            source: "models".to_string(),
        })
        .collect();

    pools.sort_by_key(|p| pool_sort_rank(&p.id));
    pools
}

fn compute_summary_metrics(
    pools: &[QuotaPoolSummary],
    models: &[ModelQuotaItem],
    now: i64,
) -> (Option<f64>, Option<i64>) {
    let mut min_frac: Option<f64> = None;
    let mut earliest_future: Option<i64> = None;
    let mut earliest_any: Option<i64> = None;

    let mut consider = |frac: Option<f64>, reset: Option<i64>| {
        if let Some(f) = frac {
            min_frac = Some(match min_frac {
                Some(cur) => cur.min(f),
                None => f,
            });
        }
        if let Some(r) = reset {
            earliest_any = Some(match earliest_any {
                Some(cur) => cur.min(r),
                None => r,
            });
            if r > now {
                earliest_future = Some(match earliest_future {
                    Some(cur) => cur.min(r),
                    None => r,
                });
            }
        }
    };

    for p in pools {
        consider(p.remaining_fraction, p.reset_at_ms);
    }
    if pools.is_empty() {
        for m in models {
            consider(m.remaining_fraction, m.reset_at_ms);
        }
    }

    (min_frac, earliest_future.or(earliest_any))
}

const OAUTH_CONFIG_BIN_NAME: &str = "oauth-client.bin";
const OAUTH_CONFIG_JSON_NAME: &str = "oauth-client.json";

fn default_config_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("antigravity-usage-dashboard"))
}

/// 本地 OAuth 刷新凭据配置状态（绝不向前端回传密钥原文）。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OAuthClientStatus {
    pub configured: bool,
    pub source: String,
    pub client_id_preview: Option<String>,
    pub config_path: String,
}

pub fn mask_client_id(id: &str) -> String {
    let trimmed = id.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() > 18 {
        let head: String = chars[..8].iter().collect();
        let tail: String = chars[chars.len() - 6..].iter().collect();
        format!("{head}...{tail}")
    } else if chars.len() > 6 {
        let head: String = chars[..4].iter().collect();
        format!("{head}...")
    } else {
        "***".to_string()
    }
}

pub fn parse_oauth_client_config_json(raw: &str) -> Option<(String, String)> {
    let val: Value = serde_json::from_str(raw).ok()?;
    let id = non_empty_str(val.get("clientId")).or_else(|| non_empty_str(val.get("client_id")))?;
    let secret = non_empty_str(val.get("clientSecret"))
        .or_else(|| non_empty_str(val.get("client_secret")))?;
    Some((id, secret))
}

fn load_oauth_client_config_from_env() -> Option<(String, String)> {
    let id = std::env::var("AGYDOCK_OAUTH_CLIENT_ID").ok()?;
    let secret = std::env::var("AGYDOCK_OAUTH_CLIENT_SECRET").ok()?;
    let id = id.trim().to_string();
    let secret = secret.trim().to_string();
    if !id.is_empty() && !secret.is_empty() {
        Some((id, secret))
    } else {
        None
    }
}

fn load_oauth_client_config_from_dir(base_dir: &Path) -> Option<(String, String, &'static str)> {
    let bin_path = base_dir.join(OAUTH_CONFIG_BIN_NAME);
    if let Ok(encrypted) = fs::read(&bin_path) {
        if !encrypted.is_empty() {
            if let Ok(plain) = credential::unprotect(&encrypted) {
                if let Ok(text) = String::from_utf8(plain) {
                    if let Some((id, secret)) = parse_oauth_client_config_json(&text) {
                        return Some((id, secret, "dpapi"));
                    }
                }
            }
        }
    }

    let json_path = base_dir.join(OAUTH_CONFIG_JSON_NAME);
    if let Ok(raw) = fs::read_to_string(&json_path) {
        if let Some((id, secret)) = parse_oauth_client_config_json(&raw) {
            return Some((id, secret, "file"));
        }
    }

    None
}

fn load_oauth_client_config_in(base_dir: Option<&Path>) -> Option<(String, String, &'static str)> {
    if let Some((id, secret)) = load_oauth_client_config_from_env() {
        return Some((id, secret, "env"));
    }
    base_dir.and_then(load_oauth_client_config_from_dir)
}

fn load_oauth_client_config() -> Option<(String, String)> {
    load_oauth_client_config_in(default_config_dir().as_deref()).map(|(id, secret, _)| (id, secret))
}

fn get_oauth_client_status_in(base_dir: Option<&Path>) -> OAuthClientStatus {
    let dpapi_path = base_dir
        .map(|d| d.join(OAUTH_CONFIG_BIN_NAME).to_string_lossy().to_string())
        .unwrap_or_default();

    match load_oauth_client_config_in(base_dir) {
        Some((id, _, source)) => {
            let active_path = if source == "file" {
                base_dir
                    .map(|d| d.join(OAUTH_CONFIG_JSON_NAME).to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                dpapi_path
            };
            OAuthClientStatus {
                configured: true,
                source: source.to_string(),
                client_id_preview: Some(mask_client_id(&id)),
                config_path: active_path,
            }
        }
        None => OAuthClientStatus {
            configured: false,
            source: "none".to_string(),
            client_id_preview: None,
            config_path: dpapi_path,
        },
    }
}

pub fn get_oauth_client_status() -> OAuthClientStatus {
    get_oauth_client_status_in(default_config_dir().as_deref())
}

fn save_oauth_client_config_in(
    base_dir: &Path,
    client_id: &str,
    client_secret: &str,
) -> Result<OAuthClientStatus, String> {
    let id = client_id.trim();
    let secret = client_secret.trim();
    if id.is_empty() || secret.is_empty() {
        return Err("Client ID 和 Client Secret 不能为空".to_string());
    }

    fs::create_dir_all(base_dir)
        .map_err(|e| format!("无法创建配置目录 {}: {e}", base_dir.display()))?;
    let payload = json!({
        "clientId": id,
        "clientSecret": secret,
    });
    let bytes = serde_json::to_vec(&payload)
        .map_err(|e| format!("序列化 OAuth 刷新凭据失败: {e}"))?;
    let encrypted = credential::protect(&bytes)?;
    write_atomic(&base_dir.join(OAUTH_CONFIG_BIN_NAME), &encrypted)?;

    // 若存在历史明文 oauth-client.json，保存 DPAPI 密文后自动清理
    let legacy_json = base_dir.join(OAUTH_CONFIG_JSON_NAME);
    if legacy_json.exists() {
        let _ = fs::remove_file(&legacy_json);
    }

    Ok(get_oauth_client_status_in(Some(base_dir)))
}

pub fn save_oauth_client_config(
    client_id: &str,
    client_secret: &str,
) -> Result<OAuthClientStatus, String> {
    let base_dir = default_config_dir().ok_or_else(|| "无法定位本地配置目录".to_string())?;
    save_oauth_client_config_in(&base_dir, client_id, client_secret)
}

fn clear_oauth_client_config_in(base_dir: &Path) -> Result<OAuthClientStatus, String> {
    for name in [OAUTH_CONFIG_BIN_NAME, OAUTH_CONFIG_JSON_NAME] {
        let path = base_dir.join(name);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!("删除 {} 失败: {err}", path.display()));
            }
        }
    }
    Ok(get_oauth_client_status_in(Some(base_dir)))
}

pub fn clear_oauth_client_config() -> Result<OAuthClientStatus, String> {
    let base_dir = default_config_dir().ok_or_else(|| "无法定位本地配置目录".to_string())?;
    clear_oauth_client_config_in(&base_dir)
}

fn refresh_oauth_access_token(
    refresh_token: &str,
    proxy: Option<&str>,
) -> Result<(String, i64), String> {
    let (client_id, client_secret) = load_oauth_client_config().ok_or_else(|| {
        "缓存的 access_token 已过期。请启动 Antigravity 或 T3 Code 自动续期，或在「账号管理 → 离线续期」中配置本地 OAuth 刷新凭据".to_string()
    })?;

    let body = format!(
        "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
        url_encode_component(&client_id),
        url_encode_component(&client_secret),
        url_encode_component(refresh_token),
    );
    let headers = [(
        "Content-Type",
        "application/x-www-form-urlencoded",
    )];
    let resp = winhttp_client::post_https(
        OAUTH_HOST,
        OAUTH_PATH,
        DEFAULT_USER_AGENT,
        &headers,
        body.as_bytes(),
        proxy,
    )?;

    let parsed: Value = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);
    if !(200..300).contains(&resp.status) {
        let code = non_empty_str(parsed.get("error"));
        let desc = non_empty_str(parsed.get("error_description"));
        if code.as_deref() == Some("invalid_grant") {
            return Err("刷新令牌已失效，请在 Antigravity 中重新登录".to_string());
        }
        let detail = match (code, desc) {
            (Some(c), Some(d)) => format!("{c}: {d}"),
            (Some(c), None) => c,
            (None, Some(d)) => d,
            (None, None) => format!("HTTP {}", resp.status),
        };
        return Err(format!("刷新 OAuth 令牌失败 ({detail})"));
    }

    let access_token = non_empty_str(parsed.get("access_token"))
        .ok_or_else(|| "OAuth 响应缺少 access_token".to_string())?;
    let ttl_sec = parsed
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .filter(|&s| s > 0)
        .unwrap_or(DEFAULT_TOKEN_TTL_SEC);
    let expires_at_ms = now_ms().saturating_add(ttl_sec.saturating_mul(1000));
    Ok((access_token, expires_at_ms))
}

fn resolve_access_token(
    token_cache: &Arc<Mutex<TokenCache>>,
    cache_key: &str,
    tokens: &CredentialTokens,
    proxy: Option<&str>,
    force_refresh: bool,
) -> Result<(String, bool), String> {
    let now = now_ms();
    let can_refresh = tokens.refresh_token.is_some() && load_oauth_client_config().is_some();
    if !(force_refresh && can_refresh) {
        let cached = token_cache
            .lock()
            .ok()
            .and_then(|guard| guard.get_valid_with_expiry(cache_key, now));
        let disk_valid = tokens.access_token.as_ref().and_then(|tok| {
            let valid = tokens
                .expires_at_ms
                .map(|exp| exp > now + TOKEN_REFRESH_SKEW_MS)
                .unwrap_or(true);
            valid.then(|| (tok.clone(), tokens.expires_at_ms))
        });

        match (cached, disk_valid) {
            (Some((_ctok, cexp)), Some((dtok, Some(dexp)))) if dexp > cexp => {
                if let Ok(mut guard) = token_cache.lock() {
                    guard.set(cache_key.to_string(), dtok.clone(), dexp);
                }
                return Ok((dtok, false));
            }
            (Some((ctok, _)), _) => {
                return Ok((ctok, false));
            }
            (None, Some((dtok, dexp_opt))) => {
                if let Some(dexp) = dexp_opt {
                    if let Ok(mut guard) = token_cache.lock() {
                        guard.set(cache_key.to_string(), dtok.clone(), dexp);
                    }
                }
                return Ok((dtok, false));
            }
            (None, None) => {}
        }
    }

    let refresh_token = tokens.refresh_token.as_deref().ok_or_else(|| {
        "凭据已过期且缺少 refresh_token，请在 Antigravity 中重新登录".to_string()
    })?;
    let (fresh, exp_ms) = refresh_oauth_access_token(refresh_token, proxy)?;
    if let Ok(mut guard) = token_cache.lock() {
        guard.set(cache_key.to_string(), fresh.clone(), exp_ms);
    }
    Ok((fresh, true))
}

fn parse_cloudcode_error_detail(status: u16, body: &[u8]) -> String {
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        if let Some(msg) = v
            .get("error")
            .and_then(|e| non_empty_str(e.get("message")))
            .or_else(|| non_empty_str(v.get("message")))
        {
            return format!("HTTP {status}: {msg}");
        }
    }
    format!("HTTP {status}")
}

fn post_cloudcode_single(
    host: &str,
    path: &str,
    access_token: &str,
    body_bytes: &[u8],
    proxy: Option<&str>,
) -> Result<HttpResponse, String> {
    let auth_header = format!("Bearer {access_token}");
    let headers = [
        ("Authorization", auth_header.as_str()),
        ("Content-Type", "application/json"),
        ("Accept-Encoding", "gzip"),
        ("X-Goog-Api-Client", API_CLIENT_HEADER),
        ("Client-Metadata", CLIENT_METADATA_HEADER),
    ];
    winhttp_client::post_https(host, path, DEFAULT_USER_AGENT, &headers, body_bytes, proxy)
}

fn post_cloudcode_with_retry(
    hosts: &[&str],
    path: &str,
    payload: &Value,
    access_token: &mut String,
    already_refreshed: &mut bool,
    tokens: &CredentialTokens,
    token_cache: &Arc<Mutex<TokenCache>>,
    cache_key: &str,
    proxy: Option<&str>,
) -> Result<(String, Value), String> {
    let body_bytes = serde_json::to_vec(payload).map_err(|e| format!("序列化请求失败: {e}"))?;
    let mut last_err = String::new();

    for &host in hosts {
        match post_cloudcode_single(host, path, access_token, &body_bytes, proxy) {
            Ok(mut resp) => {
                if resp.status == 401 && !*already_refreshed {
                    if let Some(rt) = tokens.refresh_token.as_deref() {
                        if let Ok(mut guard) = token_cache.lock() {
                            guard.remove(cache_key);
                        }
                        match refresh_oauth_access_token(rt, proxy) {
                            Ok((fresh, exp_ms)) => {
                                *already_refreshed = true;
                                *access_token = fresh.clone();
                                if let Ok(mut guard) = token_cache.lock() {
                                    guard.set(cache_key.to_string(), fresh, exp_ms);
                                }
                                if let Ok(retried) = post_cloudcode_single(
                                    host,
                                    path,
                                    access_token,
                                    &body_bytes,
                                    proxy,
                                ) {
                                    resp = retried;
                                }
                            }
                            Err(refresh_err) => {
                                return Err(refresh_err);
                            }
                        }
                    }
                }

                if (200..300).contains(&resp.status) {
                    let parsed: Value = serde_json::from_slice(&resp.body)
                        .map_err(|e| format!("解析 {host} 响应 JSON 失败: {e}"))?;
                    return Ok((host.to_string(), parsed));
                }

                last_err = parse_cloudcode_error_detail(resp.status, &resp.body);
                if resp.status == 401 || resp.status == 403 {
                    break;
                }
            }
            Err(err) => {
                last_err = err;
            }
        }
    }

    Err(if last_err.is_empty() {
        format!("请求 {path} 失败")
    } else {
        last_err
    })
}

pub fn error_snapshot_with_fallback(
    email: String,
    account_id: Option<String>,
    via_proxy: Option<String>,
    error: String,
    cached_fallback: Option<AccountQuotaSnapshot>,
) -> AccountQuotaSnapshot {
    if let Some(mut cached) = cached_fallback {
        cached.account_id = account_id.or(cached.account_id);
        if via_proxy.is_some() {
            cached.via_proxy = via_proxy;
        }
        cached.error = Some(error);
        return cached;
    }
    AccountQuotaSnapshot {
        email,
        account_id,
        fetched_at_ms: now_ms(),
        via_proxy,
        error: Some(error),
        ..Default::default()
    }
}

/// 使用给定的凭据 blob 查询账号套餐等级、剩余额度与刷新时间（不直接写磁盘，由调用方统一持久化）。
pub fn fetch_quota_for_blob(
    blob: &[u8],
    extra_blobs: &[Vec<u8>],
    account_id: Option<String>,
    settings: &ProxySettings,
    token_cache: &Arc<Mutex<TokenCache>>,
    cached_map: &HashMap<String, AccountQuotaSnapshot>,
    force_refresh: bool,
) -> AccountQuotaSnapshot {
    let primary_tokens = credential::extract_tokens(blob);
    let tokens = credential::select_freshest_tokens(&primary_tokens, extra_blobs, None, now_ms());
    let email = tokens
        .email
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let cache_key = email.trim().to_ascii_lowercase();
    let cached_fallback = cached_map.get(&cache_key).cloned();

    let (port, _, is_listening, _) = proxy::resolve_port(settings);
    let (winhttp_proxy, via_proxy) = if is_listening && port > 0 {
        (
            Some(format_winhttp_proxy(&settings.proxy_host, port)),
            Some(format!("http://{}:{}", settings.proxy_host.trim(), port)),
        )
    } else {
        (None, None)
    };
    let proxy_ref = winhttp_proxy.as_deref();

    let (mut access_token, mut already_refreshed) = match resolve_access_token(
        token_cache,
        &cache_key,
        &tokens,
        proxy_ref,
        force_refresh,
    ) {
        Ok(res) => res,
        Err(err) => {
            return error_snapshot_with_fallback(
                email,
                account_id,
                via_proxy,
                err,
                cached_fallback,
            );
        }
    };

    let load_body = json!({
        "metadata": {
            "ideType": "ANTIGRAVITY",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI"
        }
    });

    let load_res = post_cloudcode_with_retry(
        &CLOUDCODE_HOSTS,
        "/v1internal:loadCodeAssist",
        &load_body,
        &mut access_token,
        &mut already_refreshed,
        &tokens,
        token_cache,
        &cache_key,
        proxy_ref,
    );

    let (preferred_host, project_id, plan) = match load_res {
        Ok((host, val)) => {
            let (proj, plan) = parse_load_code_assist(&val);
            (host, proj, plan)
        }
        Err(err) => {
            return error_snapshot_with_fallback(
                email,
                account_id,
                via_proxy,
                err,
                cached_fallback,
            );
        }
    };

    let mut ordered_hosts: Vec<&str> = Vec::with_capacity(CLOUDCODE_HOSTS.len());
    ordered_hosts.push(preferred_host.as_str());
    for &h in &CLOUDCODE_HOSTS {
        if h != preferred_host {
            ordered_hosts.push(h);
        }
    }

    // Best-effort 调用 retrieveUserQuotaSummary，失败不影响后续 fetchAvailableModels
    let mut summary_buckets = Vec::new();
    let empty_body = json!({});
    if let Ok((_, summary_val)) = post_cloudcode_with_retry(
        &[preferred_host.as_str()],
        "/v1internal:retrieveUserQuotaSummary",
        &empty_body,
        &mut access_token,
        &mut already_refreshed,
        &tokens,
        token_cache,
        &cache_key,
        proxy_ref,
    ) {
        summary_buckets = parse_quota_summary(&summary_val);
    } else if let Some(proj) = project_id.as_deref() {
        let proj_body = json!({ "project": proj });
        if let Ok((_, summary_val)) = post_cloudcode_with_retry(
            &[preferred_host.as_str()],
            "/v1internal:retrieveUserQuotaSummary",
            &proj_body,
            &mut access_token,
            &mut already_refreshed,
            &tokens,
            token_cache,
            &cache_key,
            proxy_ref,
        ) {
            summary_buckets = parse_quota_summary(&summary_val);
        }
    }

    let models_body = match project_id.as_deref() {
        Some(proj) if !proj.is_empty() => json!({ "project": proj }),
        _ => json!({}),
    };

    let (host_used, models) = match post_cloudcode_with_retry(
        &ordered_hosts,
        "/v1internal:fetchAvailableModels",
        &models_body,
        &mut access_token,
        &mut already_refreshed,
        &tokens,
        token_cache,
        &cache_key,
        proxy_ref,
    ) {
        Ok((h, val)) => (h, parse_available_models(&val)),
        Err(err) => {
            // 即使模型列表获取失败，若 loadCodeAssist 成功仍保留基础套餐信息并附带错误
            let now = now_ms();
            let pools = build_quota_pools(&summary_buckets, &[]);
            let (min_remaining_fraction, earliest_reset_at_ms) =
                compute_summary_metrics(&pools, &[], now);
            return AccountQuotaSnapshot {
                email,
                account_id,
                fetched_at_ms: now,
                host_used: Some(preferred_host),
                via_proxy,
                project_id,
                plan,
                pools,
                summary_buckets,
                models: Vec::new(),
                min_remaining_fraction,
                earliest_reset_at_ms,
                error: Some(err),
            };
        }
    };

    let now = now_ms();
    let pools = build_quota_pools(&summary_buckets, &models);
    let (min_remaining_fraction, earliest_reset_at_ms) =
        compute_summary_metrics(&pools, &models, now);

    AccountQuotaSnapshot {
        email,
        account_id,
        fetched_at_ms: now,
        host_used: Some(host_used),
        via_proxy,
        project_id,
        plan,
        pools,
        summary_buckets,
        models,
        min_remaining_fraction,
        earliest_reset_at_ms,
        error: None,
    }
}

/// 使用有界并发线程池批量查询多个账号的额度。
pub fn fetch_all_accounts_quota(
    items: Vec<(Option<String>, Vec<u8>)>,
    extra_blobs: &[Vec<u8>],
    settings: &ProxySettings,
    token_cache: &Arc<Mutex<TokenCache>>,
    cached_map: &HashMap<String, AccountQuotaSnapshot>,
) -> Vec<AccountQuotaSnapshot> {
    if items.is_empty() {
        return Vec::new();
    }

    let total = items.len();
    let queue: Mutex<VecDeque<(usize, Option<String>, Vec<u8>)>> = Mutex::new(
        items
            .into_iter()
            .enumerate()
            .map(|(idx, (id, blob))| (idx, id, blob))
            .collect(),
    );
    let results: Mutex<Vec<Option<AccountQuotaSnapshot>>> = Mutex::new(vec![None; total]);

    let worker_count = total.min(BATCH_CONCURRENCY).max(1);
    std::thread::scope(|s| {
        for _ in 0..worker_count {
            s.spawn(|| loop {
                let job = {
                    let Ok(mut q) = queue.lock() else {
                        break;
                    };
                    q.pop_front()
                };
                let Some((idx, acc_id, blob)) = job else {
                    break;
                };
                let snap = fetch_quota_for_blob(
                    &blob,
                    extra_blobs,
                    acc_id,
                    settings,
                    token_cache,
                    cached_map,
                    false,
                );
                if let Ok(mut out) = results.lock() {
                    out[idx] = Some(snap);
                }
            });
        }
    });

    results
        .into_inner()
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_load_code_assist_free_pro_ultra_standard_and_mismatch() {
        let free_val = json!({
            "cloudaicompanionProject": "projects/free-proj",
            "currentTier": { "id": "free-tier", "name": "Antigravity" }
        });
        let (proj, plan) = parse_load_code_assist(&free_val);
        assert_eq!(proj.as_deref(), Some("projects/free-proj"));
        assert_eq!(plan.tier_code, "free");
        assert_eq!(plan.tier_label, "Free");
        assert!(!plan.entitlement_mismatch);
        assert_eq!(plan.refresh_cadence_hint, "按周刷新");

        let pro_val = json!({
            "cloudaicompanionProject": { "id": "aicode-consumers" },
            "currentTier": { "id": "g1-pro-tier", "name": "Google AI Pro" },
            "paidTier": {
                "id": "g1-pro-tier",
                "name": "Google AI Pro",
                "availableCredits": [
                    { "creditType": "GOOGLE_ONE_AI", "creditAmount": "1000", "minimumCreditAmountForUsage": "50" }
                ]
            }
        });
        let (proj2, plan2) = parse_load_code_assist(&pro_val);
        assert_eq!(proj2.as_deref(), Some("aicode-consumers"));
        assert_eq!(plan2.tier_code, "pro");
        assert_eq!(plan2.tier_label, "Google AI Pro");
        assert!(!plan2.entitlement_mismatch);
        assert_eq!(plan2.credits.len(), 1);
        assert_eq!(plan2.credits[0].amount.as_deref(), Some("1000"));
        assert_eq!(plan2.credits[0].min_for_usage.as_deref(), Some("50"));

        let ultra_val = json!({
            "currentTier": { "id": "free-tier", "name": "Free" },
            "paidTier": {
                "id": "g1-ultra-tier",
                "name": "Google AI Ultra",
                "availableCredits": 2500
            },
            "ineligibleTiers": [{ "reasonCode": "TOS_PENDING", "reasonMessage": "Accept ToS" }]
        });
        let (_, plan3) = parse_load_code_assist(&ultra_val);
        assert_eq!(plan3.tier_code, "ultra");
        assert_eq!(plan3.tier_label, "Google AI Ultra");
        assert!(!plan3.entitlement_mismatch);
        assert_eq!(plan3.credits[0].amount.as_deref(), Some("2500"));
        assert_eq!(plan3.ineligible_reason.as_deref(), Some("Accept ToS"));

        let std_val = json!({
            "currentTier": { "id": "standard-tier", "name": "Gemini Code Assist" }
        });
        let (_, plan4) = parse_load_code_assist(&std_val);
        assert_eq!(plan4.tier_code, "standard");
        assert_eq!(plan4.tier_label, "Gemini Code Assist");
    }

    #[test]
    fn parses_quota_summary_nested_groups_and_fractions() {
        let val = json!({
            "response": {
                "groups": [
                    {
                        "id": "gemini",
                        "displayName": "Gemini Models",
                        "buckets": [
                            {
                                "bucketId": "gemini-5h",
                                "displayName": "5h Rolling",
                                "window": "5h",
                                "remainingFraction": 0.75,
                                "resetTime": "2026-11-24T18:00:00Z"
                            },
                            {
                                "bucketId": "gemini-weekly",
                                "displayName": "Weekly",
                                "window": "weekly",
                                "usedFraction": 0.4,
                                "resetTime": "2026-11-30T00:00:00Z"
                            }
                        ]
                    }
                ]
            }
        });
        let buckets = parse_quota_summary(&val);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].remaining_fraction, Some(0.75));
        assert_eq!(buckets[0].window.as_deref(), Some("5h"));
        assert!(buckets[0].reset_at_ms.is_some());
        assert!((buckets[1].remaining_fraction.unwrap() - 0.6).abs() < 1e-9);
    }

    #[test]
    fn parses_available_models_skips_internal_handles_exhausted_and_classifies_pools() {
        let val = json!({
            "models": {
                "internal-debug": {
                    "displayName": "Internal",
                    "isInternal": true,
                    "quotaInfo": { "remainingFraction": 0.5 }
                },
                "gemini-3-pro": {
                    "displayName": "Gemini 3 Pro",
                    "recommended": true,
                    "apiProvider": "API_PROVIDER_GOOGLE_GEMINI",
                    "quotaInfo": { "remainingFraction": 0.85, "resetTime": "2026-11-24T18:00:00Z" }
                },
                "gemini-3-flash": {
                    "displayName": "Gemini 3 Flash",
                    "quotaInfo": { "resetTime": "2026-11-24T17:00:00Z" }
                },
                "claude-sonnet-4-6": {
                    "displayName": "Claude Sonnet 4.6",
                    "apiProvider": "API_PROVIDER_ANTHROPIC_VERTEX",
                    "quotaInfo": { "remainingFraction": 0.42, "resetTime": "2026-11-25T10:00:00Z" }
                },
                "imagen-4": {
                    "displayName": "Imagen 4",
                    "quotaInfo": { "remainingFraction": 1.0, "resetTime": "2026-11-24T20:00:00Z" }
                }
            }
        });

        let models = parse_available_models(&val);
        assert_eq!(models.len(), 4);
        // recommended first
        assert_eq!(models[0].model_id, "gemini-3-pro");
        assert_eq!(models[0].pool_key, "gemini_pro");
        // exhausted model omitted remainingFraction -> 0.0
        let flash = models.iter().find(|m| m.model_id == "gemini-3-flash").unwrap();
        assert_eq!(flash.remaining_fraction, Some(0.0));
        assert_eq!(flash.pool_key, "gemini_flash");
        assert!(flash.reset_at_ms.is_some());

        let claude = models.iter().find(|m| m.model_id == "claude-sonnet-4-6").unwrap();
        assert_eq!(claude.pool_key, "third_party");

        let img = models.iter().find(|m| m.model_id == "imagen-4").unwrap();
        assert_eq!(img.pool_key, "image");

        let pools = build_quota_pools(&[], &models);
        assert_eq!(pools.len(), 4);
        assert_eq!(pools[0].id, "gemini_pro");
        assert_eq!(pools[0].remaining_fraction, Some(0.85));
        assert_eq!(pools[1].id, "gemini_flash");
        assert_eq!(pools[1].remaining_fraction, Some(0.0));
    }

    #[test]
    fn build_quota_pools_prefers_summary_buckets_when_present() {
        let buckets = vec![
            QuotaBucketItem {
                group_id: "default".into(),
                group_label: "Claude and GPT models".into(),
                bucket_id: "3p-weekly".into(),
                label: "Weekly Limit Remaining".into(),
                window: Some("weekly".into()),
                remaining_fraction: Some(1.0),
                reset_at_ms: Some(1_800_100_000_000),
                reset_time_raw: None,
            },
            QuotaBucketItem {
                group_id: "default".into(),
                group_label: "Gemini Models".into(),
                bucket_id: "gemini-weekly".into(),
                label: "Weekly Limit Remaining".into(),
                window: Some("weekly".into()),
                remaining_fraction: Some(0.55),
                reset_at_ms: Some(1_800_050_000_000),
                reset_time_raw: None,
            },
            QuotaBucketItem {
                group_id: "default".into(),
                group_label: "Gemini Models".into(),
                bucket_id: "gemini-5h".into(),
                label: "Five Hour Limit Remaining".into(),
                window: Some("5h".into()),
                remaining_fraction: Some(0.9),
                reset_at_ms: Some(1_800_000_000_000),
                reset_time_raw: None,
            },
        ];
        let pools = build_quota_pools(&buckets, &[]);
        assert_eq!(pools.len(), 3);
        assert_eq!(pools[0].source, "summary");
        assert_eq!(pools[0].label, "Gemini Models · 5h 额度");
        assert_eq!(pools[0].remaining_fraction, Some(0.9));
        assert_eq!(pools[1].label, "Gemini Models · 周额度");
        assert_eq!(pools[2].label, "Claude and GPT models · 周额度");
    }

    #[test]
    fn quota_cache_store_roundtrip_and_corrupt_tolerance() {
        let dir = std::env::temp_dir().join(format!("agydock-quota-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("quota-cache.json");
        let store = QuotaCacheStore::new(&path);

        assert!(store.load_all().is_empty());

        let snap = AccountQuotaSnapshot {
            email: "User@Example.COM".into(),
            fetched_at_ms: 123456,
            plan: PlanTierInfo {
                tier_code: "pro".into(),
                tier_label: "Google AI Pro".into(),
                ..Default::default()
            },
            min_remaining_fraction: Some(0.8),
            ..Default::default()
        };
        store.upsert(&snap).unwrap();

        let loaded = store.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.get("user@example.com").unwrap().plan.tier_code, "pro");

        fs::write(&path, "{broken-json").unwrap();
        assert!(store.load_all().is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn formats_winhttp_proxy_and_url_encoding() {
        assert_eq!(format_winhttp_proxy("127.0.0.1", 7897), "127.0.0.1:7897");
        assert_eq!(format_winhttp_proxy("localhost", 8080), "localhost:8080");
        assert_eq!(format_winhttp_proxy("::1", 7897), "[::1]:7897");
        assert_eq!(format_winhttp_proxy("[::1]", 7897), "[::1]:7897");
        assert_eq!(
            url_encode_component("a+b/c=d&e@f.com"),
            "a%2Bb%2Fc%3Dd%26e%40f.com"
        );
    }

    #[test]
    fn parses_oauth_client_config_json() {
        assert_eq!(
            parse_oauth_client_config_json(
                r#"{"clientId":" test-client-id ","clientSecret":" test-client-secret "}"#
            ),
            Some(("test-client-id".into(), "test-client-secret".into()))
        );
        assert_eq!(
            parse_oauth_client_config_json(
                r#"{"client_id":"snake-id","client_secret":"snake-secret"}"#
            ),
            Some(("snake-id".into(), "snake-secret".into()))
        );
        assert_eq!(
            parse_oauth_client_config_json(r#"{"clientId":"","clientSecret":"x"}"#),
            None
        );
        assert_eq!(parse_oauth_client_config_json("not-json"), None);
    }

    #[test]
    fn masks_client_id_safely() {
        assert_eq!(
            mask_client_id("123456789012-abcdefghijklmnopqrstuvwxyz.client.example"),
            "12345678...xample"
        );
        assert_eq!(mask_client_id("1234567890"), "1234...");
        assert_eq!(mask_client_id("short"), "***");
        assert_eq!(mask_client_id("  1234567890123456789  "), "12345678...456789");
    }

    #[cfg(windows)]
    #[test]
    fn oauth_client_config_dpapi_roundtrip_and_clear() {
        let dir = std::env::temp_dir().join(format!(
            "agydock-oauth-dpapi-{}-{}",
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let env_unset = load_oauth_client_config_from_env().is_none();
        if env_unset {
            let empty_status = get_oauth_client_status_in(Some(&dir));
            assert!(!empty_status.configured);
            assert_eq!(empty_status.source, "none");
        }

        // Prepare legacy plaintext file to verify it gets cleaned up after DPAPI save
        let legacy_path = dir.join(OAUTH_CONFIG_JSON_NAME);
        fs::write(&legacy_path, r#"{"clientId":"legacy","clientSecret":"legacy-sec"}"#).unwrap();

        let client_id = "12345678-exampleclientid.example";
        let client_secret = "super-secret-value-for-test-only";
        let saved_status = save_oauth_client_config_in(&dir, client_id, client_secret).unwrap();

        let bin_path = dir.join(OAUTH_CONFIG_BIN_NAME);
        assert!(bin_path.is_file());
        assert!(!legacy_path.exists());

        let raw_bin = fs::read(&bin_path).unwrap();
        let raw_lossy = String::from_utf8_lossy(&raw_bin);
        assert!(!raw_lossy.contains(client_secret));
        assert!(!raw_lossy.contains(client_id));

        let (loaded_id, loaded_secret, loaded_src) =
            load_oauth_client_config_from_dir(&dir).unwrap();
        assert_eq!(loaded_id, client_id);
        assert_eq!(loaded_secret, client_secret);
        assert_eq!(loaded_src, "dpapi");

        if env_unset {
            assert!(saved_status.configured);
            assert_eq!(saved_status.source, "dpapi");
            assert_eq!(
                saved_status.client_id_preview,
                Some(mask_client_id(client_id))
            );
            assert!(!saved_status
                .client_id_preview
                .as_deref()
                .unwrap_or_default()
                .contains(client_secret));
        }

        let cleared_status = clear_oauth_client_config_in(&dir).unwrap();
        assert!(!bin_path.exists());
        assert!(load_oauth_client_config_from_dir(&dir).is_none());
        if env_unset {
            assert!(!cleared_status.configured);
            assert_eq!(cleared_status.source, "none");
            assert!(cleared_status.client_id_preview.is_none());
        }

        let _ = fs::remove_dir_all(&dir);
    }
}
