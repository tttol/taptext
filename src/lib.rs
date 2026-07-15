#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
compile_error!("TapText supports only Apple Silicon Macs");

mod audio;
mod capture;
mod cli;
mod model;
mod pipeline;
mod recognition;
mod transcript;
mod vad;

use std::{
    fs::OpenOptions,
    io::{self, BufWriter, IsTerminal},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};

use capture::CaptureSession;
pub use cli::Cli;
use pipeline::TranscriptProcessor;
use recognition::WhisperTranscriber;
use transcript::TranscriptOutput;

pub fn execute(cli: Cli) -> Result<()> {
    whisper_rs::install_logging_hooks();
    let output_path = cli.output_path()?;
    let models = model::ensure_models()?;
    eprintln!(
        "Whisperモデルを読み込んでいます: {}",
        models.recognition.display()
    );
    let transcriber = WhisperTranscriber::load(&models.recognition)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .with_context(|| {
            format!(
                "出力ファイルを新規作成できませんでした（既存ファイルは上書きしません）: {}",
                output_path.display()
            )
        })?;
    let stdout = io::stdout();
    let show_partials = stdout.is_terminal();
    let output = TranscriptOutput::new(stdout.lock(), BufWriter::new(file), show_partials);
    let mut processor = TranscriptProcessor::new(transcriber, output);
    let stop_requested = Arc::new(AtomicBool::new(false));
    let signal_flag = Arc::clone(&stop_requested);
    ctrlc::set_handler(move || signal_flag.store(true, Ordering::Release))
        .context("Ctrl+Cハンドラーを登録できませんでした")?;
    eprintln!("システム音声の取得を開始します。終了するにはCtrl+Cを押してください");
    eprintln!("VAD: Silero v6.2.0（発話中は1秒ごとに途中字幕を更新）");
    eprintln!("保存先: {}", output_path.display());
    let mut session = CaptureSession::start(&models.vad)?;
    let capture_result = capture_loop(&session, &stop_requested, &mut processor);
    let stop_result = session.stop();
    capture_result?;
    stop_result?;
    if session.is_overloaded() {
        bail!("文字起こし処理が音声入力に追いつかなかったため停止しました");
    }
    while let Ok(job) = session.try_receive() {
        processor.process(&job)?;
    }
    eprintln!("文字起こしを保存しました: {}", output_path.display());
    Ok(())
}

fn capture_loop<T, O, F>(
    session: &CaptureSession,
    stop_requested: &AtomicBool,
    processor: &mut TranscriptProcessor<T, O, F>,
) -> Result<()>
where
    T: recognition::Transcriber,
    O: io::Write,
    F: io::Write,
{
    loop {
        if stop_requested.load(Ordering::Acquire) {
            return Ok(());
        }
        if session.is_overloaded() {
            bail!("文字起こし処理が音声入力に追いつかなかったため停止しました");
        }
        if let Some(message) = session.error_message() {
            bail!("システム音声の取得中にエラーが発生しました: {message}");
        }
        match session.receive(Duration::from_millis(100)) {
            Ok(job) => processor.process(&job)?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(message) = session.error_message() {
                    bail!("VAD処理中にエラーが発生しました: {message}");
                }
                bail!("VAD処理が予期せず終了しました");
            }
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use std::collections::VecDeque;

    use anyhow::Result;

    use crate::{
        audio::{VadSegmenter, VoiceActivityDetector},
        pipeline::TranscriptProcessor,
        recognition::Transcriber,
        transcript::TranscriptOutput,
    };

    struct FakeVad {
        decisions: VecDeque<bool>,
    }

    impl VoiceActivityDetector for FakeVad {
        fn has_recent_speech(&mut self, _samples: &[f32]) -> Result<bool> {
            Ok(self.decisions.pop_front().unwrap_or(false))
        }
    }

    struct FakeTranscriber;

    impl Transcriber for FakeTranscriber {
        fn transcribe(&mut self, _samples: &[f32]) -> Result<Option<String>> {
            Ok(Some("Hello world".into()))
        }
    }

    #[test]
    fn saves_a_vad_utterance_to_terminal_and_file() -> Result<()> {
        // GIVEN
        let decisions = [false, true, true, true]
            .into_iter()
            .chain(std::iter::repeat_n(false, 7))
            .collect();
        let mut segmenter = VadSegmenter::new(FakeVad { decisions });
        let samples = vec![0.5; 1_600 * 11];
        let output = TranscriptOutput::new(Vec::new(), Vec::new(), false);
        let mut processor = TranscriptProcessor::new(FakeTranscriber, output);
        let expected = b"[00:00:00] Hello world\n".to_vec();

        // WHEN
        let jobs = segmenter.push(&samples)?;
        jobs.iter().try_for_each(|job| processor.process(job))?;

        // THEN
        let (_, output) = processor.into_parts();
        assert_eq!(output.into_inner(), (expected.clone(), expected));
        Ok(())
    }

    #[test]
    fn saves_active_speech_when_the_segmenter_is_flushed() -> Result<()> {
        // GIVEN
        let decisions = [true, true, true].into_iter().collect();
        let mut segmenter = VadSegmenter::new(FakeVad { decisions });
        let samples = vec![0.5; 1_600 * 3];
        let jobs = segmenter.push(&samples)?;
        assert!(jobs.is_empty());
        let output = TranscriptOutput::new(Vec::new(), Vec::new(), false);
        let mut processor = TranscriptProcessor::new(FakeTranscriber, output);
        let expected = b"[00:00:00] Hello world\n".to_vec();

        // WHEN
        let final_job = segmenter.flush();
        final_job
            .as_ref()
            .map(|job| processor.process(job))
            .transpose()?;

        // THEN
        let (_, output) = processor.into_parts();
        assert_eq!(output.into_inner(), (expected.clone(), expected));
        Ok(())
    }
}
