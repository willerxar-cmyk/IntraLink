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
