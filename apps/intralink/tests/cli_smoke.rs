//! Exercise the actual process entry point: a unit test cannot detect a
//! Windows main-thread stack overflow before command dispatch.
use std::process::Command;

fn stdout(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_intralink"))
        .args(args)
        .output()
        .expect("Start IntraLink CLI");
    assert!(
        output.status.success(),
        "{args:?}: status={:?}; stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("UTF-8 CLI output")
}

#[test]
fn cli_help_and_version_exit_successfully() {
    let help = stdout(&["--help"]);
    assert!(help.contains("IntraLink") && help.contains("Commands"));
    assert!(stdout(&["--version"]).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn setup_cli_exposes_base_client_and_rejects_unattended_creation() {
    let help = stdout(&["setup", "--help"]);
    assert!(help.contains("base") && help.contains("client") && !help.contains("hybrid"));
    assert!(stdout(&["set-role", "--help"]).contains("--role"));
    assert!(stdout(&["rename-device", "--help"]).contains("--name"));
    let temp = tempfile::tempdir().expect("Temporary local profile");
    let path = temp.path().join("profile.vault");
    let output = Command::new(env!("CARGO_BIN_EXE_intralink"))
        .arg("--vault")
        .arg(&path)
        .args(["setup", "--role", "base", "--name", "Local test"])
        .output()
        .expect("Run noninteractive setup");
    assert!(!output.status.success());
    assert!(!path.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("interactive terminal"));
}

#[test]
fn graphical_pairing_is_explicit_and_rejects_unattended_mutation() {
    for command in [
        "connection-card",
        "invite",
        "accept-invite",
        "confirm-invite",
    ] {
        assert!(stdout(&[command, "--help"]).contains("--gui"));
    }
    let directory = tempfile::tempdir().expect("Temporary pairing directory");
    let vault = directory.path().join("profile.vault");
    let id = iroh::SecretKey::from_bytes(&[7; 32]).public().to_string();
    for args in [
        vec!["connection-card", "--gui"],
        vec![
            "invite",
            "--controller-id",
            &id,
            "--host",
            "192.168.10.2",
            "--port",
            "45100",
            "--gui",
        ],
        vec!["accept-invite", "--name", "client", "--gui"],
        vec!["confirm-invite", "--name", "base", "--gui"],
        vec!["accept-invite", "--name", "client", "--qr", "--gui"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intralink"))
            .arg("--vault")
            .arg(&vault)
            .args(&args)
            .output()
            .expect("Run noninteractive graphical pairing");
        assert!(!output.status.success(), "{args:?}");
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            0,
            "Graphical preflight must fail before creating a vault or lock"
        );
    }
}

#[cfg(all(target_os = "windows", feature = "nvenc-preview"))]
#[test]
fn h264_diagnostic_help_does_not_initialize_driver_or_network() {
    let help = stdout(&["desktop-h264-check", "--help"]);
    assert!(
        help.contains("--height") && help.contains("--samples") && help.contains("--adapter-index")
    );
    let live = stdout(&["desktop-wgc-live-check", "--help"]);
    assert!(live.contains("--cancel-after") && live.contains("--samples"));
    let invalid = Command::new(env!("CARGO_BIN_EXE_intralink"))
        .args([
            "desktop-wgc-live-check",
            "--samples",
            "10",
            "--cancel-after",
            "10",
        ])
        .output()
        .expect("Start invalid live diagnostic command");
    assert!(!invalid.status.success());
    assert!(
        String::from_utf8_lossy(&invalid.stderr)
            .contains("Cancellation must precede the final frame")
    );
    let share = stdout(&["share-desktop", "--help"]);
    assert!(!share.contains("h264"));
}

#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
#[test]
fn portable_share_defaults_to_1080p_and_adaptive_quality() {
    let help = stdout(&["share-desktop", "--help"]);
    assert!(help.contains("--height") && help.contains("[default: 1080]"));
    assert!(help.contains("--fixed-quality") && help.contains("adaptive by default"));
    assert!(help.contains("[default: snapshot]") && help.contains("[default: compact]"));
    assert!(!help.contains("h264"));
    assert!(help.contains("--adaptive-resolution"));
    assert!(stdout(&["view-desktop", "--help"]).contains("--adaptive-resolution"));
    assert!(stdout(&["workspace", "--help"]).contains("--adaptive-resolution"));
    for command in ["share-desktop", "view-desktop", "workspace"] {
        assert!(stdout(&[command, "--help"]).contains("--negotiate-codec"));
    }
    let conflicting = Command::new(env!("CARGO_BIN_EXE_intralink"))
        .args([
            "view-desktop",
            "--device",
            "test",
            "--adaptive-resolution",
            "--negotiate-codec",
        ])
        .output()
        .expect("Parse conflicting protocol flags");
    assert!(!conflicting.status.success());
    assert!(String::from_utf8_lossy(&conflicting.stderr).contains("cannot be used with"));
}

#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
#[test]
fn bounded_profile_cli_rejects_invalid_limits_before_session_start() {
    for command in ["share-desktop", "view-desktop", "workspace"] {
        assert!(stdout(&[command, "--help"]).contains("--negotiate-profile"));
    }
    let workspace = stdout(&["workspace", "--help"]);
    for option in ["--max-height", "--max-fps", "--device-limit"] {
        assert!(workspace.contains(option));
    }
    for args in [
        vec!["view-desktop", "--device", "test", "--max-height", "540"],
        vec![
            "view-desktop",
            "--device",
            "test",
            "--negotiate-profile",
            "--max-height",
            "1440",
        ],
        vec![
            "view-desktop",
            "--device",
            "test",
            "--negotiate-profile",
            "--max-fps",
            "61",
        ],
        vec![
            "view-desktop",
            "--device",
            "test",
            "--negotiate-profile",
            "--negotiate-codec",
        ],
        vec![
            "workspace",
            "--device",
            "test",
            "--negotiate-profile",
            "--device-limit",
            "unknown=540:15",
        ],
        vec![
            "workspace",
            "--device",
            "test",
            "--negotiate-profile",
            "--max-height",
            "540",
            "--device-limit",
            "test=1080:60",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intralink"))
            .args(&args)
            .output()
            .expect("Parse invalid receiver limits");
        assert!(!output.status.success(), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("requires")
                || stderr.contains("required")
                || stderr.contains("Receiver limits")
                || stderr.contains("invalid value")
                || stderr.contains("cannot be used with")
                || stderr.contains("unselected host")
                || stderr.contains("exceeds controller ceiling"),
            "{args:?}: {stderr}"
        );
    }
}

#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
#[test]
fn calibration_cli_is_explicit_and_validated_before_connection() {
    let help = stdout(&["desktop-profile-check", "--help"]);
    assert!(help.contains("--slots") && help.contains("--hardware"));
    for command in ["view-desktop", "workspace"] {
        assert!(stdout(&[command, "--help"]).contains("--auto-profile"));
        let output = Command::new(env!("CARGO_BIN_EXE_intralink"))
            .args([command, "--device", "test", "--auto-profile"])
            .output()
            .expect("Validate auto-profile flag");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--negotiate-profile"));
    }
}

#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
#[test]
fn host_calibration_is_opt_in_and_keeps_explicit_capture() {
    let help = stdout(&["share-desktop", "--help"]);
    assert!(help.contains("snapshot") && help.contains("wgc-gpu") && help.contains("auto"));
    assert!(help.contains("[default: snapshot]"));
    let help = stdout(&["desktop-profile-check", "--help"]);
    assert!(help.contains("--host"));
    for conflicting in [vec!["--host", "--slots", "2"], vec!["--host", "--hardware"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_intralink"))
            .arg("desktop-profile-check")
            .args(&conflicting)
            .output()
            .expect("Validate calibration command");
        assert!(!output.status.success(), "{conflicting:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
    }
}

#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
#[test]
fn encoder_diagnostic_is_local_and_explicit() {
    assert!(stdout(&["desktop-profile-check", "--help"]).contains("--encoder"));
    let output = Command::new(env!("CARGO_BIN_EXE_intralink"))
        .args(["desktop-profile-check", "--encoder", "--host"])
        .output()
        .expect("Validate diagnostic mode");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
}

#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
#[test]
fn pipeline_requires_explicit_fixed_profile_before_connection() {
    let help = stdout(&["share-desktop", "--help"]);
    assert!(help.contains("--pipeline"));
    let common = [
        "share-desktop",
        "--bind",
        "127.0.0.1:12345",
        "--controller",
        "test",
        "--monitor",
        "1",
        "--pipeline",
    ];
    for extra in [
        vec![],
        vec!["--fixed-quality", "--adaptive-resolution"],
        vec!["--fixed-quality", "--negotiate-codec"],
        vec!["--fixed-quality", "--negotiate-profile"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intralink"))
            .args(common)
            .args(extra)
            .output()
            .expect("Parse pipeline limits");
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("required") || stderr.contains("cannot be used with"),
            "{stderr}"
        );
    }
}

#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
#[test]
fn visual_fidelity_diagnostic_is_explicit_and_separate_from_benchmarks() {
    let help = stdout(&["desktop-profile-check", "--help"]);
    assert!(help.contains("--visual"));
    for flag in ["--pipeline", "--encoder", "--host", "--hardware"] {
        let output = Command::new(env!("CARGO_BIN_EXE_intralink"))
            .args(["desktop-profile-check", "--visual", flag])
            .output()
            .expect("Parse visual diagnostic mode");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
    }
}

#[cfg(any(
    all(target_os = "windows", feature = "input-preview"),
    all(target_os = "linux", feature = "input-x11-preview")
))]
#[test]
fn input_host_is_explicit_and_rejects_unattended_start() {
    let help = stdout(&["input", "--help"]);
    assert!(help.contains("host"));
    assert!(help.contains("workspace"));
    let workspace = stdout(&["input", "workspace", "--help"]);
    assert!(workspace.contains("--device") && workspace.contains("--lan"));
    #[cfg(target_os = "linux")]
    assert!(!workspace.contains("--spatial") && !workspace.contains("--choose"));
    #[cfg(target_os = "windows")]
    assert!(workspace.contains("--choose") && workspace.contains("--spatial"));
    let host = stdout(&["input", "host", "--help"]);
    assert!(
        host.contains("--bind")
            && host.contains("--controller")
            && host.contains("--seconds")
            && host.contains("--advertise-lan")
    );
    let temporary = tempfile::tempdir().expect("Temporary local profile");
    let path = temporary.path().join("profile.vault");
    let output = Command::new(env!("CARGO_BIN_EXE_intralink"))
        .arg("--vault")
        .arg(&path)
        .args([
            "input",
            "host",
            "--bind",
            "192.168.1.20:45100",
            "--controller",
            "test",
            "--advertise-lan",
        ])
        .output()
        .expect("Run noninteractive input command");
    assert!(!output.status.success());
    assert!(!path.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("local interactive terminal"));
}

#[cfg(all(target_os = "windows", feature = "input-preview"))]
#[test]
fn graphical_input_selection_is_explicit_and_never_runs_unattended() {
    let temp = tempfile::tempdir().expect("Temporary selector directory");
    let path = temp.path().join("profile.vault");
    for args in [
        vec!["input", "workspace", "--choose"],
        vec!["input", "workspace", "--choose", "--device", "test"],
        vec!["input", "workspace"],
        vec!["input", "workspace", "--device", "test", "--lan"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intralink"))
            .arg("--vault")
            .arg(&path)
            .args(&args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{args:?}");
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
        if args.len() == 3 || args.contains(&"--lan") {
            assert!(String::from_utf8_lossy(&output.stderr).contains("local interactive terminal"));
        }
    }
}

#[cfg(all(
    feature = "desktop-preview",
    any(target_os = "windows", target_os = "linux")
))]
#[test]
fn graphical_screen_workspace_rejects_unattended_start_and_conflicting_options() {
    assert!(stdout(&["workspace", "--help"]).contains("--choose"));
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("profile.vault");
    for args in [
        vec!["workspace", "--choose"],
        vec!["workspace", "--choose", "--device", "client"],
        vec![
            "workspace",
            "--choose",
            "--negotiate-profile",
            "--device-limit",
            "client=720:30",
        ],
        vec!["workspace"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_intralink"))
            .arg("--vault")
            .arg(&path)
            .args(&args)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
        if args == ["workspace", "--choose"] {
            assert!(
                String::from_utf8_lossy(&result.stderr).contains("interactive graphical terminal")
            );
        }
    }
}

#[cfg(all(target_os = "windows", feature = "input-preview"))]
#[test]
fn topology_editor_and_geometry_workspace_reject_unattended_or_conflicting_modes() {
    assert!(stdout(&["input", "topology", "--help"]).contains("--device"));
    assert!(stdout(&["input", "workspace", "--help"]).contains("--geometry"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.vault");
    for args in [
        vec!["input", "topology", "--device", "client"],
        vec!["input", "workspace", "--choose", "--geometry"],
        vec![
            "input",
            "workspace",
            "--device",
            "client",
            "--geometry",
            "--spatial",
        ],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_intralink"))
            .arg("--vault")
            .arg(&path)
            .args(&args)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{args:?}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        if !args.contains(&"--spatial") {
            assert!(String::from_utf8_lossy(&result.stderr).contains("local interactive terminal"));
        }
    }
}

#[cfg(all(target_os = "windows", feature = "input-preview"))]
#[test]
fn display_negotiation_requires_explicit_geometry_and_local_interaction() {
    assert!(stdout(&["input", "host", "--help"]).contains("--display-geometry"));
    assert!(stdout(&["input", "workspace", "--help"]).contains("--detect-displays"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.vault");
    for args in [
        vec![
            "input",
            "host",
            "--bind",
            "192.168.1.20:45100",
            "--controller",
            "base",
            "--display-geometry",
        ],
        vec![
            "input",
            "workspace",
            "--choose",
            "--geometry",
            "--detect-displays",
        ],
        vec!["input", "workspace", "--choose", "--detect-displays"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_intralink"))
            .arg("--vault")
            .arg(&path)
            .args(&args)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{args:?}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        if args.contains(&"--geometry") || args.contains(&"--display-geometry") {
            assert!(String::from_utf8_lossy(&result.stderr).contains("local interactive terminal"));
        }
    }
}

#[cfg(all(target_os = "windows", feature = "input-preview"))]
#[test]
fn native_edge_check_and_arrival_cue_are_explicit_and_never_unattended() {
    assert!(stdout(&["input", "edge-check", "--help"]).contains("--seconds"));
    assert!(stdout(&["input", "host", "--help"]).contains("--entry-portal"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.vault");
    for args in [
        vec!["input", "edge-check", "--device", "client"],
        vec![
            "input",
            "edge-check",
            "--device",
            "client",
            "--seconds",
            "121",
        ],
        vec![
            "input",
            "host",
            "--bind",
            "192.168.1.20:45100",
            "--controller",
            "base",
            "--display-geometry",
            "--entry-portal",
            "full",
        ],
        vec![
            "input",
            "host",
            "--bind",
            "192.168.1.20:45100",
            "--controller",
            "base",
            "--entry-portal",
            "reduced",
        ],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_intralink"))
            .arg("--vault")
            .arg(&path)
            .args(&args)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{args:?}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

#[cfg(all(target_os = "windows", feature = "input-preview"))]
#[test]
fn desktop_edge_mode_is_explicit_and_requires_authenticated_geometry() {
    assert!(stdout(&["input", "workspace", "--help"]).contains("--desktop-edges"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.vault");
    for args in [
        vec!["input", "workspace", "--choose", "--desktop-edges"],
        vec![
            "input",
            "workspace",
            "--choose",
            "--geometry",
            "--desktop-edges",
        ],
        vec![
            "input",
            "workspace",
            "--choose",
            "--geometry",
            "--detect-displays",
            "--desktop-edges",
        ],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_intralink"))
            .arg("--vault")
            .arg(&path)
            .args(&args)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{args:?}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        if args.contains(&"--detect-displays") {
            assert!(String::from_utf8_lossy(&result.stderr).contains("local interactive terminal"));
        }
    }
}
