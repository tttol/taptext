#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
compile_error!("TapText supports only Apple Silicon Macs");

mod audio;
mod capture;
mod cli;
mod model;
mod pipeline;
mod recognition;
mod transcript;

use std::{
    fs::OpenOptions,
    io::{self, BufWriter},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};

use capture::CaptureSession;
pub use cli::Cli;
use pipeline::process_chunk;
use recognition::WhisperTranscriber;
use transcript::TranscriptOutput;

pub fn execute(cli: Cli) -> Result<()> {
    let output_path = cli.output_path()?;
    let window_seconds = cli.window_seconds;
    let model_path = model::ensure_model()?;
    eprintln!("Whisperモデルを読み込んでいます: {}", model_path.display());
    let mut transcriber = WhisperTranscriber::load(&model_path)?;
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
    let mut output = TranscriptOutput::new(stdout.lock(), BufWriter::new(file));
    let stop_requested = Arc::new(AtomicBool::new(false));
    let signal_flag = Arc::clone(&stop_requested);
    ctrlc::set_handler(move || signal_flag.store(true, Ordering::Release))
        .context("Ctrl+Cハンドラーを登録できませんでした")?;
    eprintln!("システム音声の取得を開始します。終了するにはCtrl+Cを押してください");
    eprintln!("認識窓: {window_seconds}秒（1秒ごとに更新）");
    eprintln!("保存先: {}", output_path.display());
    let mut session = CaptureSession::start(window_seconds)?;
    let capture_result = capture_loop(&session, &stop_requested, &mut transcriber, &mut output);
    let stop_result = session.stop();
    capture_result?;
    stop_result?;
    if session.is_overloaded() {
        bail!("文字起こし処理が音声入力に追いつかなかったため停止しました");
    }
    while let Ok(chunk) = session.try_receive() {
        process_chunk(&mut transcriber, &mut output, &chunk)?;
    }
    eprintln!("文字起こしを保存しました: {}", output_path.display());
    Ok(())
}

fn capture_loop<T, O, F>(
    session: &CaptureSession,
    stop_requested: &AtomicBool,
    transcriber: &mut T,
    output: &mut TranscriptOutput<O, F>,
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
            Ok(chunk) => process_chunk(transcriber, output, &chunk)?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                bail!("音声分割処理が予期せず終了しました");
            }
        }
    }
}
