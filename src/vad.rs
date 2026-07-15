use std::path::Path;

use anyhow::{Context, Result};
use whisper_rs::{WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

use crate::audio::{SAMPLE_RATE, VoiceActivityDetector};

pub(crate) struct SileroVad {
    context: WhisperVadContext,
    params: WhisperVadParams,
}

impl SileroVad {
    pub(crate) fn load(model_path: &Path) -> Result<Self> {
        let model_path = model_path
            .to_str()
            .context("VADモデルのパスをUTF-8として扱えません")?;
        let mut context_params = WhisperVadContextParams::default();
        context_params.set_n_threads(1);
        context_params.set_use_gpu(false);
        let context = WhisperVadContext::new(model_path, context_params)
            .with_context(|| format!("VADモデルを読み込めませんでした: {model_path}"))?;
        let mut params = WhisperVadParams::default();
        params.set_threshold(0.5);
        params.set_min_speech_duration(100);
        params.set_min_silence_duration(100);
        params.set_max_speech_duration(15.0);
        params.set_speech_pad(0);
        params.set_samples_overlap(0.0);
        Ok(Self { context, params })
    }
}

impl VoiceActivityDetector for SileroVad {
    fn has_recent_speech(&mut self, samples: &[f32]) -> Result<bool> {
        let segments = self
            .context
            .segments_from_samples(self.params, samples)
            .context("VADによる発話判定に失敗しました")?;
        let window_end_centiseconds = samples.len() as f32 * 100.0 / SAMPLE_RATE as f32;
        let recent_threshold = (window_end_centiseconds - 20.0).max(0.0);
        Ok(segments
            .into_iter()
            .any(|segment| segment.end >= recent_threshold))
    }
}
