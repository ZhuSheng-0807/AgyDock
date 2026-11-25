mod accounts;
mod credential;
mod proxy;
mod quota;
mod scanner;

use accounts::{AccountEntry, AccountHealth, AccountStore, AccountsList, CurrentCredential};
use quota::{AccountQuotaSnapshot, QuotaCacheStore, TokenCache};
use std::collections::HashMap;
use proxy::{
    build_cli_shell_snippets, build_dry_run_preview, build_snapshot, detect_cli_and_tools,
    disable_cli_shim, enable_cli_shim, flush_dns_cache, get_antigravity_runtime_status,
    launch_antigravity_after_stop, launch_isolated_antigravity, launch_isolated_cli_terminal,
    launch_isolated_cli_tool, open_verify_report, resolve_port, run_cumulative_audit,
    start_clash_verge, stop_antigravity_processes, stop_cli_processes, stop_cli_tool_processes,
    sync_cli_shim_if_enabled, test_cli_proxy, ActionResponse, AntigravityRuntimeStatus,
    AuditProgressPayload, AuditResult, CliDetectResult, CliShellSnippets, ProxySettings,
    ProxySnapshot,
};
use scanner::{
    aggregate_usage, build_sessions_csv, get_default_brain_dir, load_persisted_brain_dir,
    save_persisted_brain_dir, BrainScanner, SessionDetailResponse, UsageResponse,
};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

struct AppState {
    scanner: Mutex<BrainScanner>,
    proxy_settings: Mutex<ProxySettings>,
    account_lock: Arc<Mutex<()>>,
    quota_token_cache: Arc<Mutex<TokenCache>>,
    quota_cache_lock: Arc<Mutex<()>>,
    tray_signature: Mutex<String>,
    audit_cancel: Arc<AtomicBool>,
    audit_running: Arc<AtomicBool>,
    launch_running: Arc<AtomicBool>,
}

struct AtomicFlagGuard(Arc<AtomicBool>);

impl AtomicFlagGuard {
    fn try_acquire(flag: &Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self(Arc::clone(flag)))
    }
}

impl Drop for AtomicFlagGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DirPickResult {
    changed: bool,
    brain_dir: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportCsvResult {
    saved: bool,
    path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowseResult {
    selected: bool,
    path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginStartResult {
    ok: bool,
    previous_email: Option<String>,
    logs: Vec<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountSwitchResult {
    ok: bool,
    email: Option<String>,
    logs: Vec<String>,
    error: Option<String>,
}

#[tauri::command]
fn get_usage(
    state: State<'_, AppState>,
    range: Option<String>,
    granularity: Option<String>,
    model: Option<String>,
    source: Option<String>,
    day: Option<String>,
) -> Result<UsageResponse, String> {
    let mut scanner = state.scanner.lock().map_err(|_| "Scanner lock poisoned")?;
    let scan_res = scanner.scan();
    Ok(aggregate_usage(
        &scan_res,
        range.as_deref(),
        granularity.as_deref(),
        model.as_deref(),
        source.as_deref(),
        day.as_deref(),
        None,
    ))
}

#[tauri::command]
fn get_session_detail(
    state: State<'_, AppState>,
    session_id: String,
    source: Option<String>,
) -> Result<SessionDetailResponse, String> {
    let mut scanner = state.scanner.lock().map_err(|_| "Scanner lock poisoned")?;
    let scan_res = scanner.scan();
    let src_filter = source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all");
    let session = scan_res
        .sessions
        .into_iter()
        .find(|s| s.id == session_id && src_filter.is_none_or(|sf| s.source == sf))
        .ok_or_else(|| "Session not found".to_string())?;
    Ok(SessionDetailResponse {
        id: session.id,
        source: session.source,
        source_label: session.source_label,
        title: session.title,
        started_at: session.started_at,
        last_at: session.last_at,
        events: session.events,
    })
}

#[tauri::command]
fn pick_brain_dir(state: State<'_, AppState>) -> Result<DirPickResult, String> {
    let current_dir = {
        let scanner = state.scanner.lock().map_err(|_| "Scanner lock poisoned")?;
        scanner.primary_dir()
    };

    let mut dialog = rfd::FileDialog::new().set_title("选择 Antigravity brain 日志目录");
    if current_dir.exists() {
        dialog = dialog.set_directory(&current_dir);
    }

    if let Some(chosen) = dialog.pick_folder() {
        save_persisted_brain_dir(Some(&chosen));
        let mut scanner = state.scanner.lock().map_err(|_| "Scanner lock poisoned")?;
        scanner.set_dir(chosen.clone());
        Ok(DirPickResult {
            changed: true,
            brain_dir: chosen.to_string_lossy().to_string(),
        })
    } else {
        Ok(DirPickResult {
            changed: false,
            brain_dir: current_dir.to_string_lossy().to_string(),
        })
    }
}

#[tauri::command]
fn reset_brain_dir(state: State<'_, AppState>) -> Result<DirPickResult, String> {
    let default_dir = get_default_brain_dir();
    save_persisted_brain_dir(None);
    let mut scanner = state.scanner.lock().map_err(|_| "Scanner lock poisoned")?;
    scanner.reset_to_auto();
    Ok(DirPickResult {
        changed: true,
        brain_dir: default_dir.to_string_lossy().to_string(),
    })
}

#[tauri::command]
fn export_csv(
    state: State<'_, AppState>,
    range: Option<String>,
    granularity: Option<String>,
    model: Option<String>,
    source: Option<String>,
    day: Option<String>,
) -> Result<ExportCsvResult, String> {
    let data = {
        let mut scanner = state.scanner.lock().map_err(|_| "Scanner lock poisoned")?;
        let scan_res = scanner.scan();
        aggregate_usage(
            &scan_res,
            range.as_deref(),
            granularity.as_deref(),
            model.as_deref(),
            source.as_deref(),
            day.as_deref(),
            None,
        )
    };

    let suffix = data.meta.selected_day.as_deref().unwrap_or(&data.meta.range);
    let default_name = format!("antigravity-usage-{suffix}.csv");
    let mut dialog = rfd::FileDialog::new()
        .set_title("导出 Antigravity 用量明细 CSV")
        .set_file_name(&default_name)
        .add_filter("CSV 文件", &["csv"]);
    if let Some(desktop) = dirs::desktop_dir() {
        dialog = dialog.set_directory(desktop);
    }

    if let Some(path) = dialog.save_file() {
        let csv = build_sessions_csv(&data.sessions);
        fs::write(&path, csv.as_bytes()).map_err(|e| format!("写入文件失败: {e}"))?;
        Ok(ExportCsvResult {
            saved: true,
            path: Some(path.to_string_lossy().to_string()),
        })
    } else {
        Ok(ExportCsvResult {
            saved: false,
            path: None,
        })
    }
}

// ------------------------- Account Switching Commands -------------------------

const TRAY_SWITCH_PREFIX: &str = "tray-switch::";

fn compute_tray_signature(current_email: Option<&str>, saved: &[AccountEntry]) -> String {
    let mut sig = String::from(current_email.unwrap_or(""));
    for acc in saved.iter().take(15) {
        sig.push('|');
        sig.push_str(&acc.id);
        sig.push(':');
        sig.push_str(&acc.name);
        sig.push(':');
        sig.push(if acc.pinned { '1' } else { '0' });
        sig.push(if acc.backup_ok { '1' } else { '0' });
    }
    sig
}

fn build_tray_menu(
    app: &AppHandle,
    current_email: Option<&str>,
    saved: &[AccountEntry],
) -> tauri::Result<(Menu<tauri::Wry>, String)> {
    let show_item = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let hide_item = MenuItem::with_id(app, "hide", "隐藏到托盘", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;

    let current_label = match current_email {
        Some(email) => format!("当前账号: {email}"),
        None => "当前账号: 未登录".to_string(),
    };
    let current_item = MenuItem::with_id(app, "tray-current", &current_label, false, None::<&str>)?;

    let mut sub_items: Vec<MenuItem<tauri::Wry>> = Vec::new();
    for acc in saved.iter().take(15) {
        let is_cur = current_email
            .map(|e| e.eq_ignore_ascii_case(&acc.email))
            .unwrap_or(false);
        let display = if acc.name.is_empty() {
            acc.email.clone()
        } else {
            format!("{} ({})", acc.name, acc.email)
        };
        let label = if is_cur {
            format!("✓ {display}")
        } else if !acc.backup_ok {
            format!("! {display} [备份异常]")
        } else {
            display
        };
        let enabled = !is_cur && acc.backup_ok;
        let item_id = format!("{TRAY_SWITCH_PREFIX}{}", acc.id);
        sub_items.push(MenuItem::with_id(app, item_id, label, enabled, None::<&str>)?);
    }
    if sub_items.is_empty() {
        sub_items.push(MenuItem::with_id(
            app,
            "tray-switch-empty",
            "暂无已保存账号",
            false,
            None::<&str>,
        )?);
    }

    let sub_refs: Vec<&dyn IsMenuItem<tauri::Wry>> = sub_items
        .iter()
        .map(|item| item as &dyn IsMenuItem<tauri::Wry>)
        .collect();
    let switch_sub = Submenu::with_items(
        app,
        "快速切换账号 (重启 Antigravity)",
        true,
        &sub_refs,
    )?;

    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出程序", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &show_item,
            &hide_item,
            &sep1,
            &current_item,
            &switch_sub,
            &sep2,
            &quit_item,
        ],
    )?;

    let tooltip = match current_email {
        Some(email) => format!("AgyDock · {email}"),
        None => "AgyDock · Antigravity 控制台".to_string(),
    };

    Ok((menu, tooltip))
}

fn refresh_tray_menu(app: &AppHandle) {
    let Some(tray) = app.tray_by_id("main-tray") else {
        return;
    };
    let current_email = accounts::current_credential().ok().and_then(|c| c.email);
    let saved = AccountStore::default_store().list_readonly(current_email.as_deref());
    let next_sig = compute_tray_signature(current_email.as_deref(), &saved);

    let state = app.state::<AppState>();
    if let Ok(mut guard) = state.tray_signature.lock() {
        if *guard == next_sig {
            return;
        }
        *guard = next_sig;
    }

    if let Ok((menu, tooltip)) = build_tray_menu(app, current_email.as_deref(), &saved) {
        let _ = tray.set_menu(Some(menu));
        let _ = tray.set_tooltip(Some(tooltip));
    }
}

fn execute_account_switch(
    account_lock: &Arc<Mutex<()>>,
    settings: &ProxySettings,
    id: &str,
    restart: bool,
) -> Result<AccountSwitchResult, String> {
    let store = AccountStore::default_store();
    let mut logs: Vec<String> = Vec::new();

    let (entry, blob) = {
        let _guard = account_lock
            .lock()
            .map_err(|_| "Account store lock poisoned".to_string())?;
        let entry = store
            .get(id)
            .ok_or_else(|| "账号不存在或已被删除".to_string())?;
        let blob = store.load_blob(id)?;
        (entry, blob)
    };

    if restart && !Path::new(settings.antigravity_path.trim()).is_file() {
        let msg = format!(
            "找不到 Antigravity 可执行文件，已取消切换: {}",
            settings.antigravity_path
        );
        logs.push(format!("[X] {msg}"));
        return Ok(AccountSwitchResult {
            ok: false,
            email: Some(entry.email),
            logs,
            error: Some(msg),
        });
    }

    // 先检查当前是否有登录凭据：
    // - 若当前未登录（如点击「取消并恢复原账号」时），无需备份直接写入目标账号；
    // - 若当前已登录，先备份当前账号，备份失败时仅当此前已有备份才允许继续。
    match credential::read_blob() {
        Ok(None) => {
            logs.push("[i] 当前系统未登录任何账号，跳过备份步骤。".to_string());
        }
        Ok(Some(raw)) => {
            let current_email = credential::parse_blob(&raw).email;
            let save_res = {
                let _guard = account_lock
                    .lock()
                    .map_err(|_| "Account store lock poisoned".to_string())?;
                store.save_current()
            };
            match save_res {
                Ok(out) if out.email.eq_ignore_ascii_case(&entry.email) => {
                    logs.push(format!("[i] 当前登录就是 {}，已同步最新凭据。", entry.email));
                }
                Ok(out) => logs.push(format!("[OK] 已备份当前账号 {}", out.email)),
                Err(err) => {
                    let already_known = {
                        let _guard = account_lock
                            .lock()
                            .map_err(|_| "Account store lock poisoned".to_string())?;
                        current_email
                            .as_deref()
                            .map(|email| store.contains_email(email))
                            .unwrap_or(false)
                    };
                    if already_known {
                        logs.push(format!("[i] 当前账号此前已备份，继续切换: {err}"));
                    } else {
                        let msg = format!("无法备份当前账号，为避免丢失登录状态已中止: {err}");
                        logs.push(format!("[X] {msg}"));
                        return Ok(AccountSwitchResult {
                            ok: false,
                            email: Some(entry.email),
                            logs,
                            error: Some(msg),
                        });
                    }
                }
            }
        }
        Err(err) => {
            let msg = format!("读取当前登录凭据失败，为避免覆盖状态已中止: {err}");
            logs.push(format!("[X] {msg}"));
            return Ok(AccountSwitchResult {
                ok: false,
                email: Some(entry.email),
                logs,
                error: Some(msg),
            });
        }
    }

    if restart {
        let (stopped, mut stop_logs) = stop_antigravity_processes(&settings.antigravity_path);
        logs.append(&mut stop_logs);
        if !stopped {
            let msg = "旧 Antigravity 进程未能完全退出，请先手动退出后再试。".to_string();
            logs.push(format!("[X] {msg}"));
            return Ok(AccountSwitchResult {
                ok: false,
                email: Some(entry.email),
                logs,
                error: Some(msg),
            });
        }
    }

    if let Err(err) = credential::write_blob(&blob) {
        logs.push(format!("[X] 写入账号凭据失败: {err}"));
        return Ok(AccountSwitchResult {
            ok: false,
            email: Some(entry.email),
            logs,
            error: Some(err),
        });
    }
    logs.push(format!(
        "[OK] 已把 {} 的登录凭据写入 Windows 凭据管理器。",
        entry.email
    ));
    if !entry.has_refresh_token {
        logs.push(
            "[!] 该账号备份中没有 refresh_token，若 access_token 已过期可能需要重新登录一次。"
                .to_string(),
        );
    }

    if let Ok(_guard) = account_lock.lock() {
        let _ = store.mark_used(id);
    }

    if restart {
        let (ok, mut launch_logs, error) = launch_antigravity_after_stop(settings);
        logs.append(&mut launch_logs);
        Ok(AccountSwitchResult {
            ok,
            email: Some(entry.email),
            logs,
            error,
        })
    } else {
        logs.push("[i] 已仅写入凭据（未重启 Antigravity），重启 Antigravity 后即生效。".to_string());
        Ok(AccountSwitchResult {
            ok: true,
            email: Some(entry.email),
            logs,
            error: None,
        })
    }
}

#[tauri::command]
async fn accounts_list(app: AppHandle, state: State<'_, AppState>) -> Result<AccountsList, String> {
    let lock = Arc::clone(&state.account_lock);
    let res = tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock.lock().map_err(|_| "Account store lock poisoned")?;
        AccountStore::default_store().list_and_sync()
    })
    .await
    .map_err(|e| format!("Account list task failed: {e}"))??;
    refresh_tray_menu(&app);
    Ok(res)
}

#[tauri::command]
async fn accounts_current() -> Result<CurrentCredential, String> {
    tauri::async_runtime::spawn_blocking(accounts::current_credential)
        .await
        .map_err(|e| format!("Account current task failed: {e}"))?
}

#[tauri::command]
async fn accounts_runtime_status(
    state: State<'_, AppState>,
) -> Result<AntigravityRuntimeStatus, String> {
    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    tauri::async_runtime::spawn_blocking(move || get_antigravity_runtime_status(&settings))
        .await
        .map_err(|e| format!("Runtime status task failed: {e}"))
}

#[tauri::command]
async fn accounts_save_current(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AccountEntry, String> {
    let lock = Arc::clone(&state.account_lock);
    let entry = tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock.lock().map_err(|_| "Account store lock poisoned")?;
        AccountStore::default_store().save_current()
    })
    .await
    .map_err(|e| format!("Account save task failed: {e}"))??;
    refresh_tray_menu(&app);
    Ok(entry)
}

#[tauri::command]
async fn accounts_rename(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> Result<AccountEntry, String> {
    let lock = Arc::clone(&state.account_lock);
    let entry = tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock.lock().map_err(|_| "Account store lock poisoned")?;
        AccountStore::default_store().rename(&id, &name)
    })
    .await
    .map_err(|e| format!("Account rename task failed: {e}"))??;
    refresh_tray_menu(&app);
    Ok(entry)
}

#[tauri::command]
async fn accounts_toggle_pin(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    pinned: bool,
) -> Result<AccountEntry, String> {
    let lock = Arc::clone(&state.account_lock);
    let entry = tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock.lock().map_err(|_| "Account store lock poisoned")?;
        AccountStore::default_store().set_pinned(&id, pinned)
    })
    .await
    .map_err(|e| format!("Account pin task failed: {e}"))??;
    refresh_tray_menu(&app);
    Ok(entry)
}

#[tauri::command]
async fn accounts_remove(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    let lock = Arc::clone(&state.account_lock);
    let removed = tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock.lock().map_err(|_| "Account store lock poisoned")?;
        AccountStore::default_store().remove(&id)
    })
    .await
    .map_err(|e| format!("Account remove task failed: {e}"))??;
    refresh_tray_menu(&app);
    Ok(removed)
}

#[tauri::command]
async fn accounts_verify(state: State<'_, AppState>) -> Result<Vec<AccountHealth>, String> {
    let lock = Arc::clone(&state.account_lock);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock.lock().map_err(|_| "Account store lock poisoned")?;
        Ok::<_, String>(AccountStore::default_store().verify_all())
    })
    .await
    .map_err(|e| format!("Account verify task failed: {e}"))?
}

#[tauri::command]
fn accounts_open_store_dir() -> Result<String, String> {
    let store = AccountStore::default_store();
    store.ensure_dir()?;
    let dir = store.dir().to_path_buf();
    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("无法打开账号目录: {e}"))?;
    }
    Ok(dir.to_string_lossy().to_string())
}

/// 添加账号：备份当前账号 → 停止 Antigravity → 清除登录凭据 → 重新启动（等待用户登录）。
#[tauri::command]
async fn accounts_begin_login(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<LoginStartResult, String> {
    let Some(run_guard) = AtomicFlagGuard::try_acquire(&state.launch_running) else {
        return Ok(LoginStartResult {
            ok: false,
            previous_email: None,
            logs: vec!["[i] 正在执行启动或切换任务，请稍候再试。".to_string()],
            error: Some("已有正在进行的启动/切换任务".to_string()),
        });
    };

    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    let account_lock = Arc::clone(&state.account_lock);

    let res = tauri::async_runtime::spawn_blocking(move || {
        let _run_guard = run_guard;
        let store = AccountStore::default_store();
        let mut logs: Vec<String> = Vec::new();

        if !Path::new(settings.antigravity_path.trim()).is_file() {
            let msg = format!(
                "找不到 Antigravity 可执行文件，已取消添加账号: {}",
                settings.antigravity_path
            );
            logs.push(format!("[X] {msg}"));
            return LoginStartResult {
                ok: false,
                previous_email: None,
                logs,
                error: Some(msg),
            };
        }

        let has_credential = match credential::read_blob() {
            Ok(blob) => blob.is_some(),
            Err(err) => {
                logs.push(format!("[X] 读取当前登录凭据失败: {err}"));
                return LoginStartResult {
                    ok: false,
                    previous_email: None,
                    logs,
                    error: Some(err),
                };
            }
        };

        // 安全前提：只要当前存在登录凭据，必须先成功备份，否则绝不删除凭据。
        let previous_email = if has_credential {
            let save_res = match account_lock.lock() {
                Ok(_g) => store.save_current(),
                Err(_) => Err("Account store lock poisoned".to_string()),
            };
            match save_res {
                Ok(entry) => {
                    logs.push(format!("[OK] 已备份当前账号 {}，之后可一键切回。", entry.email));
                    Some(entry.email)
                }
                Err(err) => {
                    let msg = format!("无法备份当前账号，为避免丢失登录状态已中止: {err}");
                    logs.push(format!("[X] {msg}"));
                    return LoginStartResult {
                        ok: false,
                        previous_email: None,
                        logs,
                        error: Some(msg),
                    };
                }
            }
        } else {
            logs.push("[i] 当前没有登录凭据，无需备份。".to_string());
            None
        };

        let (stopped, mut stop_logs) = stop_antigravity_processes(&settings.antigravity_path);
        logs.append(&mut stop_logs);
        if !stopped {
            let msg = "旧 Antigravity 进程未能完全退出，请先手动退出后再试。".to_string();
            logs.push(format!("[X] {msg}"));
            return LoginStartResult {
                ok: false,
                previous_email,
                logs,
                error: Some(msg),
            };
        }

        match credential::delete() {
            Ok(true) => {
                logs.push("[OK] 已清除当前登录凭据，重启后会要求登录新账号。".to_string());
            }
            Ok(false) => logs.push("[i] 当前没有登录凭据可清除。".to_string()),
            Err(err) => {
                logs.push(format!("[X] 清除登录凭据失败: {err}"));
                return LoginStartResult {
                    ok: false,
                    previous_email,
                    logs,
                    error: Some(err),
                };
            }
        }

        let (ok, mut launch_logs, error) = launch_antigravity_after_stop(&settings);
        logs.append(&mut launch_logs);
        if ok {
            logs.push("[..] 请在 Antigravity 登录新账号；登录成功后本工具会自动保存该账号。".to_string());
        }
        LoginStartResult {
            ok,
            previous_email,
            logs,
            error,
        }
    })
    .await
    .map_err(|e| format!("Account login task failed: {e}"))?;

    refresh_tray_menu(&app);
    Ok(res)
}

/// 退出当前登录：先安全备份当前账号 → 停止 Antigravity → 清除 Windows 凭据。
#[tauri::command]
async fn accounts_logout(
    app: AppHandle,
    state: State<'_, AppState>,
    stop_app: Option<bool>,
) -> Result<AccountSwitchResult, String> {
    let Some(run_guard) = AtomicFlagGuard::try_acquire(&state.launch_running) else {
        return Ok(AccountSwitchResult {
            ok: false,
            email: None,
            logs: vec!["[i] 正在执行启动或切换任务，请稍候再试。".to_string()],
            error: Some("已有正在进行的启动/切换任务".to_string()),
        });
    };

    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    let account_lock = Arc::clone(&state.account_lock);
    let should_stop = stop_app.unwrap_or(true);

    let res = tauri::async_runtime::spawn_blocking(move || {
        let _run_guard = run_guard;
        let store = AccountStore::default_store();
        let mut logs: Vec<String> = Vec::new();

        let current_email = match credential::read_blob() {
            Ok(Some(raw)) => {
                let email = credential::parse_blob(&raw).email;
                let save_res = match account_lock.lock() {
                    Ok(_g) => store.save_current(),
                    Err(_) => Err("Account store lock poisoned".to_string()),
                };
                match save_res {
                    Ok(entry) => {
                        logs.push(format!("[OK] 退出前已备份当前账号 {}。", entry.email));
                        Some(entry.email)
                    }
                    Err(err) => {
                        let already_known = account_lock
                            .lock()
                            .ok()
                            .and_then(|_| email.as_deref().map(|e| store.contains_email(e)))
                            .unwrap_or(false);
                        if already_known {
                            logs.push(format!("[i] 当前账号此前已备份，继续退出: {err}"));
                            email
                        } else {
                            let msg = format!("退出前无法备份当前账号，已中止以防丢失登录状态: {err}");
                            logs.push(format!("[X] {msg}"));
                            return AccountSwitchResult {
                                ok: false,
                                email,
                                logs,
                                error: Some(msg),
                            };
                        }
                    }
                }
            }
            Ok(None) => {
                logs.push("[i] 当前未登录任何账号。".to_string());
                return AccountSwitchResult {
                    ok: true,
                    email: None,
                    logs,
                    error: None,
                };
            }
            Err(err) => {
                logs.push(format!("[X] 读取当前登录凭据失败: {err}"));
                return AccountSwitchResult {
                    ok: false,
                    email: None,
                    logs,
                    error: Some(err),
                };
            }
        };

        if should_stop {
            let (stopped, mut stop_logs) = stop_antigravity_processes(&settings.antigravity_path);
            logs.append(&mut stop_logs);
            if !stopped {
                let msg = "旧 Antigravity 进程未能完全退出，请先手动退出后再试。".to_string();
                logs.push(format!("[X] {msg}"));
                return AccountSwitchResult {
                    ok: false,
                    email: current_email,
                    logs,
                    error: Some(msg),
                };
            }
        }

        match credential::delete() {
            Ok(true) => {
                logs.push("[OK] 已退出当前账号并清除系统凭据（本地备份已保留）。".to_string());
                AccountSwitchResult {
                    ok: true,
                    email: current_email,
                    logs,
                    error: None,
                }
            }
            Ok(false) => {
                logs.push("[i] 系统凭据已不存在。".to_string());
                AccountSwitchResult {
                    ok: true,
                    email: current_email,
                    logs,
                    error: None,
                }
            }
            Err(err) => {
                logs.push(format!("[X] 清除系统凭据失败: {err}"));
                AccountSwitchResult {
                    ok: false,
                    email: current_email,
                    logs,
                    error: Some(err),
                }
            }
        }
    })
    .await
    .map_err(|e| format!("Account logout task failed: {e}"))?;

    refresh_tray_menu(&app);
    Ok(res)
}

/// 切换账号：备份当前账号 →（可选停止 Antigravity）→ 写回目标账号凭据 →（可选重启）。
#[tauri::command]
async fn accounts_switch(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    restart: Option<bool>,
) -> Result<AccountSwitchResult, String> {
    let Some(run_guard) = AtomicFlagGuard::try_acquire(&state.launch_running) else {
        return Ok(AccountSwitchResult {
            ok: false,
            email: None,
            logs: vec!["[i] 正在执行启动或切换任务，请稍候再试。".to_string()],
            error: Some("已有正在进行的启动/切换任务".to_string()),
        });
    };

    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    let account_lock = Arc::clone(&state.account_lock);
    let should_restart = restart.unwrap_or(true);

    let res = tauri::async_runtime::spawn_blocking(move || {
        let _run_guard = run_guard;
        execute_account_switch(&account_lock, &settings, &id, should_restart)
    })
    .await
    .map_err(|e| format!("Account switch task failed: {e}"))??;

    refresh_tray_menu(&app);
    Ok(res)
}

// ------------------------- Quota & Plan Tier Commands -------------------------

#[tauri::command]
async fn quota_get_cached(
    state: State<'_, AppState>,
) -> Result<HashMap<String, AccountQuotaSnapshot>, String> {
    let lock = Arc::clone(&state.quota_cache_lock);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock.lock().map_err(|_| "Quota cache lock poisoned")?;
        Ok(QuotaCacheStore::default_store().load_all())
    })
    .await
    .map_err(|e| format!("Quota cache load failed: {e}"))?
}

fn collect_live_token_blobs(include_cred_mgr: bool) -> Vec<Vec<u8>> {
    let mut blobs = credential::read_local_token_file_blobs();
    if include_cred_mgr {
        if let Ok(Some(blob)) = credential::read_blob() {
            blobs.push(blob);
        }
    }
    blobs
}

#[tauri::command]
async fn quota_fetch_current(
    state: State<'_, AppState>,
    force_refresh: Option<bool>,
) -> Result<AccountQuotaSnapshot, String> {
    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    let token_cache = Arc::clone(&state.quota_token_cache);
    let cache_lock = Arc::clone(&state.quota_cache_lock);
    let account_lock = Arc::clone(&state.account_lock);
    let force = force_refresh.unwrap_or(false);

    tauri::async_runtime::spawn_blocking(move || {
        let local_file_blobs = credential::read_local_token_file_blobs();
        let cred_mgr_blob = credential::read_blob()?;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let blob = match cred_mgr_blob
            .or_else(|| credential::pick_freshest_any_blob(&local_file_blobs, now_ms))
        {
            Some(b) => b,
            None => {
                return Ok(AccountQuotaSnapshot {
                    error: Some("当前未登录 Antigravity 账号，请先登录或切换账号".to_string()),
                    ..Default::default()
                });
            }
        };

        let info = credential::parse_blob(&blob);
        let account_id = info.email.as_deref().and_then(|email| {
            let _guard = account_lock.lock().ok()?;
            AccountStore::default_store()
                .list_readonly(Some(email))
                .into_iter()
                .find(|a| a.email.eq_ignore_ascii_case(email))
                .map(|a| a.id)
        });

        let cached_map = {
            let _guard = cache_lock
                .lock()
                .map_err(|_| "Quota cache lock poisoned")?;
            QuotaCacheStore::default_store().load_all()
        };

        let snap = quota::fetch_quota_for_blob(
            &blob,
            &local_file_blobs,
            account_id,
            &settings,
            &token_cache,
            &cached_map,
            force,
        );

        if snap.error.is_none() {
            let _guard = cache_lock
                .lock()
                .map_err(|_| "Quota cache lock poisoned")?;
            let _ = QuotaCacheStore::default_store().upsert(&snap);
        }

        Ok(snap)
    })
    .await
    .map_err(|e| format!("Quota fetch current failed: {e}"))?
}

#[tauri::command]
async fn quota_fetch_account(
    state: State<'_, AppState>,
    id: String,
    force_refresh: Option<bool>,
) -> Result<AccountQuotaSnapshot, String> {
    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    let token_cache = Arc::clone(&state.quota_token_cache);
    let cache_lock = Arc::clone(&state.quota_cache_lock);
    let account_lock = Arc::clone(&state.account_lock);
    let force = force_refresh.unwrap_or(true);

    tauri::async_runtime::spawn_blocking(move || {
        let (entry, blob) = {
            let _guard = account_lock
                .lock()
                .map_err(|_| "Account store lock poisoned")?;
            let store = AccountStore::default_store();
            let entry = store
                .get(&id)
                .ok_or_else(|| "账号不存在或已被删除".to_string())?;
            let blob = store.load_blob(&id)?;
            (entry, blob)
        };

        let live_blobs = collect_live_token_blobs(true);

        let cached_map = {
            let _guard = cache_lock
                .lock()
                .map_err(|_| "Quota cache lock poisoned")?;
            QuotaCacheStore::default_store().load_all()
        };

        let mut snap = quota::fetch_quota_for_blob(
            &blob,
            &live_blobs,
            Some(entry.id.clone()),
            &settings,
            &token_cache,
            &cached_map,
            force,
        );
        if snap.email == "unknown" && !entry.email.is_empty() {
            snap.email = entry.email;
        }

        if snap.error.is_none() {
            let _guard = cache_lock
                .lock()
                .map_err(|_| "Quota cache lock poisoned")?;
            let _ = QuotaCacheStore::default_store().upsert(&snap);
        }

        Ok(snap)
    })
    .await
    .map_err(|e| format!("Quota fetch account failed: {e}"))?
}

#[tauri::command]
async fn quota_fetch_all(state: State<'_, AppState>) -> Result<Vec<AccountQuotaSnapshot>, String> {
    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    let token_cache = Arc::clone(&state.quota_token_cache);
    let cache_lock = Arc::clone(&state.quota_cache_lock);
    let account_lock = Arc::clone(&state.account_lock);

    tauri::async_runtime::spawn_blocking(move || {
        let cached_map = {
            let _guard = cache_lock
                .lock()
                .map_err(|_| "Quota cache lock poisoned")?;
            QuotaCacheStore::default_store().load_all()
        };

        let mut jobs: Vec<(Option<String>, Vec<u8>)> = Vec::new();
        let mut pre_errors: Vec<AccountQuotaSnapshot> = Vec::new();
        let mut seen_emails: Vec<String> = Vec::new();

        {
            let _guard = account_lock
                .lock()
                .map_err(|_| "Account store lock poisoned")?;
            let store = AccountStore::default_store();
            let list = store.list_and_sync()?;
            for acc in list.accounts {
                seen_emails.push(acc.email.to_ascii_lowercase());
                match store.load_blob(&acc.id) {
                    Ok(blob) => jobs.push((Some(acc.id), blob)),
                    Err(err) => {
                        let key = acc.email.to_ascii_lowercase();
                        pre_errors.push(quota::error_snapshot_with_fallback(
                            acc.email,
                            Some(acc.id),
                            None,
                            err,
                            cached_map.get(&key).cloned(),
                        ));
                    }
                }
            }
            if let Ok(Some(cur_blob)) = credential::read_blob() {
                if let Some(cur_email) = credential::parse_blob(&cur_blob).email {
                    let lower = cur_email.trim().to_ascii_lowercase();
                    if !seen_emails.iter().any(|e| e == &lower) {
                        jobs.push((None, cur_blob));
                    }
                }
            }
        }

        let live_blobs = collect_live_token_blobs(true);
        let mut results =
            quota::fetch_all_accounts_quota(jobs, &live_blobs, &settings, &token_cache, &cached_map);
        results.extend(pre_errors);

        {
            let _guard = cache_lock
                .lock()
                .map_err(|_| "Quota cache lock poisoned")?;
            let _ = QuotaCacheStore::default_store().upsert_many(&results);
        }

        Ok(results)
    })
    .await
    .map_err(|e| format!("Quota fetch all failed: {e}"))?
}

#[tauri::command]
async fn quota_oauth_config_status() -> Result<quota::OAuthClientStatus, String> {
    tauri::async_runtime::spawn_blocking(quota::get_oauth_client_status)
        .await
        .map_err(|e| format!("Load OAuth config status failed: {e}"))
}

#[tauri::command]
async fn quota_oauth_config_save(
    client_id: String,
    client_secret: String,
) -> Result<quota::OAuthClientStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        quota::save_oauth_client_config(&client_id, &client_secret)
    })
    .await
    .map_err(|e| format!("Save OAuth config failed: {e}"))?
}

#[tauri::command]
async fn quota_oauth_config_clear() -> Result<quota::OAuthClientStatus, String> {
    tauri::async_runtime::spawn_blocking(quota::clear_oauth_client_config)
        .await
        .map_err(|e| format!("Clear OAuth config failed: {e}"))?
}

// ------------------------- Isolated Proxy Commands -------------------------

#[tauri::command]
fn proxy_get_settings(state: State<'_, AppState>) -> Result<ProxySettings, String> {
    let s = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?;
    Ok(s.clone())
}

#[tauri::command]
fn proxy_save_settings(
    state: State<'_, AppState>,
    settings: ProxySettings,
) -> Result<ProxySettings, String> {
    let clean = settings.sanitized();
    clean.save();
    let _ = sync_cli_shim_if_enabled(&clean);
    let mut guard = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?;
    *guard = clean.clone();
    Ok(clean)
}

#[tauri::command]
async fn proxy_get_snapshot(state: State<'_, AppState>) -> Result<ProxySnapshot, String> {
    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    tauri::async_runtime::spawn_blocking(move || build_snapshot(&settings))
        .await
        .map_err(|e| format!("Snapshot task failed: {e}"))
}

#[tauri::command]
fn proxy_browse_path(kind: String, current: Option<String>) -> Result<BrowseResult, String> {
    let cur_str = current.unwrap_or_default();
    let cur_path = Path::new(cur_str.trim());

    if kind == "workspace" || kind == "tool_dir" {
        let title = if kind == "tool_dir" {
            "选择外部工具的工作目录"
        } else {
            "选择要在 Antigravity 中打开的工作区目录"
        };
        let mut dlg = rfd::FileDialog::new().set_title(title);
        if cur_path.is_dir() {
            dlg = dlg.set_directory(cur_path);
        }
        return Ok(match dlg.pick_folder() {
            Some(p) => BrowseResult {
                selected: true,
                path: p.to_string_lossy().to_string(),
            },
            None => BrowseResult {
                selected: false,
                path: cur_str,
            },
        });
    }

    let (title, default_file, extensions): (&str, &str, &[&str]) = match kind.as_str() {
        "clash" => ("选择 clash-verge.exe", "clash-verge.exe", &["exe"]),
        "cli" => ("选择 Antigravity CLI (agy.exe)", "agy.exe", &["exe"]),
        "tool" => ("选择外部 CLI 工具可执行文件", "", &["exe", "cmd", "bat"]),
        _ => ("选择 Antigravity.exe", "Antigravity.exe", &["exe"]),
    };

    let mut dlg = rfd::FileDialog::new()
        .set_title(title)
        .add_filter("可执行文件", extensions);
    if !default_file.is_empty() {
        dlg = dlg.set_file_name(default_file);
    }
    if let Some(parent) = cur_path.parent() {
        if parent.is_dir() {
            dlg = dlg.set_directory(parent);
        }
    }

    Ok(match dlg.pick_file() {
        Some(p) => BrowseResult {
            selected: true,
            path: p.to_string_lossy().to_string(),
        },
        None => BrowseResult {
            selected: false,
            path: cur_str,
        },
    })
}

#[tauri::command]
async fn proxy_detect_cli_and_tools() -> Result<CliDetectResult, String> {
    tauri::async_runtime::spawn_blocking(detect_cli_and_tools)
        .await
        .map_err(|e| format!("Detect CLI task failed: {e}"))
}

#[tauri::command]
async fn proxy_launch_cli_tool(
    state: State<'_, AppState>,
    tool_id: String,
    force_restart: bool,
) -> Result<ActionResponse, String> {
    let Some(run_guard) = AtomicFlagGuard::try_acquire(&state.launch_running) else {
        return Ok(ActionResponse {
            ok: false,
            need_confirm: false,
            process_count: 0,
            resolved_port: 0,
            logs: vec!["[i] 正在执行启动任务，请勿重复点击。".to_string()],
            error: None,
        });
    };

    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();

    tauri::async_runtime::spawn_blocking(move || {
        let _run_guard = run_guard;
        launch_isolated_cli_tool(&settings, &tool_id, force_restart)
    })
    .await
    .map_err(|e| format!("Launch tool task failed: {e}"))
}

#[tauri::command]
async fn proxy_stop_cli_tool(
    state: State<'_, AppState>,
    tool_id: String,
) -> Result<ActionResponse, String> {
    let tool = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .cli_tools
        .iter()
        .find(|t| t.id == tool_id)
        .cloned();

    let Some(tool) = tool else {
        let msg = format!("找不到指定的 CLI 工具配置: {tool_id}");
        return Ok(ActionResponse {
            ok: false,
            need_confirm: false,
            process_count: 0,
            resolved_port: 0,
            logs: vec![format!("[X] {msg}")],
            error: Some(msg),
        });
    };

    tauri::async_runtime::spawn_blocking(move || {
        let (ok, logs) = stop_cli_tool_processes(&tool.exe_path, &tool.name);
        ActionResponse {
            ok,
            need_confirm: false,
            process_count: 0,
            resolved_port: 0,
            logs,
            error: None,
        }
    })
    .await
    .map_err(|e| format!("Stop tool task failed: {e}"))
}

#[tauri::command]
async fn proxy_stop_cli_processes() -> Result<ActionResponse, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let (ok, logs) = stop_cli_processes();
        ActionResponse {
            ok,
            need_confirm: false,
            process_count: 0,
            resolved_port: 0,
            logs,
            error: None,
        }
    })
    .await
    .map_err(|e| format!("Stop CLI task failed: {e}"))
}

#[tauri::command]
async fn proxy_enable_cli_shim(state: State<'_, AppState>) -> Result<ActionResponse, String> {
    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    tauri::async_runtime::spawn_blocking(move || enable_cli_shim(&settings))
        .await
        .map_err(|e| format!("Enable CLI shim failed: {e}"))
}

#[tauri::command]
async fn proxy_disable_cli_shim() -> Result<ActionResponse, String> {
    tauri::async_runtime::spawn_blocking(disable_cli_shim)
        .await
        .map_err(|e| format!("Disable CLI shim failed: {e}"))
}

#[tauri::command]
async fn proxy_launch_cli_terminal(state: State<'_, AppState>) -> Result<ActionResponse, String> {
    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    tauri::async_runtime::spawn_blocking(move || launch_isolated_cli_terminal(&settings))
        .await
        .map_err(|e| format!("Launch CLI terminal failed: {e}"))
}

#[tauri::command]
async fn proxy_test_cli(state: State<'_, AppState>) -> Result<ActionResponse, String> {
    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    tauri::async_runtime::spawn_blocking(move || test_cli_proxy(&settings))
        .await
        .map_err(|e| format!("Test CLI failed: {e}"))
}

#[tauri::command]
async fn proxy_cli_snippets(state: State<'_, AppState>) -> Result<CliShellSnippets, String> {
    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (port, _, _, _) = resolve_port(&settings);
        build_cli_shell_snippets(&settings, port)
    })
    .await
    .map_err(|e| format!("Build CLI snippets failed: {e}"))
}

#[tauri::command]
async fn proxy_launch_antigravity(
    state: State<'_, AppState>,
    force_restart: bool,
) -> Result<ActionResponse, String> {
    if state
        .launch_running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(ActionResponse {
            ok: false,
            need_confirm: false,
            process_count: 0,
            resolved_port: 0,
            logs: vec!["[i] 正在启动中，请勿重复点击。".to_string()],
            error: None,
        });
    }

    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| {
            state.launch_running.store(false, Ordering::SeqCst);
            "Settings lock poisoned".to_string()
        })?
        .clone();
    let launch_flag = Arc::clone(&state.launch_running);

    let res = tauri::async_runtime::spawn_blocking(move || {
        let out = launch_isolated_antigravity(&settings, force_restart);
        launch_flag.store(false, Ordering::SeqCst);
        out
    })
    .await
    .map_err(|e| {
        state.launch_running.store(false, Ordering::SeqCst);
        format!("Launch task failed: {e}")
    })?;

    Ok(res)
}

#[tauri::command]
async fn proxy_stop_antigravity(state: State<'_, AppState>) -> Result<ActionResponse, String> {
    let agy_path = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .antigravity_path
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (ok, logs) = stop_antigravity_processes(&agy_path);
        ActionResponse {
            ok,
            need_confirm: false,
            process_count: 0,
            resolved_port: 0,
            logs,
            error: None,
        }
    })
    .await
    .map_err(|e| format!("Stop task failed: {e}"))
}

#[tauri::command]
async fn proxy_start_clash(state: State<'_, AppState>) -> Result<ActionResponse, String> {
    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (port, logs) = start_clash_verge(&settings);
        ActionResponse {
            ok: port > 0,
            need_confirm: false,
            process_count: 0,
            resolved_port: port,
            logs,
            error: if port == 0 {
                Some("等待 Clash Verge 端口就绪超时或路径无效".to_string())
            } else {
                None
            },
        }
    })
    .await
    .map_err(|e| format!("Start Clash task failed: {e}"))
}

#[tauri::command]
async fn proxy_flush_dns() -> Result<ActionResponse, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let logs = flush_dns_cache();
        ActionResponse {
            ok: true,
            need_confirm: false,
            process_count: 0,
            resolved_port: 0,
            logs,
            error: None,
        }
    })
    .await
    .map_err(|e| format!("Flush DNS task failed: {e}"))
}

#[tauri::command]
async fn proxy_dry_run(state: State<'_, AppState>) -> Result<String, String> {
    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (port, _, _, _) = resolve_port(&settings);
        build_dry_run_preview(&settings, port)
    })
    .await
    .map_err(|e| format!("DryRun task failed: {e}"))
}

#[tauri::command]
async fn proxy_run_audit(
    app: AppHandle,
    state: State<'_, AppState>,
    seconds: Option<u32>,
) -> Result<AuditResult, String> {
    if state
        .audit_running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("已有正在进行的验收任务".to_string());
    }

    let settings = state
        .proxy_settings
        .lock()
        .map_err(|_| "Settings lock poisoned")?
        .clone();
    let cancel_flag = Arc::clone(&state.audit_cancel);
    let running_flag = Arc::clone(&state.audit_running);
    cancel_flag.store(false, Ordering::SeqCst);
    let secs = seconds.unwrap_or(20).clamp(3, 60);

    let res = tauri::async_runtime::spawn_blocking(move || {
        let result = run_cumulative_audit(&settings, secs, &cancel_flag, |elapsed, total, snap| {
            let _ = app.emit(
                "proxy-audit-progress",
                AuditProgressPayload {
                    elapsed,
                    total,
                    snapshot: snap.clone(),
                },
            );
        });
        running_flag.store(false, Ordering::SeqCst);
        result
    })
    .await
    .map_err(|e| {
        state.audit_running.store(false, Ordering::SeqCst);
        format!("Audit task failed: {e}")
    })?;

    Ok(res)
}

#[tauri::command]
fn proxy_cancel_audit(state: State<'_, AppState>) -> Result<(), String> {
    state.audit_cancel.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
fn proxy_open_report() -> Result<String, String> {
    open_verify_report()
}

#[cfg(windows)]
mod single_instance {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use std::thread;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
        INFINITE,
    };

    const MUTEX_NAME: &str = "Local\\AntigravityUsageDashboard_SingleInstance_Mutex";
    const WAKE_EVENT_NAME: &str = "Local\\AntigravityUsageDashboard_SingleInstance_WakeEvent";

    fn to_wide_null(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    pub struct SingleInstanceGuard {
        mutex_handle: HANDLE,
        wake_event_handle: HANDLE,
    }

    unsafe impl Send for SingleInstanceGuard {}
    unsafe impl Sync for SingleInstanceGuard {}

    impl SingleInstanceGuard {
        pub fn acquire() -> Option<Self> {
            Self::acquire_named(MUTEX_NAME, WAKE_EVENT_NAME)
        }

        pub fn acquire_named(mutex_name: &str, event_name: &str) -> Option<Self> {
            let mutex_w = to_wide_null(mutex_name);
            let event_w = to_wide_null(event_name);

            let mutex_handle = unsafe { CreateMutexW(ptr::null(), 0, mutex_w.as_ptr()) };
            if mutex_handle.is_null() {
                return None;
            }

            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe { CloseHandle(mutex_handle) };
                let ev = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, event_w.as_ptr()) };
                if !ev.is_null() {
                    unsafe {
                        SetEvent(ev);
                        CloseHandle(ev);
                    }
                }
                return None;
            }

            let wake_event_handle = unsafe { CreateEventW(ptr::null(), 0, 0, event_w.as_ptr()) };
            Some(Self {
                mutex_handle,
                wake_event_handle,
            })
        }

        pub fn spawn_wake_listener<F>(&self, on_wake: F)
        where
            F: Fn() + Send + 'static,
        {
            if self.wake_event_handle.is_null() {
                return;
            }
            let ev_addr = self.wake_event_handle as usize;
            thread::spawn(move || {
                let ev = ev_addr as HANDLE;
                loop {
                    let wait = unsafe { WaitForSingleObject(ev, INFINITE) };
                    if wait == WAIT_OBJECT_0 {
                        on_wake();
                    } else {
                        break;
                    }
                }
            });
        }
    }

    impl Drop for SingleInstanceGuard {
        fn drop(&mut self) {
            unsafe {
                if !self.wake_event_handle.is_null() {
                    CloseHandle(self.wake_event_handle);
                }
                if !self.mutex_handle.is_null() {
                    CloseHandle(self.mutex_handle);
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        #[test]
        fn prevents_second_instance_and_signals_wake_event() {
            let pid = std::process::id();
            let mutex = format!("Local\\AntigravityUsage_TestMutex_{pid}");
            let event = format!("Local\\AntigravityUsage_TestEvent_{pid}");

            let first = SingleInstanceGuard::acquire_named(&mutex, &event);
            assert!(first.is_some(), "first instance should acquire mutex");
            let first = first.unwrap();

            let woke = Arc::new(AtomicBool::new(false));
            let woke_clone = Arc::clone(&woke);
            first.spawn_wake_listener(move || {
                woke_clone.store(true, Ordering::SeqCst);
            });

            let second = SingleInstanceGuard::acquire_named(&mutex, &event);
            assert!(second.is_none(), "second instance must be rejected");

            for _ in 0..20 {
                if woke.load(Ordering::SeqCst) {
                    break;
                }
                thread::sleep(Duration::from_millis(25));
            }
            assert!(woke.load(Ordering::SeqCst), "first instance should receive wake signal");
        }
    }
}

#[cfg(not(windows))]
mod single_instance {
    pub struct SingleInstanceGuard;
    impl SingleInstanceGuard {
        pub fn acquire() -> Option<Self> {
            Some(Self)
        }
        pub fn spawn_wake_listener<F>(&self, _on_wake: F)
        where
            F: Fn() + Send + 'static,
        {
        }
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let Some(_instance_guard) = single_instance::SingleInstanceGuard::acquire() else {
        return;
    };

    let initial_scanner = if let Some(custom_dir) = load_persisted_brain_dir() {
        BrainScanner::new(custom_dir)
    } else if let Some(env_dir) = std::env::var("ANTIGRAVITY_BRAIN_DIR")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        BrainScanner::new(PathBuf::from(env_dir))
    } else {
        BrainScanner::new_auto()
    };
    let initial_proxy_settings = ProxySettings::load();

    tauri::Builder::default()
        .manage(AppState {
            scanner: Mutex::new(initial_scanner),
            proxy_settings: Mutex::new(initial_proxy_settings),
            account_lock: Arc::new(Mutex::new(())),
            quota_token_cache: Arc::new(Mutex::new(TokenCache::new())),
            quota_cache_lock: Arc::new(Mutex::new(())),
            tray_signature: Mutex::new(String::new()),
            audit_cancel: Arc::new(AtomicBool::new(false)),
            audit_running: Arc::new(AtomicBool::new(false)),
            launch_running: Arc::new(AtomicBool::new(false)),
        })
        .setup(move |app| {
            let wake_handle = app.handle().clone();
            _instance_guard.spawn_wake_listener(move || {
                show_main_window(&wake_handle);
            });
            app.manage(_instance_guard);
            let init_email = accounts::current_credential().ok().and_then(|c| c.email);
            let init_saved = AccountStore::default_store().list_readonly(init_email.as_deref());
            let init_sig = compute_tray_signature(init_email.as_deref(), &init_saved);
            if let Ok(mut guard) = app.state::<AppState>().tray_signature.lock() {
                *guard = init_sig;
            }
            let (menu, tooltip) = build_tray_menu(app.handle(), init_email.as_deref(), &init_saved)?;

            let mut tray_builder = TrayIconBuilder::with_id("main-tray")
                .menu(&menu)
                .tooltip(&tooltip)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    let id = event.id.as_ref();
                    if let Some(acc_id) = id.strip_prefix(TRAY_SWITCH_PREFIX) {
                        let target_id = acc_id.to_string();
                        let app_handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let state = app_handle.state::<AppState>();
                            let Some(run_guard) = AtomicFlagGuard::try_acquire(&state.launch_running)
                            else {
                                return;
                            };
                            let settings = match state.proxy_settings.lock() {
                                Ok(g) => g.clone(),
                                Err(_) => return,
                            };
                            let account_lock = Arc::clone(&state.account_lock);
                            let res = tauri::async_runtime::spawn_blocking(move || {
                                let _run_guard = run_guard;
                                execute_account_switch(&account_lock, &settings, &target_id, true)
                            })
                            .await;
                            refresh_tray_menu(&app_handle);
                            if let Ok(Ok(switch_res)) = res {
                                let _ = app_handle.emit("accounts-changed", switch_res);
                            }
                        });
                        return;
                    }
                    match id {
                        "show" => show_main_window(app),
                        "hide" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.hide();
                            }
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                });

            if let Some(icon) = app.default_window_icon().cloned() {
                tray_builder = tray_builder.icon(icon);
            }
            tray_builder.build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_usage,
            get_session_detail,
            pick_brain_dir,
            reset_brain_dir,
            export_csv,
            accounts_list,
            accounts_current,
            accounts_runtime_status,
            accounts_save_current,
            accounts_rename,
            accounts_toggle_pin,
            accounts_remove,
            accounts_verify,
            accounts_open_store_dir,
            accounts_begin_login,
            accounts_logout,
            accounts_switch,
            quota_get_cached,
            quota_fetch_current,
            quota_fetch_account,
            quota_fetch_all,
            quota_oauth_config_status,
            quota_oauth_config_save,
            quota_oauth_config_clear,
            proxy_get_settings,
            proxy_save_settings,
            proxy_get_snapshot,
            proxy_browse_path,
            proxy_detect_cli_and_tools,
            proxy_launch_cli_tool,
            proxy_stop_cli_tool,
            proxy_stop_cli_processes,
            proxy_enable_cli_shim,
            proxy_disable_cli_shim,
            proxy_launch_cli_terminal,
            proxy_test_cli,
            proxy_cli_snippets,
            proxy_launch_antigravity,
            proxy_stop_antigravity,
            proxy_start_clash,
            proxy_flush_dns,
            proxy_dry_run,
            proxy_run_audit,
            proxy_cancel_audit,
            proxy_open_report,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Antigravity Usage desktop application");
}
