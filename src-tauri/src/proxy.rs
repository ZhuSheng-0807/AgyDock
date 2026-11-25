use chrono::Local;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const FALLBACK_HTTP_PORTS: [u16; 8] = [7897, 7899, 7890, 7891, 10809, 10808, 2080, 1080];

fn re_mixed_port() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^\s*(?:verge_mixed_port|mixed-port)\s*:\s*(\d+)").unwrap())
}

fn re_fallback_port() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^\s*(?:verge_port|port)\s*:\s*(\d+)").unwrap())
}

fn re_clash_mode() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^\s*mode\s*:\s*([A-Za-z0-9_-]+)").unwrap())
}

fn re_tun_desc() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)Meta|Mihomo|Clash|Tun|Wintun|Sing-box").unwrap())
}

fn re_tun_name() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^(Meta|Mihomo|Clash|utun)").unwrap())
}

fn re_fake_ip() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^198\.1[89]\.").unwrap())
}

fn default_antigravity_path() -> String {
    if let Some(local_app) = dirs::data_local_dir() {
        let p = local_app
            .join("Programs")
            .join("antigravity")
            .join("Antigravity.exe");
        if p.exists() {
            return p.to_string_lossy().to_string();
        }
        return p.to_string_lossy().to_string();
    }
    String::new()
}

fn default_clash_verge_path() -> String {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(pf) = std::env::var("ProgramFiles") {
        candidates.push(PathBuf::from(&pf).join("Clash Verge").join("clash-verge.exe"));
        candidates.push(PathBuf::from(&pf).join("Clash Verge Rev").join("clash-verge.exe"));
    }
    if let Some(local_app) = dirs::data_local_dir() {
        candidates.push(
            local_app
                .join("Programs")
                .join("Clash Verge")
                .join("clash-verge.exe"),
        );
        candidates.push(
            local_app
                .join("Programs")
                .join("clash-verge")
                .join("clash-verge.exe"),
        );
    }
    candidates.push(PathBuf::from(r"C:\Program Files\Clash Verge\clash-verge.exe"));

    for c in &candidates {
        if c.exists() {
            return c.to_string_lossy().to_string();
        }
    }
    candidates
        .first()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn default_true() -> bool {
    true
}

pub fn cli_shim_dir() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::config_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("AgyDock")
        .join("cli-shim")
}

pub fn normalize_path_key(p: &str) -> String {
    let replaced = p.trim().replace('/', "\\");
    let trimmed = replaced.trim_end_matches('\\');
    if trimmed.len() == 2 && trimmed.as_bytes()[1] == b':' {
        format!("{}\\", trimmed.to_lowercase())
    } else {
        trimmed.to_lowercase()
    }
}

pub fn is_inside_dir(path: &Path, dir: &Path) -> bool {
    let p = normalize_path_key(&path.to_string_lossy());
    let d = normalize_path_key(&dir.to_string_lossy());
    if d.is_empty() || p.is_empty() {
        return false;
    }
    if p == d {
        return true;
    }
    if d.ends_with('\\') {
        p.starts_with(&d)
    } else {
        p.starts_with(&format!("{d}\\"))
    }
}

fn push_unique_cli_candidate(
    candidates: &mut Vec<String>,
    seen: &mut HashSet<String>,
    path: &Path,
    shim_dir: &Path,
) {
    if is_inside_dir(path, shim_dir) || !path.is_file() {
        return;
    }
    let raw = path.to_string_lossy().to_string();
    let key = normalize_path_key(&raw);
    if !key.is_empty() && seen.insert(key) {
        candidates.push(raw);
    }
}

pub fn detect_cli_candidates() -> Vec<String> {
    let shim_dir = cli_shim_dir();
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    if let Some(path_env) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_env) {
            push_unique_cli_candidate(&mut candidates, &mut seen, &dir.join("agy.exe"), &shim_dir);
            push_unique_cli_candidate(
                &mut candidates,
                &mut seen,
                &dir.join("antigravity-cli.exe"),
                &shim_dir,
            );
        }
    }

    if let Some(home) = dirs::home_dir() {
        push_unique_cli_candidate(
            &mut candidates,
            &mut seen,
            &home.join(".local").join("bin").join("agy.exe"),
            &shim_dir,
        );
        push_unique_cli_candidate(
            &mut candidates,
            &mut seen,
            &home.join(".antigravity").join("bin").join("agy.exe"),
            &shim_dir,
        );
    }

    if let Some(local_app) = dirs::data_local_dir() {
        push_unique_cli_candidate(
            &mut candidates,
            &mut seen,
            &local_app
                .join("Programs")
                .join("antigravity-cli")
                .join("agy.exe"),
            &shim_dir,
        );
        push_unique_cli_candidate(
            &mut candidates,
            &mut seen,
            &local_app.join("antigravity").join("agy.exe"),
            &shim_dir,
        );
    }

    if crate::scanner::is_t3_code_installed() {
        if let Some(home) = dirs::home_dir() {
            let t3_acp_root = home
                .join(".t3")
                .join("tools")
                .join("antigravity-acp")
                .join("win32-x64");
            let active_json = t3_acp_root.join("active.json");
            if let Ok(raw) = fs::read_to_string(&active_json) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(rel_id) = val.get("releaseId").and_then(|v| v.as_str()) {
                        let rel_id = rel_id.trim();
                        if !rel_id.is_empty() {
                            push_unique_cli_candidate(
                                &mut candidates,
                                &mut seen,
                                &t3_acp_root.join("versions").join(rel_id).join("agy.exe"),
                                &shim_dir,
                            );
                        }
                    }
                }
            }

            let versions_dir = t3_acp_root.join("versions");
            if let Ok(entries) = fs::read_dir(&versions_dir) {
                let mut versioned: Vec<(SystemTime, PathBuf)> = Vec::new();
                for entry in entries.flatten() {
                    let exe = entry.path().join("agy.exe");
                    if exe.is_file() {
                        let mtime = fs::metadata(&exe)
                            .and_then(|m| m.modified())
                            .unwrap_or(SystemTime::UNIX_EPOCH);
                        versioned.push((mtime, exe));
                    }
                }
                versioned.sort_by(|a, b| b.0.cmp(&a.0));
                for (_, exe) in versioned {
                    push_unique_cli_candidate(&mut candidates, &mut seen, &exe, &shim_dir);
                }
            }
        }
    }

    candidates
}

fn default_cli_path() -> String {
    detect_cli_candidates().into_iter().next().unwrap_or_default()
}

fn parse_app_version_dir(name: &str) -> Option<Vec<u32>> {
    let rest = name
        .strip_prefix("app-")
        .or_else(|| name.strip_prefix("APP-"))
        .or_else(|| {
            if name.len() > 4 && name[..4].eq_ignore_ascii_case("app-") {
                Some(&name[4..])
            } else {
                None
            }
        })?;
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Option<Vec<u32>> = trimmed
        .split('.')
        .map(|seg| {
            let digits: String = seg.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                None
            } else {
                digits.parse::<u32>().ok()
            }
        })
        .collect();
    parts.filter(|v| !v.is_empty())
}

pub fn is_generic_app_dir(dir: &Path) -> bool {
    let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
        return true;
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        "bin"
            | "scripts"
            | "system32"
            | "syswow64"
            | "windows"
            | "tools"
            | "shims"
            | "cli-shim"
            | "programs"
            | "program files"
            | "program files (x86)"
            | "local"
            | "roaming"
            | "appdata"
            | "users"
            | "google"
    )
}

pub fn tool_install_root(exe_path: &Path) -> Option<PathBuf> {
    let parent = exe_path.parent().filter(|p| !p.as_os_str().is_empty())?;
    let parent_is_versioned = parent
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(parse_app_version_dir)
        .is_some();
    if parent_is_versioned {
        if let Some(grand) = parent.parent().filter(|p| !p.as_os_str().is_empty()) {
            if !is_generic_app_dir(grand) {
                return Some(grand.to_path_buf());
            }
        }
    }
    if !is_generic_app_dir(parent) {
        Some(parent.to_path_buf())
    } else {
        None
    }
}

/// 若配置路径指向 Squirrel / NSIS 风格的根目录启动壳（同目录下存在 `app-<version>\<exe>`），
/// 解析并返回版本号最高的真实主程序，避免启动壳丢弃 `--proxy-server` 等 Chromium 参数。
pub fn resolve_versioned_app_exe(exe_path: &Path) -> PathBuf {
    let Some(parent) = exe_path.parent() else {
        return exe_path.to_path_buf();
    };
    let Some(file_name) = exe_path.file_name() else {
        return exe_path.to_path_buf();
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return exe_path.to_path_buf();
    };

    let mut versioned: Vec<(Vec<u32>, SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let dir_path = entry.path();
        if !dir_path.is_dir() {
            continue;
        }
        let Some(dir_name) = dir_path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(ver) = parse_app_version_dir(dir_name) else {
            continue;
        };
        let candidate = dir_path.join(file_name);
        if candidate.is_file() {
            let mtime = fs::metadata(&candidate)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            versioned.push((ver, mtime, candidate));
        }
    }

    versioned.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    versioned
        .into_iter()
        .next()
        .map(|(_, _, p)| p)
        .unwrap_or_else(|| exe_path.to_path_buf())
}

fn detect_gemini_desktop_exe() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(local_app) = dirs::data_local_dir() {
        let google_gemini = local_app.join("Google").join("Gemini");
        candidates.push(google_gemini.join("Gemini.exe"));
        let resolved_ver = resolve_versioned_app_exe(&google_gemini.join("Gemini.exe"));
        if resolved_ver != google_gemini.join("Gemini.exe") {
            candidates.push(resolved_ver);
        }
        candidates.push(local_app.join("Programs").join("Gemini").join("Gemini.exe"));
        candidates.push(
            local_app
                .join("Programs")
                .join("Google")
                .join("Gemini")
                .join("Gemini.exe"),
        );
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        candidates.push(PathBuf::from(&pf).join("Google").join("Gemini").join("Gemini.exe"));
        candidates.push(PathBuf::from(&pf).join("Gemini").join("Gemini.exe"));
    }

    candidates.into_iter().find(|p| p.is_file())
}

fn is_gemini_desktop_tool(tool: &CliToolEntry) -> bool {
    if tool.id == "gemini-desktop" {
        return true;
    }
    let norm = normalize_path_key(&tool.exe_path);
    norm.contains(r"\google\gemini\") && norm.ends_with(r"\gemini.exe")
}

pub fn detect_default_cli_tools() -> Vec<CliToolEntry> {
    let mut tools = Vec::new();

    if let Some(exe) = detect_gemini_desktop_exe() {
        tools.push(CliToolEntry {
            id: "gemini-desktop".to_string(),
            name: "Gemini Desktop".to_string(),
            exe_path: exe.to_string_lossy().to_string(),
            args: String::new(),
            working_dir: String::new(),
            is_chromium: false,
            monitor: true,
        });
    }

    let mut t3_candidates: Vec<PathBuf> = Vec::new();
    if let Some(local_app) = dirs::data_local_dir() {
        t3_candidates.push(
            local_app
                .join("Programs")
                .join("t3code")
                .join("T3 Code (Beta).exe"),
        );
        t3_candidates.push(local_app.join("Programs").join("t3code").join("T3 Code.exe"));
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        t3_candidates.push(PathBuf::from(&pf).join("T3 Code").join("T3 Code.exe"));
        t3_candidates.push(PathBuf::from(&pf).join("T3 Code").join("T3 Code (Beta).exe"));
    }

    if let Some(exe) = t3_candidates.into_iter().find(|p| p.is_file()) {
        tools.push(CliToolEntry {
            id: "t3-code".to_string(),
            name: "T3 Code".to_string(),
            exe_path: exe.to_string_lossy().to_string(),
            args: String::new(),
            working_dir: String::new(),
            is_chromium: true,
            monitor: true,
        });
    }

    tools
}

pub fn detect_cli_and_tools() -> CliDetectResult {
    let cli_candidates = detect_cli_candidates();
    let cli_path = cli_candidates.first().cloned().unwrap_or_default();
    let detected_tools = detect_default_cli_tools();
    CliDetectResult {
        cli_path,
        cli_candidates,
        detected_tools,
    }
}

/// 内置国内常用域名后缀（不含 .cn，.cn 已单独通配），用于在启用「国内/内网直连绕过」时
/// 避免访问国内站点走海外代理节点触发风控。
const CHINA_DIRECT_DOMAINS: &[&str] = &[
    "baidu.com",
    "bdstatic.com",
    "bcebos.com",
    "qq.com",
    "gtimg.com",
    "qcloud.com",
    "myqcloud.com",
    "tencent.com",
    "tencentcloudapi.com",
    "alibaba.com",
    "alicdn.com",
    "aliyun.com",
    "aliyuncs.com",
    "taobao.com",
    "tmall.com",
    "alipay.com",
    "dingtalk.com",
    "1688.com",
    "amap.com",
    "npmmirror.com",
    "yuque.com",
    "bytedance.com",
    "byteimg.com",
    "douyin.com",
    "toutiao.com",
    "larkoffice.com",
    "volces.com",
    "volcengine.com",
    "jd.com",
    "360buyimg.com",
    "pinduoduo.com",
    "meituan.com",
    "dianping.com",
    "ctrip.com",
    "bilibili.com",
    "hdslb.com",
    "zhihu.com",
    "zhimg.com",
    "weibo.com",
    "sina.com",
    "xiaohongshu.com",
    "xhscdn.com",
    "douban.com",
    "163.com",
    "126.net",
    "csdn.net",
    "cnblogs.com",
    "oschina.net",
    "gitee.com",
    "segmentfault.com",
    "mi.com",
    "xiaomi.com",
    "huawei.com",
    "huaweicloud.com",
    "deepseek.com",
    "baichuan-ai.com",
    "qiniu.com",
    "qiniucdn.com",
    "upyun.com",
    "ccb.com",
    "icbc.com",
    "cmbchina.com",
    "abchina.com",
];

const LAN_BYPASS_CIDRS: &[&str] = &[
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "169.254.0.0/16",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliToolEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub exe_path: String,
    #[serde(default)]
    pub args: String,
    #[serde(default)]
    pub working_dir: String,
    #[serde(default = "default_true")]
    pub is_chromium: bool,
    #[serde(default = "default_true")]
    pub monitor: bool,
}

fn slugify_tool_id(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "tool".to_string()
    } else {
        trimmed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxySettings {
    pub proxy_host: String,
    pub auto_detect_port: bool,
    pub custom_port: u16,
    pub antigravity_path: String,
    pub clash_verge_path: String,
    pub workspace_path: String,
    pub auto_start_clash: bool,
    pub disable_quic: bool,
    pub show_loopback_ipc: bool,
    #[serde(default = "default_true")]
    pub bypass_china_and_lan: bool,
    #[serde(default)]
    pub custom_bypass_domains: String,
    #[serde(default)]
    pub cli_path: String,
    #[serde(default)]
    pub cli_tools: Vec<CliToolEntry>,
    #[serde(default)]
    pub cli_tools_initialized: bool,
    #[serde(default)]
    pub builtin_tools_discovered: Vec<String>,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            proxy_host: "127.0.0.1".to_string(),
            auto_detect_port: true,
            custom_port: 7897,
            antigravity_path: default_antigravity_path(),
            clash_verge_path: default_clash_verge_path(),
            workspace_path: String::new(),
            auto_start_clash: true,
            disable_quic: true,
            show_loopback_ipc: false,
            bypass_china_and_lan: true,
            custom_bypass_domains: String::new(),
            cli_path: default_cli_path(),
            cli_tools: Vec::new(),
            cli_tools_initialized: false,
            builtin_tools_discovered: Vec::new(),
        }
    }
}

impl ProxySettings {
    pub fn sanitized(mut self) -> Self {
        if self.proxy_host.trim().is_empty() {
            self.proxy_host = "127.0.0.1".to_string();
        } else {
            self.proxy_host = self.proxy_host.trim().to_string();
        }
        if self.custom_port == 0 {
            self.custom_port = 7897;
        }
        if self.antigravity_path.trim().is_empty() {
            self.antigravity_path = default_antigravity_path();
        } else {
            self.antigravity_path = self.antigravity_path.trim().to_string();
        }
        if self.clash_verge_path.trim().is_empty() {
            self.clash_verge_path = default_clash_verge_path();
        } else {
            self.clash_verge_path = self.clash_verge_path.trim().to_string();
        }
        self.workspace_path = self.workspace_path.trim().to_string();
        self.custom_bypass_domains = self.custom_bypass_domains.trim().to_string();

        if self.cli_path.trim().is_empty() {
            self.cli_path = default_cli_path();
        } else {
            self.cli_path = self.cli_path.trim().to_string();
        }

        let mut cleaned_tools = Vec::new();
        let mut used_ids = HashSet::new();
        for mut tool in self.cli_tools {
            tool.exe_path = tool.exe_path.trim().to_string();
            if tool.exe_path.is_empty() {
                continue;
            }
            tool.name = tool.name.trim().to_string();
            tool.args = tool.args.trim().to_string();
            tool.working_dir = tool.working_dir.trim().to_string();
            let stem = Path::new(&tool.exe_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("tool")
                .to_string();
            if tool.name.is_empty() {
                tool.name = stem.clone();
            }
            let base_id = if tool.id.trim().is_empty() {
                slugify_tool_id(&tool.name)
            } else {
                slugify_tool_id(tool.id.trim())
            };
            let mut candidate_id = base_id.clone();
            let mut suffix = 2usize;
            while !used_ids.insert(candidate_id.clone()) {
                candidate_id = format!("{base_id}-{suffix}");
                suffix += 1;
            }
            tool.id = candidate_id;
            if is_gemini_desktop_tool(&tool) {
                // Gemini Desktop refuses to start when --proxy-server is passed on the command line.
                tool.is_chromium = false;
            }
            // Drop stale built-in T3 Code entry if it has been uninstalled from disk.
            if tool.id == "t3-code" && !Path::new(&tool.exe_path).is_file() {
                continue;
            }
            cleaned_tools.push(tool);
        }
        self.cli_tools = cleaned_tools;

        let mut discovered_set: HashSet<String> = self
            .builtin_tools_discovered
            .iter()
            .map(|s| slugify_tool_id(s))
            .collect();
        if self.cli_tools_initialized && discovered_set.is_empty() {
            // Legacy settings from before builtin_tools_discovered existed only had t3-code as builtin.
            discovered_set.insert("t3-code".to_string());
        }

        let detected_defaults = detect_default_cli_tools();
        if !self.cli_tools_initialized && self.cli_tools.is_empty() {
            for tool in &detected_defaults {
                discovered_set.insert(tool.id.clone());
            }
            self.cli_tools = detected_defaults;
            self.cli_tools_initialized = true;
        } else {
            self.cli_tools_initialized = true;
            let mut existing_ids: HashSet<String> =
                self.cli_tools.iter().map(|t| t.id.clone()).collect();
            let mut existing_paths: HashSet<String> = self
                .cli_tools
                .iter()
                .map(|t| normalize_path_key(&t.exe_path))
                .collect();
            for tool in detected_defaults {
                let is_new_builtin = discovered_set.insert(tool.id.clone());
                if !is_new_builtin {
                    continue;
                }
                let path_key = normalize_path_key(&tool.exe_path);
                if !existing_ids.contains(&tool.id) && !existing_paths.contains(&path_key) {
                    existing_ids.insert(tool.id.clone());
                    existing_paths.insert(path_key);
                    self.cli_tools.push(tool);
                }
            }
        }

        for tool in &self.cli_tools {
            if matches!(tool.id.as_str(), "t3-code" | "gemini-desktop") {
                discovered_set.insert(tool.id.clone());
            }
        }
        let mut discovered_vec: Vec<String> = discovered_set.into_iter().collect();
        discovered_vec.sort();
        self.builtin_tools_discovered = discovered_vec;
        self
    }

    fn settings_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("antigravity-usage-dashboard").join("proxy-settings.json"))
    }

    pub fn load() -> Self {
        if let Some(p) = Self::settings_path() {
            if let Ok(raw) = fs::read_to_string(&p) {
                if let Ok(parsed) = serde_json::from_str::<ProxySettings>(&raw) {
                    return parsed.sanitized();
                }
            }
        }
        Self::default().sanitized()
    }

    pub fn save(&self) {
        let Some(p) = Self::settings_path() else {
            return;
        };
        if let Some(parent) = p.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(p, json);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliShimStatus {
    pub enabled: bool,
    pub in_user_path: bool,
    pub shim_dir: String,
    pub target_exe: String,
    pub proxy_url: String,
    pub out_of_sync: bool,
    pub cli_detected: bool,
    pub cli_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliDetectResult {
    pub cli_path: String,
    pub cli_candidates: Vec<String>,
    pub detected_tools: Vec<CliToolEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliShellSnippets {
    pub proxy_url: String,
    pub powershell: String,
    pub cmd: String,
    pub bash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionCategory {
    Proxied,
    TunFakeIp,
    PublicDirect,
    LocalIpc,
}

impl ConnectionCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Proxied => "走代理",
            Self::TunFakeIp => "TUN Fake-IP",
            Self::PublicDirect => "公网直连",
            Self::LocalIpc => "本地/内网",
        }
    }

    fn sort_order(self) -> u8 {
        match self {
            Self::PublicDirect => 0,
            Self::TunFakeIp => 1,
            Self::Proxied => 2,
            Self::LocalIpc => 3,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TcpConnectionItem {
    pub key: String,
    pub category: ConnectionCategory,
    pub category_label: String,
    pub pid: u32,
    pub process_name: String,
    pub local_endpoint: String,
    pub remote_endpoint: String,
    pub remote_host: String,
    pub remote_port: u16,
    pub state: String,
    pub seen_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessEntry {
    pub pid: u32,
    pub name: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxySnapshot {
    pub proxy_host: String,
    pub resolved_port: u16,
    pub port_source: String,
    pub is_proxy_listening: bool,
    pub clash_mode: String,
    pub is_clash_running: bool,
    pub system_proxy_enabled: bool,
    pub system_proxy_server: String,
    pub gemini_system_proxy_managed: bool,
    pub tun_enabled: bool,
    pub tun_adapter_text: String,
    pub antigravity_proc_count: usize,
    pub language_server_proc_count: usize,
    pub cli_proc_count: usize,
    pub tool_proc_count: usize,
    pub tool_running_counts: HashMap<String, usize>,
    pub cli_shim: CliShimStatus,
    pub processes: Vec<ProcessEntry>,
    pub proxied_count: usize,
    pub tun_count: usize,
    pub direct_count: usize,
    pub connections: Vec<TcpConnectionItem>,
    pub captured_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResponse {
    pub ok: bool,
    pub need_confirm: bool,
    pub process_count: usize,
    pub resolved_port: u16,
    pub logs: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditProgressPayload {
    pub elapsed: u32,
    pub total: u32,
    pub snapshot: ProxySnapshot,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditResult {
    pub cancelled: bool,
    pub latest_snapshot: ProxySnapshot,
    pub cumulative_proxied_count: usize,
    pub cumulative_tun_count: usize,
    pub cumulative_direct_count: usize,
    pub cumulative_connections: Vec<TcpConnectionItem>,
    pub report_text: String,
    pub report_path: String,
    pub passed: Option<bool>,
}

pub fn get_clash_verge_config_dirs(clash_verge_path: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if !clash_verge_path.trim().is_empty() {
        if let Some(clash_dir) = Path::new(clash_verge_path.trim()).parent() {
            candidates.push(
                clash_dir
                    .join(".config")
                    .join("io.github.clash-verge-rev.clash-verge-rev"),
            );
            candidates.push(clash_dir.join(".config"));
            candidates.push(clash_dir.join("data"));
        }
    }

    if let Some(app_data) = dirs::config_dir() {
        candidates.push(app_data.join("io.github.clash-verge-rev.clash-verge-rev"));
        candidates.push(app_data.join("clash-verge-rev"));
        candidates.push(app_data.join("clash-verge"));
    }
    if let Some(local_data) = dirs::data_local_dir() {
        candidates.push(local_data.join("io.github.clash-verge-rev.clash-verge-rev"));
    }

    let mut seen = HashSet::new();
    let mut existing = Vec::new();
    for dir in candidates {
        if dir.is_dir() {
            let key = dir.to_string_lossy().to_lowercase();
            if seen.insert(key) {
                existing.push(dir);
            }
        }
    }
    existing
}

#[cfg(test)]
pub fn parse_clash_yaml_port_and_mode(content: &str) -> (u16, Option<String>) {
    let port = re_mixed_port()
        .captures(content)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u16>().ok())
        .filter(|&p| p > 0)
        .or_else(|| {
            re_fallback_port()
                .captures(content)
                .and_then(|c| c.get(1))
                .and_then(|m| m.as_str().parse::<u16>().ok())
                .filter(|&p| p > 0)
        })
        .unwrap_or(0);

    let mode = re_clash_mode()
        .captures(content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
        .filter(|s| !s.is_empty());

    (port, mode)
}

pub fn read_clash_verge_config(clash_verge_path: &str) -> (u16, String) {
    let dirs = get_clash_verge_config_dirs(clash_verge_path);
    let mut port: u16 = 0;
    let mut mode = "未知".to_string();

    for dir in &dirs {
        for fname in ["verge.yaml", "config.yaml", "clash-verge.yaml"] {
            let file = dir.join(fname);
            if let Ok(content) = fs::read_to_string(&file) {
                if let Some(p) = re_mixed_port()
                    .captures(&content)
                    .and_then(|c| c.get(1))
                    .and_then(|m| m.as_str().parse::<u16>().ok())
                    .filter(|&v| v > 0)
                {
                    port = p;
                    break;
                }
            }
        }
        if port > 0 {
            break;
        }
    }

    if port == 0 {
        for dir in &dirs {
            for fname in ["verge.yaml", "config.yaml"] {
                let file = dir.join(fname);
                if let Ok(content) = fs::read_to_string(&file) {
                    if let Some(p) = re_fallback_port()
                        .captures(&content)
                        .and_then(|c| c.get(1))
                        .and_then(|m| m.as_str().parse::<u16>().ok())
                        .filter(|&v| v > 0)
                    {
                        port = p;
                        break;
                    }
                }
            }
            if port > 0 {
                break;
            }
        }
    }

    for dir in &dirs {
        for fname in ["config.yaml", "clash-verge.yaml"] {
            let file = dir.join(fname);
            if let Ok(content) = fs::read_to_string(&file) {
                if let Some(m) = re_clash_mode()
                    .captures(&content)
                    .and_then(|c| c.get(1))
                    .map(|g| g.as_str().trim().to_string())
                    .filter(|s| !s.is_empty())
                {
                    mode = m;
                    break;
                }
            }
        }
        if mode != "未知" {
            break;
        }
    }

    (port, mode)
}

pub fn test_tcp_port(host: &str, port: u16, timeout_ms: u64) -> bool {
    if port == 0 || host.trim().is_empty() {
        return false;
    }
    let timeout = Duration::from_millis(timeout_ms);
    if let Ok(ip) = host.trim().parse::<IpAddr>() {
        return TcpStream::connect_timeout(&SocketAddr::new(ip, port), timeout).is_ok();
    }
    let addr_str = format!("{}:{}", host.trim(), port);
    if let Ok(mut addrs) = addr_str.to_socket_addrs() {
        if let Some(addr) = addrs.next() {
            return TcpStream::connect_timeout(&addr, timeout).is_ok();
        }
    }
    false
}

pub fn resolve_port(settings: &ProxySettings) -> (u16, String, bool, String) {
    let (cfg_port, mode) = read_clash_verge_config(&settings.clash_verge_path);

    if !settings.auto_detect_port {
        let manual = if settings.custom_port > 0 {
            settings.custom_port
        } else {
            7897
        };
        let ok = test_tcp_port(&settings.proxy_host, manual, 350);
        return (manual, "手动指定".to_string(), ok, mode);
    }

    if cfg_port > 0 && test_tcp_port(&settings.proxy_host, cfg_port, 350) {
        return (cfg_port, "Clash Verge 配置".to_string(), true, mode);
    }

    for candidate in FALLBACK_HTTP_PORTS {
        if test_tcp_port(&settings.proxy_host, candidate, 160) {
            return (candidate, "本地端口探测".to_string(), true, mode);
        }
    }

    if cfg_port > 0 {
        return (cfg_port, "Clash Verge 配置".to_string(), false, mode);
    }

    (7897, "默认端口".to_string(), false, mode)
}

fn is_local_or_private_ip(host: &str) -> bool {
    if let Ok(ip) = host.trim().parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
            }
            IpAddr::V6(v6) => {
                let seg0 = v6.segments()[0];
                v6.is_loopback()
                    || v6.is_unspecified()
                    || (seg0 & 0xfe00) == 0xfc00
                    || (seg0 & 0xffc0) == 0xfe80
            }
        };
    }
    host.starts_with("127.") || host == "0.0.0.0" || host == "::1" || host == "::"
}

pub fn classify_remote(
    remote_host: &str,
    remote_port: u16,
    proxy_host: &str,
    proxy_port: u16,
) -> ConnectionCategory {
    let host_is_local_proxy = remote_host.eq_ignore_ascii_case(proxy_host)
        || remote_host == "127.0.0.1"
        || remote_host == "::1";

    if proxy_port > 0 && host_is_local_proxy && remote_port == proxy_port {
        return ConnectionCategory::Proxied;
    }

    if re_fake_ip().is_match(remote_host) {
        return ConnectionCategory::TunFakeIp;
    }

    if is_local_or_private_ip(remote_host) {
        return ConnectionCategory::LocalIpc;
    }

    ConnectionCategory::PublicDirect
}

#[derive(Debug, Clone)]
struct RawTcpSocket {
    pid: u32,
    local_endpoint: String,
    remote_host: String,
    remote_port: u16,
    remote_endpoint: String,
    state: String,
}

#[derive(Debug, Clone)]
pub struct SavedSystemProxy {
    pub enabled: bool,
    pub server: Option<String>,
    pub override_list: Option<String>,
}

#[derive(Debug, Clone)]
struct ManagedSystemProxy {
    generation: u64,
    previous: SavedSystemProxy,
    proxy_server: String,
}

fn managed_system_proxy_store() -> &'static Mutex<Option<ManagedSystemProxy>> {
    static STORE: OnceLock<Mutex<Option<ManagedSystemProxy>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(None))
}

fn next_managed_proxy_generation() -> u64 {
    static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    GEN.fetch_add(1, Ordering::SeqCst)
}

pub fn is_gemini_system_proxy_managed() -> bool {
    managed_system_proxy_store()
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false)
}

fn build_wininet_proxy_override(settings: &ProxySettings) -> String {
    let mut entries: Vec<String> = vec![
        "localhost".to_string(),
        "127.*".to_string(),
        "[::1]".to_string(),
    ];
    let mut seen: HashSet<String> = HashSet::new();

    if settings.bypass_china_and_lan {
        for lan in ["10.*", "172.16.*", "192.168.*", "169.254.*"] {
            entries.push(lan.to_string());
        }
        for suffix in ["local", "lan", "cn"] {
            entries.push(format!("*.{suffix}"));
            seen.insert(suffix.to_string());
        }
        for domain in CHINA_DIRECT_DOMAINS {
            if seen.insert((*domain).to_string()) {
                entries.push((*domain).to_string());
                entries.push(format!("*.{domain}"));
            }
        }
    }

    for token in settings
        .custom_bypass_domains
        .split(|c: char| matches!(c, ',' | ';' | '\n' | '\r' | '\t' | ' '))
    {
        let Some(clean) = normalize_custom_bypass_entry(token) else {
            continue;
        };
        if !seen.insert(clean.clone()) {
            continue;
        }
        if clean.contains('/') || clean.parse::<IpAddr>().is_ok() || clean.starts_with('[') {
            entries.push(clean);
        } else {
            entries.push(clean.clone());
            entries.push(format!("*.{clean}"));
        }
    }

    entries.push("<local>".to_string());
    entries.join(";")
}

fn activate_managed_system_proxy(
    settings: &ProxySettings,
    target_port: u16,
    logs: &mut Vec<String>,
) -> Option<u64> {
    let (tun_enabled, _) = win_native::get_tun_adapter_state();
    if tun_enabled {
        logs.push("[i] 当前已开启 TUN 模式，无需切换 Windows 系统代理。".to_string());
        return None;
    }

    let proxy_server = format!("{}:{}", settings.proxy_host, target_port);
    let override_list = build_wininet_proxy_override(settings);

    let mut guard = match managed_system_proxy_store().lock() {
        Ok(g) => g,
        Err(_) => return None,
    };

    if let Some(existing) = guard.as_mut() {
        let gen = next_managed_proxy_generation();
        existing.generation = gen;
        existing.proxy_server = proxy_server.clone();
        if let Err(err) = win_native::set_system_proxy(true, Some(&proxy_server), Some(&override_list))
        {
            logs.push(format!("[!] 更新 Windows 系统代理失败: {err}"));
        } else {
            logs.push(format!(
                "[OK] 已保持自动托管 Windows 系统代理 -> {proxy_server}"
            ));
        }
        return Some(gen);
    }

    let previous = win_native::read_full_system_proxy();
    if previous.enabled {
        let srv = previous.server.as_deref().unwrap_or("已开启");
        logs.push(format!(
            "[i] Windows 系统代理已手动开启 ({srv})，直接复用，关闭 Gemini 时不会改动原设置。"
        ));
        return None;
    }

    match win_native::set_system_proxy(true, Some(&proxy_server), Some(&override_list)) {
        Ok(()) => {
            let gen = next_managed_proxy_generation();
            *guard = Some(ManagedSystemProxy {
                generation: gen,
                previous,
                proxy_server: proxy_server.clone(),
            });
            logs.push(format!(
                "[OK] 已为 Gemini Desktop 自动开启 Windows 系统代理 -> {proxy_server}（退出 Gemini 后将自动关闭）"
            ));
            Some(gen)
        }
        Err(err) => {
            logs.push(format!("[!] 自动开启 Windows 系统代理失败: {err}"));
            None
        }
    }
}

pub fn restore_managed_system_proxy_if_active() -> Option<Result<(), String>> {
    let managed = {
        let Ok(mut guard) = managed_system_proxy_store().lock() else {
            return None;
        };
        guard.take()
    }?;
    Some(win_native::restore_system_proxy(&managed.previous))
}

fn spawn_gemini_exit_watcher(exe_path: String, generation: u64) {
    thread::spawn(move || {
        let started = Instant::now();
        let mut saw_running = false;
        let mut zero_streak = 0u8;

        loop {
            thread::sleep(Duration::from_millis(1200));

            let still_current = managed_system_proxy_store()
                .lock()
                .map(|g| g.as_ref().map(|m| m.generation == generation).unwrap_or(false))
                .unwrap_or(false);
            if !still_current {
                return;
            }

            let running = !win_native::scan_tool_process_tree(&exe_path).is_empty();
            if running {
                saw_running = true;
                zero_streak = 0;
                continue;
            }

            if saw_running || started.elapsed() >= Duration::from_secs(6) {
                zero_streak = zero_streak.saturating_add(1);
                if zero_streak >= 2 {
                    let to_restore = managed_system_proxy_store()
                        .lock()
                        .ok()
                        .and_then(|mut g| {
                            if g.as_ref().map(|m| m.generation == generation).unwrap_or(false) {
                                g.take()
                            } else {
                                None
                            }
                        });
                    if let Some(managed) = to_restore {
                        let _ = win_native::restore_system_proxy(&managed.previous);
                    }
                    return;
                }
            }
        }
    });
}

#[cfg(windows)]
mod win_native {
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_BUFFER_OVERFLOW, ERROR_INSUFFICIENT_BUFFER, INVALID_HANDLE_VALUE,
        NO_ERROR,
    };
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GetExtendedTcpTable, GAA_FLAG_INCLUDE_PREFIX,
        IP_ADAPTER_ADDRESSES_LH, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
        TCP_TABLE_OWNER_PID_ALL,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };
    use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::{RegKey, RegValue};

    const MIB_TCP_STATE_SYN_SENT: u32 = 3;
    const MIB_TCP_STATE_ESTAB: u32 = 5;
    const HWND_BROADCAST: isize = 0xFFFF;
    const WM_SETTINGCHANGE: u32 = 0x001A;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    const INTERNET_OPTION_REFRESH: u32 = 37;
    const INTERNET_OPTION_SETTINGS_CHANGED: u32 = 39;

    #[link(name = "user32")]
    extern "system" {
        fn SendMessageTimeoutW(
            hwnd: isize,
            msg: u32,
            wparam: usize,
            lparam: *const u16,
            flags: u32,
            timeout: u32,
            lpdw_result: *mut usize,
        ) -> isize;
    }

    #[link(name = "wininet")]
    extern "system" {
        fn InternetSetOptionW(
            hinternet: *mut core::ffi::c_void,
            dwoption: u32,
            lpbuffer: *mut core::ffi::c_void,
            dwbufferlength: u32,
        ) -> i32;
    }

    const INTERNET_SETTINGS_SUBKEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

    fn notify_wininet_proxy_changed() {
        unsafe {
            InternetSetOptionW(
                std::ptr::null_mut(),
                INTERNET_OPTION_SETTINGS_CHANGED,
                std::ptr::null_mut(),
                0,
            );
            InternetSetOptionW(
                std::ptr::null_mut(),
                INTERNET_OPTION_REFRESH,
                std::ptr::null_mut(),
                0,
            );
        }
    }

    pub fn get_system_proxy_state() -> (bool, String) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(key) = hkcu.open_subkey(INTERNET_SETTINGS_SUBKEY) else {
            return (false, String::new());
        };
        let enabled = key.get_value::<u32, _>("ProxyEnable").unwrap_or(0) == 1;
        let server = key.get_value::<String, _>("ProxyServer").unwrap_or_default();
        (enabled, server)
    }

    pub fn read_full_system_proxy() -> SavedSystemProxy {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(key) = hkcu.open_subkey(INTERNET_SETTINGS_SUBKEY) else {
            return SavedSystemProxy {
                enabled: false,
                server: None,
                override_list: None,
            };
        };
        SavedSystemProxy {
            enabled: key.get_value::<u32, _>("ProxyEnable").unwrap_or(0) == 1,
            server: key.get_value::<String, _>("ProxyServer").ok(),
            override_list: key.get_value::<String, _>("ProxyOverride").ok(),
        }
    }

    pub fn set_system_proxy(
        enabled: bool,
        server: Option<&str>,
        override_list: Option<&str>,
    ) -> Result<(), String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey_with_flags(INTERNET_SETTINGS_SUBKEY, KEY_READ | KEY_WRITE)
            .map_err(|e| format!("无法打开 Internet Settings 注册表键: {e}"))?;
        let enable_val: u32 = if enabled { 1 } else { 0 };
        key.set_value("ProxyEnable", &enable_val)
            .map_err(|e| format!("写入 ProxyEnable 失败: {e}"))?;
        if let Some(srv) = server {
            key.set_value("ProxyServer", &srv)
                .map_err(|e| format!("写入 ProxyServer 失败: {e}"))?;
        }
        if let Some(ov) = override_list {
            key.set_value("ProxyOverride", &ov)
                .map_err(|e| format!("写入 ProxyOverride 失败: {e}"))?;
        }
        notify_wininet_proxy_changed();
        Ok(())
    }

    pub fn restore_system_proxy(saved: &SavedSystemProxy) -> Result<(), String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey_with_flags(INTERNET_SETTINGS_SUBKEY, KEY_READ | KEY_WRITE)
            .map_err(|e| format!("无法打开 Internet Settings 注册表键: {e}"))?;
        let enable_val: u32 = if saved.enabled { 1 } else { 0 };
        key.set_value("ProxyEnable", &enable_val)
            .map_err(|e| format!("恢复 ProxyEnable 失败: {e}"))?;
        match &saved.server {
            Some(srv) => {
                let _ = key.set_value("ProxyServer", srv);
            }
            None => {
                let _ = key.delete_value("ProxyServer");
            }
        }
        match &saved.override_list {
            Some(ov) => {
                let _ = key.set_value("ProxyOverride", ov);
            }
            None => {
                let _ = key.delete_value("ProxyOverride");
            }
        }
        notify_wininet_proxy_changed();
        Ok(())
    }

    pub fn read_user_path() -> Result<(String, bool), String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu
            .open_subkey_with_flags("Environment", KEY_READ)
            .map_err(|e| format!("无法读取 HKCU\\Environment: {e}"))?;
        match key.get_raw_value("Path") {
            Ok(val) => {
                let is_expand_sz = !matches!(val.vtype, RegType::REG_SZ);
                let mut u16s: Vec<u16> = val
                    .bytes
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                while u16s.last() == Some(&0) {
                    u16s.pop();
                }
                Ok((String::from_utf16_lossy(&u16s), is_expand_sz))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok((String::new(), true)),
            Err(err) => Err(format!("读取用户 PATH 失败: {err}")),
        }
    }

    pub fn write_user_path(value: &str, is_expand_sz: bool) -> Result<(), String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
            .map_err(|e| format!("无法打开 HKCU\\Environment 写入权限: {e}"))?;
        let wide: Vec<u16> = OsStr::new(value).encode_wide().chain(Some(0)).collect();
        let mut bytes = Vec::with_capacity(wide.len() * 2);
        for w in wide {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        let reg_val = RegValue {
            vtype: if is_expand_sz {
                RegType::REG_EXPAND_SZ
            } else {
                RegType::REG_SZ
            },
            bytes: bytes.into(),
        };
        key.set_raw_value("Path", &reg_val)
            .map_err(|e| format!("写入用户 PATH 失败: {e}"))?;
        broadcast_environment_change();
        Ok(())
    }

    fn broadcast_environment_change() {
        let env_wide: Vec<u16> = OsStr::new("Environment")
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut result: usize = 0;
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                env_wide.as_ptr(),
                SMTO_ABORTIFHUNG,
                1000,
                &mut result,
            );
        }
    }

    unsafe fn pwstr_to_string(ptr: *const u16) -> String {
        if ptr.is_null() {
            return String::new();
        }
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        OsString::from_wide(std::slice::from_raw_parts(ptr, len))
            .to_string_lossy()
            .to_string()
    }

    pub fn get_tun_adapter_state() -> (bool, String) {
        let mut buf_len: u32 = 16 * 1024;
        let mut buf = vec![0u8; buf_len as usize];
        let mut ret = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                GAA_FLAG_INCLUDE_PREFIX,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                &mut buf_len,
            )
        };
        if ret == ERROR_BUFFER_OVERFLOW && buf_len > 0 {
            buf.resize(buf_len as usize, 0);
            ret = unsafe {
                GetAdaptersAddresses(
                    AF_UNSPEC as u32,
                    GAA_FLAG_INCLUDE_PREFIX,
                    std::ptr::null_mut(),
                    buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                    &mut buf_len,
                )
            };
        }
        if ret != NO_ERROR {
            return (false, String::new());
        }

        let mut matches = Vec::new();
        let mut cur = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !cur.is_null() {
            let adapter = unsafe { &*cur };
            if adapter.OperStatus == IfOperStatusUp {
                let name = unsafe { pwstr_to_string(adapter.FriendlyName) };
                let desc = unsafe { pwstr_to_string(adapter.Description) };
                if re_tun_desc().is_match(&desc) || re_tun_name().is_match(&name) {
                    matches.push(format!("{name} ({desc})"));
                }
            }
            cur = adapter.Next;
        }

        if matches.is_empty() {
            (false, String::new())
        } else {
            (true, matches.join(", "))
        }
    }

    fn query_process_image_path(pid: u32) -> Option<String> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size) };
        unsafe { CloseHandle(handle) };
        if ok == 0 || size == 0 {
            return None;
        }
        Some(
            OsString::from_wide(&buf[..size as usize])
                .to_string_lossy()
                .to_string(),
        )
    }

    #[derive(Debug, Clone)]
    struct RawProcessInfo {
        pid: u32,
        parent_pid: u32,
        stem: String,
        stem_lower: String,
    }

    fn collect_process_snapshot() -> Vec<RawProcessInfo> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return Vec::new();
        }

        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        let mut list = Vec::new();
        let mut has_next = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while has_next {
            let nul_pos = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe_name = OsString::from_wide(&entry.szExeFile[..nul_pos])
                .to_string_lossy()
                .to_string();
            let stem = exe_name
                .strip_suffix(".exe")
                .or_else(|| exe_name.strip_suffix(".EXE"))
                .unwrap_or(&exe_name)
                .to_string();
            let stem_lower = stem.to_lowercase();
            list.push(RawProcessInfo {
                pid: entry.th32ProcessID,
                parent_pid: entry.th32ParentProcessID,
                stem,
                stem_lower,
            });
            has_next = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }

        unsafe { CloseHandle(snapshot) };
        list
    }

    fn is_antigravity_app_stem(stem_lower: &str) -> bool {
        stem_lower.starts_with("antigravity")
            && stem_lower != "uninstall antigravity"
            && stem_lower != "agydock"
            && stem_lower != "antigravityusage"
            && stem_lower != "antigravity-usage-desktop"
            && stem_lower != "antigravitytunpanel"
            && stem_lower != "antigravity-cli"
    }

    fn is_cli_stem(stem_lower: &str) -> bool {
        stem_lower == "agy" || stem_lower == "antigravity-cli"
    }

    pub fn scan_antigravity_and_clash(antigravity_path: &str) -> (Vec<ProcessEntry>, bool) {
        let install_dir = Path::new(antigravity_path.trim())
            .parent()
            .filter(|p| !p.as_os_str().is_empty());

        let snapshot = collect_process_snapshot();
        let mut procs = Vec::new();
        let mut clash_running = false;

        for p in snapshot {
            if p.stem_lower.contains("clash-verge") || p.stem_lower.contains("verge-mihomo") {
                clash_running = true;
            }

            let is_ls = p.stem_lower.starts_with("language_server");
            let is_agy = is_antigravity_app_stem(&p.stem_lower);
            if is_agy || is_ls {
                if let Some(dir) = install_dir {
                    if let Some(full_path) = query_process_image_path(p.pid) {
                        if is_inside_dir(Path::new(&full_path), dir) {
                            procs.push(ProcessEntry {
                                pid: p.pid,
                                name: p.stem,
                                role: if is_ls {
                                    "language_server".to_string()
                                } else {
                                    "antigravity".to_string()
                                },
                                tool_id: None,
                            });
                        }
                    }
                }
            }
        }

        procs.sort_by_key(|p| p.pid);
        (procs, clash_running)
    }

    pub fn scan_all_tracked_processes(
        settings: &ProxySettings,
    ) -> (Vec<ProcessEntry>, HashMap<String, usize>, bool) {
        let install_dir = Path::new(settings.antigravity_path.trim())
            .parent()
            .filter(|p| !p.as_os_str().is_empty());

        let mut tool_by_stem: HashMap<String, Vec<(String, String, Option<PathBuf>)>> =
            HashMap::new();
        for tool in &settings.cli_tools {
            if !tool.monitor {
                continue;
            }
            let exe_norm = normalize_path_key(&tool.exe_path);
            if exe_norm.is_empty() {
                continue;
            }
            let exe_path = Path::new(tool.exe_path.trim());
            let stem_lower = exe_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if stem_lower.is_empty() {
                continue;
            }
            let root = tool_install_root(exe_path);
            tool_by_stem
                .entry(stem_lower)
                .or_default()
                .push((tool.id.clone(), exe_norm, root));
        }

        let snapshot = collect_process_snapshot();
        let mut procs = Vec::new();
        let mut tool_counts: HashMap<String, usize> = HashMap::new();
        let mut clash_running = false;

        for p in snapshot {
            if p.stem_lower.contains("clash-verge") || p.stem_lower.contains("verge-mihomo") {
                clash_running = true;
            }

            if is_cli_stem(&p.stem_lower) {
                procs.push(ProcessEntry {
                    pid: p.pid,
                    name: p.stem,
                    role: "cli".to_string(),
                    tool_id: None,
                });
                continue;
            }

            let is_ls = p.stem_lower.starts_with("language_server");
            let is_agy = is_antigravity_app_stem(&p.stem_lower);
            if is_agy || is_ls {
                if let Some(dir) = install_dir {
                    if let Some(full_path) = query_process_image_path(p.pid) {
                        if is_inside_dir(Path::new(&full_path), dir) {
                            procs.push(ProcessEntry {
                                pid: p.pid,
                                name: p.stem,
                                role: if is_ls {
                                    "language_server".to_string()
                                } else {
                                    "antigravity".to_string()
                                },
                                tool_id: None,
                            });
                            continue;
                        }
                    }
                }
            }

            if let Some(candidates) = tool_by_stem.get(&p.stem_lower) {
                if let Some(full_path) = query_process_image_path(p.pid) {
                    let full_norm = normalize_path_key(&full_path);
                    let full_path_ref = Path::new(&full_path);
                    if let Some((tool_id, _, _)) = candidates.iter().find(|(_, exe_norm, root)| {
                        *exe_norm == full_norm
                            || root
                                .as_deref()
                                .map(|dir| is_inside_dir(full_path_ref, dir))
                                .unwrap_or(false)
                    }) {
                        *tool_counts.entry(tool_id.clone()).or_insert(0) += 1;
                        procs.push(ProcessEntry {
                            pid: p.pid,
                            name: p.stem,
                            role: "tool".to_string(),
                            tool_id: Some(tool_id.clone()),
                        });
                    }
                }
            }
        }

        procs.sort_by_key(|p| p.pid);
        (procs, tool_counts, clash_running)
    }

    pub fn scan_cli_processes() -> Vec<ProcessEntry> {
        let mut procs: Vec<ProcessEntry> = collect_process_snapshot()
            .into_iter()
            .filter(|p| is_cli_stem(&p.stem_lower))
            .map(|p| ProcessEntry {
                pid: p.pid,
                name: p.stem,
                role: "cli".to_string(),
                tool_id: None,
            })
            .collect();
        procs.sort_by_key(|p| p.pid);
        procs
    }

    pub fn scan_tool_process_tree(exe_path: &str) -> Vec<ProcessEntry> {
        let target_norm = normalize_path_key(exe_path);
        if target_norm.is_empty() {
            return Vec::new();
        }
        let target_path = Path::new(exe_path.trim());
        let target_stem_lower = target_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if target_stem_lower.is_empty() {
            return Vec::new();
        }
        let install_root = tool_install_root(target_path);
        let allow_dir_children = install_root.is_some();

        let snapshot = collect_process_snapshot();
        let mut matched_pids = HashSet::new();
        let mut procs = Vec::new();

        for p in &snapshot {
            if p.stem_lower == target_stem_lower {
                if let Some(full_path) = query_process_image_path(p.pid) {
                    let exact = normalize_path_key(&full_path) == target_norm;
                    let inside_root = install_root
                        .as_deref()
                        .map(|dir| is_inside_dir(Path::new(&full_path), dir))
                        .unwrap_or(false);
                    if (exact || inside_root) && matched_pids.insert(p.pid) {
                        procs.push(ProcessEntry {
                            pid: p.pid,
                            name: p.stem.clone(),
                            role: "tool".to_string(),
                            tool_id: None,
                        });
                    }
                }
            }
        }

        if matched_pids.is_empty() {
            return procs;
        }

        loop {
            let mut added = false;
            for p in &snapshot {
                if matched_pids.contains(&p.pid) || !matched_pids.contains(&p.parent_pid) {
                    continue;
                }
                let is_cli = is_cli_stem(&p.stem_lower);
                let is_ls = p.stem_lower.starts_with("language_server");
                let same_stem = p.stem_lower == target_stem_lower;
                let inside_tool_dir = if !same_stem && !is_cli && !is_ls && allow_dir_children {
                    install_root
                        .as_deref()
                        .zip(query_process_image_path(p.pid))
                        .map(|(dir, fp)| is_inside_dir(Path::new(&fp), dir))
                        .unwrap_or(false)
                } else {
                    false
                };

                if same_stem || is_cli || is_ls || inside_tool_dir {
                    matched_pids.insert(p.pid);
                    procs.push(ProcessEntry {
                        pid: p.pid,
                        name: p.stem.clone(),
                        role: if is_cli {
                            "cli".to_string()
                        } else if is_ls {
                            "language_server".to_string()
                        } else {
                            "tool".to_string()
                        },
                        tool_id: None,
                    });
                    added = true;
                }
            }
            if !added {
                break;
            }
        }

        procs.sort_by_key(|p| p.pid);
        procs
    }

    pub fn terminate_pid(pid: u32) -> Result<(), String> {
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
        if handle.is_null() {
            return Err("无法打开进程句柄".to_string());
        }
        let ok = unsafe { TerminateProcess(handle, 1) };
        unsafe { CloseHandle(handle) };
        if ok == 0 {
            Err("TerminateProcess 调用失败".to_string())
        } else {
            Ok(())
        }
    }

    fn decode_port(raw_port: u32) -> u16 {
        (((raw_port & 0xFF) << 8) | ((raw_port >> 8) & 0xFF)) as u16
    }

    fn state_label(state: u32) -> Option<&'static str> {
        match state {
            MIB_TCP_STATE_ESTAB => Some("ESTABLISHED"),
            MIB_TCP_STATE_SYN_SENT => Some("SYN_SENT"),
            _ => None,
        }
    }

    fn normalize_ipv6(bytes: [u8; 16]) -> IpAddr {
        let v6 = Ipv6Addr::from(bytes);
        if let Some(v4) = v6.to_ipv4_mapped() {
            IpAddr::V4(v4)
        } else {
            IpAddr::V6(v6)
        }
    }

    fn format_endpoint(ip: IpAddr, port: u16) -> String {
        match ip {
            IpAddr::V4(v4) => format!("{v4}:{port}"),
            IpAddr::V6(v6) => format!("[{v6}]:{port}"),
        }
    }

    fn query_tcp_table(af: u32, pid_filter: &HashSet<u32>, is_ipv6: bool, out: &mut Vec<RawTcpSocket>) {
        let mut size: u32 = 0;
        let mut ret = unsafe {
            GetExtendedTcpTable(
                std::ptr::null_mut(),
                &mut size,
                0,
                af,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
        if ret != ERROR_INSUFFICIENT_BUFFER && ret != NO_ERROR {
            return;
        }
        if size == 0 {
            return;
        }

        let mut buf: Vec<u8> = Vec::new();
        for _ in 0..3 {
            buf.resize(size as usize, 0);
            ret = unsafe {
                GetExtendedTcpTable(
                    buf.as_mut_ptr() as *mut _,
                    &mut size,
                    0,
                    af,
                    TCP_TABLE_OWNER_PID_ALL,
                    0,
                )
            };
            if ret == NO_ERROR {
                break;
            }
            if ret != ERROR_INSUFFICIENT_BUFFER {
                return;
            }
        }
        if ret != NO_ERROR || buf.len() < 4 {
            return;
        }

        let num_entries = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if !is_ipv6 {
            let row_size = std::mem::size_of::<MIB_TCPROW_OWNER_PID>();
            let mut offset = 4usize;
            for _ in 0..num_entries {
                if offset + row_size > buf.len() {
                    break;
                }
                let row = unsafe {
                    std::ptr::read_unaligned(buf.as_ptr().add(offset) as *const MIB_TCPROW_OWNER_PID)
                };
                offset += row_size;
                if let Some(st) = state_label(row.dwState) {
                    if pid_filter.contains(&row.dwOwningPid) {
                        let local_ip = IpAddr::V4(Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()));
                        let remote_ip = IpAddr::V4(Ipv4Addr::from(row.dwRemoteAddr.to_ne_bytes()));
                        let local_port = decode_port(row.dwLocalPort);
                        let remote_port = decode_port(row.dwRemotePort);
                        out.push(RawTcpSocket {
                            pid: row.dwOwningPid,
                            local_endpoint: format_endpoint(local_ip, local_port),
                            remote_host: remote_ip.to_string(),
                            remote_port,
                            remote_endpoint: format_endpoint(remote_ip, remote_port),
                            state: st.to_string(),
                        });
                    }
                }
            }
        } else {
            let row_size = std::mem::size_of::<MIB_TCP6ROW_OWNER_PID>();
            let mut offset = 4usize;
            for _ in 0..num_entries {
                if offset + row_size > buf.len() {
                    break;
                }
                let row = unsafe {
                    std::ptr::read_unaligned(buf.as_ptr().add(offset) as *const MIB_TCP6ROW_OWNER_PID)
                };
                offset += row_size;
                if let Some(st) = state_label(row.dwState) {
                    if pid_filter.contains(&row.dwOwningPid) {
                        let local_ip = normalize_ipv6(row.ucLocalAddr);
                        let remote_ip = normalize_ipv6(row.ucRemoteAddr);
                        let local_port = decode_port(row.dwLocalPort);
                        let remote_port = decode_port(row.dwRemotePort);
                        out.push(RawTcpSocket {
                            pid: row.dwOwningPid,
                            local_endpoint: format_endpoint(local_ip, local_port),
                            remote_host: remote_ip.to_string(),
                            remote_port,
                            remote_endpoint: format_endpoint(remote_ip, remote_port),
                            state: st.to_string(),
                        });
                    }
                }
            }
        }
    }

    pub fn get_active_tcp_sockets(pid_filter: &HashSet<u32>) -> Vec<RawTcpSocket> {
        let mut out = Vec::new();
        if pid_filter.is_empty() {
            return out;
        }
        query_tcp_table(AF_INET as u32, pid_filter, false, &mut out);
        query_tcp_table(AF_INET6 as u32, pid_filter, true, &mut out);
        out
    }
}

#[cfg(not(windows))]
mod win_native {
    use super::*;
    pub fn get_system_proxy_state() -> (bool, String) {
        (false, String::new())
    }
    pub fn read_full_system_proxy() -> SavedSystemProxy {
        SavedSystemProxy {
            enabled: false,
            server: None,
            override_list: None,
        }
    }
    pub fn set_system_proxy(
        _enabled: bool,
        _server: Option<&str>,
        _override_list: Option<&str>,
    ) -> Result<(), String> {
        Err("仅支持 Windows".to_string())
    }
    pub fn restore_system_proxy(_saved: &SavedSystemProxy) -> Result<(), String> {
        Ok(())
    }
    pub fn read_user_path() -> Result<(String, bool), String> {
        Ok((String::new(), true))
    }
    pub fn write_user_path(_value: &str, _is_expand_sz: bool) -> Result<(), String> {
        Err("仅支持 Windows".to_string())
    }
    pub fn get_tun_adapter_state() -> (bool, String) {
        (false, String::new())
    }
    pub fn scan_antigravity_and_clash(_antigravity_path: &str) -> (Vec<ProcessEntry>, bool) {
        (Vec::new(), false)
    }
    pub fn scan_all_tracked_processes(
        _settings: &ProxySettings,
    ) -> (Vec<ProcessEntry>, HashMap<String, usize>, bool) {
        (Vec::new(), HashMap::new(), false)
    }
    pub fn scan_cli_processes() -> Vec<ProcessEntry> {
        Vec::new()
    }
    pub fn scan_tool_process_tree(_exe_path: &str) -> Vec<ProcessEntry> {
        Vec::new()
    }
    pub fn terminate_pid(_pid: u32) -> Result<(), String> {
        Err("仅支持 Windows".to_string())
    }
    pub fn get_active_tcp_sockets(_pid_filter: &HashSet<u32>) -> Vec<RawTcpSocket> {
        Vec::new()
    }
}

pub fn build_snapshot(settings: &ProxySettings) -> ProxySnapshot {
    let (resolved_port, port_source, is_listening, clash_mode) = resolve_port(settings);
    let (sys_proxy_enabled, sys_proxy_server) = win_native::get_system_proxy_state();
    let (tun_enabled, tun_adapter_text) = win_native::get_tun_adapter_state();
    let (procs, tool_running_counts, clash_running) = win_native::scan_all_tracked_processes(settings);

    let mut pid_map: HashMap<u32, String> = HashMap::new();
    let mut pid_set: HashSet<u32> = HashSet::new();
    let mut agy_count = 0usize;
    let mut ls_count = 0usize;
    let mut cli_count = 0usize;
    let mut tool_count = 0usize;

    for p in &procs {
        pid_map.insert(p.pid, p.name.clone());
        pid_set.insert(p.pid);
        match p.role.as_str() {
            "language_server" => ls_count += 1,
            "cli" => cli_count += 1,
            "tool" => tool_count += 1,
            _ => agy_count += 1,
        }
    }

    let seen_at = Local::now().format("%H:%M:%S").to_string();
    let raw_sockets = win_native::get_active_tcp_sockets(&pid_set);
    let mut unique: HashMap<String, TcpConnectionItem> = HashMap::new();

    for sock in raw_sockets {
        let key = format!("{}|{}->{}", sock.pid, sock.local_endpoint, sock.remote_endpoint);
        if unique.contains_key(&key) {
            continue;
        }
        let cat = classify_remote(
            &sock.remote_host,
            sock.remote_port,
            &settings.proxy_host,
            resolved_port,
        );
        let proc_name = pid_map
            .get(&sock.pid)
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        unique.insert(
            key.clone(),
            TcpConnectionItem {
                key,
                category: cat,
                category_label: cat.label().to_string(),
                pid: sock.pid,
                process_name: proc_name,
                local_endpoint: sock.local_endpoint,
                remote_endpoint: sock.remote_endpoint,
                remote_host: sock.remote_host,
                remote_port: sock.remote_port,
                state: sock.state,
                seen_at: seen_at.clone(),
            },
        );
    }

    let mut connections: Vec<TcpConnectionItem> = unique.into_values().collect();
    connections.sort_by(|a, b| {
        a.category
            .sort_order()
            .cmp(&b.category.sort_order())
            .then_with(|| a.process_name.to_lowercase().cmp(&b.process_name.to_lowercase()))
            .then_with(|| a.pid.cmp(&b.pid))
            .then_with(|| a.remote_endpoint.cmp(&b.remote_endpoint))
    });

    let proxied_count = connections
        .iter()
        .filter(|c| c.category == ConnectionCategory::Proxied)
        .count();
    let tun_count = connections
        .iter()
        .filter(|c| c.category == ConnectionCategory::TunFakeIp)
        .count();
    let direct_count = connections
        .iter()
        .filter(|c| c.category == ConnectionCategory::PublicDirect)
        .count();
    let cli_shim = get_cli_shim_status(settings, resolved_port);

    ProxySnapshot {
        proxy_host: settings.proxy_host.clone(),
        resolved_port,
        port_source,
        is_proxy_listening: is_listening,
        clash_mode,
        is_clash_running: clash_running,
        system_proxy_enabled: sys_proxy_enabled,
        system_proxy_server: sys_proxy_server,
        gemini_system_proxy_managed: is_gemini_system_proxy_managed(),
        tun_enabled,
        tun_adapter_text,
        antigravity_proc_count: agy_count,
        language_server_proc_count: ls_count,
        cli_proc_count: cli_count,
        tool_proc_count: tool_count,
        tool_running_counts,
        cli_shim,
        processes: procs,
        proxied_count,
        tun_count,
        direct_count,
        connections,
        captured_at: seen_at,
    }
}

pub fn start_clash_verge(settings: &ProxySettings) -> (u16, Vec<String>) {
    let mut logs = Vec::new();
    let path = Path::new(settings.clash_verge_path.trim());
    if settings.clash_verge_path.trim().is_empty() || !path.is_file() {
        logs.push(format!(
            "[X] 找不到 Clash Verge 可执行文件: {}",
            settings.clash_verge_path
        ));
        return (0, logs);
    }

    let (cfg_port, _) = read_clash_verge_config(&settings.clash_verge_path);
    let expected_port = if !settings.auto_detect_port && settings.custom_port > 0 {
        settings.custom_port
    } else if cfg_port > 0 {
        cfg_port
    } else {
        7897
    };

    logs.push(format!(
        "[..] 正在启动/唤醒 Clash Verge: {}",
        settings.clash_verge_path
    ));

    let mut cmd = Command::new(path);
    if let Some(dir) = path.parent() {
        cmd.current_dir(dir);
    }
    if let Err(err) = cmd.spawn() {
        logs.push(format!("[X] 启动 Clash Verge 失败: {err}"));
        return (0, logs);
    }

    for _ in 0..40 {
        thread::sleep(Duration::from_millis(500));
        if test_tcp_port(&settings.proxy_host, expected_port, 350) {
            logs.push(format!(
                "[OK] Clash Verge 本地代理端口 {}:{} 已就绪。",
                settings.proxy_host, expected_port
            ));
            return (expected_port, logs);
        }
        let (p, _, listening, _) = resolve_port(settings);
        if listening {
            logs.push(format!(
                "[OK] 检测到本地代理端口 {}:{} 已就绪。",
                settings.proxy_host, p
            ));
            return (p, logs);
        }
    }

    logs.push("[!] 等待 Clash Verge 本地代理端口就绪超时（20 秒）。".to_string());
    (0, logs)
}

pub fn flush_dns_cache() -> Vec<String> {
    let mut logs = vec!["[..] 正在执行 ipconfig /flushdns ...".to_string()];
    let mut cmd = Command::new("ipconfig.exe");
    cmd.arg("/flushdns");
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    match cmd.output() {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let combined = if stderr.trim().is_empty() {
                stdout.to_string()
            } else {
                format!("{stdout}\n{stderr}")
            };
            for line in combined.lines() {
                let t = line.trim();
                if !t.is_empty() {
                    logs.push(format!("    {t}"));
                }
            }
            logs.push(format!(
                "[OK] DNS 刷新完成 (退出码 {})。",
                out.status.code().unwrap_or(-1)
            ));
        }
        Err(err) => {
            logs.push(format!("[X] 无法执行 ipconfig.exe: {err}"));
        }
    }
    logs
}

fn stop_processes_with<F>(
    mut scan: F,
    empty_msg: &str,
    closing_label: &str,
    done_msg: &str,
) -> (bool, Vec<String>)
where
    F: FnMut() -> Vec<ProcessEntry>,
{
    let mut logs = Vec::new();
    let procs = scan();
    if procs.is_empty() {
        logs.push(empty_msg.to_string());
        return (true, logs);
    }

    logs.push(format!("[..] 正在关闭 {} 个{closing_label}...", procs.len()));
    for p in &procs {
        if let Err(err) = win_native::terminate_pid(p.pid) {
            logs.push(format!("    忽略关闭 {}(PID {}): {err}", p.name, p.pid));
        }
    }

    for _ in 0..16 {
        thread::sleep(Duration::from_millis(300));
        let rem = scan();
        if rem.is_empty() {
            logs.push(done_msg.to_string());
            return (true, logs);
        }
    }

    let remaining = scan();
    logs.push(format!("[!] 仍有 {} 个进程未完全退出。", remaining.len()));
    (remaining.is_empty(), logs)
}

pub fn stop_antigravity_processes(antigravity_path: &str) -> (bool, Vec<String>) {
    stop_processes_with(
        || win_native::scan_antigravity_and_clash(antigravity_path).0,
        "[i] 当前没有正在运行的 Antigravity 进程。",
        " Antigravity 相关进程",
        "[OK] 所有旧 Antigravity 进程已退出。",
    )
}

fn is_gemini_desktop_path(exe_path: &str) -> bool {
    let norm = normalize_path_key(exe_path);
    norm.contains(r"\google\gemini\") && norm.ends_with(r"\gemini.exe")
}

fn stop_cli_tool_processes_inner(
    exe_path: &str,
    tool_name: &str,
    restore_managed_proxy: bool,
) -> (bool, Vec<String>) {
    let (ok, mut logs) = stop_processes_with(
        || win_native::scan_tool_process_tree(exe_path),
        &format!("[i] 当前没有正在运行的 {tool_name} 进程。"),
        &format!(" {tool_name} 相关进程"),
        &format!("[OK] 所有旧 {tool_name} 相关进程已退出。"),
    );
    if restore_managed_proxy && is_gemini_desktop_path(exe_path) {
        if let Some(res) = restore_managed_system_proxy_if_active() {
            match res {
                Ok(()) => logs.push("[OK] 已自动关闭 Windows 系统代理（恢复启动前状态）。".to_string()),
                Err(err) => logs.push(format!("[!] 自动恢复 Windows 系统代理失败: {err}")),
            }
        }
    }
    (ok, logs)
}

pub fn stop_cli_tool_processes(exe_path: &str, tool_name: &str) -> (bool, Vec<String>) {
    stop_cli_tool_processes_inner(exe_path, tool_name, true)
}

pub fn stop_cli_processes() -> (bool, Vec<String>) {
    stop_processes_with(
        win_native::scan_cli_processes,
        "[i] 当前没有正在运行的 agy CLI 进程。",
        " agy CLI 进程",
        "[OK] 所有 agy CLI 进程已退出。",
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityRuntimeStatus {
    pub exe_valid: bool,
    pub antigravity_path: String,
    pub running_count: usize,
    pub proxy_listening: bool,
    pub proxy_endpoint: String,
    pub auto_start_clash: bool,
}

pub fn get_antigravity_runtime_status(settings: &ProxySettings) -> AntigravityRuntimeStatus {
    let exe_valid = Path::new(settings.antigravity_path.trim()).is_file();
    let (procs, _) = win_native::scan_antigravity_and_clash(&settings.antigravity_path);
    let (port, _, proxy_listening, _) = resolve_port(settings);
    AntigravityRuntimeStatus {
        exe_valid,
        antigravity_path: settings.antigravity_path.clone(),
        running_count: procs.len(),
        proxy_listening,
        proxy_endpoint: format!("{}:{}", settings.proxy_host, port),
        auto_start_clash: settings.auto_start_clash,
    }
}

fn normalize_custom_bypass_entry(raw: &str) -> Option<String> {
    let mut s = raw.trim().to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    {
        s = rest.to_string();
    }
    if let Some(idx) = s.find(['/', '?', '#']) {
        // 保留 CIDR 网段中的 /prefix（例如 10.20.0.0/16）
        let is_cidr = s
            .split_once('/')
            .map(|(ip_part, prefix)| {
                ip_part.parse::<IpAddr>().is_ok() && prefix.parse::<u8>().is_ok()
            })
            .unwrap_or(false);
        if !is_cidr {
            s.truncate(idx);
        }
    }
    if !s.contains('/') && !s.starts_with('[') && s.matches(':').count() == 1 {
        if let Some((host, port)) = s.split_once(':') {
            if port.chars().all(|c| c.is_ascii_digit()) {
                s = host.to_string();
            }
        }
    }
    let s = s.trim_start_matches("*.").trim_start_matches('.').trim();
    if s.is_empty()
        || !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '/' | '[' | ']'))
    {
        return None;
    }
    Some(s.to_string())
}

/// 生成 Chromium `--proxy-bypass-list` 与通用环境变量 `NO_PROXY` 规则列表。
///
/// - Chromium 支持 `domain;*.domain`、`*.cn`、CIDR 与 `<local>`；
/// - Go / Node (`NODE_USE_ENV_PROXY=1`) / curl / Python 的 `NO_PROXY` 使用 `.domain` 后缀与 CIDR（不带 `*`）。
pub fn build_bypass_lists(settings: &ProxySettings) -> (String, String, usize) {
    let mut chromium: Vec<String> = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "[::1]".to_string(),
        "<local>".to_string(),
    ];
    let mut no_proxy: Vec<String> = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
        "[::1]".to_string(),
    ];

    let mut seen_domains: HashSet<String> = HashSet::new();

    if settings.bypass_china_and_lan {
        for cidr in LAN_BYPASS_CIDRS {
            chromium.push((*cidr).to_string());
            no_proxy.push((*cidr).to_string());
        }
        for suffix in ["local", "lan", "cn"] {
            chromium.push(format!("*.{suffix}"));
            no_proxy.push(format!(".{suffix}"));
            seen_domains.insert(suffix.to_string());
        }
        for domain in CHINA_DIRECT_DOMAINS {
            if seen_domains.insert((*domain).to_string()) {
                chromium.push((*domain).to_string());
                chromium.push(format!("*.{domain}"));
                no_proxy.push(format!(".{domain}"));
            }
        }
    }

    let mut custom_count = 0usize;
    for token in settings
        .custom_bypass_domains
        .split(|c: char| matches!(c, ',' | ';' | '\n' | '\r' | '\t' | ' '))
    {
        let Some(clean) = normalize_custom_bypass_entry(token) else {
            continue;
        };
        if !seen_domains.insert(clean.clone()) {
            continue;
        }
        custom_count += 1;
        let is_ip_or_cidr = clean.contains('/') || clean.parse::<IpAddr>().is_ok() || clean.starts_with('[');
        if is_ip_or_cidr {
            chromium.push(clean.clone());
            no_proxy.push(clean);
        } else {
            chromium.push(clean.clone());
            chromium.push(format!("*.{clean}"));
            no_proxy.push(format!(".{clean}"));
        }
    }

    (chromium.join(";"), no_proxy.join(","), custom_count)
}

fn apply_proxy_env_only(cmd: &mut Command, settings: &ProxySettings, proxy_url: &str) -> String {
    let (_, env_no_proxy, _) = build_bypass_lists(settings);
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        cmd.env(key, proxy_url);
    }
    for key in ["NO_PROXY", "no_proxy"] {
        cmd.env(key, &env_no_proxy);
    }
    cmd.env("NODE_USE_ENV_PROXY", "1");
    env_no_proxy
}

fn apply_proxy_env_and_args(cmd: &mut Command, settings: &ProxySettings, proxy_url: &str) {
    let (chromium_bypass, _, _) = build_bypass_lists(settings);
    cmd.arg(format!("--proxy-server={proxy_url}"));
    cmd.arg(format!("--proxy-bypass-list={chromium_bypass}"));
    apply_proxy_env_only(cmd, settings, proxy_url);
}

pub fn split_command_args(raw: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut has_token = false;
    let mut in_double = false;
    let mut in_single = false;
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_double {
            if ch == '\\' && chars.peek() == Some(&'"') {
                cur.push(chars.next().unwrap());
            } else if ch == '"' {
                in_double = false;
            } else {
                cur.push(ch);
            }
        } else if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                cur.push(ch);
            }
        } else {
            match ch {
                '"' => {
                    in_double = true;
                    has_token = true;
                }
                '\'' => {
                    in_single = true;
                    has_token = true;
                }
                c if c.is_whitespace() => {
                    if has_token {
                        args.push(std::mem::take(&mut cur));
                        has_token = false;
                    }
                }
                c => {
                    cur.push(c);
                    has_token = true;
                }
            }
        }
    }

    if has_token {
        args.push(cur);
    }
    args
}

fn ensure_listening_port(settings: &ProxySettings, logs: &mut Vec<String>) -> Result<u16, (u16, String)> {
    let (mut target_port, _, is_listening, _) = resolve_port(settings);
    if is_listening {
        return Ok(target_port);
    }
    if settings.auto_start_clash && Path::new(settings.clash_verge_path.trim()).is_file() {
        let (started_port, clash_logs) = start_clash_verge(settings);
        logs.extend(clash_logs);
        if started_port == 0 {
            return Err((
                target_port,
                "自动启动 Clash Verge 后端口仍未就绪，请确认 Clash Verge 正常运行。".to_string(),
            ));
        }
        target_port = started_port;
        Ok(target_port)
    } else {
        let msg = format!(
            "本地代理端口 {}:{} 未监听，请先启动 Clash Verge。",
            settings.proxy_host, target_port
        );
        logs.push(format!("[X] {msg}"));
        Err((target_port, msg))
    }
}

/// 账号切换后重启 Antigravity：调用前进程应已停止。
///
/// 与 `launch_isolated_antigravity` 的区别：若代理端口未监听且开启了自动拉起 Clash，
/// 会先尝试拉起 Clash Verge；若仍未就绪也不阻塞，退化为普通启动并在日志里说明。
pub fn launch_antigravity_after_stop(settings: &ProxySettings) -> (bool, Vec<String>, Option<String>) {
    let mut logs = Vec::new();
    let agy_path = Path::new(settings.antigravity_path.trim());
    if !agy_path.is_file() {
        let msg = format!("找不到 Antigravity 可执行文件: {}", settings.antigravity_path);
        logs.push(format!("[X] {msg}"));
        return (false, logs, Some(msg));
    }

    let (mut port, _source, mut is_listening, _) = resolve_port(settings);
    if !is_listening
        && settings.auto_start_clash
        && Path::new(settings.clash_verge_path.trim()).is_file()
    {
        let (started_port, clash_logs) = start_clash_verge(settings);
        logs.extend(clash_logs);
        if started_port > 0 {
            port = started_port;
            is_listening = true;
        }
    }

    let proxy_url = if is_listening {
        Some(format!("http://{}:{}", settings.proxy_host, port))
    } else {
        None
    };

    let mut cmd = Command::new(agy_path);
    if let Some(dir) = agy_path.parent() {
        cmd.current_dir(dir);
    }

    match &proxy_url {
        Some(url) => {
            apply_proxy_env_and_args(&mut cmd, settings, url);
            let bypass_note = if settings.bypass_china_and_lan {
                "（已启用国内/内网直连绕过）"
            } else {
                ""
            };
            logs.push(format!(
                "[OK] Antigravity 已以隔离代理方式启动 -> {url}{bypass_note}"
            ));
        }
        None => {
            logs.push(
                "[i] 本地代理端口未监听，本次以普通方式启动 Antigravity（不经过本地代理）。".to_string(),
            );
        }
    }

    if settings.disable_quic {
        cmd.arg("--disable-quic");
    }
    if !settings.workspace_path.trim().is_empty() {
        cmd.arg(settings.workspace_path.trim());
    }

    match cmd.spawn() {
        Ok(child) => {
            logs.push(format!("[OK] Antigravity 已启动 (PID {})", child.id()));
            (true, logs, None)
        }
        Err(err) => {
            let msg = format!("启动 Antigravity 失败: {err}");
            logs.push(format!("[X] {msg}"));
            (false, logs, Some(msg))
        }
    }
}

pub fn prepend_path_entry(existing: &str, dir: &str) -> (String, bool) {
    let target_key = normalize_path_key(dir);
    if target_key.is_empty() {
        return (existing.to_string(), false);
    }
    let clean_dir = dir.trim().trim_end_matches(['\\', '/']);
    let existing_entries: Vec<&str> = existing
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    if existing_entries.first().map(|s| normalize_path_key(s)) == Some(target_key.clone())
        && existing_entries
            .iter()
            .skip(1)
            .all(|s| normalize_path_key(s) != target_key)
    {
        return (existing.to_string(), false);
    }

    let mut next: Vec<String> = Vec::with_capacity(existing_entries.len() + 1);
    next.push(clean_dir.to_string());
    for entry in existing_entries {
        if normalize_path_key(entry) != target_key {
            next.push(entry.to_string());
        }
    }
    (next.join(";"), true)
}

pub fn remove_path_entry(existing: &str, dir: &str) -> (String, bool) {
    let target_key = normalize_path_key(dir);
    if target_key.is_empty() {
        return (existing.to_string(), false);
    }
    let mut kept = Vec::new();
    let mut removed = false;
    for entry in existing.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        if normalize_path_key(entry) == target_key {
            removed = true;
        } else {
            kept.push(entry.to_string());
        }
    }
    if !removed {
        return (existing.to_string(), false);
    }
    (kept.join(";"), true)
}

fn path_contains_entry(existing: &str, dir: &str) -> bool {
    let target_key = normalize_path_key(dir);
    if target_key.is_empty() {
        return false;
    }
    existing
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|s| normalize_path_key(s) == target_key)
}

pub fn render_cli_shim_scripts(target_exe: &str, proxy_url: &str, no_proxy: &str) -> (String, String, String) {
    let esc_cmd = |s: &str| s.replace('%', "%%");
    let esc_ps = |s: &str| s.replace('\'', "''");
    let esc_sh = |s: &str| s.replace('\'', "'\\''");

    let proxy_cmd = esc_cmd(proxy_url);
    let no_proxy_cmd = esc_cmd(no_proxy);
    let target_cmd = esc_cmd(target_exe);

    let agy_cmd = format!(
        "@echo off\r\n\
setlocal EnableExtensions\r\n\
set \"HTTP_PROXY={proxy_cmd}\"\r\n\
set \"HTTPS_PROXY={proxy_cmd}\"\r\n\
set \"ALL_PROXY={proxy_cmd}\"\r\n\
set \"http_proxy={proxy_cmd}\"\r\n\
set \"https_proxy={proxy_cmd}\"\r\n\
set \"all_proxy={proxy_cmd}\"\r\n\
set \"NO_PROXY={no_proxy_cmd}\"\r\n\
set \"no_proxy={no_proxy_cmd}\"\r\n\
set \"NODE_USE_ENV_PROXY=1\"\r\n\
\"{target_cmd}\" %*\r\n\
set \"AGYDOCK_EXIT=%ERRORLEVEL%\"\r\n\
endlocal & exit /b %AGYDOCK_EXIT%\r\n"
    );

    let proxy_ps = esc_ps(proxy_url);
    let no_proxy_ps = esc_ps(no_proxy);
    let target_ps = esc_ps(target_exe);

    let agy_ps1 = format!(
        "$ErrorActionPreference = 'Stop'\r\n\
$__agydockPrev = @{{}}\r\n\
$__agydockVars = @{{\r\n\
  'HTTP_PROXY' = '{proxy_ps}'\r\n\
  'HTTPS_PROXY' = '{proxy_ps}'\r\n\
  'ALL_PROXY' = '{proxy_ps}'\r\n\
  'http_proxy' = '{proxy_ps}'\r\n\
  'https_proxy' = '{proxy_ps}'\r\n\
  'all_proxy' = '{proxy_ps}'\r\n\
  'NO_PROXY' = '{no_proxy_ps}'\r\n\
  'no_proxy' = '{no_proxy_ps}'\r\n\
  'NODE_USE_ENV_PROXY' = '1'\r\n\
}}\r\n\
foreach ($k in $__agydockVars.Keys) {{\r\n\
  $__agydockPrev[$k] = [Environment]::GetEnvironmentVariable($k, 'Process')\r\n\
  [Environment]::SetEnvironmentVariable($k, $__agydockVars[$k], 'Process')\r\n\
}}\r\n\
$__agydockExit = 0\r\n\
try {{\r\n\
  & '{target_ps}' @args\r\n\
  if ($null -ne $LASTEXITCODE) {{ $__agydockExit = [int]$LASTEXITCODE }}\r\n\
}} finally {{\r\n\
  foreach ($k in $__agydockPrev.Keys) {{\r\n\
    [Environment]::SetEnvironmentVariable($k, $__agydockPrev[$k], 'Process')\r\n\
  }}\r\n\
}}\r\n\
exit $__agydockExit\r\n"
    );

    let proxy_sh = esc_sh(proxy_url);
    let no_proxy_sh = esc_sh(no_proxy);
    let target_unix = target_exe.replace('\\', "/");
    let target_unix_sh = esc_sh(&target_unix);

    let agy_sh = format!(
        "#!/usr/bin/env sh\n\
export HTTP_PROXY='{proxy_sh}'\n\
export HTTPS_PROXY='{proxy_sh}'\n\
export ALL_PROXY='{proxy_sh}'\n\
export http_proxy='{proxy_sh}'\n\
export https_proxy='{proxy_sh}'\n\
export all_proxy='{proxy_sh}'\n\
export NO_PROXY='{no_proxy_sh}'\n\
export no_proxy='{no_proxy_sh}'\n\
export NODE_USE_ENV_PROXY='1'\n\
exec '{target_unix_sh}' \"$@\"\n"
    );

    (agy_cmd, agy_ps1, agy_sh)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShimMeta {
    target_exe: String,
    proxy_url: String,
    no_proxy: String,
    updated_at: String,
}

fn write_cli_shim_files(
    shim_dir: &Path,
    settings: &ProxySettings,
    proxy_port: u16,
) -> Result<String, String> {
    let target_exe = settings.cli_path.trim();
    if target_exe.is_empty() || !Path::new(target_exe).is_file() {
        return Err(format!("找不到 Antigravity CLI (agy.exe): {target_exe}"));
    }
    if is_inside_dir(Path::new(target_exe), shim_dir) {
        return Err("CLI 路径不能指向 AgyDock Shim 目录自身，请先选择真实的 agy.exe。".to_string());
    }

    fs::create_dir_all(shim_dir).map_err(|e| format!("无法创建 Shim 目录: {e}"))?;
    let proxy_url = format!("http://{}:{}", settings.proxy_host, proxy_port);
    let (_, no_proxy, _) = build_bypass_lists(settings);
    let (cmd_script, ps1_script, sh_script) =
        render_cli_shim_scripts(target_exe, &proxy_url, &no_proxy);

    fs::write(shim_dir.join("agy.cmd"), cmd_script.as_bytes())
        .map_err(|e| format!("写入 agy.cmd 失败: {e}"))?;
    fs::write(shim_dir.join("agy.ps1"), ps1_script.as_bytes())
        .map_err(|e| format!("写入 agy.ps1 失败: {e}"))?;
    fs::write(shim_dir.join("agy"), sh_script.as_bytes())
        .map_err(|e| format!("写入 agy 失败: {e}"))?;

    let meta = ShimMeta {
        target_exe: target_exe.to_string(),
        proxy_url: proxy_url.clone(),
        no_proxy,
        updated_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    };
    let meta_json = serde_json::to_string_pretty(&meta)
        .map_err(|e| format!("序列化 Shim 元数据失败: {e}"))?;
    fs::write(shim_dir.join("shim-meta.json"), meta_json.as_bytes())
        .map_err(|e| format!("写入 shim-meta.json 失败: {e}"))?;

    Ok(proxy_url)
}

pub fn get_cli_shim_status(settings: &ProxySettings, resolved_port: u16) -> CliShimStatus {
    let shim_dir = cli_shim_dir();
    let shim_dir_str = shim_dir.to_string_lossy().to_string();
    let in_user_path = win_native::read_user_path()
        .map(|(p, _)| path_contains_entry(&p, &shim_dir_str))
        .unwrap_or(false);

    let scripts_ok = shim_dir.join("agy.cmd").is_file()
        && shim_dir.join("agy.ps1").is_file()
        && shim_dir.join("agy").is_file();
    let meta = fs::read_to_string(shim_dir.join("shim-meta.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<ShimMeta>(&s).ok());

    let proxy_url = format!("http://{}:{}", settings.proxy_host, resolved_port);
    let (_, expected_no_proxy, _) = build_bypass_lists(settings);

    let cli_path_trimmed = settings.cli_path.trim().to_string();
    let cli_detected = !cli_path_trimmed.is_empty()
        && Path::new(&cli_path_trimmed).is_file()
        && !is_inside_dir(Path::new(&cli_path_trimmed), &shim_dir);

    let enabled = in_user_path;
    let out_of_sync = enabled
        && (!scripts_ok
            || meta
                .as_ref()
                .map(|m| {
                    normalize_path_key(&m.target_exe) != normalize_path_key(&cli_path_trimmed)
                        || m.proxy_url != proxy_url
                        || m.no_proxy != expected_no_proxy
                })
                .unwrap_or(true));

    let target_exe = meta.map(|m| m.target_exe).unwrap_or_default();

    CliShimStatus {
        enabled,
        in_user_path,
        shim_dir: shim_dir_str,
        target_exe,
        proxy_url,
        out_of_sync,
        cli_detected,
        cli_path: cli_path_trimmed,
    }
}

pub fn enable_cli_shim(settings: &ProxySettings) -> ActionResponse {
    let mut logs = Vec::new();
    let shim_dir = cli_shim_dir();
    let shim_dir_str = shim_dir.to_string_lossy().to_string();
    let (port, _, is_listening, _) = resolve_port(settings);

    let proxy_url = match write_cli_shim_files(&shim_dir, settings, port) {
        Ok(url) => url,
        Err(msg) => {
            logs.push(format!("[X] {msg}"));
            return ActionResponse {
                ok: false,
                need_confirm: false,
                process_count: 0,
                resolved_port: port,
                logs,
                error: Some(msg),
            };
        }
    };

    match win_native::read_user_path() {
        Ok((existing, is_expand_sz)) => {
            let (next_path, changed) = prepend_path_entry(&existing, &shim_dir_str);
            if changed {
                if let Err(err) = win_native::write_user_path(&next_path, is_expand_sz) {
                    logs.push(format!("[X] {err}"));
                    return ActionResponse {
                        ok: false,
                        need_confirm: false,
                        process_count: 0,
                        resolved_port: port,
                        logs,
                        error: Some(err),
                    };
                }
                logs.push(format!(
                    "[OK] 已将隔离 CLI Shim 目录加入用户 PATH: {shim_dir_str}"
                ));
            } else {
                logs.push(format!(
                    "[OK] 已同步 CLI Shim 脚本 -> {proxy_url} ({})",
                    settings.cli_path
                ));
            }
        }
        Err(err) => {
            logs.push(format!("[X] {err}"));
            return ActionResponse {
                ok: false,
                need_confirm: false,
                process_count: 0,
                resolved_port: port,
                logs,
                error: Some(err),
            };
        }
    }

    logs.push(format!(
        "[OK] CLI Proxy Shim 已就绪: agy -> {proxy_url}（新开终端或新启动的 PATH 调用工具自动生效）"
    ));
    if !is_listening {
        logs.push(format!(
            "[!] 当前本地代理端口 {}:{} 未监听，启动 Clash Verge 后即可正常连网。",
            settings.proxy_host, port
        ));
    }

    ActionResponse {
        ok: true,
        need_confirm: false,
        process_count: 0,
        resolved_port: port,
        logs,
        error: None,
    }
}

pub fn disable_cli_shim() -> ActionResponse {
    let mut logs = Vec::new();
    let shim_dir = cli_shim_dir();
    let shim_dir_str = shim_dir.to_string_lossy().to_string();

    match win_native::read_user_path() {
        Ok((existing, is_expand_sz)) => {
            let (next_path, changed) = remove_path_entry(&existing, &shim_dir_str);
            if changed {
                if let Err(err) = win_native::write_user_path(&next_path, is_expand_sz) {
                    logs.push(format!("[X] {err}"));
                    return ActionResponse {
                        ok: false,
                        need_confirm: false,
                        process_count: 0,
                        resolved_port: 0,
                        logs,
                        error: Some(err),
                    };
                }
                logs.push(format!(
                    "[OK] 已从用户 PATH 移除 CLI Shim 目录: {shim_dir_str}"
                ));
            } else {
                logs.push("[i] 用户 PATH 中未包含 CLI Shim 目录。".to_string());
            }
        }
        Err(err) => {
            logs.push(format!("[X] {err}"));
            return ActionResponse {
                ok: false,
                need_confirm: false,
                process_count: 0,
                resolved_port: 0,
                logs,
                error: Some(err),
            };
        }
    }

    for fname in ["agy.cmd", "agy.ps1", "agy", "shim-meta.json"] {
        let p = shim_dir.join(fname);
        if p.is_file() {
            let _ = fs::remove_file(&p);
        }
    }
    if shim_dir.is_dir() {
        let _ = fs::remove_dir(&shim_dir);
    }

    logs.push("[OK] 已清理 CLI Proxy Shim 脚本（对原 agy.exe 与系统环境变量零改动）。".to_string());
    ActionResponse {
        ok: true,
        need_confirm: false,
        process_count: 0,
        resolved_port: 0,
        logs,
        error: None,
    }
}

pub fn sync_cli_shim_if_enabled(settings: &ProxySettings) -> bool {
    let shim_dir = cli_shim_dir();
    let shim_dir_str = shim_dir.to_string_lossy().to_string();
    let Ok((user_path, _)) = win_native::read_user_path() else {
        return false;
    };
    if !path_contains_entry(&user_path, &shim_dir_str) {
        return false;
    }
    let (port, _, _, _) = resolve_port(settings);
    write_cli_shim_files(&shim_dir, settings, port).is_ok()
}

pub fn launch_isolated_cli_tool(
    settings: &ProxySettings,
    tool_id: &str,
    force_restart: bool,
) -> ActionResponse {
    let mut logs = Vec::new();
    let Some(tool) = settings.cli_tools.iter().find(|t| t.id == tool_id) else {
        let msg = format!("找不到指定的 CLI 工具配置: {tool_id}");
        logs.push(format!("[X] {msg}"));
        return ActionResponse {
            ok: false,
            need_confirm: false,
            process_count: 0,
            resolved_port: 0,
            logs,
            error: Some(msg),
        };
    };

    let exe_path = Path::new(tool.exe_path.trim());
    if tool.exe_path.trim().is_empty() || !exe_path.is_file() {
        let msg = format!("找不到 {} 可执行文件: {}", tool.name, tool.exe_path);
        logs.push(format!("[X] {msg}"));
        return ActionResponse {
            ok: false,
            need_confirm: false,
            process_count: 0,
            resolved_port: 0,
            logs,
            error: Some(msg),
        };
    }

    let target_port = match ensure_listening_port(settings, &mut logs) {
        Ok(p) => p,
        Err((p, msg)) => {
            return ActionResponse {
                ok: false,
                need_confirm: false,
                process_count: 0,
                resolved_port: p,
                logs,
                error: Some(msg),
            };
        }
    };

    let existing = win_native::scan_tool_process_tree(&tool.exe_path);
    if !existing.is_empty() {
        if !force_restart {
            return ActionResponse {
                ok: false,
                need_confirm: true,
                process_count: existing.len(),
                resolved_port: target_port,
                logs,
                error: None,
            };
        }
        let (stopped, stop_logs) = stop_cli_tool_processes_inner(&tool.exe_path, &tool.name, false);
        logs.extend(stop_logs);
        if !stopped {
            return ActionResponse {
                ok: false,
                need_confirm: false,
                process_count: existing.len(),
                resolved_port: target_port,
                logs,
                error: Some(format!(
                    "部分旧 {} 进程未能退出，请手动完全退出后再试。",
                    tool.name
                )),
            };
        }
    }

    let proxy_url = format!("http://{}:{}", settings.proxy_host, target_port);
    let launch_exe = resolve_versioned_app_exe(exe_path);
    if normalize_path_key(&launch_exe.to_string_lossy()) != normalize_path_key(tool.exe_path.trim()) {
        logs.push(format!(
            "[i] 已解析版本化主程序: {}",
            launch_exe.to_string_lossy()
        ));
    }

    let ext_lower = launch_exe
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut cmd = if ext_lower == "cmd" || ext_lower == "bat" {
        let mut c = Command::new("cmd.exe");
        c.args(["/C", &launch_exe.to_string_lossy()]);
        c
    } else {
        Command::new(&launch_exe)
    };

    let work_dir = tool.working_dir.trim();
    if !work_dir.is_empty() && Path::new(work_dir).is_dir() {
        cmd.current_dir(work_dir);
    } else if let Some(dir) = launch_exe.parent() {
        cmd.current_dir(dir);
    }

    let is_gemini = is_gemini_desktop_tool(tool);
    let use_chromium_flags = tool.is_chromium && !is_gemini;

    let managed_gen = if is_gemini {
        activate_managed_system_proxy(settings, target_port, &mut logs)
    } else {
        None
    };

    if use_chromium_flags {
        apply_proxy_env_and_args(&mut cmd, settings, &proxy_url);
        if settings.disable_quic {
            cmd.arg("--disable-quic");
        }
    } else {
        apply_proxy_env_only(&mut cmd, settings, &proxy_url);
    }

    for arg in split_command_args(&tool.args) {
        cmd.arg(arg);
    }

    match cmd.spawn() {
        Ok(mut child) => {
            let pid = child.id();
            thread::sleep(Duration::from_millis(300));
            if let Ok(Some(status)) = child.try_wait() {
                if !status.success() {
                    if managed_gen.is_some() {
                        let _ = restore_managed_system_proxy_if_active();
                    }
                    let code = status
                        .code()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "未知".to_string());
                    let msg = format!(
                        "{} 启动后立即退出 (PID {}, 退出码 {})，请检查程序日志或启动参数。",
                        tool.name, pid, code
                    );
                    logs.push(format!("[X] {msg}"));
                    return ActionResponse {
                        ok: false,
                        need_confirm: false,
                        process_count: 0,
                        resolved_port: target_port,
                        logs,
                        error: Some(msg),
                    };
                }
            }
            if let Some(gen) = managed_gen {
                spawn_gemini_exit_watcher(tool.exe_path.clone(), gen);
            }
            if is_gemini {
                logs.push(format!(
                    "[OK] {} 已启动 (PID {})，已纳入进程与连接监控。",
                    tool.name, pid
                ));
            } else {
                let mode_tag = if use_chromium_flags {
                    "Chromium + 环境变量"
                } else {
                    "环境变量隔离"
                };
                logs.push(format!(
                    "[OK] {} 已以独立代理启动 (PID {}, {}) -> {}",
                    tool.name, pid, mode_tag, proxy_url
                ));
            }
            ActionResponse {
                ok: true,
                need_confirm: false,
                process_count: 1,
                resolved_port: target_port,
                logs,
                error: None,
            }
        }
        Err(err) => {
            if managed_gen.is_some() {
                let _ = restore_managed_system_proxy_if_active();
            }
            let msg = format!("启动 {} 失败: {err}", tool.name);
            logs.push(format!("[X] {msg}"));
            ActionResponse {
                ok: false,
                need_confirm: false,
                process_count: 0,
                resolved_port: target_port,
                logs,
                error: Some(msg),
            }
        }
    }
}

fn find_executable_on_path(name: &str) -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_env) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn encode_powershell_utf16le_base64(script: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    let mut out = String::with_capacity(((bytes.len() + 2) / 3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub fn launch_isolated_cli_terminal(settings: &ProxySettings) -> ActionResponse {
    let mut logs = Vec::new();
    let target_port = match ensure_listening_port(settings, &mut logs) {
        Ok(p) => p,
        Err((p, msg)) => {
            return ActionResponse {
                ok: false,
                need_confirm: false,
                process_count: 0,
                resolved_port: p,
                logs,
                error: Some(msg),
            };
        }
    };

    let proxy_url = format!("http://{}:{}", settings.proxy_host, target_port);
    let (_, env_no_proxy, _) = build_bypass_lists(settings);

    let workdir = {
        let ws = settings.workspace_path.trim();
        if !ws.is_empty() && Path::new(ws).is_dir() {
            PathBuf::from(ws)
        } else {
            dirs::home_dir()
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."))
        }
    };

    let cli_dir = Path::new(settings.cli_path.trim())
        .parent()
        .filter(|_| !settings.cli_path.trim().is_empty() && Path::new(settings.cli_path.trim()).is_file())
        .map(|p| p.to_string_lossy().to_string());

    let esc_ps = |s: &str| s.replace('\'', "''");
    let mut ps_lines = vec![
        format!("$env:HTTP_PROXY = '{}'", esc_ps(&proxy_url)),
        format!("$env:HTTPS_PROXY = '{}'", esc_ps(&proxy_url)),
        format!("$env:ALL_PROXY = '{}'", esc_ps(&proxy_url)),
        format!("$env:http_proxy = '{}'", esc_ps(&proxy_url)),
        format!("$env:https_proxy = '{}'", esc_ps(&proxy_url)),
        format!("$env:all_proxy = '{}'", esc_ps(&proxy_url)),
        format!("$env:NO_PROXY = '{}'", esc_ps(&env_no_proxy)),
        format!("$env:no_proxy = '{}'", esc_ps(&env_no_proxy)),
        "$env:NODE_USE_ENV_PROXY = '1'".to_string(),
    ];
    if let Some(ref dir) = cli_dir {
        ps_lines.push(format!(
            "$env:PATH = '{};' + $env:PATH",
            esc_ps(dir)
        ));
    }
    ps_lines.push(format!(
        "Write-Host '[AgyDock] 隔离代理终端已就绪 -> {} (仅当前终端有效)' -ForegroundColor Green",
        esc_ps(&proxy_url)
    ));
    if cli_dir.is_some() {
        ps_lines.push("Write-Host '[AgyDock] 可直接执行 agy / agy models / agy acp' -ForegroundColor DarkGray".to_string());
    } else {
        ps_lines.push("Write-Host '[AgyDock] 未检测到 agy.exe，请先在设置中配置 Antigravity CLI 路径' -ForegroundColor Yellow".to_string());
    }

    let encoded = encode_powershell_utf16le_base64(&ps_lines.join("\n"));

    let wt_exe = find_executable_on_path("wt.exe");
    let pwsh_exe = find_executable_on_path("pwsh.exe");
    let powershell_exe = find_executable_on_path("powershell.exe");

    let (term_label, mut cmd) = if let Some(wt) = wt_exe {
        let shell_bin = pwsh_exe
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "powershell.exe".to_string());
        let mut c = Command::new(wt);
        let wd_raw = workdir.to_string_lossy().to_string();
        let wd_safe = if wd_raw.len() > 3 {
            wd_raw.trim_end_matches('\\').to_string()
        } else {
            wd_raw
        };
        c.arg("-d")
            .arg(wd_safe)
            .arg(shell_bin)
            .arg("-NoExit")
            .arg("-EncodedCommand")
            .arg(&encoded);
        ("Windows Terminal", c)
    } else if let Some(pwsh) = pwsh_exe {
        let mut c = Command::new(pwsh);
        c.arg("-NoExit").arg("-EncodedCommand").arg(&encoded);
        ("PowerShell 7", c)
    } else if let Some(ps) = powershell_exe {
        let mut c = Command::new(ps);
        c.arg("-NoExit").arg("-EncodedCommand").arg(&encoded);
        ("Windows PowerShell", c)
    } else {
        let esc_cmd = |s: &str| s.replace('%', "%%");
        let path_part = cli_dir
            .as_ref()
            .map(|d| format!("set \"PATH={};%PATH%\" & ", esc_cmd(d)))
            .unwrap_or_default();
        let init_cmd = format!(
            "set \"HTTP_PROXY={p}\" & set \"HTTPS_PROXY={p}\" & set \"ALL_PROXY={p}\" & set \"http_proxy={p}\" & set \"https_proxy={p}\" & set \"all_proxy={p}\" & set \"NO_PROXY={np}\" & set \"no_proxy={np}\" & set \"NODE_USE_ENV_PROXY=1\" & {path_part}echo [AgyDock] 隔离代理终端已就绪 -^> {p}",
            p = esc_cmd(&proxy_url),
            np = esc_cmd(&env_no_proxy)
        );
        let mut c = Command::new("cmd.exe");
        c.args(["/K", &init_cmd]);
        ("cmd.exe", c)
    };

    cmd.current_dir(&workdir);
    apply_proxy_env_only(&mut cmd, settings, &proxy_url);
    if let Some(ref dir) = cli_dir {
        let current_path = std::env::var("PATH").unwrap_or_default();
        let (next_path, _) = prepend_path_entry(&current_path, dir);
        cmd.env("PATH", next_path);
    }

    match cmd.spawn() {
        Ok(child) => {
            logs.push(format!(
                "[OK] 已打开 {term_label} 隔离代理终端 (PID {}) -> {proxy_url}",
                child.id()
            ));
            ActionResponse {
                ok: true,
                need_confirm: false,
                process_count: 1,
                resolved_port: target_port,
                logs,
                error: None,
            }
        }
        Err(err) => {
            let msg = format!("打开隔离代理终端失败: {err}");
            logs.push(format!("[X] {msg}"));
            ActionResponse {
                ok: false,
                need_confirm: false,
                process_count: 0,
                resolved_port: target_port,
                logs,
                error: Some(msg),
            }
        }
    }
}

pub fn test_cli_proxy(settings: &ProxySettings) -> ActionResponse {
    let mut logs = Vec::new();
    let cli_trimmed = settings.cli_path.trim();
    let cli_path = Path::new(cli_trimmed);
    if cli_trimmed.is_empty() || !cli_path.is_file() {
        let msg = "未配置或找不到 Antigravity CLI (agy.exe)，请先在设置中配置或点击「重新探测」。".to_string();
        logs.push(format!("[X] {msg}"));
        return ActionResponse {
            ok: false,
            need_confirm: false,
            process_count: 0,
            resolved_port: 0,
            logs,
            error: Some(msg),
        };
    }
    if is_inside_dir(cli_path, &cli_shim_dir()) {
        let msg = "CLI 路径不能指向 AgyDock Shim 目录自身，请重新探测真实的 agy.exe。".to_string();
        logs.push(format!("[X] {msg}"));
        return ActionResponse {
            ok: false,
            need_confirm: false,
            process_count: 0,
            resolved_port: 0,
            logs,
            error: Some(msg),
        };
    }

    let target_port = match ensure_listening_port(settings, &mut logs) {
        Ok(p) => p,
        Err((p, msg)) => {
            return ActionResponse {
                ok: false,
                need_confirm: false,
                process_count: 0,
                resolved_port: p,
                logs,
                error: Some(msg),
            };
        }
    };

    let proxy_url = format!("http://{}:{}", settings.proxy_host, target_port);
    logs.push(format!(
        "[..] 正在通过隔离代理 ({proxy_url}) 测试 CLI: \"{cli_trimmed}\" models"
    ));

    let mut cmd = Command::new(cli_path);
    cmd.arg("models")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(parent) = cli_path.parent() {
        cmd.current_dir(parent);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    apply_proxy_env_only(&mut cmd, settings, &proxy_url);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            let msg = format!("启动 agy.exe 失败: {err}");
            logs.push(format!("[X] {msg}"));
            return ActionResponse {
                ok: false,
                need_confirm: false,
                process_count: 0,
                resolved_port: target_port,
                logs,
                error: Some(msg),
            };
        }
    };

    let stdout_handle = child.stdout.take().map(|mut pipe| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).to_string()
        })
    });
    let stderr_handle = child.stderr.take().map(|mut pipe| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).to_string()
        })
    });

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut exit_status = None;
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                exit_status = Some(status);
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(err) => {
                let msg = format!("等待 CLI 返回失败: {err}");
                logs.push(format!("[X] {msg}"));
                return ActionResponse {
                    ok: false,
                    need_confirm: false,
                    process_count: 0,
                    resolved_port: target_port,
                    logs,
                    error: Some(msg),
                };
            }
        }
    }

    let stdout_text = stdout_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    let stderr_text = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();

    if timed_out {
        let msg = "CLI 连通性测试超时（15 秒），请确认代理节点可用。".to_string();
        logs.push(format!("[X] {msg}"));
        return ActionResponse {
            ok: false,
            need_confirm: false,
            process_count: 0,
            resolved_port: target_port,
            logs,
            error: Some(msg),
        };
    }

    let ok = exit_status.map(|s| s.success()).unwrap_or(false);
    let model_lines: Vec<&str> = stdout_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("Fetching"))
        .collect();

    if ok && !model_lines.is_empty() {
        logs.push(format!(
            "[OK] CLI 隔离代理连通性测试通过：获取 {} 个可用模型。",
            model_lines.len()
        ));
        for line in model_lines.iter().take(5) {
            logs.push(format!("    {line}"));
        }
        if model_lines.len() > 5 {
            logs.push(format!("    ... 及另外 {} 个模型", model_lines.len() - 5));
        }
        ActionResponse {
            ok: true,
            need_confirm: false,
            process_count: 0,
            resolved_port: target_port,
            logs,
            error: None,
        }
    } else {
        let code = exit_status.and_then(|s| s.code()).unwrap_or(-1);
        for line in stderr_text.lines().chain(stdout_text.lines()).take(10) {
            let t = line.trim();
            if !t.is_empty() {
                logs.push(format!("    {t}"));
            }
        }
        let msg = format!("CLI 测试未通过 (退出码 {code})，请检查节点连通性或 CLI 登录状态。");
        logs.push(format!("[X] {msg}"));
        ActionResponse {
            ok: false,
            need_confirm: false,
            process_count: 0,
            resolved_port: target_port,
            logs,
            error: Some(msg),
        }
    }
}

/// CLI 静默续期结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliRefreshOutcome {
    /// 已通过本地 agy CLI 成功触发一次令牌续期（或刚完成续期）。
    Refreshed,
    /// 本机未检测到可用的 agy CLI（纯桌面端环境）。
    NotInstalled,
    /// 处于后台冷却窗口内，跳过重复拉起 CLI。
    Cooldown,
    /// CLI 执行失败或超时（包含简要原因）。
    Failed(String),
}

/// 解析本机真实可用的 `agy.exe` 路径（排除 AgyDock 自身的 Shim 目录）。
pub fn resolve_valid_cli_path(settings: &ProxySettings) -> Option<PathBuf> {
    let shim_dir = cli_shim_dir();
    let configured = settings.cli_path.trim();
    if !configured.is_empty() {
        let p = PathBuf::from(configured);
        if p.is_file() && !is_inside_dir(&p, &shim_dir) {
            return Some(p);
        }
    }
    detect_cli_candidates().into_iter().next().map(PathBuf::from)
}

/// 若本机存在 `agy` CLI，则通过当前隔离代理（若监听）或系统网络静默执行一次 `agy models`，
/// 促使 CLI 自动续期当前登录账号的 OAuth `access_token`。
///
/// 内置并发互斥锁与冷却时间（后台自动触发 60s 冷却；手动触发 5s 内复用刚成功的结果），
/// 避免定时轮询反复拉起子进程。
pub fn silent_refresh_token_via_cli(settings: &ProxySettings, force: bool) -> CliRefreshOutcome {
    const AUTO_COOLDOWN: Duration = Duration::from_secs(60);
    const MANUAL_DEDUP: Duration = Duration::from_secs(5);
    const CLI_TIMEOUT: Duration = Duration::from_secs(12);

    static CLI_REFRESH_GUARD: OnceLock<Mutex<Option<(Instant, bool)>>> = OnceLock::new();
    let guard_mutex = CLI_REFRESH_GUARD.get_or_init(|| Mutex::new(None));

    let Some(cli_path) = resolve_valid_cli_path(settings) else {
        return CliRefreshOutcome::NotInstalled;
    };

    let Ok(mut last_run) = guard_mutex.lock() else {
        return CliRefreshOutcome::Failed("CLI 续期锁异常".to_string());
    };

    if let Some((when, last_ok)) = *last_run {
        let elapsed = when.elapsed();
        if last_ok && elapsed < MANUAL_DEDUP {
            return CliRefreshOutcome::Refreshed;
        }
        if !force && elapsed < AUTO_COOLDOWN {
            return CliRefreshOutcome::Cooldown;
        }
    }

    let mut cmd = Command::new(&cli_path);
    cmd.arg("models")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(parent) = cli_path.parent() {
        cmd.current_dir(parent);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let (port, _, is_listening, _) = resolve_port(settings);
    if is_listening && port > 0 {
        let proxy_url = format!("http://{}:{}", settings.proxy_host.trim(), port);
        apply_proxy_env_only(&mut cmd, settings, &proxy_url);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            *last_run = Some((Instant::now(), false));
            return CliRefreshOutcome::Failed(format!("无法启动 agy CLI: {err}"));
        }
    };

    let stderr_handle = child.stderr.take().map(|pipe| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.take(8192).read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).to_string()
        })
    });

    let deadline = Instant::now() + CLI_TIMEOUT;
    let mut exit_status = None;
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                exit_status = Some(status);
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                thread::sleep(Duration::from_millis(80));
            }
            Err(err) => {
                *last_run = Some((Instant::now(), false));
                return CliRefreshOutcome::Failed(format!("等待 CLI 返回失败: {err}"));
            }
        }
    }

    let stderr_text = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();

    if timed_out {
        *last_run = Some((Instant::now(), false));
        return CliRefreshOutcome::Failed("CLI 自动续期超时 (12s)，请检查代理节点".to_string());
    }

    if exit_status.map(|s| s.success()).unwrap_or(false) {
        *last_run = Some((Instant::now(), true));
        CliRefreshOutcome::Refreshed
    } else {
        *last_run = Some((Instant::now(), false));
        let first_err = stderr_text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| l.chars().take(90).collect::<String>());
        let code = exit_status.and_then(|s| s.code()).unwrap_or(-1);
        let detail = match first_err {
            Some(line) => format!("退出码 {code}: {line}"),
            None => format!("退出码 {code}"),
        };
        CliRefreshOutcome::Failed(detail)
    }
}

pub fn build_cli_shell_snippets(settings: &ProxySettings, proxy_port: u16) -> CliShellSnippets {
    let proxy_url = format!("http://{}:{}", settings.proxy_host, proxy_port);
    let (_, env_no_proxy, _) = build_bypass_lists(settings);
    let cli_dir = Path::new(settings.cli_path.trim())
        .parent()
        .filter(|_| !settings.cli_path.trim().is_empty() && Path::new(settings.cli_path.trim()).is_file())
        .map(|p| p.to_string_lossy().to_string());

    let esc_ps = |s: &str| s.replace('\'', "''");
    let esc_sh = |s: &str| s.replace('\'', "'\\''");
    let esc_cmd = |s: &str| s.replace('%', "%%");

    let mut ps_parts = vec![
        format!("$env:HTTP_PROXY='{}'", esc_ps(&proxy_url)),
        format!("$env:HTTPS_PROXY='{}'", esc_ps(&proxy_url)),
        format!("$env:ALL_PROXY='{}'", esc_ps(&proxy_url)),
        format!("$env:http_proxy='{}'", esc_ps(&proxy_url)),
        format!("$env:https_proxy='{}'", esc_ps(&proxy_url)),
        format!("$env:all_proxy='{}'", esc_ps(&proxy_url)),
        format!("$env:NO_PROXY='{}'", esc_ps(&env_no_proxy)),
        format!("$env:no_proxy='{}'", esc_ps(&env_no_proxy)),
        "$env:NODE_USE_ENV_PROXY='1'".to_string(),
    ];
    if let Some(ref dir) = cli_dir {
        ps_parts.push(format!("$env:PATH='{};'+$env:PATH", esc_ps(dir)));
    }

    let mut cmd_parts = vec![
        format!("set \"HTTP_PROXY={}\"", esc_cmd(&proxy_url)),
        format!("set \"HTTPS_PROXY={}\"", esc_cmd(&proxy_url)),
        format!("set \"ALL_PROXY={}\"", esc_cmd(&proxy_url)),
        format!("set \"http_proxy={}\"", esc_cmd(&proxy_url)),
        format!("set \"https_proxy={}\"", esc_cmd(&proxy_url)),
        format!("set \"all_proxy={}\"", esc_cmd(&proxy_url)),
        format!("set \"NO_PROXY={}\"", esc_cmd(&env_no_proxy)),
        format!("set \"no_proxy={}\"", esc_cmd(&env_no_proxy)),
        "set \"NODE_USE_ENV_PROXY=1\"".to_string(),
    ];
    if let Some(ref dir) = cli_dir {
        cmd_parts.push(format!("set \"PATH={};%PATH%\"", esc_cmd(dir)));
    }

    let mut bash_parts = vec![
        format!("export HTTP_PROXY='{}'", esc_sh(&proxy_url)),
        format!("export HTTPS_PROXY='{}'", esc_sh(&proxy_url)),
        format!("export ALL_PROXY='{}'", esc_sh(&proxy_url)),
        format!("export http_proxy='{}'", esc_sh(&proxy_url)),
        format!("export https_proxy='{}'", esc_sh(&proxy_url)),
        format!("export all_proxy='{}'", esc_sh(&proxy_url)),
        format!("export NO_PROXY='{}'", esc_sh(&env_no_proxy)),
        format!("export no_proxy='{}'", esc_sh(&env_no_proxy)),
        "export NODE_USE_ENV_PROXY='1'".to_string(),
    ];
    if let Some(ref dir) = cli_dir {
        let unix_dir = dir.replace('\\', "/");
        bash_parts.push(format!("export PATH='{}:$PATH'", esc_sh(&unix_dir)));
    }

    CliShellSnippets {
        proxy_url,
        powershell: ps_parts.join("; "),
        cmd: cmd_parts.join(" && "),
        bash: bash_parts.join("; "),
    }
}

pub fn build_dry_run_preview(settings: &ProxySettings, proxy_port: u16) -> String {
    let proxy_url = format!("http://{}:{}", settings.proxy_host, proxy_port);
    let (chromium_bypass, env_no_proxy, custom_count) = build_bypass_lists(settings);
    let mut args = vec![
        format!("--proxy-server={proxy_url}"),
        format!("--proxy-bypass-list={chromium_bypass}"),
    ];
    if settings.disable_quic {
        args.push("--disable-quic".to_string());
    }
    if !settings.workspace_path.trim().is_empty() {
        args.push(format!("\"{}\"", settings.workspace_path.trim()));
    }

    let bypass_summary = if settings.bypass_china_and_lan {
        format!(
            "开启（含 LAN 网段 + .cn + {} 个内置国内常用域名{}）",
            CHINA_DIRECT_DOMAINS.len(),
            if custom_count > 0 {
                format!(" + {custom_count} 条自定义规则")
            } else {
                String::new()
            }
        )
    } else if custom_count > 0 {
        format!("仅自定义（{custom_count} 条规则）")
    } else {
        "关闭（仅绕过本机回环地址）".to_string()
    };

    let cli_display = if settings.cli_path.trim().is_empty() {
        "未检测到".to_string()
    } else {
        settings.cli_path.clone()
    };
    let shim = get_cli_shim_status(settings, proxy_port);
    let shim_display = if shim.enabled {
        format!(
            "已启用{} ({})",
            if shim.out_of_sync { " [待同步]" } else { "" },
            shim.shim_dir
        )
    } else {
        format!("未启用 ({})", shim.shim_dir)
    };
    let tools_display = if settings.cli_tools.is_empty() {
        "暂无".to_string()
    } else {
        settings
            .cli_tools
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };

    [
        "[DryRun 启动参数预览]".to_string(),
        format!("  目标程序   : {}", settings.antigravity_path),
        format!("  CLI 路径   : {cli_display}"),
        format!("  CLI Shim   : {shim_display}"),
        format!(
            "  独立应用   : {} 项 ({tools_display})",
            settings.cli_tools.len()
        ),
        format!("  国内/内网直连: {bypass_summary}"),
        format!("  启动参数   : {}", args.join(" ")),
        format!("  子进程环境 : HTTP_PROXY = HTTPS_PROXY = ALL_PROXY = {proxy_url}"),
        format!("  子进程环境 : NO_PROXY = {env_no_proxy}"),
        "  子进程环境 : NODE_USE_ENV_PROXY = 1".to_string(),
        "  系统影响   : 不改动 Windows 注册表系统代理，不改动 Clash Verge 配置，不开 TUN。".to_string(),
    ]
    .join("\n")
}

pub fn launch_isolated_antigravity(settings: &ProxySettings, force_restart: bool) -> ActionResponse {
    let mut logs = Vec::new();
    let agy_path = Path::new(settings.antigravity_path.trim());
    if !agy_path.is_file() {
        let msg = format!("找不到 Antigravity 可执行文件: {}", settings.antigravity_path);
        logs.push(format!("[X] {msg}"));
        return ActionResponse {
            ok: false,
            need_confirm: false,
            process_count: 0,
            resolved_port: 0,
            logs,
            error: Some(msg),
        };
    }

    let target_port = match ensure_listening_port(settings, &mut logs) {
        Ok(p) => p,
        Err((p, msg)) => {
            return ActionResponse {
                ok: false,
                need_confirm: false,
                process_count: 0,
                resolved_port: p,
                logs,
                error: Some(msg),
            };
        }
    };

    let (existing, _) = win_native::scan_antigravity_and_clash(&settings.antigravity_path);
    if !existing.is_empty() {
        if !force_restart {
            return ActionResponse {
                ok: false,
                need_confirm: true,
                process_count: existing.len(),
                resolved_port: target_port,
                logs,
                error: None,
            };
        }
        let (stopped, stop_logs) = stop_antigravity_processes(&settings.antigravity_path);
        logs.extend(stop_logs);
        if !stopped {
            return ActionResponse {
                ok: false,
                need_confirm: false,
                process_count: existing.len(),
                resolved_port: target_port,
                logs,
                error: Some("部分旧进程未能退出，请手动完全退出 Antigravity 后再试。".to_string()),
            };
        }
    }

    let proxy_url = format!("http://{}:{}", settings.proxy_host, target_port);
    let mut cmd = Command::new(agy_path);
    if let Some(dir) = agy_path.parent() {
        cmd.current_dir(dir);
    }
    apply_proxy_env_and_args(&mut cmd, settings, &proxy_url);
    if settings.disable_quic {
        cmd.arg("--disable-quic");
    }
    if !settings.workspace_path.trim().is_empty() {
        cmd.arg(settings.workspace_path.trim());
    }

    match cmd.spawn() {
        Ok(child) => {
            let bypass_note = if settings.bypass_china_and_lan {
                "（已启用国内/内网直连绕过）"
            } else {
                ""
            };
            logs.push(format!(
                "[OK] Antigravity 已以隔离代理方式启动 (PID {}) -> {}{}",
                child.id(),
                proxy_url,
                bypass_note
            ));
            ActionResponse {
                ok: true,
                need_confirm: false,
                process_count: 1,
                resolved_port: target_port,
                logs,
                error: None,
            }
        }
        Err(err) => {
            let msg = format!("启动 Antigravity 失败: {err}");
            logs.push(format!("[X] {msg}"));
            ActionResponse {
                ok: false,
                need_confirm: false,
                process_count: 0,
                resolved_port: target_port,
                logs,
                error: Some(msg),
            }
        }
    }
}

pub fn verify_report_path() -> PathBuf {
    std::env::temp_dir().join("antigravity-launcher-verify.txt")
}

fn is_core_proxy_process_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("language_server") || lower == "agy" || lower == "antigravity-cli"
}

pub fn run_cumulative_audit<F>(
    settings: &ProxySettings,
    seconds: u32,
    cancel_flag: &Arc<AtomicBool>,
    mut on_tick: F,
) -> AuditResult
where
    F: FnMut(u32, u32, &ProxySnapshot),
{
    let secs = seconds.max(1);
    let total_ticks = ((secs as f64) / 0.8).ceil().max(1.0) as u32;
    let mut seen_sockets: HashMap<String, TcpConnectionItem> = HashMap::new();
    let mut latest = build_snapshot(settings);
    let mut cancelled = false;

    for tick in 1..=total_ticks {
        if cancel_flag.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        latest = build_snapshot(settings);
        for item in &latest.connections {
            seen_sockets.insert(item.key.clone(), item.clone());
        }
        let elapsed = ((tick as f64 * 0.8).round() as u32).min(secs);
        on_tick(elapsed, secs, &latest);

        if tick < total_ticks {
            for _ in 0..8 {
                if cancel_flag.load(Ordering::SeqCst) {
                    cancelled = true;
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            if cancelled {
                break;
            }
        }
    }

    let mut cumulative: Vec<TcpConnectionItem> = seen_sockets.into_values().collect();
    cumulative.sort_by(|a, b| {
        a.category
            .sort_order()
            .cmp(&b.category.sort_order())
            .then_with(|| a.process_name.to_lowercase().cmp(&b.process_name.to_lowercase()))
            .then_with(|| a.remote_endpoint.cmp(&b.remote_endpoint))
    });

    let proxied = cumulative
        .iter()
        .filter(|c| c.category == ConnectionCategory::Proxied)
        .count();
    let tun = cumulative
        .iter()
        .filter(|c| c.category == ConnectionCategory::TunFakeIp)
        .count();
    let direct = cumulative
        .iter()
        .filter(|c| c.category == ConnectionCategory::PublicDirect)
        .count();
    let core_direct = cumulative
        .iter()
        .filter(|c| {
            c.category == ConnectionCategory::PublicDirect
                && is_core_proxy_process_name(&c.process_name)
        })
        .count();
    let cli_direct = cumulative
        .iter()
        .filter(|c| {
            c.category == ConnectionCategory::PublicDirect
                && matches!(c.process_name.to_lowercase().as_str(), "agy" | "antigravity-cli")
        })
        .count();
    let bypass_enabled =
        settings.bypass_china_and_lan || !settings.custom_bypass_domains.trim().is_empty();

    let passed = if tun > 0 || core_direct > 0 || (!bypass_enabled && direct > 0) {
        Some(false)
    } else if proxied > 0 {
        Some(true)
    } else {
        None
    };

    let now_str = Local::now().format("%Y-%m-%d %H:%M:%S");
    let mut lines = vec![
        format!("[{now_str}] Antigravity 隔离代理检查 ({secs} 秒累计)"),
        format!(
            "  代理端口      : http://{}:{} ({}, Clash 模式: {})",
            settings.proxy_host, latest.resolved_port, latest.port_source, latest.clash_mode
        ),
        format!(
            "  国内/内网直连 : {}",
            if settings.bypass_china_and_lan {
                "已开启（命中直连规则的站点不走代理）"
            } else {
                "关闭"
            }
        ),
        format!(
            "  进程数        : Antigravity x{}, language_server x{}, agy CLI x{}, 工具 x{}",
            latest.antigravity_proc_count,
            latest.language_server_proc_count,
            latest.cli_proc_count,
            latest.tool_proc_count
        ),
    ];

    if latest.system_proxy_enabled {
        lines.push(format!(
            "  [!] Windows 系统代理开启中: {}",
            latest.system_proxy_server
        ));
    }
    if latest.tun_enabled {
        lines.push(format!(
            "  [!] TUN 虚拟网卡开启中: {}",
            latest.tun_adapter_text
        ));
    }
    if latest.clash_mode.eq_ignore_ascii_case("global") {
        lines.push(
            "  [!] Clash 当前处于 Global 模式，未命中直连列表的国内站点可能走海外节点触发风控，建议切为 Rule 模式。"
                .to_string(),
        );
    }
    lines.push(format!("  走代理的连接  : {proxied} 条"));
    lines.push(format!("  TUN 接管连接  : {tun} 条"));
    lines.push(format!("  公网直连连接  : {direct} 条"));

    if tun > 0 {
        lines.push(
            "  [X] 检测到 Clash TUN Fake-IP (198.18.x.x) 连接，尚未纯隔离到本地代理端口。"
                .to_string(),
        );
        for item in cumulative
            .iter()
            .filter(|c| c.category == ConnectionCategory::TunFakeIp)
            .take(8)
        {
            lines.push(format!(
                "      - {} ({}, PID {})",
                item.remote_endpoint, item.process_name, item.pid
            ));
        }
    }
    if direct > 0 {
        if bypass_enabled && core_direct == 0 {
            lines.push(
                "  [i] 检测到非核心模型进程存在公网直连（已开启国内/内网直连绕过，访问国内站点时属正常现象）："
                    .to_string(),
            );
        } else if cli_direct > 0 {
            lines.push(
                "  [X] 检测到 agy CLI 存在公网直连：请从「CLI 与工具独立代理」中隔离重启宿主工具（如 T3 Code），或开启 CLI Shim / 使用隔离终端。"
                    .to_string(),
            );
        } else {
            lines.push(
                "  [X] 检测到公网直连，请关闭旧实例后用本面板重新启动 Antigravity 或对应工具。"
                    .to_string(),
            );
        }
        for item in cumulative
            .iter()
            .filter(|c| c.category == ConnectionCategory::PublicDirect)
            .take(8)
        {
            lines.push(format!(
                "      - {} ({}, PID {})",
                item.remote_endpoint, item.process_name, item.pid
            ));
        }
    }
    if passed == Some(true) {
        if bypass_enabled && direct > 0 {
            lines.push(
                "  [OK] 核心代理流量均落在本地代理端口，且已按配置允许国内/内网直连绕过。"
                    .to_string(),
            );
        } else {
            lines.push(
                "  [OK] 观察期内无 TUN 接管和公网直连，出网连接均落在本地代理端口上。".to_string(),
            );
        }
    } else if passed.is_none() && !cancelled {
        lines.push(
            "  [..] 观察期内应用未产生外部连接（空闲）。请在 Antigravity、agy CLI 或接入工具中触发请求后再验收。"
                .to_string(),
        );
    }

    let report_text = lines.join("\n");
    let report_path = verify_report_path();
    if !cancelled {
        let bom_content = format!("\u{FEFF}{report_text}\r\n");
        let _ = fs::write(&report_path, bom_content.as_bytes());
    }

    AuditResult {
        cancelled,
        latest_snapshot: latest,
        cumulative_proxied_count: proxied,
        cumulative_tun_count: tun,
        cumulative_direct_count: direct,
        cumulative_connections: cumulative,
        report_text,
        report_path: report_path.to_string_lossy().to_string(),
        passed,
    }
}

pub fn open_verify_report() -> Result<String, String> {
    let path = verify_report_path();
    if !path.is_file() {
        return Err(format!(
            "尚未生成验收报告: {}，请先运行一次「验收 20s」。",
            path.to_string_lossy()
        ));
    }
    #[cfg(windows)]
    {
        Command::new("notepad.exe")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("无法打开报告: {e}"))?;
    }
    Ok(path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_remote_endpoints_accurately() {
        assert_eq!(
            classify_remote("127.0.0.1", 7897, "127.0.0.1", 7897),
            ConnectionCategory::Proxied
        );
        assert_eq!(
            classify_remote("198.18.0.12", 443, "127.0.0.1", 7897),
            ConnectionCategory::TunFakeIp
        );
        assert_eq!(
            classify_remote("127.0.0.1", 51234, "127.0.0.1", 7897),
            ConnectionCategory::LocalIpc
        );
        assert_eq!(
            classify_remote("192.168.1.25", 443, "127.0.0.1", 7897),
            ConnectionCategory::LocalIpc
        );
        assert_eq!(
            classify_remote("10.0.0.8", 8080, "127.0.0.1", 7897),
            ConnectionCategory::LocalIpc
        );
        assert_eq!(
            classify_remote("142.250.190.78", 443, "127.0.0.1", 7897),
            ConnectionCategory::PublicDirect
        );
    }

    #[test]
    fn builds_china_and_custom_bypass_lists_in_dual_formats() {
        let mut s = ProxySettings::default();
        s.bypass_china_and_lan = true;
        s.custom_bypass_domains = "https://www.MyCorp.com/api, 10.20.0.0/16\n*.example.org".to_string();
        let (chromium, no_proxy, custom_count) = build_bypass_lists(&s);
        assert_eq!(custom_count, 3);
        assert!(chromium.contains("*.cn"));
        assert!(chromium.contains("baidu.com;*.baidu.com"));
        assert!(chromium.contains("www.mycorp.com;*.www.mycorp.com"));
        assert!(chromium.contains("example.org;*.example.org"));
        assert!(chromium.contains("10.20.0.0/16"));

        assert!(no_proxy.contains(".cn"));
        assert!(no_proxy.contains(".baidu.com"));
        assert!(no_proxy.contains(".www.mycorp.com"));
        assert!(no_proxy.contains(".example.org"));
        assert!(no_proxy.contains("10.20.0.0/16"));
        assert!(!no_proxy.contains('*'), "NO_PROXY must not contain * wildcards");

        s.bypass_china_and_lan = false;
        s.custom_bypass_domains = String::new();
        let (chromium_off, no_proxy_off, count_off) = build_bypass_lists(&s);
        assert_eq!(count_off, 0);
        assert!(!chromium_off.contains("*.cn"));
        assert!(!no_proxy_off.contains(".cn"));
    }

    #[test]
    fn parses_clash_yaml_port_and_mode() {
        let yaml = "verge_mixed_port: 7897\nmode: global\n";
        let (port, mode) = parse_clash_yaml_port_and_mode(yaml);
        assert_eq!(port, 7897);
        assert_eq!(mode.as_deref(), Some("global"));
    }

    #[test]
    fn builds_dry_run_preview_with_env_and_args() {
        let s = ProxySettings::default();
        let preview = build_dry_run_preview(&s, 7897);
        assert!(preview.contains("--proxy-server=http://127.0.0.1:7897"));
        assert!(preview.contains("*.cn"));
        assert!(preview.contains("NODE_USE_ENV_PROXY = 1"));
        assert!(preview.contains("CLI 路径"));
        assert!(preview.contains("CLI Shim"));
    }

    #[test]
    fn splits_command_args_preserving_quotes_and_windows_paths() {
        let args = split_command_args(
            "--config \"C:\\Program Files\\My App\\cfg.json\" --profile 'work space' --verbose",
        );
        assert_eq!(
            args,
            vec![
                "--config",
                "C:\\Program Files\\My App\\cfg.json",
                "--profile",
                "work space",
                "--verbose"
            ]
        );
        assert!(split_command_args("   ").is_empty());
    }

    #[test]
    fn prepends_and_removes_path_entries_case_insensitively() {
        let shim = r"C:\Users\alice\AppData\Local\AgyDock\cli-shim";
        let base = r"C:\Windows\System32;D:\Tools";
        let (added, changed) = prepend_path_entry(base, shim);
        assert!(changed);
        assert_eq!(added, format!("{shim};{base}"));

        // Already first: no-op
        let (same, changed_again) = prepend_path_entry(&added, &shim.to_uppercase());
        assert!(!changed_again);
        assert_eq!(same, added);

        // Move from later position to front and deduplicate
        let buried = format!(r"C:\Windows\System32;{shim}\;D:\Tools;{shim}");
        let (moved, changed_moved) = prepend_path_entry(&buried, shim);
        assert!(changed_moved);
        assert_eq!(moved, format!(r"{shim};C:\Windows\System32;D:\Tools"));

        // Remove cleanly
        let (removed, changed_rm) = remove_path_entry(&moved, &shim.to_lowercase());
        assert!(changed_rm);
        assert_eq!(removed, r"C:\Windows\System32;D:\Tools");

        // Remove when absent: no-op
        let (noop, changed_noop) = remove_path_entry(&removed, shim);
        assert!(!changed_noop);
        assert_eq!(noop, removed);
    }

    #[test]
    fn renders_cli_shim_scripts_with_isolation_and_escaping() {
        let (cmd, ps1, sh) = render_cli_shim_scripts(
            r"C:\Users\o'neil\%APP%\agy.exe",
            "http://127.0.0.1:7897",
            "localhost,127.0.0.1,.cn",
        );
        assert!(cmd.contains("setlocal EnableExtensions"));
        assert!(cmd.contains("endlocal & exit /b %AGYDOCK_EXIT%"));
        assert!(cmd.contains(r"C:\Users\o'neil\%%APP%%\agy.exe"));
        assert!(cmd.contains("set \"HTTPS_PROXY=http://127.0.0.1:7897\""));

        assert!(ps1.contains("try {"));
        assert!(ps1.contains("finally {"));
        assert!(ps1.contains(r"C:\Users\o''neil\%APP%\agy.exe"));

        assert!(sh.contains("#!/usr/bin/env sh"));
        assert!(sh.contains("exec 'C:/Users/o'\\''neil/%APP%/agy.exe' \"$@\""));
    }

    #[test]
    fn builds_cli_shell_snippets_for_all_shells() {
        let s = ProxySettings::default();
        let snippets = build_cli_shell_snippets(&s, 7890);
        assert_eq!(snippets.proxy_url, "http://127.0.0.1:7890");
        assert!(snippets.powershell.contains("$env:HTTPS_PROXY='http://127.0.0.1:7890'"));
        assert!(snippets.cmd.contains("set \"HTTPS_PROXY=http://127.0.0.1:7890\""));
        assert!(snippets.bash.contains("export HTTPS_PROXY='http://127.0.0.1:7890'"));
    }

    #[test]
    fn deserializes_legacy_proxy_settings_without_cli_fields() {
        let legacy_json = r#"{
            "proxyHost": "127.0.0.1",
            "autoDetectPort": true,
            "customPort": 7897,
            "antigravityPath": "C:\\Apps\\Antigravity.exe",
            "clashVergePath": "D:\\Clash\\clash-verge.exe",
            "workspacePath": "",
            "autoStartClash": true,
            "disableQuic": true,
            "showLoopbackIpc": false
        }"#;
        let parsed: ProxySettings = serde_json::from_str(legacy_json).unwrap();
        assert!(parsed.bypass_china_and_lan);
        assert_eq!(parsed.cli_path, "");
        assert!(parsed.cli_tools.is_empty());
        assert!(!parsed.cli_tools_initialized);
        let sanitized = parsed.sanitized();
        assert!(sanitized.cli_tools_initialized);
    }

    #[test]
    fn resolves_highest_versioned_app_exe_and_install_root() {
        let unique = format!(
            "agydock_ver_test_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let v1 = root.join("app-1.9.2");
        let v2 = root.join("app-1.21.1");
        let v3 = root.join("app-1.20.9");
        fs::create_dir_all(&v1).unwrap();
        fs::create_dir_all(&v2).unwrap();
        fs::create_dir_all(&v3).unwrap();

        let stub = root.join("Gemini.exe");
        let exe1 = v1.join("Gemini.exe");
        let exe2 = v2.join("Gemini.exe");
        let exe3 = v3.join("Gemini.exe");
        fs::write(&stub, b"stub").unwrap();
        fs::write(&exe1, b"v1").unwrap();
        fs::write(&exe2, b"v2").unwrap();
        fs::write(&exe3, b"v3").unwrap();

        let resolved = resolve_versioned_app_exe(&stub);
        assert_eq!(resolved, exe2);

        let root_from_stub = tool_install_root(&stub).unwrap();
        let root_from_ver = tool_install_root(&exe2).unwrap();
        assert_eq!(root_from_stub, root);
        assert_eq!(root_from_ver, root);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parses_app_version_dir_names() {
        assert_eq!(parse_app_version_dir("app-1.21.1"), Some(vec![1, 21, 1]));
        assert_eq!(parse_app_version_dir("APP-2.0.3-beta"), Some(vec![2, 0, 3]));
        assert_eq!(parse_app_version_dir("Crashpad"), None);
        assert_eq!(parse_app_version_dir("app-"), None);
    }

    #[test]
    fn forces_gemini_desktop_off_chromium_proxy_flags() {
        let mut s = ProxySettings::default();
        s.cli_tools_initialized = true;
        s.builtin_tools_discovered = vec!["gemini-desktop".to_string(), "t3-code".to_string()];
        s.cli_tools = vec![CliToolEntry {
            id: "gemini-desktop".to_string(),
            name: "Gemini Desktop".to_string(),
            exe_path: r"C:\Users\alice\AppData\Local\Google\Gemini\Gemini.exe".to_string(),
            args: String::new(),
            working_dir: String::new(),
            is_chromium: true,
            monitor: true,
        }];
        let sanitized = s.sanitized();
        let gemini = sanitized
            .cli_tools
            .iter()
            .find(|t| t.id == "gemini-desktop")
            .unwrap();
        assert!(!gemini.is_chromium);
    }

    #[test]
    fn builds_wininet_proxy_override_with_local_and_china_rules() {
        let mut s = ProxySettings::default();
        s.bypass_china_and_lan = true;
        s.custom_bypass_domains = "internal.corp".to_string();
        let ov = build_wininet_proxy_override(&s);
        assert!(ov.contains("localhost"));
        assert!(ov.contains("127.*"));
        assert!(ov.contains("192.168.*"));
        assert!(ov.contains("*.cn"));
        assert!(ov.contains("*.internal.corp"));
        assert!(ov.ends_with("<local>"));
    }
}
