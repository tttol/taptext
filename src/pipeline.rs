use std::io::Write;

use anyhow::Result;

use crate::{
    audio::{AudioChunk, is_silent},
    recognition::Transcriber,
    transcript::TranscriptOutput,
};

pub(crate) fn process_chunk<T, O, F>(
    transcriber: &mut T,
    output: &mut TranscriptOutput<O, F>,
    chunk: &AudioChunk,
) -> Result<()>
where
    T: Transcriber,
    O: Write,
    F: Write,
{
    if is_silent(&chunk.samples) {
        return Ok(());
    }
    if let Some(line) = transcriber.transcribe(chunk)? {
        output.write_line(&line)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use crate::{
        audio::AudioChunk,
        recognition::Transcriber,
        transcript::{TranscriptLine, TranscriptOutput},
    };

    use super::process_chunk;

    struct FakeTranscriber;

    impl Transcriber for FakeTranscriber {
        fn transcribe(&mut self, chunk: &AudioChunk) -> Result<Option<TranscriptLine>> {
            Ok(Some(TranscriptLine {
                elapsed_centiseconds: chunk.start_centiseconds + 100,
                text: "One two three four five six seven".into(),
            }))
        }
    }

    #[test]
    fn sends_a_full_transcribed_chunk_to_both_outputs() -> Result<()> {
        // GIVEN
        let chunk = AudioChunk {
            samples: vec![0.5; 16_000],
            start_centiseconds: 400,
            discard_before_centiseconds: 100,
        };
        let expected = b"[00:00:05] One two three four five six seven\n".to_vec();
        let mut transcriber = FakeTranscriber;
        let mut output = TranscriptOutput::new(Vec::new(), Vec::new());

        // WHEN
        process_chunk(&mut transcriber, &mut output, &chunk)?;

        // THEN
        let actual = output.into_inner();
        assert_eq!(actual, (expected.clone(), expected));
        Ok(())
    }
}
