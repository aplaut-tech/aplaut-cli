mod support;

use support::{aplaut, TempDir};

#[test]
fn version_shows_cli_and_spec_versions() {
    let home = TempDir::new("version");
    let out = aplaut(home.path(), &["--version"], &[], "");
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, "aplaut 0.1.0 (Platform API 4.1.0)\n");
}

#[test]
fn short_version_flag_is_not_supported() {
    let home = TempDir::new("short-version");
    let out = aplaut(home.path(), &["-V"], &[], "");
    assert_eq!(out.code, 2);
}
