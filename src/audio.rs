#[derive(Debug, PartialEq)]
pub(crate) struct AudioChunk {
    pub(crate) samples: Vec<f32>,
    pub(crate) start_centiseconds: u64,
    pub(crate) discard_before_centiseconds: i64,
}

pub(crate) struct AudioChunker {
    sample_rate: usize,
    window_samples: usize,
    overlap_samples: usize,
    overlap_centiseconds: i64,
    minimum_flush_samples: usize,
    buffer: Vec<f32>,
    next_start_centiseconds: u64,
    emitted: bool,
}

impl AudioChunker {
    pub(crate) fn new(sample_rate: usize, window_seconds: u8) -> Self {
        let overlap_seconds = window_seconds.saturating_sub(1);
        let window_sample_seconds = usize::from(window_seconds);
        let overlap_sample_seconds = usize::from(overlap_seconds);
        Self {
            sample_rate,
            window_samples: sample_rate * window_sample_seconds,
            overlap_samples: sample_rate * overlap_sample_seconds,
            overlap_centiseconds: i64::from(overlap_seconds) * 100,
            minimum_flush_samples: sample_rate / 2,
            buffer: Vec::with_capacity(sample_rate * (window_sample_seconds + 1)),
            next_start_centiseconds: 0,
            emitted: false,
        }
    }

    pub(crate) fn push(&mut self, samples: &[f32]) -> Vec<AudioChunk> {
        self.buffer.extend_from_slice(samples);
        let step_samples = self.window_samples - self.overlap_samples;
        let step_centiseconds = (step_samples * 100 / self.sample_rate) as u64;
        std::iter::from_fn(|| {
            (self.buffer.len() >= self.window_samples).then(|| {
                let chunk = AudioChunk {
                    samples: self.buffer[..self.window_samples].to_vec(),
                    start_centiseconds: self.next_start_centiseconds,
                    discard_before_centiseconds: i64::from(self.emitted)
                        * self.overlap_centiseconds,
                };
                self.buffer.drain(..step_samples);
                self.next_start_centiseconds += step_centiseconds;
                self.emitted = true;
                chunk
            })
        })
        .collect()
    }

    pub(crate) fn flush(&mut self) -> Option<AudioChunk> {
        let retained_overlap = usize::from(self.emitted) * self.overlap_samples;
        let new_sample_count = self.buffer.len().saturating_sub(retained_overlap);
        (new_sample_count >= self.minimum_flush_samples).then(|| AudioChunk {
            samples: std::mem::take(&mut self.buffer),
            start_centiseconds: self.next_start_centiseconds,
            discard_before_centiseconds: i64::from(self.emitted) * self.overlap_centiseconds,
        })
    }
}

pub(crate) fn is_silent(samples: &[f32]) -> bool {
    const SILENCE_RMS_THRESHOLD: f64 = 0.003_162_277_660_168_379_4;
    let mean_square = samples
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum::<f64>()
        / samples.len().max(1) as f64;
    mean_square.sqrt() < SILENCE_RMS_THRESHOLD
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{AudioChunker, is_silent};

    #[test]
    fn creates_overlapping_three_second_chunks_every_second() {
        // GIVEN
        let mut chunker = AudioChunker::new(10, 3);
        let samples = (0..50).map(|value| value as f32).collect::<Vec<_>>();

        // WHEN
        let actual = chunker.push(&samples);

        // THEN
        let expected = vec![
            (samples[0..30].to_vec(), 0, 0),
            (samples[10..40].to_vec(), 100, 200),
            (samples[20..50].to_vec(), 200, 200),
        ];
        let actual = actual
            .into_iter()
            .map(|chunk| {
                (
                    chunk.samples,
                    chunk.start_centiseconds,
                    chunk.discard_before_centiseconds,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn creates_configured_five_second_chunks_every_second() {
        // GIVEN
        let mut chunker = AudioChunker::new(10, 5);
        let samples = (0..70).map(|value| value as f32).collect::<Vec<_>>();
        let expected = vec![
            (samples[0..50].to_vec(), 0, 0),
            (samples[10..60].to_vec(), 100, 400),
            (samples[20..70].to_vec(), 200, 400),
        ];

        // WHEN
        let actual = chunker
            .push(&samples)
            .into_iter()
            .map(|chunk| {
                (
                    chunk.samples,
                    chunk.start_centiseconds,
                    chunk.discard_before_centiseconds,
                )
            })
            .collect::<Vec<_>>();

        // THEN
        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case(vec![0.0; 20], true)]
    #[case(vec![0.01; 20], false)]
    fn classifies_silence(#[case] samples: Vec<f32>, #[case] expected: bool) {
        // GIVEN

        // WHEN
        let actual = is_silent(&samples);

        // THEN
        assert_eq!(actual, expected);
    }

    #[test]
    fn flushes_remaining_half_second_or_more() {
        // GIVEN
        let mut chunker = AudioChunker::new(10, 3);
        let samples = vec![0.5; 25];
        let expected = samples.clone();
        let chunks = chunker.push(&samples);
        assert!(chunks.is_empty());

        // WHEN
        let actual = chunker.flush();

        // THEN
        assert_eq!(actual.map(|chunk| chunk.samples), Some(expected));
    }
}
