use std::{
    env,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub schema_version: u8,
    pub observed_at_unix_ms: u64,
    pub host: HostReport,
    pub runtime: RuntimeReport,
    pub host_capabilities: HostCapabilities,
    pub feature_readiness: Vec<FeatureReadiness>,
    pub errors: Vec<DoctorError>,
}

#[derive(Debug, Serialize)]
pub struct HostReport {
    pub os: &'static str,
    pub architecture: &'static str,
    pub os_version: Option<String>,
    pub distribution: Option<String>,
    pub kernel_version: Option<String>,
    pub environment: &'static str,
    pub session_type: &'static str,
    pub xwayland_available: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeReport {
    pub steam: RuntimeSignal,
    pub proton: RuntimeSignal,
    pub wine: RuntimeSignal,
}

#[derive(Debug, Serialize)]
pub struct RuntimeSignal {
    pub status: &'static str,
    pub detected: bool,
    pub detected_by: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct HostCapabilities {
    pub process_memory_read: Capability,
    pub clipboard: Capability,
    pub global_hotkeys: Capability,
    pub wayland_global_shortcuts_portal: Capability,
}

#[derive(Debug, Serialize)]
pub struct Capability {
    pub detected: &'static str,
    pub accessible: &'static str,
    pub implementation_status: &'static str,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct FeatureReadiness {
    pub feature: &'static str,
    pub detected: &'static str,
    pub accessible: &'static str,
    pub implementation_status: &'static str,
    pub readiness: &'static str,
    pub failure_reasons: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct DoctorError {
    pub field: &'static str,
    pub code: &'static str,
    pub message: String,
}

pub fn inspect() -> DoctorReport {
    let mut errors = Vec::new();
    let (os, os_version, distribution, kernel_version) = host_version(&mut errors);
    let platform = current_platform();
    let environment = classify_environment(platform, kernel_version.as_deref());
    let session = session_detection(platform);
    let runtime = RuntimeReport {
        steam: runtime_signal(&[
            "SteamAppId",
            "SteamGameId",
            "SteamOverlayGameId",
            "SteamPath",
            "STEAM_COMPAT_CLIENT_INSTALL_PATH",
            "STEAM_RUNTIME",
        ]),
        proton: runtime_signal(&[
            "STEAM_COMPAT_DATA_PATH",
            "STEAM_COMPAT_TOOL_PATHS",
            "PROTON_VERSION",
            "PROTONPATH",
        ]),
        wine: runtime_signal(&[
            "WINEPREFIX",
            "WINELOADER",
            "WINELOADERNOEXEC",
            "WINEDLLOVERRIDES",
            "WINEDEBUG",
        ]),
    };
    let host_capabilities =
        host_capabilities(platform, session.session_type, session.xwayland_available);
    let feature_readiness = feature_readiness(platform, &host_capabilities);

    DoctorReport {
        schema_version: 1,
        observed_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64,
        host: HostReport {
            os,
            architecture: env::consts::ARCH,
            os_version,
            distribution,
            kernel_version,
            environment,
            session_type: session.session_type,
            xwayland_available: session.xwayland_available,
        },
        runtime,
        host_capabilities,
        feature_readiness,
        errors,
    }
}

impl DoctorReport {
    pub fn to_human_text(&self) -> String {
        let mut output = format!(
            "DASRU Doctor\nHost\n  OS: {}\n  Architecture: {}\n  OS version: {}\n  Distribution: {}\n  Kernel: {}\n  Environment: {}\n  Session: {}\n  XWayland available: {}\nRuntime\n",
            self.host.os,
            self.host.architecture,
            display_option(&self.host.os_version),
            display_option(&self.host.distribution),
            display_option(&self.host.kernel_version),
            self.host.environment,
            self.host.session_type,
            display_bool_option(self.host.xwayland_available),
        );
        for (name, signal) in [
            ("Steam", &self.runtime.steam),
            ("Proton", &self.runtime.proton),
            ("Wine", &self.runtime.wine),
        ] {
            let sources = if signal.detected {
                format!(" ({})", signal.detected_by.join(", "))
            } else {
                String::new()
            };
            output.push_str(&format!("  {name}: {}{sources}\n", yes_no(signal.detected)));
        }
        output.push_str("Host capabilities\n");
        for (name, capability) in [
            (
                "Process memory read",
                &self.host_capabilities.process_memory_read,
            ),
            ("Clipboard", &self.host_capabilities.clipboard),
            ("Global hotkeys", &self.host_capabilities.global_hotkeys),
            (
                "Wayland GlobalShortcuts portal",
                &self.host_capabilities.wayland_global_shortcuts_portal,
            ),
        ] {
            output.push_str(&format!(
                "  {name}: detected={}, accessible={}, implementation={} ({})\n",
                capability.detected,
                capability.accessible,
                capability.implementation_status,
                capability.reason,
            ));
        }
        output.push_str("Feature readiness\n");
        for feature in &self.feature_readiness {
            output.push_str(&format!(
                "  {}: {} (detected: {}, accessible: {}, implementation: {})\n",
                feature.feature,
                feature.readiness,
                feature.detected,
                feature.accessible,
                feature.implementation_status,
            ));
            for reason in &feature.failure_reasons {
                output.push_str(&format!("    Reason: {reason}\n"));
            }
        }
        for error in &self.errors {
            output.push_str(&format!(
                "Error [{}] {}: {}\n",
                error.field, error.code, error.message
            ));
        }
        output
    }
}

fn display_option(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("NOT_DETECTED")
}

fn display_bool_option(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "YES",
        Some(false) => "NO",
        None => "NOT_APPLICABLE",
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "YES" } else { "NO" }
}

fn runtime_signal(variables: &[&'static str]) -> RuntimeSignal {
    runtime_signal_with(variables, |name| env::var_os(name).is_some())
}

fn runtime_signal_with(
    variables: &[&'static str],
    mut is_present: impl FnMut(&str) -> bool,
) -> RuntimeSignal {
    let detected_by = variables
        .iter()
        .copied()
        .filter(|name| is_present(name))
        .collect::<Vec<_>>();
    RuntimeSignal {
        status: if detected_by.is_empty() {
            "NOT_DETECTED"
        } else {
            "DETECTED"
        },
        detected: !detected_by.is_empty(),
        detected_by,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostPlatform {
    Windows,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Linux,
    #[allow(dead_code)]
    Other,
}

fn current_platform() -> HostPlatform {
    #[cfg(windows)]
    {
        HostPlatform::Windows
    }
    #[cfg(target_os = "linux")]
    {
        HostPlatform::Linux
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        HostPlatform::Other
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SessionDetection {
    session_type: &'static str,
    xwayland_available: Option<bool>,
}

fn session_detection(platform: HostPlatform) -> SessionDetection {
    let session = env::var("XDG_SESSION_TYPE").unwrap_or_default();
    classify_session(
        platform,
        (!session.is_empty()).then_some(session.as_str()),
        env::var_os("DISPLAY").is_some(),
        env::var_os("WAYLAND_DISPLAY").is_some(),
    )
}

fn classify_session(
    platform: HostPlatform,
    xdg_session_type: Option<&str>,
    has_display: bool,
    has_wayland: bool,
) -> SessionDetection {
    let (session_type, is_wayland) = match platform {
        HostPlatform::Windows => ("WINDOWS", false),
        HostPlatform::Linux => {
            let session = xdg_session_type.unwrap_or_default().to_ascii_lowercase();
            match session.as_str() {
                "wayland" => ("WAYLAND", true),
                "x11" => ("X11", false),
                "" if has_wayland => ("WAYLAND", true),
                "" if has_display => ("X11", false),
                "" => ("HEADLESS", false),
                _ => ("UNKNOWN", false),
            }
        }
        HostPlatform::Other => ("UNKNOWN", false),
    };
    SessionDetection {
        session_type,
        xwayland_available: if platform == HostPlatform::Linux {
            Some(is_wayland && has_display)
        } else {
            None
        },
    }
}

fn classify_environment(platform: HostPlatform, kernel_version: Option<&str>) -> &'static str {
    match platform {
        HostPlatform::Windows => "NATIVE",
        HostPlatform::Linux => match kernel_version {
            Some(version)
                if version
                    .to_ascii_lowercase()
                    .contains("microsoft-standard-wsl2") =>
            {
                "WSL2"
            }
            Some(_) => "NATIVE",
            None => "UNKNOWN",
        },
        HostPlatform::Other => "UNKNOWN",
    }
}

fn host_capabilities(
    platform: HostPlatform,
    session: &'static str,
    xwayland_available: Option<bool>,
) -> HostCapabilities {
    HostCapabilities {
        process_memory_read: process_memory_capability(platform),
        clipboard: clipboard_capability(platform, session, xwayland_available),
        global_hotkeys: hotkey_capability(platform, session),
        wayland_global_shortcuts_portal: portal_capability(platform, session),
    }
}

fn process_memory_capability(platform: HostPlatform) -> Capability {
    match platform {
        HostPlatform::Windows => Capability {
            detected: "DETECTED",
            accessible: "UNKNOWN",
            implementation_status: "IMPLEMENTED",
            reason: "Windows process-memory APIs are used by DASRU, but access to a target process was not tested.".to_owned(),
        },
        HostPlatform::Linux => Capability {
            detected: "UNKNOWN",
            accessible: "UNKNOWN",
            implementation_status: "NOT_IMPLEMENTED",
            reason: "DASRU has no Linux process-memory backend; host ptrace access was not tested.".to_owned(),
        },
        HostPlatform::Other => Capability {
            detected: "UNKNOWN",
            accessible: "UNKNOWN",
            implementation_status: "NOT_IMPLEMENTED",
            reason: "DASRU has no process-memory backend for this platform.".to_owned(),
        },
    }
}

fn clipboard_capability(
    platform: HostPlatform,
    session: &'static str,
    xwayland_available: Option<bool>,
) -> Capability {
    clipboard_capability_from_detection(platform, clipboard_utility(session, xwayland_available))
}

fn clipboard_capability_from_detection(
    platform: HostPlatform,
    utility: UtilityDetection,
) -> Capability {
    match platform {
        HostPlatform::Windows => Capability {
            detected: "DETECTED",
            accessible: "UNKNOWN",
            implementation_status: "NOT_IMPLEMENTED",
            reason: "Windows has a clipboard API, but doctor did not open it and DASRU has no clipboard feature.".to_owned(),
        },
        HostPlatform::Linux => match utility {
            UtilityDetection::Found(utility) => Capability {
                detected: "DETECTED",
                accessible: "UNKNOWN",
                implementation_status: "NOT_IMPLEMENTED",
                reason: format!("Found {utility} on PATH; doctor did not test clipboard access and DASRU has no clipboard feature."),
            },
            UtilityDetection::Missing => Capability {
                detected: "NOT_DETECTED",
                accessible: "UNKNOWN",
                implementation_status: "NOT_IMPLEMENTED",
                reason: "No session-appropriate clipboard utility was found on PATH.".to_owned(),
            },
            UtilityDetection::Unknown => Capability {
                detected: "UNKNOWN",
                accessible: "UNKNOWN",
                implementation_status: "NOT_IMPLEMENTED",
                reason: "Clipboard utilities could not be checked because PATH or file permissions were unavailable.".to_owned(),
            },
        },
        HostPlatform::Other => Capability {
            detected: "UNKNOWN",
            accessible: "UNKNOWN",
            implementation_status: "NOT_IMPLEMENTED",
            reason: "DASRU has no clipboard integration for this platform.".to_owned(),
        },
    }
}

fn hotkey_capability(platform: HostPlatform, session: &'static str) -> Capability {
    match (platform, session) {
        (HostPlatform::Windows, _) => Capability {
            detected: "DETECTED",
            accessible: "UNKNOWN",
            implementation_status: "NOT_IMPLEMENTED",
            reason: "Windows supports global-hotkey registration, but doctor registered no hotkeys and DASRU has no hotkey feature.".to_owned(),
        },
        (HostPlatform::Linux, "X11") => Capability {
            detected: "DETECTED",
            accessible: "UNKNOWN",
            implementation_status: "NOT_IMPLEMENTED",
            reason: "An X11 session is present; permissions and hotkey registration were not tested.".to_owned(),
        },
        (HostPlatform::Linux, "WAYLAND") => Capability {
            detected: "UNKNOWN",
            accessible: "UNKNOWN",
            implementation_status: "NOT_IMPLEMENTED",
            reason: "Global hotkey support depends on compositor or portal policy; no access check was performed.".to_owned(),
        },
        (HostPlatform::Linux, _) => Capability {
            detected: "NOT_DETECTED",
            accessible: "UNKNOWN",
            implementation_status: "NOT_IMPLEMENTED",
            reason: "No desktop session for global hotkeys was detected.".to_owned(),
        },
        (HostPlatform::Other, _) => Capability {
            detected: "UNKNOWN",
            accessible: "UNKNOWN",
            implementation_status: "NOT_IMPLEMENTED",
            reason: "DASRU has no global-hotkey integration for this platform.".to_owned(),
        },
    }
}

fn portal_capability(platform: HostPlatform, session: &'static str) -> Capability {
    match (platform, session) {
        (HostPlatform::Linux, "WAYLAND") => Capability {
            detected: "UNKNOWN",
            accessible: "UNKNOWN",
            implementation_status: "NOT_IMPLEMENTED",
            reason: "The GlobalShortcuts portal service was not queried; Wayland session detection does not establish portal availability.".to_owned(),
        },
        _ => Capability {
            detected: "NOT_APPLICABLE",
            accessible: "NOT_APPLICABLE",
            implementation_status: "NOT_IMPLEMENTED",
            reason: "No Wayland GlobalShortcuts portal check applies to this session.".to_owned(),
        },
    }
}

#[derive(Clone, Copy)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum UtilityDetection {
    Found(&'static str),
    Missing,
    Unknown,
}

fn clipboard_utility(session: &'static str, xwayland_available: Option<bool>) -> UtilityDetection {
    #[cfg(target_os = "linux")]
    {
        let utilities: &[&'static str] = match session {
            "WAYLAND" if xwayland_available == Some(true) => &["wl-copy", "xclip", "xsel"],
            "WAYLAND" => &["wl-copy"],
            "X11" => &["xclip", "xsel"],
            _ => return UtilityDetection::Missing,
        };
        find_executable(utilities, env::var_os("PATH").as_deref())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (session, xwayland_available);
        UtilityDetection::Unknown
    }
}

fn feature_readiness(
    platform: HostPlatform,
    capabilities: &HostCapabilities,
) -> Vec<FeatureReadiness> {
    let (process_detected, process_accessible, process_impl, process_ready, process_reason) =
        if platform == HostPlatform::Windows {
            (
                "DETECTED",
                "UNKNOWN",
                "IMPLEMENTED",
                "NOT_PROBED",
                "Target process access was not tested.",
            )
        } else {
            (
                "UNKNOWN",
                "UNKNOWN",
                "NOT_IMPLEMENTED",
                "UNSUPPORTED_PLATFORM",
                "DASRU process discovery is implemented on Windows only.",
            )
        };
    let (osd_detected, osd_accessible, osd_impl, osd_ready, osd_reason) =
        if platform == HostPlatform::Windows {
            (
                "DETECTED",
                "UNKNOWN",
                "EXPERIMENTAL",
                "EXPERIMENTAL_OPT_IN",
                "A recognized executable and explicit write opt-in are required.",
            )
        } else {
            (
                "UNKNOWN",
                "UNKNOWN",
                "NOT_IMPLEMENTED",
                "UNSUPPORTED_PLATFORM",
                "The native OSD writer is Windows-only.",
            )
        };
    vec![
        FeatureReadiness {
            feature: "process_probe",
            detected: process_detected,
            accessible: process_accessible,
            implementation_status: process_impl,
            readiness: process_ready,
            failure_reasons: vec![process_reason],
        },
        FeatureReadiness {
            feature: "memory_scan",
            detected: capabilities.process_memory_read.detected,
            accessible: capabilities.process_memory_read.accessible,
            implementation_status: capabilities.process_memory_read.implementation_status,
            readiness: if platform == HostPlatform::Windows {
                "NOT_PROBED"
            } else {
                "UNSUPPORTED_PLATFORM"
            },
            failure_reasons: vec!["A target process was not opened by doctor."],
        },
        FeatureReadiness {
            feature: "native_osd_write",
            detected: osd_detected,
            accessible: osd_accessible,
            implementation_status: osd_impl,
            readiness: osd_ready,
            failure_reasons: vec![osd_reason],
        },
        FeatureReadiness {
            feature: "clipboard",
            detected: capabilities.clipboard.detected,
            accessible: capabilities.clipboard.accessible,
            implementation_status: "NOT_IMPLEMENTED",
            readiness: "NOT_IMPLEMENTED",
            failure_reasons: vec!["DASRU has no clipboard feature."],
        },
        FeatureReadiness {
            feature: "global_hotkeys",
            detected: capabilities.global_hotkeys.detected,
            accessible: capabilities.global_hotkeys.accessible,
            implementation_status: "NOT_IMPLEMENTED",
            readiness: "NOT_IMPLEMENTED",
            failure_reasons: vec!["DASRU has no global-hotkey feature."],
        },
        FeatureReadiness {
            feature: "wayland_global_shortcuts_portal",
            detected: capabilities.wayland_global_shortcuts_portal.detected,
            accessible: capabilities.wayland_global_shortcuts_portal.accessible,
            implementation_status: "NOT_IMPLEMENTED",
            readiness: "NOT_IMPLEMENTED",
            failure_reasons: vec!["DASRU does not query or use the GlobalShortcuts portal."],
        },
    ]
}

#[cfg(target_os = "linux")]
fn find_executable(names: &[&'static str], path: Option<&std::ffi::OsStr>) -> UtilityDetection {
    let Some(path) = path else {
        return UtilityDetection::Unknown;
    };
    let mut unreadable = false;
    for directory in env::split_paths(&path) {
        for name in names {
            match std::fs::metadata(directory.join(name)) {
                Ok(metadata) => {
                    use std::os::unix::fs::PermissionsExt;
                    if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                        return UtilityDetection::Found(name);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => unreadable = true,
            }
        }
    }
    if unreadable {
        UtilityDetection::Unknown
    } else {
        UtilityDetection::Missing
    }
}

fn host_version(
    errors: &mut Vec<DoctorError>,
) -> (&'static str, Option<String>, Option<String>, Option<String>) {
    #[cfg(windows)]
    {
        let version = windows_version().map(|value| format!("{}.{}.{}", value.0, value.1, value.2));
        if version.is_none() {
            errors.push(DoctorError {
                field: "host.os_version",
                code: "WINDOWS_VERSION_UNAVAILABLE",
                message: "RtlGetVersion failed.".to_owned(),
            });
        }
        ("windows", version, None, None)
    }
    #[cfg(target_os = "linux")]
    {
        let distribution = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|contents| parse_pretty_name(&contents));
        if distribution.is_none() {
            errors.push(DoctorError {
                field: "host.distribution",
                code: "OS_RELEASE_UNAVAILABLE",
                message: "Could not read PRETTY_NAME from /etc/os-release.".to_owned(),
            });
        }
        let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        if kernel.is_none() {
            errors.push(DoctorError {
                field: "host.kernel_version",
                code: "KERNEL_VERSION_UNAVAILABLE",
                message: "Could not read /proc/sys/kernel/osrelease.".to_owned(),
            });
        }
        ("linux", None, distribution, kernel)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = errors;
        (env::consts::OS, None, None, None)
    }
}

#[cfg(target_os = "linux")]
fn parse_pretty_name(contents: &str) -> Option<String> {
    // shortcut: only simple surrounding quotes are handled; add escape parsing if distro labels require it.
    contents.lines().find_map(|line| {
        let value = line.strip_prefix("PRETTY_NAME=")?.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                value
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(value);
        (!value.is_empty()).then(|| value.to_owned())
    })
}

#[cfg(windows)]
fn windows_version() -> Option<(u32, u32, u32)> {
    use std::mem::size_of;
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
    }

    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // RtlGetVersion avoids application-manifest version virtualization.
    // SAFETY: version is initialized with the required size and remains writable for the call.
    (unsafe { RtlGetVersion(&mut version) } == 0).then_some((
        version.dwMajorVersion,
        version.dwMinorVersion,
        version.dwBuildNumber,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_classification_covers_native_and_desktop_sessions() {
        assert_eq!(
            classify_session(HostPlatform::Windows, None, false, false).session_type,
            "WINDOWS"
        );
        assert_eq!(
            classify_session(HostPlatform::Linux, Some("x11"), true, false).session_type,
            "X11"
        );
        assert_eq!(
            classify_session(HostPlatform::Linux, Some("wayland"), false, true).session_type,
            "WAYLAND"
        );
        assert_eq!(
            classify_session(HostPlatform::Linux, Some("wayland"), true, true).session_type,
            "WAYLAND"
        );
        assert_eq!(
            classify_session(HostPlatform::Linux, Some("wayland"), true, true).xwayland_available,
            Some(true)
        );
        let x11 = classify_session(HostPlatform::Linux, Some("x11"), true, true);
        assert_eq!(x11.session_type, "X11");
        assert_eq!(x11.xwayland_available, Some(false));
        assert_eq!(
            classify_session(HostPlatform::Linux, None, false, false).session_type,
            "HEADLESS"
        );
        assert_eq!(
            classify_session(HostPlatform::Windows, None, true, true).xwayland_available,
            None
        );
    }

    #[test]
    fn wsl2_is_reported_as_environment_separate_from_display_session() {
        assert_eq!(
            classify_environment(
                HostPlatform::Linux,
                Some("5.15.167.4-microsoft-standard-WSL2")
            ),
            "WSL2"
        );
        assert_eq!(
            classify_environment(HostPlatform::Linux, Some("6.8.0-generic")),
            "NATIVE"
        );
        assert_eq!(classify_environment(HostPlatform::Windows, None), "NATIVE");
    }

    #[test]
    fn unsupported_host_is_unknown_instead_of_claiming_access() {
        assert_eq!(
            classify_session(HostPlatform::Other, None, false, false).session_type,
            "UNKNOWN"
        );
        assert_eq!(classify_environment(HostPlatform::Other, None), "UNKNOWN");
    }

    #[test]
    fn runtime_environment_missing_signals_are_structured_not_detected() {
        let signal = runtime_signal_with(&["SteamAppId"], |_| false);
        assert_eq!(signal.status, "NOT_DETECTED");
        assert!(!signal.detected);
        assert!(signal.detected_by.is_empty());
    }

    #[test]
    fn a_clipboard_utility_never_means_accessible_or_implemented() {
        let capability = clipboard_capability_from_detection(
            HostPlatform::Linux,
            UtilityDetection::Found("wl-copy"),
        );
        assert_eq!(capability.detected, "DETECTED");
        assert_eq!(capability.accessible, "UNKNOWN");
        assert_eq!(capability.implementation_status, "NOT_IMPLEMENTED");

        let missing =
            clipboard_capability_from_detection(HostPlatform::Linux, UtilityDetection::Missing);
        assert_eq!(missing.detected, "NOT_DETECTED");
        assert_eq!(missing.accessible, "UNKNOWN");
        assert_eq!(missing.implementation_status, "NOT_IMPLEMENTED");

        let unknown =
            clipboard_capability_from_detection(HostPlatform::Linux, UtilityDetection::Unknown);
        assert_eq!(unknown.detected, "UNKNOWN");
        assert_eq!(unknown.accessible, "UNKNOWN");
    }

    #[test]
    fn linux_process_permission_is_unknown_without_a_backend_probe() {
        let capability = process_memory_capability(HostPlatform::Linux);
        assert_eq!(capability.detected, "UNKNOWN");
        assert_eq!(capability.accessible, "UNKNOWN");
        assert_eq!(capability.implementation_status, "NOT_IMPLEMENTED");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn clipboard_dependency_detection_handles_missing_path_and_tools() {
        assert!(matches!(
            find_executable(&["wl-copy"], None),
            UtilityDetection::Unknown
        ));
        assert!(matches!(
            find_executable(&["wl-copy"], Some(std::ffi::OsStr::new("/no/such/path"))),
            UtilityDetection::Missing
        ));
    }

    #[test]
    fn wayland_session_does_not_claim_global_shortcuts_portal_availability() {
        let portal = portal_capability(HostPlatform::Linux, "WAYLAND");
        assert_eq!(portal.detected, "UNKNOWN");
        assert_eq!(portal.accessible, "UNKNOWN");
        assert_eq!(portal.implementation_status, "NOT_IMPLEMENTED");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn os_release_parser_extracts_only_distribution_name() {
        assert_eq!(
            parse_pretty_name("NAME=Linux\nPRETTY_NAME=\"Example OS 1\"\n"),
            Some("Example OS 1".to_owned())
        );
        assert_eq!(parse_pretty_name("NAME=Linux\n"), None);
    }
}
