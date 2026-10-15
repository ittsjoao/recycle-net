//! Turn on object-access auditing for deletes without touching any file:
//! "File System" success auditing + a global (resource) SACL for DELETE,
//! restricted to Domain Users. Uses auditpol.exe / wevtutil.exe (they enable the
//! needed privileges themselves) and LSA only to find the domain SID.
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

/// "Sistema de Arquivos" / "File System" subcategory (language independent).
pub const FILE_SYSTEM_SUBCATEGORY: &str = "{0CCE921D-69AE-11D9-BED3-505054503030}";
/// Enough for ~290k audited operations at ~1.8 KB each (measured).
pub const MIN_SECURITY_LOG_BYTES: u64 = 512 * 1024 * 1024;
/// What existed before rnetrbin changed the audit policy (read by uninstall).
pub const STATE_PATH: &str = r"C:\ProgramData\rnetrbin\audit-state.toml";
/// DELETE access right, as auditpol's hex access mask.
const DELETE_MASK: &str = "0x10000";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditState {
    /// auditpol "Setting Value" before we changed it: 0 none, 1 success, 2 failure, 3 both.
    pub policy_before: u8,
    /// Account the global SACL entry was created for.
    pub sacl_sid: String,
    /// The admin already had a global SACL entry for that account; we leave it alone.
    #[serde(default)]
    pub sacl_existed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AuditStatus {
    /// "File System" success auditing is on right now (a GPO may turn it off).
    pub policy_success: bool,
    /// rnetrbin configured auditing on this machine (state file exists).
    pub configured: bool,
    /// The SACL covers BUILTIN\Users (no domain): every local user, more events.
    pub builtin_users: bool,
}

/// BUILTIN\Users, audited outside a domain.
pub const BUILTIN_USERS_SID: &str = "S-1-5-32-545";

/// The `audit-state.toml` text records a SACL for BUILTIN\Users.
pub fn state_audits_builtin_users(state_toml: &str) -> bool {
    toml::from_str::<AuditState>(state_toml).is_ok_and(|s| s.sacl_sid == BUILTIN_USERS_SID)
}

/// "Setting Value" (last column) of the `auditpol /backup` CSV line for `guid`.
pub fn parse_backup_setting(csv: &str, guid: &str) -> Option<u8> {
    let guid = guid.to_uppercase();
    csv.lines()
        .find(|l| l.to_uppercase().contains(&guid))
        .and_then(|l| l.trim().rsplit(',').next())
        .and_then(|v| v.trim().parse().ok())
}

/// `maxSize:` from `wevtutil gl Security` (keys are not localized).
pub fn parse_max_size(out: &str) -> Option<u64> {
    out.lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("maxSize:"))
        .and_then(|v| v.trim().parse().ok())
}

/// Domain Users of the machine's domain, or BUILTIN\Users outside a domain.
pub fn audit_users_sid(domain_sid: Option<&str>) -> String {
    match domain_sid {
        Some(d) => format!("{d}-513"),
        None => BUILTIN_USERS_SID.into(),
    }
}

pub fn policy_args(setting: u8) -> Vec<String> {
    let flag = |on: bool| if on { "enable" } else { "disable" };
    vec![
        "/set".into(),
        format!("/subcategory:{FILE_SYSTEM_SUBCATEGORY}"),
        format!("/success:{}", flag(setting & 1 != 0)),
        format!("/failure:{}", flag(setting & 2 != 0)),
    ]
}

pub fn sacl_args(sid: &str) -> Vec<String> {
    ["/resourceSACL", "/set", "/type:File", "/success"]
        .iter()
        .map(|s| s.to_string())
        .chain([format!("/user:{{{sid}}}"), format!("/access:{DELETE_MASK}")])
        .collect()
}

pub fn sacl_view_args(sid: &str) -> Vec<String> {
    vec!["/resourceSACL".into(), "/type:File".into(), "/view".into(), format!("/user:{{{sid}}}")]
}

/// A global SACL entry exists: some line's value after the first ':' is "File" (language independent).
pub fn parse_sacl_view_has_entry(out: &str) -> bool {
    out.lines().any(|l| l.split_once(':').is_some_and(|(_, v)| v.trim() == "File"))
}

/// The entry audits DELETE (an access line equal to "DELETE").
pub fn sacl_view_has_delete(out: &str) -> bool {
    out.lines().any(|l| l.trim() == "DELETE")
}

/// Full path under System32 so a planted exe is never run as SYSTEM.
fn tool(name: &str) -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    format!(r"{root}\System32\{name}.exe")
}

fn run(cmd: &str, args: &[String]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let out = Command::new(cmd)
        .args(args)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output().map_err(|e| format!("{cmd}: {e}"))?;
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    if out.status.success() {
        Ok(text(&out.stdout))
    } else {
        Err(format!("{cmd} {}: {} {}", args.join(" "), text(&out.stdout), text(&out.stderr)))
    }
}

/// Current "Setting Value" of the File System subcategory (Err when the backup cannot be parsed).
fn current_setting() -> Result<u8, String> {
    // Not %TEMP%: as SYSTEM a predictable name there could be pre-planted. ProgramData\rnetrbin
    // is ours; the name is unique per process and instant, and any leftover is removed first.
    let dir = Path::new(STATE_PATH).parent().unwrap_or(Path::new(r"C:\ProgramData\rnetrbin"));
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let file = dir.join(format!("auditpol-{}-{nanos}.csv", std::process::id()));
    if file.exists() {
        std::fs::remove_file(&file).map_err(|e| e.to_string())?;
    }
    run(&tool("auditpol"), &["/backup".into(), format!("/file:{}", file.display())])?;
    let bytes = std::fs::read(&file).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&file);
    let bytes = bytes?;
    // auditpol may write UTF-16LE (with BOM) or ANSI depending on the Windows build.
    let csv = if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    if csv.trim().is_empty() {
        return Ok(0);
    }
    parse_backup_setting(&csv, FILE_SYSTEM_SUBCATEGORY).ok_or_else(|| "auditpol backup has no File System line".to_string())
}

/// SID of the machine's primary domain, Ok(None) in a workgroup, Err when LSA fails.
pub fn primary_domain_sid() -> Result<Option<String>, String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Authentication::Identity::{
        LsaClose, LsaFreeMemory, LsaOpenPolicy, LsaQueryInformationPolicy, PolicyPrimaryDomainInformation,
        LSA_HANDLE, LSA_OBJECT_ATTRIBUTES, POLICY_PRIMARY_DOMAIN_INFO, POLICY_VIEW_LOCAL_INFORMATION,
    };
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    unsafe {
        let attrs = LSA_OBJECT_ATTRIBUTES::default();
        let mut policy = LSA_HANDLE::default();
        let st = LsaOpenPolicy(None, &attrs, POLICY_VIEW_LOCAL_INFORMATION as u32, &mut policy);
        if st.is_err() {
            return Err(format!("LsaOpenPolicy: {st:?}"));
        }
        let mut buf: *mut core::ffi::c_void = std::ptr::null_mut();
        let st = LsaQueryInformationPolicy(policy, PolicyPrimaryDomainInformation, &mut buf);
        let _ = LsaClose(policy);
        if st.is_err() || buf.is_null() {
            return Err(format!("LsaQueryInformationPolicy: {st:?}"));
        }
        let info = &*(buf as *const POLICY_PRIMARY_DOMAIN_INFO);
        let mut out = None;
        if !info.Sid.is_invalid() {
            let mut s = PWSTR::null();
            if ConvertSidToStringSidW(info.Sid, &mut s).is_ok() {
                out = s.to_string().ok();
                let _ = LocalFree(HLOCAL(s.0 as _));
            }
        }
        let _ = LsaFreeMemory(Some(buf));
        Ok(out)
    }
}

/// Idempotent. Records the previous policy and global SACL once, then turns on
/// success auditing, sets the global DELETE SACL (unless the admin already has an
/// entry for the account) and grows the Security log.
pub fn ensure() -> Result<(), String> {
    let before = current_setting()?;
    let state = match std::fs::read_to_string(STATE_PATH).ok().and_then(|t| toml::from_str::<AuditState>(&t).ok()) {
        Some(state) => state,
        None => {
            let domain = primary_domain_sid().unwrap_or_else(|e| {
                tracing::warn!("audit: cannot read the domain SID ({e}); falling back to BUILTIN\\Users");
                None
            });
            let sid = audit_users_sid(domain.as_deref());
            if domain.is_none() {
                tracing::warn!("audit: auditing deletes of every local user (BUILTIN\\Users); more events in the log");
            }
            let view = run(&tool("auditpol"), &sacl_view_args(&sid))?;
            let sacl_existed = parse_sacl_view_has_entry(&view);
            if sacl_existed && !sacl_view_has_delete(&view) {
                tracing::warn!("existing global SACL entry for {sid} does not audit DELETE; leaving it untouched");
            }
            let state = AuditState { policy_before: before, sacl_sid: sid, sacl_existed };
            if let Some(parent) = Path::new(STATE_PATH).parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(STATE_PATH, toml::to_string(&state).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            state
        }
    };
    if before & 1 == 0 {
        run(&tool("auditpol"), &policy_args(before | 1))?;
    }
    if !state.sacl_existed {
        run(&tool("auditpol"), &sacl_args(&state.sacl_sid))?;
    }
    match parse_max_size(&run(&tool("wevtutil"), &["gl".into(), "Security".into()])?) {
        Some(size) if size < MIN_SECURITY_LOG_BYTES => {
            run(&tool("wevtutil"), &["sl".into(), "Security".into(), format!("/ms:{MIN_SECURITY_LOG_BYTES}")])?;
        }
        Some(_) => {}
        None => tracing::warn!("could not read the Security log size; not resizing it"),
    }
    Ok(())
}

pub fn status() -> AuditStatus {
    AuditStatus {
        policy_success: current_setting().map(|s| s & 1 != 0).unwrap_or(false),
        configured: Path::new(STATE_PATH).exists(),
        builtin_users: std::fs::read_to_string(STATE_PATH).is_ok_and(|t| state_audits_builtin_users(&t)),
    }
}

/// Undo only what `ensure` changed; the Security log size is kept.
pub fn undo() -> Result<(), String> {
    let Ok(text) = std::fs::read_to_string(STATE_PATH) else { return Ok(()) };
    let state: AuditState = toml::from_str(&text).map_err(|e| e.to_string())?;
    run(&tool("auditpol"), &policy_args(state.policy_before))?;
    if !state.sacl_existed {
        let remove = ["/resourceSACL".into(), "/remove".into(), "/type:File".into(), format!("/user:{{{}}}", state.sacl_sid)];
        run(&tool("auditpol"), &remove)?;
    }
    std::fs::remove_file(STATE_PATH).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BACKUP: &str = "Machine Name,Policy Target,Subcategory,Subcategory GUID,Inclusion Setting,Exclusion Setting,Setting Value\r\n\
SRV,System,Sistema de Arquivos,{0CCE921D-69AE-11D9-BED3-505054503030},Sem Auditoria,,0\r\n\
SRV,System,Logon,{0CCE9215-69AE-11D9-BED3-505054503030},Êxito,,1\r\n";

    #[test]
    fn status_from_backup_csv() {
        assert_eq!(parse_backup_setting(BACKUP, FILE_SYSTEM_SUBCATEGORY), Some(0));
        assert_eq!(parse_backup_setting(&BACKUP.replace(",,0", ",,3"), FILE_SYSTEM_SUBCATEGORY), Some(3));
        assert_eq!(parse_backup_setting("Machine Name\r\n", FILE_SYSTEM_SUBCATEGORY), None);
    }

    #[test]
    fn parses_wevtutil_max_size() {
        let out = "name: Security\r\nenabled: true\r\nlogging:\r\n  logFileName: %SystemRoot%\\x.evtx\r\n  retention: false\r\n  autoBackup: false\r\n  maxSize: 20971520\r\n";
        assert_eq!(parse_max_size(out), Some(20971520));
        assert_eq!(parse_max_size("name: Security\r\n"), None);
    }

    #[test]
    fn audit_users_are_domain_users_or_builtin_users() {
        assert_eq!(audit_users_sid(Some("S-1-5-21-1-2-3")), "S-1-5-21-1-2-3-513");
        assert_eq!(audit_users_sid(None), "S-1-5-32-545");
    }

    const SACL_ENTRY: &str = "Entrada:            1
Tipo de Recurso:    File
Usuário:             BUILTIN\\Usuários
Sinalizadores:            Êxito
Condição:        (null)
Acessos:
    DELETE

Comando executado com êxito.
";
    const SACL_NONE: &str = "No momento, não há nenhum SACL global para o tipo de recurso.
Comando executado com êxito.
";

    #[test]
    fn reads_sacl_view() {
        assert!(parse_sacl_view_has_entry(SACL_ENTRY));
        assert!(!parse_sacl_view_has_entry(SACL_NONE));
        assert!(sacl_view_has_delete(SACL_ENTRY));
        assert!(!sacl_view_has_delete(&SACL_ENTRY.replace("DELETE", "WRITE_DAC")));
        assert!(!sacl_view_has_delete(SACL_NONE));
    }

    #[test]
    fn state_tells_builtin_users_auditing() {
        assert!(state_audits_builtin_users("policy_before = 0\nsacl_sid = \"S-1-5-32-545\"\nsacl_existed = false\n"));
        assert!(!state_audits_builtin_users("policy_before = 0\nsacl_sid = \"S-1-5-21-1-2-3-513\"\n"));
        assert!(!state_audits_builtin_users("garbage"));
    }

    #[test]
    fn command_arguments() {
        assert_eq!(
            policy_args(1),
            ["/set", "/subcategory:{0CCE921D-69AE-11D9-BED3-505054503030}", "/success:enable", "/failure:disable"]
        );
        assert_eq!(policy_args(2)[2..], ["/success:disable", "/failure:enable"]);
        assert_eq!(
            sacl_args("S-1-5-21-1-513"),
            ["/resourceSACL", "/set", "/type:File", "/success", "/user:{S-1-5-21-1-513}", "/access:0x10000"]
        );
    }
}
