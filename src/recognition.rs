use std::path::Path;

use anyhow::{Context, Result};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use crate::{
    audio::AudioChunk,
    transcript::{TimedToken, TranscriptLine, line_from_tokens},
};

pub(crate) trait Transcriber {
    fn transcribe(&mut self, chunk: &AudioChunk) -> Result<Option<TranscriptLine>>;
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
    fn transcribe(&mut self, chunk: &AudioChunk) -> Result<Option<TranscriptLine>> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(self.threads);
        params.set_language(Some("en"));
        params.set_translate(false);
        params.set_no_context(true);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_token_timestamps(true);
        self.state
            .full(params, &chunk.samples)
            .context("Whisperの文字起こしに失敗しました")?;
        let tokens = self
            .state
            .as_iter()
            .flat_map(|segment| {
                (0..segment.n_tokens())
                    .filter_map(|index| segment.get_token(index))
                    .map(|token| {
                        let data = token.token_data();
                        token.to_str_lossy().map(|text| TimedToken {
                            start_centiseconds: data.t0,
                            text: text.into_owned(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Result<Vec<_>, _>>()
            .context("Whisperトークンを読み取れませんでした")?;
        Ok(line_from_tokens(
            chunk.start_centiseconds,
            chunk.discard_before_centiseconds,
            &tokens,
        ))
    }
}
