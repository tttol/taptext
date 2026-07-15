use std::collections::VecDeque;

use anyhow::Result;

pub(crate) const SAMPLE_RATE: usize = 16_000;
const ANALYSIS_STEP_SAMPLES: usize = SAMPLE_RATE / 10;
const ANALYSIS_WINDOW_SAMPLES: usize = SAMPLE_RATE;
const PRE_ROLL_SAMPLES: usize = SAMPLE_RATE * 3 / 10;
const SPEECH_CONFIRMATION_STEPS: usize = 2;
const SILENCE_CONFIRMATION_STEPS: usize = 7;
const PARTIAL_INTERVAL_SAMPLES: usize = SAMPLE_RATE;
const MAX_UTTERANCE_SAMPLES: usize = SAMPLE_RATE * 15;

pub(crate) trait VoiceActivityDetector {
    fn has_recent_speech(&mut self, samples: &[f32]) -> Result<bool>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UtteranceKind {
    Partial,
    Final,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UtteranceJob {
    pub(crate) id: u64,
    pub(crate) kind: UtteranceKind,
    pub(crate) start_sample: u64,
    pub(crate) samples: Vec<f32>,
    pub(crate) deduplicate_prefix: bool,
}

enum VadState {
    Idle {
        pre_roll: VecDeque<f32>,
        speech_steps: usize,
    },
    Speaking {
        id: u64,
        start_sample: u64,
        samples: Vec<f32>,
        silence_steps: usize,
        next_partial_sample_count: usize,
        deduplicate_prefix: bool,
    },
}

pub(crate) struct VadSegmenter<D> {
    detector: D,
    state: VadState,
    pending: Vec<f32>,
    analysis: VecDeque<f32>,
    processed_samples: u64,
    next_utterance_id: u64,
}

impl<D> VadSegmenter<D>
where
    D: VoiceActivityDetector,
{
    pub(crate) fn new(detector: D) -> Self {
        Self {
            detector,
            state: VadState::Idle {
                pre_roll: VecDeque::with_capacity(PRE_ROLL_SAMPLES),
                speech_steps: 0,
            },
            pending: Vec::with_capacity(ANALYSIS_STEP_SAMPLES * 2),
            analysis: VecDeque::with_capacity(ANALYSIS_WINDOW_SAMPLES),
            processed_samples: 0,
            next_utterance_id: 0,
        }
    }

    pub(crate) fn push(&mut self, samples: &[f32]) -> Result<Vec<UtteranceJob>> {
        self.pending.extend_from_slice(samples);
        let step_count = self.pending.len() / ANALYSIS_STEP_SAMPLES;
        let processed_count = step_count * ANALYSIS_STEP_SAMPLES;
        let steps = self.pending[..processed_count]
            .chunks_exact(ANALYSIS_STEP_SAMPLES)
            .map(<[f32]>::to_vec)
            .collect::<Vec<_>>();
        self.pending.drain(..processed_count);
        steps
            .into_iter()
            .map(|step| self.process_step(&step))
            .collect::<Result<Vec<_>>>()
            .map(|jobs| jobs.into_iter().flatten().collect())
    }

    pub(crate) fn flush(&mut self) -> Option<UtteranceJob> {
        let pending = std::mem::take(&mut self.pending);
        match std::mem::replace(
            &mut self.state,
            VadState::Idle {
                pre_roll: VecDeque::with_capacity(PRE_ROLL_SAMPLES),
                speech_steps: 0,
            },
        ) {
            VadState::Speaking {
                id,
                start_sample,
                mut samples,
                deduplicate_prefix,
                ..
            } => {
                samples.extend_from_slice(&pending);
                Some(UtteranceJob {
                    id,
                    kind: UtteranceKind::Final,
                    start_sample,
                    samples,
                    deduplicate_prefix,
                })
            }
            VadState::Idle { .. } => None,
        }
    }

    fn process_step(&mut self, step: &[f32]) -> Result<Vec<UtteranceJob>> {
        self.analysis.extend(step.iter().copied());
        if self.analysis.len() > ANALYSIS_WINDOW_SAMPLES {
            self.analysis
                .drain(..self.analysis.len() - ANALYSIS_WINDOW_SAMPLES);
        }
        let analysis = self.analysis.iter().copied().collect::<Vec<_>>();
        let has_speech = self.detector.has_recent_speech(&analysis)?;
        self.processed_samples += step.len() as u64;
        Ok(self.transition(step, has_speech))
    }

    fn transition(&mut self, step: &[f32], has_speech: bool) -> Vec<UtteranceJob> {
        let state = std::mem::replace(
            &mut self.state,
            VadState::Idle {
                pre_roll: VecDeque::with_capacity(PRE_ROLL_SAMPLES),
                speech_steps: 0,
            },
        );
        let (next_state, jobs) = match state {
            VadState::Idle {
                mut pre_roll,
                speech_steps,
            } => {
                retain_tail(&mut pre_roll, step, PRE_ROLL_SAMPLES);
                let next_speech_steps = if has_speech { speech_steps + 1 } else { 0 };
                if next_speech_steps < SPEECH_CONFIRMATION_STEPS {
                    (
                        VadState::Idle {
                            pre_roll,
                            speech_steps: next_speech_steps,
                        },
                        Vec::new(),
                    )
                } else {
                    let id = self.next_utterance_id;
                    self.next_utterance_id += 1;
                    let samples = pre_roll.into_iter().collect::<Vec<_>>();
                    let start_sample = self.processed_samples.saturating_sub(samples.len() as u64);
                    (
                        VadState::Speaking {
                            id,
                            start_sample,
                            samples,
                            silence_steps: 0,
                            next_partial_sample_count: PARTIAL_INTERVAL_SAMPLES,
                            deduplicate_prefix: false,
                        },
                        Vec::new(),
                    )
                }
            }
            VadState::Speaking {
                id,
                start_sample,
                mut samples,
                silence_steps,
                mut next_partial_sample_count,
                deduplicate_prefix,
            } => {
                samples.extend_from_slice(step);
                let next_silence_steps = if has_speech { 0 } else { silence_steps + 1 };
                if next_silence_steps >= SILENCE_CONFIRMATION_STEPS {
                    let trailing_silence = SILENCE_CONFIRMATION_STEPS * ANALYSIS_STEP_SAMPLES;
                    let keep_silence = PRE_ROLL_SAMPLES.min(trailing_silence);
                    samples.truncate(
                        samples
                            .len()
                            .saturating_sub(trailing_silence - keep_silence),
                    );
                    let final_job = UtteranceJob {
                        id,
                        kind: UtteranceKind::Final,
                        start_sample,
                        samples,
                        deduplicate_prefix,
                    };
                    let pre_roll = self
                        .analysis
                        .iter()
                        .rev()
                        .take(PRE_ROLL_SAMPLES)
                        .copied()
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    (
                        VadState::Idle {
                            pre_roll,
                            speech_steps: 0,
                        },
                        vec![final_job],
                    )
                } else if samples.len() >= MAX_UTTERANCE_SAMPLES {
                    let carry_start = samples.len().saturating_sub(PRE_ROLL_SAMPLES);
                    let carry = samples[carry_start..].to_vec();
                    let final_job = UtteranceJob {
                        id,
                        kind: UtteranceKind::Final,
                        start_sample,
                        samples,
                        deduplicate_prefix,
                    };
                    let next_state = if has_speech {
                        let next_id = self.next_utterance_id;
                        self.next_utterance_id += 1;
                        VadState::Speaking {
                            id: next_id,
                            start_sample: self.processed_samples.saturating_sub(carry.len() as u64),
                            samples: carry,
                            silence_steps: next_silence_steps,
                            next_partial_sample_count: PARTIAL_INTERVAL_SAMPLES,
                            deduplicate_prefix: true,
                        }
                    } else {
                        VadState::Idle {
                            pre_roll: carry.into_iter().collect(),
                            speech_steps: 0,
                        }
                    };
                    (next_state, vec![final_job])
                } else {
                    let jobs = if samples.len() >= next_partial_sample_count {
                        next_partial_sample_count += PARTIAL_INTERVAL_SAMPLES;
                        vec![UtteranceJob {
                            id,
                            kind: UtteranceKind::Partial,
                            start_sample,
                            samples: samples.clone(),
                            deduplicate_prefix,
                        }]
                    } else {
                        Vec::new()
                    };
                    (
                        VadState::Speaking {
                            id,
                            start_sample,
                            samples,
                            silence_steps: next_silence_steps,
                            next_partial_sample_count,
                            deduplicate_prefix,
                        },
                        jobs,
                    )
                }
            }
        };
        self.state = next_state;
        jobs
    }
}

fn retain_tail(buffer: &mut VecDeque<f32>, samples: &[f32], capacity: usize) {
    buffer.extend(samples.iter().copied());
    if buffer.len() > capacity {
        buffer.drain(..buffer.len() - capacity);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use anyhow::Result;

    use super::{
        ANALYSIS_STEP_SAMPLES, MAX_UTTERANCE_SAMPLES, PRE_ROLL_SAMPLES, UtteranceKind,
        VadSegmenter, VoiceActivityDetector,
    };

    struct FakeVad {
        decisions: VecDeque<bool>,
    }

    impl FakeVad {
        fn new(decisions: impl IntoIterator<Item = bool>) -> Self {
            Self {
                decisions: decisions.into_iter().collect(),
            }
        }
    }

    impl VoiceActivityDetector for FakeVad {
        fn has_recent_speech(&mut self, _samples: &[f32]) -> Result<bool> {
            Ok(self.decisions.pop_front().unwrap_or(false))
        }
    }

    #[test]
    fn includes_three_hundred_milliseconds_before_confirmed_speech() -> Result<()> {
        // GIVEN
        let detector = FakeVad::new([false, true, true]);
        let mut segmenter = VadSegmenter::new(detector);
        let samples = vec![0.5; ANALYSIS_STEP_SAMPLES * 3];

        // WHEN
        let jobs = segmenter.push(&samples)?;
        let actual = segmenter.flush();

        // THEN
        assert!(jobs.is_empty());
        assert_eq!(actual.map(|job| job.samples.len()), Some(PRE_ROLL_SAMPLES));
        Ok(())
    }

    #[test]
    fn ends_speech_after_seven_hundred_milliseconds_of_silence() -> Result<()> {
        // GIVEN
        let detector = FakeVad::new(
            [true, true]
                .into_iter()
                .chain(std::iter::repeat_n(false, 7)),
        );
        let mut segmenter = VadSegmenter::new(detector);
        let samples = vec![0.5; ANALYSIS_STEP_SAMPLES * 9];
        let expected_length = ANALYSIS_STEP_SAMPLES * 2 + PRE_ROLL_SAMPLES;

        // WHEN
        let actual = segmenter.push(&samples)?;

        // THEN
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].kind, UtteranceKind::Final);
        assert_eq!(actual[0].samples.len(), expected_length);
        Ok(())
    }

    #[test]
    fn preserves_short_silence_inside_an_utterance() -> Result<()> {
        // GIVEN
        let detector = FakeVad::new(
            [true, true]
                .into_iter()
                .chain(std::iter::repeat_n(false, 6))
                .chain([true]),
        );
        let mut segmenter = VadSegmenter::new(detector);
        let samples = (0..ANALYSIS_STEP_SAMPLES * 9)
            .map(|value| value as f32)
            .collect::<Vec<_>>();

        // WHEN
        let jobs = segmenter.push(&samples)?;
        let actual = segmenter.flush();

        // THEN
        assert!(jobs.is_empty());
        assert_eq!(actual.map(|job| job.samples), Some(samples));
        Ok(())
    }

    #[test]
    fn emits_partial_jobs_once_per_second() -> Result<()> {
        // GIVEN
        let steps = MAX_UTTERANCE_SAMPLES / ANALYSIS_STEP_SAMPLES - 1;
        let detector = FakeVad::new(std::iter::repeat_n(true, steps));
        let mut segmenter = VadSegmenter::new(detector);
        let samples = vec![0.5; ANALYSIS_STEP_SAMPLES * steps];

        // WHEN
        let actual = segmenter.push(&samples)?;

        // THEN
        let partials = actual
            .iter()
            .filter(|job| job.kind == UtteranceKind::Partial)
            .count();
        assert_eq!(partials, 14);
        Ok(())
    }

    #[test]
    fn splits_continuous_speech_at_fifteen_seconds() -> Result<()> {
        // GIVEN
        let steps = MAX_UTTERANCE_SAMPLES / ANALYSIS_STEP_SAMPLES;
        let detector = FakeVad::new(std::iter::repeat_n(true, steps));
        let mut segmenter = VadSegmenter::new(detector);
        let samples = vec![0.5; MAX_UTTERANCE_SAMPLES];

        // WHEN
        let actual = segmenter.push(&samples)?;

        // THEN
        let final_job = actual.iter().find(|job| job.kind == UtteranceKind::Final);
        assert_eq!(
            final_job.map(|job| job.samples.len()),
            Some(MAX_UTTERANCE_SAMPLES)
        );
        Ok(())
    }

    #[test]
    fn flushes_detected_speech_on_shutdown() -> Result<()> {
        // GIVEN
        let detector = FakeVad::new([true, true, true]);
        let mut segmenter = VadSegmenter::new(detector);
        let samples = vec![0.5; ANALYSIS_STEP_SAMPLES * 3];
        let jobs = segmenter.push(&samples)?;
        assert!(jobs.is_empty());

        // WHEN
        let actual = segmenter.flush();

        // THEN
        assert_eq!(actual.map(|job| job.kind), Some(UtteranceKind::Final));
        Ok(())
    }
}
