use std::io::Write;

use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimedToken {
    pub(crate) start_centiseconds: i64,
    pub(crate) text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranscriptLine {
    pub(crate) elapsed_centiseconds: u64,
    pub(crate) text: String,
}

fn is_ignored_token(text: &str) -> bool {
    let trimmed = text.trim();
    (trimmed.starts_with("<|") && trimmed.ends_with("|>"))
        || (trimmed.starts_with("[_") && trimmed.ends_with(']'))
        || trimmed.eq_ignore_ascii_case("[music]")
}

pub(crate) fn line_from_tokens(
    chunk_start_centiseconds: u64,
    discard_before_centiseconds: i64,
    tokens: &[TimedToken],
) -> Option<TranscriptLine> {
    let selected = tokens
        .iter()
        .filter(|token| token.start_centiseconds >= discard_before_centiseconds)
        .filter(|token| !is_ignored_token(&token.text))
        .collect::<Vec<_>>();
    let first = selected.first()?;
    let text = selected
        .iter()
        .map(|token| token.text.as_str())
        .collect::<String>()
        .trim()
        .to_owned();
    (!text.is_empty()).then(|| TranscriptLine {
        elapsed_centiseconds: chunk_start_centiseconds
            + u64::try_from(first.start_centiseconds).unwrap_or_default(),
        text,
    })
}

pub(crate) struct TranscriptOutput<T, F>
where
    T: Write,
    F: Write,
{
    terminal: T,
    file: F,
}

impl<T, F> TranscriptOutput<T, F>
where
    T: Write,
    F: Write,
{
    pub(crate) fn new(terminal: T, file: F) -> Self {
        Self { terminal, file }
    }

    pub(crate) fn write_line(&mut self, line: &TranscriptLine) -> Result<()> {
        let formatted = format!(
            "[{}] {}\n",
            format_elapsed(line.elapsed_centiseconds),
            line.text
        );
        self.terminal
            .write_all(formatted.as_bytes())
            .context("端末へ文字起こしを表示できませんでした")?;
        self.terminal
            .flush()
            .context("端末出力をflushできませんでした")?;
        self.file
            .write_all(formatted.as_bytes())
            .context("TXTへ文字起こしを書き込めませんでした")?;
        self.file.flush().context("TXTをflushできませんでした")
    }

    #[cfg(test)]
    pub(crate) fn into_inner(self) -> (T, F) {
        (self.terminal, self.file)
    }
}

fn format_elapsed(centiseconds: u64) -> String {
    let total_seconds = centiseconds / 100;
    let hours = total_seconds / 3_600;
    let minutes = total_seconds % 3_600 / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rstest::rstest;

    use super::{TimedToken, TranscriptLine, TranscriptOutput, format_elapsed, line_from_tokens};

    #[test]
    fn discards_tokens_from_the_overlapping_prefix() {
        // GIVEN
        let tokens = vec![
            TimedToken {
                start_centiseconds: 40,
                text: " repeated".into(),
            },
            TimedToken {
                start_centiseconds: 110,
                text: " new".into(),
            },
            TimedToken {
                start_centiseconds: 130,
                text: " words".into(),
            },
        ];
        let expected = Some(TranscriptLine {
            elapsed_centiseconds: 510,
            text: "new words".into(),
        });

        // WHEN
        let actual = line_from_tokens(400, 100, &tokens);

        // THEN
        assert_eq!(actual, expected);
    }

    #[test]
    fn removes_whisper_internal_and_music_tokens() {
        // GIVEN
        let tokens = vec![
            TimedToken {
                start_centiseconds: 0,
                text: " Hello.".into(),
            },
            TimedToken {
                start_centiseconds: 230,
                text: "[_TT_230]".into(),
            },
            TimedToken {
                start_centiseconds: 240,
                text: " [Music]".into(),
            },
            TimedToken {
                start_centiseconds: 250,
                text: " [Speaker]".into(),
            },
        ];
        let expected = Some(TranscriptLine {
            elapsed_centiseconds: 0,
            text: "Hello. [Speaker]".into(),
        });

        // WHEN
        let actual = line_from_tokens(0, 0, &tokens);

        // THEN
        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case(0, "00:00:00")]
    #[case(6_500, "00:01:05")]
    #[case(366_100, "01:01:01")]
    fn formats_elapsed_time(#[case] centiseconds: u64, #[case] expected: &str) {
        // GIVEN

        // WHEN
        let actual = format_elapsed(centiseconds);

        // THEN
        assert_eq!(actual, expected);
    }

    #[test]
    fn writes_the_same_line_to_terminal_and_file() -> Result<()> {
        // GIVEN
        let line = TranscriptLine {
            elapsed_centiseconds: 500,
            text: "Hello from TapText.".into(),
        };
        let expected = b"[00:00:05] Hello from TapText.\n".to_vec();
        let mut output = TranscriptOutput::new(Vec::new(), Vec::new());

        // WHEN
        output.write_line(&line)?;

        // THEN
        let actual = output.into_inner();
        assert_eq!(actual, (expected.clone(), expected));
        Ok(())
    }
}
