//! Optional Windows firewall setup. Only a user-clicked helper is elevated.
//! The streaming process remains unelevated; tests never invoke policy mutation.
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::os::windows::process::CommandExt;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
};
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
const HELPER: &str = "--configure-firewall";
const CHECK: &str = "--firewall-status";
const POLICY_BLOCKED: u8 = 2;
const WAIT_TIMEOUT_MS: u32 = 45_000;
const WAIT_TIMEOUT_CODE: u32 = 0x0000_0102;
const SCRIPT: &str = include_str!("firewall.ps1");
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Status {
    configured: bool,
    app_udp_blocks: usize,
    restricted_profiles: usize,
    rule: String,
}
#[derive(Serialize, Deserialize)]
struct Preference {
    executable: PathBuf,
    dismissed: bool,
}
pub(crate) fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if rest.len() >= 4 && rest[..4].eq_ignore_ascii_case(r"UNC\") {
            PathBuf::from(format!(r"\\{}", &rest[4..]))
        } else {
            PathBuf::from(rest)
        }
    } else {
        path.to_path_buf()
    }
}
fn diagnostic_path(program: &Path) -> Option<PathBuf> {
    program
        .parent()
        .map(|home| home.join("data/firewall/last-helper-error.txt"))
}
fn write_helper_diagnostic(program: &Path, message: &str) {
    if let Some(path) = diagnostic_path(program) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let truncated: String = message.chars().take(4000).collect();
        let _ = std::fs::write(path, truncated);
    }
}
fn clear_helper_diagnostic(program: &Path) {
    if let Some(path) = diagnostic_path(program) {
        let _ = std::fs::remove_file(path);
    }
}
fn take_helper_diagnostic(program: &Path) -> Option<String> {
    let path = diagnostic_path(program)?;
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(2000).collect())
    }
}
fn script(program: &Path, apply: bool) -> Result<String> {
    let normalized = strip_verbatim_prefix(program);
    let program = normalized
        .to_str()
        .context("Firewall setup requires a Unicode executable path")?;
    anyhow::ensure!(!program.contains('\0'), "Invalid executable path");
    let encoded_program = STANDARD.encode(program.as_bytes());
    Ok(format!(
        "Import-Module -Name (Join-Path $PSHOME 'Modules/NetSecurity/NetSecurity.psd1') -ErrorAction Stop\n{SCRIPT}\n$p2pProgram=[System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String('{encoded_program}'))\nInvoke-P2PFirewall -Program $p2pProgram -Apply ${apply}"
    ))
}
fn encoded(script: &str) -> String {
    STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    )
}
fn powershell() -> Result<PathBuf> {
    // Resolve the OS-owned executable through Win32, not PATH or an editable config.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    }
    let mut buffer = [0u16; 32768];
    let len = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    anyhow::ensure!(
        len > 0 && len < buffer.len(),
        "Cannot locate Windows PowerShell"
    );
    use std::os::windows::ffi::OsStringExt;
    Ok(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len]))
        .join("WindowsPowerShell/v1.0/powershell.exe"))
}
fn invoke(program: &Path, apply: bool) -> Result<Status> {
    let program = strip_verbatim_prefix(program);
    let home = program.parent().context("Missing application directory")?;
    let cache = home.join("data/firewall");
    std::fs::create_dir_all(&cache)?;
    let shell = powershell()?;
    let modules = shell
        .parent()
        .context("Missing Windows PowerShell directory")?
        .join("Modules");
    let mut child = Command::new(&shell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
        ])
        .arg(encoded(&script(&program, apply)?))
        .current_dir(home)
        .creation_flags(0x08000000)
        .env("TEMP", &cache)
        .env("TMP", &cache)
        .env("PSModuleAnalysisCachePath", cache.join("module-cache"))
        .env("PSModulePath", modules)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Could not start Windows firewall setup")?;
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = std::io::Read::read_to_end(&mut pipe, &mut buf);
        }
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = std::io::Read::read_to_end(&mut pipe, &mut buf);
        }
        buf
    });
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_millis(u64::from(WAIT_TIMEOUT_MS));
    let exit_status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_thread.join();
            let _ = err_thread.join();
            bail!("Windows firewall command timed out after 45 seconds");
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    };
    let stdout = out_thread.join().unwrap_or_default();
    let stderr = err_thread.join().unwrap_or_default();
    anyhow::ensure!(
        exit_status.success(),
        "Windows firewall command failed: {}",
        String::from_utf8_lossy(&stderr)
            .chars()
            .take(1200)
            .collect::<String>()
    );
    let text = String::from_utf8_lossy(&stdout);
    let line = text
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with('{'))
        .context("Windows returned no firewall status")?;
    serde_json::from_str(line).context("Invalid Windows firewall status")
}
/// Called before log/window/session initialization, only for the fixed helper flag.
pub fn command_mode() -> Option<std::process::ExitCode> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 2 || (args[1] != HELPER && args[1] != CHECK) {
        return None;
    }
    let applying = args[1] == HELPER;
    let exe = std::env::current_exe().map(|p| strip_verbatim_prefix(&p));
    let result = exe
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error}"))
        .and_then(|program| invoke(program, applying));
    Some(match result {
        Ok(status) if !applying => {
            println!("{}", serde_json::to_string(&status).unwrap());
            std::process::ExitCode::SUCCESS
        }
        Ok(status) if status.configured => {
            if let Ok(program) = &exe {
                clear_helper_diagnostic(program);
            }
            std::process::ExitCode::SUCCESS
        }
        Ok(status) => {
            let message = format!(
                "Firewall policy still blocks UDP (app_udp_blocks={}, restricted_profiles={})",
                status.app_udp_blocks, status.restricted_profiles
            );
            if let Ok(program) = &exe {
                write_helper_diagnostic(program, &message);
            }
            eprintln!("Firewall policy still blocks UDP: {status:?}");
            std::process::ExitCode::from(POLICY_BLOCKED)
        }
        Err(error) => {
            if applying && let Ok(program) = &exe {
                write_helper_diagnostic(program, &format!("{error:#}"));
            }
            eprintln!("Firewall setup failed: {error:#}");
            std::process::ExitCode::FAILURE
        }
    })
}
// Small shell32 ABI avoids rebuilding the large Windows Shell bindings while RAM is limited.
#[repr(C)]
#[derive(Default)]
struct ExecuteInfo {
    size: u32,
    mask: u32,
    hwnd: *mut std::ffi::c_void,
    verb: *const u16,
    file: *const u16,
    parameters: *const u16,
    directory: *const u16,
    show: i32,
    instance: *mut std::ffi::c_void,
    id_list: *mut std::ffi::c_void,
    class: *const u16,
    class_key: *mut std::ffi::c_void,
    hot_key: u32,
    icon: *mut std::ffi::c_void,
    process: *mut std::ffi::c_void,
}
fn elevate(program: &Path, hwnd: usize) -> Result<u32> {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteExW(info: *mut ExecuteInfo) -> i32;
    }
    use std::os::windows::ffi::OsStrExt;
    let program = strip_verbatim_prefix(program);
    let file: Vec<u16> = program.as_os_str().encode_wide().chain(Some(0)).collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain(Some(0)).collect();
    let parameters: Vec<u16> = HELPER.encode_utf16().chain(Some(0)).collect();
    let mut info = ExecuteInfo {
        size: std::mem::size_of::<ExecuteInfo>() as u32,
        mask: 0x40 | 0x100 | 0x400,
        hwnd: hwnd as *mut _,
        verb: verb.as_ptr(),
        file: file.as_ptr(),
        parameters: parameters.as_ptr(),
        show: 0,
        ..Default::default()
    };
    unsafe {
        use windows::Win32::System::Com::{
            COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
        };
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _apartment = Apartment;
        if ShellExecuteExW(&mut info) == 0 {
            let error = GetLastError();
            if error.0 == 1223 {
                bail!("Windows permission was cancelled. You can retry or choose Not now.");
            }
            bail!(
                "Windows could not request administrator permission: {}",
                error.0
            );
        }
        anyhow::ensure!(
            !info.process.is_null(),
            "Windows returned no firewall helper process"
        );
        let handle = HANDLE(info.process);
        struct Owned(HANDLE);
        impl Drop for Owned {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
        let _handle = Owned(handle);
        let wait = WaitForSingleObject(handle, WAIT_TIMEOUT_MS);
        if wait.0 == WAIT_TIMEOUT_CODE {
            bail!("Windows firewall setup timed out after 45 seconds");
        }
        anyhow::ensure!(wait.0 == 0, "Waiting for firewall setup failed");
        let mut code = 1;
        GetExitCodeProcess(handle, &mut code)?;
        Ok(code)
    }
}

fn apply_and_check(program: &Path, hwnd: usize) -> Result<Status> {
    let program = strip_verbatim_prefix(program);
    clear_helper_diagnostic(&program);
    let code = elevate(&program, hwnd)?;
    // A successful elevated result already verified ActiveStore. An unelevated
    // process may lack permission to read that store on some Windows setups.
    if code == 0 {
        clear_helper_diagnostic(&program);
        return Ok(Status {
            configured: true,
            ..Status::default()
        });
    }
    let diagnostic = take_helper_diagnostic(&program);
    let reason = if code == u32::from(POLICY_BLOCKED) {
        "Windows did not confirm the firewall rule.".to_owned()
    } else {
        format!("The Windows firewall helper failed (exit code {code}).")
    };
    let detail = diagnostic
        .as_deref()
        .map(|text| format!(" Helper details: {text}"))
        .unwrap_or_default();
    match invoke(&program, false) {
        Ok(status) if status.configured => Ok(status),
        Ok(status) => bail!(
            "{reason}{detail} Active inbound UDP blocks for this app: {}. Network profiles that reject inbound allow rules: {}.",
            status.app_udp_blocks,
            status.restricted_profiles
        ),
        Err(error) => bail!("{reason}{detail} Firewall status check failed: {error:#}"),
    }
}

fn is_access_denied_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("access is denied")
        || lower.contains("access denied")
        || lower.contains("permissiondenied")
        || lower.contains("unauthorizedaccess")
        || lower.contains("0x80070005")
}

pub(super) struct Firewall {
    program: PathBuf,
    preference: PathBuf,
    receiver: Option<mpsc::Receiver<Result<Status, String>>>,
    status: Option<Status>,
    error: Option<String>,
    open: bool,
    automatic: bool,
    checking_automatic: bool,
    busy: bool,
    applying: bool,
}
impl Firewall {
    pub fn new(program: PathBuf, enabled: bool) -> Self {
        let program = strip_verbatim_prefix(&program);
        let preference = program
            .parent()
            .unwrap_or(Path::new("."))
            .join("data/firewall-preference.toml");
        let dismissed = std::fs::read_to_string(&preference)
            .ok()
            .and_then(|text| toml::from_str::<Preference>(&text).ok())
            .is_some_and(|p| strip_verbatim_prefix(&p.executable) == program && p.dismissed);
        let mut this = Self {
            program,
            preference,
            receiver: None,
            status: None,
            error: None,
            open: false,
            automatic: enabled && !dismissed,
            checking_automatic: false,
            busy: false,
            applying: false,
        };
        if this.automatic {
            this.checking_automatic = true;
            this.check();
        }
        this
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
    fn check(&mut self) {
        let program = self.program.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.busy = true;
        std::thread::spawn(move || {
            let _ = tx.send(invoke(&program, false).map_err(|e| format!("{e:#}")));
        });
    }
    pub fn request(&mut self) {
        self.open = true;
        self.automatic = false;
        self.checking_automatic = false;
        if !self.busy {
            self.check();
        }
    }
    fn stop_startup_prompt(&mut self) {
        self.automatic = false;
        let result = (|| -> Result<()> {
            std::fs::create_dir_all(self.preference.parent().unwrap())?;
            std::fs::write(
                &self.preference,
                toml::to_string(&Preference {
                    executable: self.program.clone(),
                    dismissed: true,
                })?,
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(%error,"firewall_preference_save_failed");
        }
    }
    fn allow(&mut self, hwnd: usize) {
        self.automatic = false;
        self.checking_automatic = false;
        let program = self.program.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.busy = true;
        self.applying = true;
        self.error = None;
        tracing::info!(program=%self.program.display(),"firewall_permission_requested");
        std::thread::spawn(move || {
            let result = apply_and_check(&program, hwnd);
            let _ = tx.send(result.map_err(|e| format!("{e:#}")));
        });
    }
    fn poll(&mut self) {
        let result = self.receiver.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(v) => Some(v),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Firewall worker stopped unexpectedly".into()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(result) = result {
            self.receiver = None;
            self.busy = false;
            let applying = std::mem::take(&mut self.applying);
            let automatic_check = std::mem::take(&mut self.checking_automatic);
            match result {
                Ok(status) => {
                    tracing::info!(program=%self.program.display(),configured=status.configured,udp_blocks=status.app_udp_blocks,restricted_profiles=status.restricted_profiles,rule=%status.rule,"firewall_status");
                    self.error = None;
                    if applying && status.configured {
                        self.open = false;
                        self.stop_startup_prompt();
                    }
                    self.status = Some(status);
                }
                Err(error) => {
                    tracing::warn!(%error,"firewall_setup_failed");
                    if automatic_check && !self.open && is_access_denied_error(&error) {
                        // Do not pop a red startup error modal when an unelevated
                        // policy query is denied by local Windows permissions.
                        self.automatic = false;
                        self.error = None;
                    } else {
                        self.error = Some(error);
                    }
                }
            }
        }
    }
    fn copy_details(&self, ctx: &egui::Context) {
        let log = self.preference.parent().and_then(crate::diagnostics::current_log)
            .unwrap_or_else(|| self.preference.with_file_name("P2PShare.log"));
        let log_state = if !p2pshare_media::diagnostics::enabled() {
            "disabled"
        } else {
            match log.try_exists() {
                Ok(true) => "present",
                Ok(false) => "missing",
                Err(_) => "inaccessible",
            }
        };
        let status = if self.busy {
            "checking"
        } else if let Some(error) = &self.error {
            error
        } else if self.status.as_ref().is_some_and(|status| status.configured) {
            "verified"
        } else {
            "rule not verified"
        };
        ctx.copy_text(format!(
            "P2PShare {}\nExecutable: {}\nLog ({}): {}\nWindows Firewall: {}",
            env!("CARGO_PKG_VERSION"),
            self.program.display(),
            log_state,
            log.display(),
            status,
        ));
    }
    pub fn show(&mut self, ctx: &egui::Context, hwnd: usize, eligible: bool) {
        self.poll();
        if self.automatic && !self.busy && eligible {
            self.automatic = false;
            self.open = self.status.as_ref().is_none_or(|s| !s.configured);
        }
        if !self.open {
            return;
        }
        let configured = self.status.as_ref().is_some_and(|s| s.configured) && self.error.is_none();
        let mut allow = false;
        let mut dismiss = false;
        egui::Modal::new(egui::Id::new("windows-firewall-setup")).show(ctx, |ui| {
            ui.set_width(450.);
            ui.heading("Windows Firewall");
            ui.add_space(8.);
            if self.applying {
                ui.label("Waiting for Windows Firewall setup…");
            } else if self.busy {
                ui.label("Checking Windows Firewall…");
            } else if configured {
                ui.label("P2PShare is allowed to receive peer media on Domain, Private and Public networks.");
            } else {
                ui.label("Allow incoming peer connections so friends can send video, audio and previews to this app.");
                ui.add_space(8.);
                ui.label("Windows will ask for administrator permission. Setup allows UDP for this EXE on Domain, Private and Public networks and removes conflicting local inbound UDP blocks for it.");
            }
            ui.add_space(8.);
            let path=self.program.display().to_string();
            ui.add(egui::Label::new(egui::RichText::new(&path).small()).truncate()).on_hover_text(path);
            if let Some(error)=&self.error {
                ui.add_space(8.);
                let summary:String=error.lines().find(|line|!line.trim().is_empty()).unwrap_or(error).chars().take(180).collect();
                ui.colored_label(ui.visuals().error_fg_color, summary);
                ui.collapsing("Details",|ui| {
                    egui::ScrollArea::vertical().max_height(120.).show(ui,|ui| {ui.label(error);});
                });
            }
            ui.add_space(12.);
            ui.horizontal(|ui| {
                if !self.busy && !configured {allow=ui.button("Allow through firewall").clicked();}
                if ui.button("Copy details").clicked() {
                    self.copy_details(ctx);
                }
                dismiss=ui.button(if configured {"Done"} else if self.busy {"Hide"} else {"Not now"}).clicked();
            });
        });
        if allow {
            self.allow(hwnd);
        }
        if dismiss {
            self.open = false;
            if !self.busy {
                self.stop_startup_prompt();
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_abi_and_utf16_command_preserve_hostile_literal_paths() {
        assert_eq!(std::mem::size_of::<ExecuteInfo>(), 112);
        assert_eq!(std::mem::offset_of!(ExecuteInfo, process), 104);
        let path = Path::new("\\\\?\\F:\\friends'\u{2019} [$x]; `echo bad`\\P2PShare.exe");
        let command = script(path, false).unwrap();
        let expected_path = "F:\\friends'\u{2019} [$x]; `echo bad`\\P2PShare.exe";
        let expected_b64 = STANDARD.encode(expected_path.as_bytes());
        assert!(command.contains(&format!("FromBase64String('{expected_b64}')")));
        assert!(!command.contains("`echo bad`"));
        assert!(command.ends_with("-Apply $false"));
        let bytes = STANDARD.decode(encoded(&command)).unwrap();
        let utf16: Vec<_> = bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(String::from_utf16(&utf16).unwrap(), command);
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\UNC\server\share\P2PShare.exe")),
            PathBuf::from(r"\\server\share\P2PShare.exe")
        );
    }
    #[test]
    fn dismissed_choice_is_scoped_to_executable_location() {
        let pref = Preference {
            executable: PathBuf::from("F:/P2PShare.exe"),
            dismissed: true,
        };
        let loaded: Preference = toml::from_str(&toml::to_string(&pref).unwrap()).unwrap();
        assert_eq!(loaded.executable, pref.executable);
        assert_ne!(loaded.executable, PathBuf::from("D:/P2PShare.exe"));
    }
    #[test]
    fn setup_attempt_disables_future_startup_prompts() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.build/firewall-tests")
            .join(std::process::id().to_string());
        let program = root.join("P2PShare.exe");
        let mut dialog = Firewall::new(program.clone(), false);
        dialog.automatic = true;
        dialog.stop_startup_prompt();
        assert!(!dialog.automatic);
        let (tx, rx) = mpsc::channel();
        dialog.open = true;
        dialog.applying = true;
        dialog.receiver = Some(rx);
        tx.send(Ok(Status {
            configured: true,
            ..Status::default()
        }))
        .unwrap();
        dialog.poll();
        assert!(!dialog.open);

        let restarted = Firewall::new(program, true);
        assert!(!restarted.automatic);
        assert!(restarted.receiver.is_none());
        std::fs::remove_file(restarted.preference).unwrap();
    }
    #[test]
    fn firewall_dialog_fits_headlessly_without_starting_policy_or_elevation() {
        let mut dialog = Firewall {
            program: PathBuf::from("F:/P2PAPP/P2PShare.exe"),
            preference: PathBuf::new(),
            receiver: None,
            status: None,
            error: None,
            open: true,
            automatic: false,
            checking_automatic: false,
            busy: false,
            applying: false,
        };
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900., 652.),
                )),
                ..Default::default()
            };
            let output = ctx.run_ui(input, |ui| dialog.show(ui.ctx(), 0, true));
            assert!(!output.shapes.is_empty());
        }
        dialog.error = Some("Windows policy query failed. ".repeat(100));
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900., 652.),
                )),
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| dialog.show(ui.ctx(), 0, true));
        }
        let rect = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("windows-firewall-setup")))
            .unwrap();
        assert!(
            rect.width() <= 900. && rect.height() <= 652.,
            "Firewall dialog overflowed: {rect:?}"
        );
        assert!(dialog.open && dialog.receiver.is_none());
        assert!(
            !dialog.busy,
            "Rendering must not launch a helper or policy mutation"
        );
    }
}
