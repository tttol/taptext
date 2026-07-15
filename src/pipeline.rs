use std::io::Write;

use anyhow::Result;

use crate::{
    audio::{SAMPLE_RATE, UtteranceJob, UtteranceKind},
    recognition::Transcriber,
    transcript::{StablePrefix, TranscriptLine, TranscriptOutput, remove_repeated_prefix},
};

pub(crate) struct TranscriptProcessor<T, O, F>
where
    T: Transcriber,
    O: Write,
    F: Write,
{
    transcriber: T,
    output: TranscriptOutput<O, F>,
    stable_prefix: StablePrefix,
    partial_utterance_id: Option<u64>,
    previous_final: Option<String>,
}

impl<T, O, F> TranscriptProcessor<T, O, F>
where
    T: Transcriber,
    O: Write,
    F: Write,
{
    pub(crate) fn new(transcriber: T, output: TranscriptOutput<O, F>) -> Self {
        Self {
            transcriber,
            output,
            stable_prefix: StablePrefix::default(),
            partial_utterance_id: None,
            previous_final: None,
        }
    }

    pub(crate) fn process(&mut self, job: &UtteranceJob) -> Result<()> {
        match job.kind {
            UtteranceKind::Partial => self.process_partial(job),
            UtteranceKind::Final => self.process_final(job),
        }
    }

    fn process_partial(&mut self, job: &UtteranceJob) -> Result<()> {
        if self.partial_utterance_id != Some(job.id) {
            self.stable_prefix.reset();
            self.partial_utterance_id = Some(job.id);
        }
        let Some(text) = self.transcriber.transcribe(&job.samples)? else {
            return Ok(());
        };
        let Some(stable) = self.stable_prefix.update(&text) else {
            return Ok(());
        };
        self.output.update_partial(&TranscriptLine {
            elapsed_centiseconds: elapsed_centiseconds(job.start_sample),
            text: stable,
        })
    }

    fn process_final(&mut self, job: &UtteranceJob) -> Result<()> {
        let text = self.transcriber.transcribe(&job.samples)?;
        self.stable_prefix.reset();
        self.partial_utterance_id = None;
        let Some(text) = text else {
            return self.output.clear_partial();
        };
        let text = if job.deduplicate_prefix {
            self.previous_final
                .as_deref()
                .and_then(|previous| remove_repeated_prefix(previous, &text))
        } else {
            Some(text)
        };
        let Some(text) = text else {
            return self.output.clear_partial();
        };
        self.output.write_final(&TranscriptLine {
            elapsed_centiseconds: elapsed_centiseconds(job.start_sample),
            text: text.clone(),
        })?;
        self.previous_final = Some(text);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn into_parts(self) -> (T, TranscriptOutput<O, F>) {
        (self.transcriber, self.output)
    }
}

fn elapsed_centiseconds(start_sample: u64) -> u64 {
    start_sample.saturating_mul(100) / SAMPLE_RATE as u64
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use anyhow::Result;

    use crate::{
        audio::{UtteranceJob, UtteranceKind},
        recognition::Transcriber,
        transcript::TranscriptOutput,
    };

    use super::TranscriptProcessor;

    struct FakeTranscriber {
        results: VecDeque<Option<String>>,
    }

    impl Transcriber for FakeTranscriber {
        fn transcribe(&mut self, _samples: &[f32]) -> Result<Option<String>> {
            Ok(self.results.pop_front().flatten())
        }
    }

    #[test]
    fn shows_stable_partial_only_in_terminal_and_saves_final_text() -> Result<()> {
        // GIVEN
        let transcriber = FakeTranscriber {
            results: [
                Some("Hello brave world".into()),
                Some("hello, brave new world".into()),
                Some("Hello brave new world.".into()),
            ]
            .into_iter()
            .collect(),
        };
        let output = TranscriptOutput::new(Vec::new(), Vec::new(), true);
        let mut processor = TranscriptProcessor::new(transcriber, output);
        let partial = UtteranceJob {
            id: 1,
            kind: UtteranceKind::Partial,
            start_sample: 16_000,
            samples: vec![0.5; 16_000],
            deduplicate_prefix: false,
        };
        let final_job = UtteranceJob {
            kind: UtteranceKind::Final,
            ..partial.clone()
        };

        // WHEN
        processor.process(&partial)?;
        processor.process(&partial)?;
        processor.process(&final_job)?;

        // THEN
        let (_, output) = processor.into_parts();
        let (terminal, file) = output.into_inner();
        assert_eq!(file, b"[00:00:01] Hello brave new world.\n");
        assert!(String::from_utf8_lossy(&terminal).contains("hello, brave"));
        Ok(())
    }

    #[test]
    fn removes_boundary_overlap_only_from_continuation_final() -> Result<()> {
        // GIVEN
        let transcriber = FakeTranscriber {
            results: [
                Some("We need a stable prefix".into()),
                Some("stable prefix for every result".into()),
            ]
            .into_iter()
            .collect(),
        };
        let output = TranscriptOutput::new(Vec::new(), Vec::new(), false);
        let mut processor = TranscriptProcessor::new(transcriber, output);
        let first = UtteranceJob {
            id: 1,
            kind: UtteranceKind::Final,
            start_sample: 0,
            samples: vec![0.5; 16_000],
            deduplicate_prefix: false,
        };
        let continuation = UtteranceJob {
            id: 2,
            kind: UtteranceKind::Final,
            start_sample: 16_000,
            samples: vec![0.5; 16_000],
            deduplicate_prefix: true,
        };

        // WHEN
        processor.process(&first)?;
        processor.process(&continuation)?;

        // THEN
        let (_, output) = processor.into_parts();
        let expected = b"[00:00:00] We need a stable prefix\n[00:00:01] for every result\n";
        assert_eq!(output.into_inner(), (expected.to_vec(), expected.to_vec()));
        Ok(())
    }
}
