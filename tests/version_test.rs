use anyhow::Result;
use rstest::rstest;
use std::process::Command;

#[rstest]
#[case("--version")]
#[case("-V")]
fn prints_package_version_without_starting_transcription(#[case] option: &str) -> Result<()> {
    // GIVEN
    let expected = format!("taptext {}\n", env!("CARGO_PKG_VERSION"));
    // WHEN
    let output = Command::new(env!("CARGO_BIN_EXE_taptext"))
        .arg(option)
        .output()?;
    // THEN
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout)?, expected);
    assert!(output.stderr.is_empty());
    Ok(())
}
