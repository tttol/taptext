use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use time::{OffsetDateTime, format_description};

#[derive(Debug, Parser)]
#[command(
    name = "taptext",
    version,
    about = "Macのシステム音声を英語でローカル文字起こしします"
)]
pub struct Cli {
    #[arg(short, long, value_name = "PATH", help = "文字起こしTXTの保存先")]
    pub(crate) output: Option<PathBuf>,
}

impl Cli {
    pub(crate) fn output_path(&self) -> Result<PathBuf> {
        let now = match OffsetDateTime::now_local() {
            Ok(local) => local,
            Err(_) => OffsetDateTime::now_utc(),
        };
        self.output
            .clone()
            .map_or_else(|| default_output_path(Path::new("."), now), Ok)
    }
}

fn default_output_path(directory: &Path, now: OffsetDateTime) -> Result<PathBuf> {
    let format =
        format_description::parse_borrowed::<3>("[year][month][day]-[hour][minute][second]")
            .context("出力ファイル名の時刻形式を初期化できませんでした")?;
    let timestamp = now
        .format(&format)
        .context("出力ファイル名の時刻を生成できませんでした")?;
    Ok(directory.join(format!("taptext-{timestamp}.txt")))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use anyhow::Result;
    use clap::Parser;
    use rstest::rstest;
    use time::OffsetDateTime;

    use super::{Cli, default_output_path};

    #[test]
    fn parses_explicit_output_path() -> Result<()> {
        // GIVEN
        let args = ["taptext", "--output", "meeting.txt"];
        let expected = Some(PathBuf::from("meeting.txt"));

        // WHEN
        let actual = Cli::try_parse_from(args)?;

        // THEN
        assert_eq!(actual.output, expected);
        Ok(())
    }

    #[rstest]
    #[case(&["taptext", "-w", "5"])]
    #[case(&["taptext", "--window-seconds", "10"])]
    fn rejects_removed_window_options(#[case] args: &[&str]) {
        // GIVEN

        // WHEN
        let actual = Cli::try_parse_from(args.iter().copied());

        // THEN
        assert!(actual.is_err());
    }

    #[test]
    fn creates_timestamped_default_output_path() -> Result<()> {
        // GIVEN
        let now = OffsetDateTime::from_unix_timestamp(0)?;
        let expected = PathBuf::from("./taptext-19700101-000000.txt");

        // WHEN
        let actual = default_output_path(Path::new("."), now)?;

        // THEN
        assert_eq!(actual, expected);
        Ok(())
    }
}
