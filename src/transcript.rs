use std::io::Write;

use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranscriptLine {
    pub(crate) elapsed_centiseconds: u64,
    pub(crate) text: String,
}

pub(crate) fn filtered_text<'a>(tokens: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let text = tokens
        .into_iter()
        .filter(|text| !is_ignored_token(text))
        .collect::<String>()
        .trim()
        .to_owned();
    (!text.is_empty()).then_some(text)
}

fn is_ignored_token(text: &str) -> bool {
    let trimmed = text.trim();
    (trimmed.starts_with("<|") && trimmed.ends_with("|>"))
        || (trimmed.starts_with("[_") && trimmed.ends_with(']'))
        || trimmed.eq_ignore_ascii_case("[music]")
}

#[derive(Default)]
pub(crate) struct StablePrefix {
    previous_words: Option<Vec<String>>,
}

impl StablePrefix {
    pub(crate) fn update(&mut self, text: &str) -> Option<String> {
        let current_words = words(text);
        let stable_length = self.previous_words.as_ref().map_or(0, |previous| {
            previous
                .iter()
                .zip(&current_words)
                .take_while(|(left, right)| normalize_word(left) == normalize_word(right))
                .count()
        });
        self.previous_words = Some(current_words.clone());
        (stable_length > 0).then(|| current_words[..stable_length].join(" "))
    }

    pub(crate) fn reset(&mut self) {
        self.previous_words = None;
    }
}

pub(crate) fn remove_repeated_prefix(previous: &str, current: &str) -> Option<String> {
    let previous_words = words(previous);
    let current_words = words(current);
    let repeated = (1..=previous_words.len().min(current_words.len()))
        .rev()
        .find(|length| {
            previous_words[previous_words.len() - length..]
                .iter()
                .zip(&current_words[..*length])
                .all(|(left, right)| normalize_word(left) == normalize_word(right))
        })
        .unwrap_or_default();
    let text = current_words[repeated..].join(" ");
    (!text.is_empty()).then_some(text)
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

fn normalize_word(word: &str) -> String {
    word.trim_matches(|character: char| {
        character.is_ascii_punctuation() && character != '\'' && character != '-'
    })
    .to_ascii_lowercase()
}

pub(crate) struct TranscriptOutput<T, F>
where
    T: Write,
    F: Write,
{
    terminal: T,
    file: F,
    show_partials: bool,
    partial_active: bool,
}

impl<T, F> TranscriptOutput<T, F>
where
    T: Write,
    F: Write,
{
    pub(crate) fn new(terminal: T, file: F, show_partials: bool) -> Self {
        Self {
            terminal,
            file,
            show_partials,
            partial_active: false,
        }
    }

    pub(crate) fn update_partial(&mut self, line: &TranscriptLine) -> Result<()> {
        if !self.show_partials {
            return Ok(());
        }
        let formatted = format!(
            "\r\x1b[2K[{}] {}",
            format_elapsed(line.elapsed_centiseconds),
            line.text
        );
        self.terminal
            .write_all(formatted.as_bytes())
            .context("端末へ途中字幕を表示できませんでした")?;
        self.terminal
            .flush()
            .context("端末の途中字幕をflushできませんでした")?;
        self.partial_active = true;
        Ok(())
    }

    pub(crate) fn clear_partial(&mut self) -> Result<()> {
        if !self.partial_active {
            return Ok(());
        }
        self.terminal
            .write_all(b"\r\x1b[2K")
            .context("端末の途中字幕を消去できませんでした")?;
        self.terminal
            .flush()
            .context("端末の途中字幕消去をflushできませんでした")?;
        self.partial_active = false;
        Ok(())
    }

    pub(crate) fn write_final(&mut self, line: &TranscriptLine) -> Result<()> {
        self.clear_partial()?;
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

    use super::{
        StablePrefix, TranscriptLine, TranscriptOutput, filtered_text, format_elapsed,
        remove_repeated_prefix,
    };

    #[test]
    fn removes_whisper_internal_and_music_tokens() {
        // GIVEN
        let tokens = [" Hello.", "[_TT_230]", " [Music]", "<|endoftext|>"];
        let expected = Some("Hello.".to_owned());

        // WHEN
        let actual = filtered_text(tokens);

        // THEN
        assert_eq!(actual, expected);
    }

    #[test]
    fn returns_common_prefix_from_consecutive_hypotheses() {
        // GIVEN
        let mut prefix = StablePrefix::default();
        let first = prefix.update("Hello brave world");
        assert_eq!(first, None);

        // WHEN
        let actual = prefix.update("hello, brave new world");

        // THEN
        assert_eq!(actual, Some("hello, brave".to_owned()));
    }

    #[test]
    fn returns_no_prefix_when_first_word_changes() {
        // GIVEN
        let mut prefix = StablePrefix::default();
        let first = prefix.update("one two");
        assert_eq!(first, None);

        // WHEN
        let actual = prefix.update("three two");

        // THEN
        assert_eq!(actual, None);
    }

    #[test]
    fn removes_repeated_words_at_forced_segment_boundary() {
        // GIVEN
        let previous = "We need a stable prefix";
        let current = "Stable prefix for every result";
        let expected = Some("for every result".to_owned());

        // WHEN
        let actual = remove_repeated_prefix(previous, current);

        // THEN
        assert_eq!(actual, expected);
    }

    #[test]
    fn partial_output_is_written_only_to_an_interactive_terminal() -> Result<()> {
        // GIVEN
        let line = TranscriptLine {
            elapsed_centiseconds: 500,
            text: "Hello world".into(),
        };
        let mut interactive = TranscriptOutput::new(Vec::new(), Vec::new(), true);
        let mut redirected = TranscriptOutput::new(Vec::new(), Vec::new(), false);

        // WHEN
        interactive.update_partial(&line)?;
        redirected.update_partial(&line)?;

        // THEN
        assert_eq!(
            interactive.into_inner(),
            (b"\r\x1b[2K[00:00:05] Hello world".to_vec(), Vec::new())
        );
        assert_eq!(redirected.into_inner(), (Vec::new(), Vec::new()));
        Ok(())
    }

    #[test]
    fn final_output_clears_partial_and_writes_the_same_line_to_file() -> Result<()> {
        // GIVEN
        let line = TranscriptLine {
            elapsed_centiseconds: 500,
            text: "Hello world".into(),
        };
        let mut output = TranscriptOutput::new(Vec::new(), Vec::new(), true);
        output.update_partial(&line)?;
        let expected_file = b"[00:00:05] Hello world\n".to_vec();
        let expected_terminal = [
            b"\r\x1b[2K[00:00:05] Hello world".as_slice(),
            b"\r\x1b[2K",
            expected_file.as_slice(),
        ]
        .concat();

        // WHEN
        output.write_final(&line)?;

        // THEN
        assert_eq!(output.into_inner(), (expected_terminal, expected_file));
        Ok(())
    }

    #[rstest]
    #[case(0, "00:00:00")]
    #[case(366_100, "01:01:01")]
    fn formats_elapsed_time(#[case] centiseconds: u64, #[case] expected: &str) {
        // GIVEN

        // WHEN
        let actual = format_elapsed(centiseconds);

        // THEN
        assert_eq!(actual, expected);
    }
}
