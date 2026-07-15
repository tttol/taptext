use std::path::Path;

use anyhow::{Context, Result};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use crate::transcript::filtered_text;

pub(crate) trait Transcriber {
    fn transcribe(&mut self, samples: &[f32]) -> Result<Option<String>>;
}

pub(crate) struct WhisperTranscriber {
    state: WhisperState,
    threads: i32,
}

impl WhisperTranscriber {
    pub(crate) fn load(model_path: &Path) -> Result<Self> {
        let context =
            WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
                .with_context(|| {
                    format!(
                        "Whisperモデルを読み込めませんでした: {}",
                        model_path.display()
                    )
                })?;
        let state = context
            .create_state()
            .context("Whisper推論状態を初期化できませんでした")?;
        let available = std::thread::available_parallelism().map_or(4, usize::from);
        let threads = i32::try_from(available.saturating_sub(2).max(1)).unwrap_or(i32::MAX);
        Ok(Self { state, threads })
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&mut self, samples: &[f32]) -> Result<Option<String>> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(self.threads);
        params.set_language(Some("en"));
        params.set_translate(false);
        params.set_no_context(true);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_token_timestamps(false);
        self.state
            .full(params, samples)
            .context("Whisperの文字起こしに失敗しました")?;
        let tokens = self
            .state
            .as_iter()
            .flat_map(|segment| {
                (0..segment.n_tokens())
                    .filter_map(|index| segment.get_token(index))
                    .map(|token| token.to_str_lossy().map(|text| text.into_owned()))
                    .collect::<Vec<_>>()
            })
            .collect::<Result<Vec<_>, _>>()
            .context("Whisperトークンを読み取れませんでした")?;
        Ok(filtered_text(tokens.iter().map(String::as_str)))
    }
}
