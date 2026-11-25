//! Antigravity 登录凭据的读取 / 备份 / 恢复。
//!
//! Antigravity 把 Google OAuth 令牌写入 Windows 凭据管理器：
//! `LegacyGeneric:target=gemini:antigravity`（Generic，LocalMachine 持久化）。
//! 该 blob 是一段 JSON：`token.access_token` / `token.refresh_token` /
//! `id_token_email` / `auth_method` / `writer_id`。
//!
//! 因此「切换账号」不需要重新登录，只要把某个账号的 blob 原样写回凭据管理器，
//! 再重启 Antigravity 即可（refresh_token 会让 Antigravity 自动换新 access_token）。
//!
//! 本地备份文件使用 DPAPI（CryptProtectData，当前用户作用域）加密，避免明文令牌落盘。

use serde::Serialize;

/// Antigravity 在 Windows 凭据管理器中的目标名。
pub const TARGET: &str = "gemini:antigravity";
/// 凭据管理器里的用户名（固定为 antigravity）。
pub const TARGET_USER: &str = "antigravity";

/// 从凭据 blob 中解析出的公开信息（不含任何令牌内容）。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialInfo {
    pub email: Option<String>,
    pub auth_method: Option<String>,
    pub has_access_token: bool,
    pub has_refresh_token: bool,
    /// `token.expiry` 解析出的毫秒时间戳（RFC3339，可能带时区偏移）。
    pub expires_at_ms: Option<i64>,
    pub blob_len: usize,
}

impl CredentialInfo {
    pub fn is_expired(&self, now_ms: i64) -> bool {
        matches!(self.expires_at_ms, Some(exp) if exp <= now_ms)
    }
}

/// 对凭据原始字节计算 FNV-1a 64-bit 指纹（不含明文，仅用于检测令牌内容是否发生变化）。
pub fn blob_fingerprint(blob: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in blob {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn decode_base64url(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;
    for b in input.bytes() {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' | b'\r' | b'\n' | b' ' => continue,
            _ => return None,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

fn extract_email_from_jwt(jwt: &str) -> Option<String> {
    let mut parts = jwt.trim().split('.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?;
    let bytes = decode_base64url(payload_b64)?;
    let val: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    val.get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn parse_expiry_value(val: &serde_json::Value) -> Option<i64> {
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
            return Some(dt.timestamp_millis());
        }
        if let Ok(num) = trimmed.parse::<i64>() {
            return Some(if num.abs() < 100_000_000_000 {
                num * 1000
            } else {
                num
            });
        }
    } else if let Some(num) = val.as_i64() {
        return Some(if num.abs() < 100_000_000_000 {
            num * 1000
        } else {
            num
        });
    } else if let Some(num) = val.as_f64() {
        let n = num as i64;
        return Some(if n.abs() < 100_000_000_000 {
            n * 1000
        } else {
            n
        });
    }
    None
}

/// 解析凭据 blob。无法解析时返回空信息（保留 blob 原样，不影响写回）。
pub fn parse_blob(blob: &[u8]) -> CredentialInfo {
    let mut info = CredentialInfo {
        blob_len: blob.len(),
        ..Default::default()
    };
    let text = String::from_utf8_lossy(blob);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return info;
    };

    let token = value.get("token").cloned().unwrap_or(serde_json::Value::Null);

    let non_empty_str = |v: Option<&serde_json::Value>| -> Option<String> {
        v.and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    let email = non_empty_str(value.get("id_token_email"))
        .or_else(|| non_empty_str(value.get("email")))
        .or_else(|| non_empty_str(token.get("email")))
        .or_else(|| {
            non_empty_str(value.get("id_token"))
                .or_else(|| non_empty_str(token.get("id_token")))
                .and_then(|jwt| extract_email_from_jwt(&jwt))
        });
    info.email = email;

    info.auth_method = non_empty_str(value.get("auth_method"));

    info.has_access_token = token
        .get("access_token")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    info.has_refresh_token = token
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    info.expires_at_ms = token
        .get("expiry")
        .and_then(parse_expiry_value)
        .or_else(|| token.get("expires_at").and_then(parse_expiry_value))
        .or_else(|| token.get("expiry_date").and_then(parse_expiry_value));

    info
}

/// 内部使用的凭据令牌结构：绝不派生 Serialize，Debug 输出对令牌脱敏。
#[derive(Clone, Default)]
pub(crate) struct CredentialTokens {
    pub email: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_at_ms: Option<i64>,
}

impl std::fmt::Debug for CredentialTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialTokens")
            .field("email", &self.email)
            .field("access_token", &self.access_token.as_ref().map(|_| "[REDACTED]"))
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "[REDACTED]"))
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

/// 从凭据 blob 中提取 OAuth 令牌（仅供后端发起额度查询使用）。
pub(crate) fn extract_tokens(blob: &[u8]) -> CredentialTokens {
    let info = parse_blob(blob);
    let mut out = CredentialTokens {
        email: info.email,
        access_token: None,
        refresh_token: None,
        expires_at_ms: info.expires_at_ms,
    };
    let text = String::from_utf8_lossy(blob);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return out;
    };

    let non_empty_str = |v: Option<&serde_json::Value>| -> Option<String> {
        v.and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    let token = value.get("token");
    out.access_token = token
        .and_then(|t| non_empty_str(t.get("access_token")))
        .or_else(|| non_empty_str(value.get("access_token")));
    out.refresh_token = token
        .and_then(|t| non_empty_str(t.get("refresh_token")))
        .or_else(|| non_empty_str(value.get("refresh_token")));

    out
}

const MAX_LOCAL_TOKEN_FILE_BYTES: u64 = 256 * 1024;

/// 发现本机各 Antigravity 客户端（Desktop App / CLI / T3 Code ACP）保存的 `antigravity-oauth-token` 文件路径。
pub(crate) fn discover_local_token_file_paths(
    home: Option<&std::path::Path>,
) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Some(h) = home else {
        return out;
    };

    let candidates = [
        h.join(".gemini")
            .join("antigravity")
            .join("antigravity-oauth-token"),
        h.join(".gemini")
            .join("antigravity-cli")
            .join("antigravity-oauth-token"),
    ];
    for p in candidates {
        if p.is_file() {
            out.push(p);
        }
    }

    let t3_root = h
        .join(".t3")
        .join("userdata")
        .join("providers")
        .join("antigravity");
    if let Ok(entries) = std::fs::read_dir(&t3_root) {
        let mut subdirs: Vec<std::path::PathBuf> = entries
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
            let p = sub.join("antigravity-acp").join("antigravity-oauth-token");
            if p.is_file() {
                out.push(p);
            }
        }
    }

    out
}

/// 只读读取本机 Antigravity 客户端保存的 OAuth 令牌文件原始内容（单文件上限 256 KiB）。
pub(crate) fn read_local_token_file_blobs() -> Vec<Vec<u8>> {
    use std::io::Read;

    let mut blobs = Vec::new();
    for path in discover_local_token_file_paths(dirs::home_dir().as_deref()) {
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let mut buf = Vec::new();
        if file
            .take(MAX_LOCAL_TOKEN_FILE_BYTES)
            .read_to_end(&mut buf)
            .is_ok()
            && !buf.is_empty()
        {
            blobs.push(buf);
        }
    }
    blobs
}

fn is_candidate_access_fresher(
    cand: &CredentialTokens,
    best: &CredentialTokens,
    now_ms: i64,
) -> bool {
    if cand.access_token.is_none() {
        return false;
    }
    if best.access_token.is_none() {
        return true;
    }
    match (cand.expires_at_ms, best.expires_at_ms) {
        (Some(c), Some(b)) => c > b,
        (Some(c), None) => c > now_ms,
        (None, Some(b)) => b <= now_ms,
        (None, None) => false,
    }
}

/// 在多个凭据来源中为同邮箱账号挑选最新鲜的 access_token / refresh_token。
pub(crate) fn select_freshest_tokens(
    primary: &CredentialTokens,
    extra_blobs: &[Vec<u8>],
    target_email: Option<&str>,
    now_ms: i64,
) -> CredentialTokens {
    let match_email = target_email
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .or_else(|| {
            primary
                .email
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_ascii_lowercase())
        });

    let mut best = primary.clone();
    for blob in extra_blobs {
        let cand = extract_tokens(blob);
        if let Some(ref target) = match_email {
            let Some(cand_email) = cand.email.as_deref().map(str::trim).filter(|s| !s.is_empty())
            else {
                continue;
            };
            if !cand_email.eq_ignore_ascii_case(target) {
                continue;
            }
        }

        if best.email.is_none() {
            best.email = cand.email.clone();
        }
        if best.refresh_token.is_none() {
            best.refresh_token = cand.refresh_token.clone();
        }
        if is_candidate_access_fresher(&cand, &best, now_ms) {
            best.access_token = cand.access_token;
            best.expires_at_ms = cand.expires_at_ms;
            if cand.refresh_token.is_some() {
                best.refresh_token = cand.refresh_token;
            }
        }
    }

    best
}

/// 当系统凭据管理器缺失时，从本地客户端令牌文件中挑选最新鲜且含邮箱的有效 blob。
pub(crate) fn pick_freshest_any_blob(blobs: &[Vec<u8>], now_ms: i64) -> Option<Vec<u8>> {
    blobs
        .iter()
        .filter_map(|blob| {
            let tok = extract_tokens(blob);
            let has_email = tok
                .email
                .as_deref()
                .map(str::trim)
                .is_some_and(|s| !s.is_empty());
            if !has_email || (tok.access_token.is_none() && tok.refresh_token.is_none()) {
                return None;
            }
            let has_access = tok.access_token.is_some();
            let has_unexpired = has_access
                && tok
                    .expires_at_ms
                    .map(|exp| exp > now_ms)
                    .unwrap_or(true);
            let rank = (
                has_unexpired,
                tok.expires_at_ms.unwrap_or(i64::MIN),
                has_access,
            );
            Some((rank, blob))
        })
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, blob)| blob.clone())
}

/// 读取当前登录凭据的原始 blob；未登录时返回 `Ok(None)`。
pub fn read_blob() -> Result<Option<Vec<u8>>, String> {
    sys::read_blob(TARGET)
}

/// 把备份的 blob 原样写回凭据管理器（覆盖当前登录账号）。
pub fn write_blob(blob: &[u8]) -> Result<(), String> {
    if blob.is_empty() {
        return Err("凭据备份为空，无法恢复".to_string());
    }
    sys::write_blob(TARGET, TARGET_USER, blob)
}

/// 清除当前登录凭据；不存在时返回 `Ok(false)`。
pub fn delete() -> Result<bool, String> {
    sys::delete(TARGET)
}

/// DPAPI 加密（当前 Windows 用户作用域）。
pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    sys::protect(data)
}

/// DPAPI 解密。
pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    sys::unprotect(data)
}

#[cfg(windows)]
mod sys {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use windows_sys::Win32::Foundation::{GetLastError, LocalFree, FILETIME};
    use windows_sys::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
        CRED_TYPE_GENERIC,
    };
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

    const ERROR_NOT_FOUND: u32 = 1168;

    fn wide(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }

    pub fn read_blob(target: &str) -> Result<Option<Vec<u8>>, String> {
        let target_w = wide(target);
        let mut handle: *mut CREDENTIALW = ptr::null_mut();
        let ok = unsafe { CredReadW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0, &mut handle) };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            return if code == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err(format!("读取 Windows 凭据失败 (Win32 {code})"))
            };
        }
        if handle.is_null() {
            return Ok(None);
        }

        let blob = unsafe {
            let cred = &*handle;
            let bytes = if cred.CredentialBlob.is_null() || cred.CredentialBlobSize == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(
                    cred.CredentialBlob as *const u8,
                    cred.CredentialBlobSize as usize,
                )
                .to_vec()
            };
            CredFree(handle as *const core::ffi::c_void);
            bytes
        };

        Ok(Some(blob))
    }

    pub fn write_blob(target: &str, user: &str, blob: &[u8]) -> Result<(), String> {
        let mut target_w = wide(target);
        let mut user_w = wide(user);
        let mut blob_copy = blob.to_vec();

        let cred = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: target_w.as_mut_ptr(),
            Comment: ptr::null_mut(),
            LastWritten: FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            },
            CredentialBlobSize: blob_copy.len() as u32,
            CredentialBlob: blob_copy.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: ptr::null_mut(),
            TargetAlias: ptr::null_mut(),
            UserName: user_w.as_mut_ptr(),
        };

        let ok = unsafe { CredWriteW(&cred, 0) };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            return Err(format!("写入 Windows 凭据失败 (Win32 {code})"));
        }
        Ok(())
    }

    pub fn delete(target: &str) -> Result<bool, String> {
        let target_w = wide(target);
        let ok = unsafe { CredDeleteW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            return if code == ERROR_NOT_FOUND {
                Ok(false)
            } else {
                Err(format!("删除 Windows 凭据失败 (Win32 {code})"))
            };
        }
        Ok(true)
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        let ok = unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                0,
                &mut output,
            )
        };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            return Err(format!("DPAPI 加密失败 (Win32 {code})"));
        }
        let encrypted = unsafe {
            let bytes =
                std::slice::from_raw_parts(output.pbData as *const u8, output.cbData as usize)
                    .to_vec();
            LocalFree(output.pbData as *mut core::ffi::c_void);
            bytes
        };
        Ok(encrypted)
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                0,
                &mut output,
            )
        };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            return Err(format!("DPAPI 解密失败 (Win32 {code})"));
        }
        let plain = unsafe {
            let bytes =
                std::slice::from_raw_parts(output.pbData as *const u8, output.cbData as usize)
                    .to_vec();
            LocalFree(output.pbData as *mut core::ffi::c_void);
            bytes
        };
        Ok(plain)
    }
}

#[cfg(not(windows))]
mod sys {
    pub fn read_blob(_target: &str) -> Result<Option<Vec<u8>>, String> {
        Err("账号切换功能仅支持 Windows".to_string())
    }

    pub fn write_blob(_target: &str, _user: &str, _blob: &[u8]) -> Result<(), String> {
        Err("账号切换功能仅支持 Windows".to_string())
    }

    pub fn delete(_target: &str) -> Result<bool, String> {
        Err("账号切换功能仅支持 Windows".to_string())
    }

    pub fn protect(_data: &[u8]) -> Result<Vec<u8>, String> {
        Err("账号切换功能仅支持 Windows".to_string())
    }

    pub fn unprotect(_data: &[u8]) -> Result<Vec<u8>, String> {
        Err("账号切换功能仅支持 Windows".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_blob(email: &str, refresh: bool) -> Vec<u8> {
        let refresh_token = if refresh { "1//0abcdef" } else { "" };
        format!(
            r#"{{"token":{{"access_token":"ya29.token","token_type":"Bearer","refresh_token":"{refresh_token}","expiry":"2026-11-20T11:33:36.4912548+08:00","expires_in":3599}},"auth_method":"consumer","id_token_email":"{email}","writer_id":"writer-1"}}"#
        )
        .into_bytes()
    }

    #[test]
    fn parses_email_tokens_and_expiry() {
        let info = parse_blob(&sample_blob("demo@example.com", true));
        assert_eq!(info.email.as_deref(), Some("demo@example.com"));
        assert_eq!(info.auth_method.as_deref(), Some("consumer"));
        assert!(info.has_access_token);
        assert!(info.has_refresh_token);
        assert_eq!(info.expires_at_ms, Some(1795145616491));
        assert!(info.is_expired(1795145616492));
        assert!(!info.is_expired(1795145616490));
    }

    #[test]
    fn parses_blob_without_refresh_token() {
        let info = parse_blob(&sample_blob("demo@example.com", false));
        assert!(!info.has_refresh_token);
        assert!(info.has_access_token);
    }

    #[test]
    fn tolerates_garbage_and_empty_blobs() {
        let info = parse_blob(b"not-json");
        assert!(info.email.is_none());
        assert!(!info.has_refresh_token);
        assert_eq!(info.blob_len, 8);

        let empty = parse_blob(b"");
        assert_eq!(empty.blob_len, 0);
    }

    #[test]
    fn parses_fallback_jwt_email_and_numeric_expiry() {
        // payload: {"email":"jwt.user@example.com"} -> eyJlbWFpbCI6Imp3dC51c2VyQGV4YW1wbGUuY29tIn0
        let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJlbWFpbCI6Imp3dC51c2VyQGV4YW1wbGUuY29tIn0.sig";
        let raw = format!(
            r#"{{"token":{{"access_token":"ya29.x","refresh_token":"1//r","expires_at":1795145616,"id_token":"{jwt}"}}}}"#
        );
        let info = parse_blob(raw.as_bytes());
        assert_eq!(info.email.as_deref(), Some("jwt.user@example.com"));
        assert_eq!(info.expires_at_ms, Some(1795145616000));
        assert!(info.has_refresh_token);
        assert_ne!(blob_fingerprint(raw.as_bytes()), blob_fingerprint(b"other"));
    }

    #[cfg(windows)]
    #[test]
    fn credential_roundtrip_and_dpapi_roundtrip() {
        let pid = std::process::id();
        let target = format!("gemini:antigravity.test.{pid}");
        let payload = sample_blob("probe@example.com", true);

        write_blob_for_test(&target, &payload).expect("write test credential");
        let got = read_blob_for_test(&target).expect("read test credential").expect("exists");
        assert_eq!(got, payload);
        delete_for_test(&target).expect("delete test credential");
        assert!(read_blob_for_test(&target).expect("read after delete").is_none());

        let encrypted = protect(b"secret-blob").expect("protect");
        assert_ne!(encrypted, b"secret-blob".to_vec());
        assert_eq!(unprotect(&encrypted).expect("unprotect"), b"secret-blob".to_vec());
    }

    #[cfg(windows)]
    fn write_blob_for_test(target: &str, blob: &[u8]) -> Result<(), String> {
        super::sys::write_blob(target, "probe", blob)
    }

    #[cfg(windows)]
    fn read_blob_for_test(target: &str) -> Result<Option<Vec<u8>>, String> {
        super::sys::read_blob(target)
    }

    #[cfg(windows)]
    fn delete_for_test(target: &str) -> Result<bool, String> {
        super::sys::delete(target)
    }

    #[test]
    fn extract_tokens_parses_nested_and_top_level_and_redacts_debug() {
        let nested = br#"{"token":{"access_token":"  ya29.secret-abc  ","refresh_token":"1//refresh-xyz","expiry":"2026-11-20T11:33:36+08:00"},"id_token_email":"user@example.com"}"#;
        let t1 = extract_tokens(nested);
        assert_eq!(t1.email.as_deref(), Some("user@example.com"));
        assert_eq!(t1.access_token.as_deref(), Some("ya29.secret-abc"));
        assert_eq!(t1.refresh_token.as_deref(), Some("1//refresh-xyz"));
        assert_eq!(t1.expires_at_ms, Some(1795145616000));
        let dbg = format!("{t1:?}");
        assert!(!dbg.contains("ya29.secret-abc"));
        assert!(!dbg.contains("1//refresh-xyz"));
        assert!(dbg.contains("[REDACTED]"));

        let top = br#"{"access_token":"top-access","refresh_token":"","email":"top@example.com"}"#;
        let t2 = extract_tokens(top);
        assert_eq!(t2.email.as_deref(), Some("top@example.com"));
        assert_eq!(t2.access_token.as_deref(), Some("top-access"));
        assert!(t2.refresh_token.is_none());

        let garbage = extract_tokens(b"not-json");
        assert!(garbage.access_token.is_none());
        assert!(garbage.refresh_token.is_none());
    }

    #[test]
    fn discovers_local_token_files_and_selects_freshest_same_email_tokens() {
        let base = std::env::temp_dir().join(format!(
            "agydock-cred-discovery-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&base);
        let app_tok = base
            .join(".gemini")
            .join("antigravity")
            .join("antigravity-oauth-token");
        let t3_tok = base
            .join(".t3")
            .join("userdata")
            .join("providers")
            .join("antigravity")
            .join("profile-a")
            .join("antigravity-acp")
            .join("antigravity-oauth-token");
        std::fs::create_dir_all(app_tok.parent().unwrap()).unwrap();
        std::fs::create_dir_all(t3_tok.parent().unwrap()).unwrap();
        std::fs::write(&app_tok, b"{}").unwrap();
        std::fs::write(&t3_tok, b"{}").unwrap();

        let paths = discover_local_token_file_paths(Some(&base));
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&app_tok));
        assert!(paths.contains(&t3_tok));

        let primary_blob = br#"{"token":{"access_token":"old-acc","refresh_token":"old-ref","expiry":"2026-11-24T10:00:00Z"},"id_token_email":"user@example.com"}"#;
        let other_user_blob = br#"{"token":{"access_token":"other-acc","refresh_token":"other-ref","expiry":"2026-11-26T10:00:00Z"},"id_token_email":"other@example.com"}"#.to_vec();
        let fresher_same_blob = br#"{"token":{"access_token":"fresh-acc","expiry":"2026-11-25T10:00:00Z"},"id_token_email":"USER@example.com"}"#.to_vec();
        let older_same_blob = br#"{"token":{"access_token":"older-acc","refresh_token":"older-ref","expiry":"2026-11-24T09:00:00Z"},"id_token_email":"user@example.com"}"#.to_vec();

        let primary = extract_tokens(primary_blob);
        let selected = select_freshest_tokens(
            &primary,
            &[other_user_blob.clone(), fresher_same_blob.clone(), older_same_blob],
            None,
            1_795_400_000_000,
        );
        assert_eq!(selected.access_token.as_deref(), Some("fresh-acc"));
        assert_eq!(selected.refresh_token.as_deref(), Some("old-ref"));
        assert!(selected.expires_at_ms > primary.expires_at_ms);

        // When primary is already fresher, keep primary
        let selected_keep = select_freshest_tokens(
            &extract_tokens(&fresher_same_blob),
            &[primary_blob.to_vec()],
            None,
            1_795_400_000_000,
        );
        assert_eq!(selected_keep.access_token.as_deref(), Some("fresh-acc"));
        assert_eq!(selected_keep.refresh_token.as_deref(), Some("old-ref"));

        // pick_freshest_any_blob picks unexpired over expired
        let expired_blob = primary_blob.to_vec();
        let unexpired_blob = br#"{"token":{"access_token":"live-acc","expiry":"2099-01-01T00:00:00Z"},"id_token_email":"live@example.com"}"#.to_vec();
        let picked = pick_freshest_any_blob(&[expired_blob, unexpired_blob], 1_800_000_000_000).unwrap();
        assert_eq!(extract_tokens(&picked).email.as_deref(), Some("live@example.com"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
