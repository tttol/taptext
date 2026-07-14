use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use screencapturekit::{
    cm::{CMSampleBuffer, CMSampleBufferExt},
    prelude::{SCContentFilter, SCShareableContent, SCStreamConfiguration, SCStreamOutputType},
    stream::{SCStream, delegate_trait::StreamCallbacks},
};

use crate::audio::{AudioChunk, AudioChunker};

const SAMPLE_RATE: usize = 16_000;
const RAW_AUDIO_QUEUE_CAPACITY: usize = 256;
const CHUNK_QUEUE_CAPACITY: usize = 2;

pub(crate) struct CaptureSession {
    stream: Option<SCStream>,
    chunks: Receiver<AudioChunk>,
    worker: Option<JoinHandle<()>>,
    overloaded: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
}

impl CaptureSession {
    pub(crate) fn start(window_seconds: u8) -> Result<Self> {
        let (audio_sender, audio_receiver) = mpsc::sync_channel(RAW_AUDIO_QUEUE_CAPACITY);
        let (chunk_sender, chunks) = mpsc::sync_channel(CHUNK_QUEUE_CAPACITY);
        let overloaded = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let worker = spawn_chunk_worker(
            audio_receiver,
            chunk_sender,
            Arc::clone(&overloaded),
            window_seconds,
        );
        let mut stream = create_stream(Arc::clone(&error))?;
        let handler_registered = stream.add_output_handler(
            create_audio_handler(audio_sender, Arc::clone(&overloaded), Arc::clone(&error)),
            SCStreamOutputType::Audio,
        );
        if handler_registered.is_none() {
            bail!("ScreenCaptureKitへ音声ハンドラーを登録できませんでした");
        }
        if let Err(cause) = stream.start_capture() {
            drop(stream);
            let _join_result = worker.join();
            return Err(anyhow::anyhow!(cause)).context(permission_help());
        }
        Ok(Self {
            stream: Some(stream),
            chunks,
            worker: Some(worker),
            overloaded,
            error,
        })
    }

    pub(crate) fn receive(&self, timeout: Duration) -> Result<AudioChunk, RecvTimeoutError> {
        self.chunks.recv_timeout(timeout)
    }

    pub(crate) fn try_receive(&self) -> Result<AudioChunk, TryRecvError> {
        self.chunks.try_recv()
    }

    pub(crate) fn is_overloaded(&self) -> bool {
        self.overloaded.load(Ordering::Acquire)
    }

    pub(crate) fn error_message(&self) -> Option<String> {
        self.error.lock().map_or_else(
            |poisoned| poisoned.into_inner().clone(),
            |error| error.clone(),
        )
    }

    pub(crate) fn stop(&mut self) -> Result<()> {
        let stop_result = self
            .stream
            .take()
            .map(|stream| {
                let result = stream.stop_capture();
                drop(stream);
                result
            })
            .transpose();
        let join_result = self.worker.take().map(JoinHandle::join).transpose();
        stop_result.context("システム音声の取得を停止できませんでした")?;
        join_result.map_err(|_| anyhow::anyhow!("音声分割スレッドが異常終了しました"))?;
        Ok(())
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _stop_result = stream.stop_capture();
            drop(stream);
        }
        if let Some(worker) = self.worker.take() {
            let _join_result = worker.join();
        }
    }
}

fn create_stream(error: Arc<Mutex<Option<String>>>) -> Result<SCStream> {
    let content = SCShareableContent::get().with_context(permission_help)?;
    let displays = content.displays();
    let display = displays
        .first()
        .context("取得可能なディスプレイが見つかりません")?;
    let filter = SCContentFilter::create()
        .with_display(display)
        .with_excluding_windows(&[])
        .try_build()
        .context("Mac全体を対象とする取得フィルターを作成できませんでした")?;
    let configuration = SCStreamConfiguration::new()
        .with_width(2)
        .with_height(2)
        .with_captures_audio(true)
        .with_sample_rate(SAMPLE_RATE as i32)
        .with_channel_count(1)
        .with_excludes_current_process_audio(true);
    let delegate_error = Arc::clone(&error);
    let delegate = StreamCallbacks::new().on_error(move |cause| {
        record_error(&delegate_error, format!("ScreenCaptureKit: {cause}"));
    });
    Ok(SCStream::new_with_delegate(
        &filter,
        &configuration,
        delegate,
    ))
}

fn create_audio_handler(
    sender: SyncSender<Vec<f32>>,
    overloaded: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
) -> impl Fn(CMSampleBuffer, SCStreamOutputType) + Send + Sync + 'static {
    move |sample, output_type| {
        if output_type != SCStreamOutputType::Audio {
            return;
        }
        match samples_from_buffer(&sample) {
            Ok(samples) if !samples.is_empty() => match sender.try_send(samples) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    overloaded.store(true, Ordering::Release);
                }
                Err(TrySendError::Disconnected(_)) => {}
            },
            Ok(_) => {}
            Err(cause) => record_error(&error, cause.to_string()),
        }
    }
}

fn spawn_chunk_worker(
    audio_receiver: Receiver<Vec<f32>>,
    chunk_sender: SyncSender<AudioChunk>,
    overloaded: Arc<AtomicBool>,
    window_seconds: u8,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut chunker = AudioChunker::new(SAMPLE_RATE, window_seconds);
        for samples in audio_receiver {
            let send_result = chunker
                .push(&samples)
                .into_iter()
                .try_for_each(|chunk| send_chunk(&chunk_sender, chunk, &overloaded));
            if send_result.is_err() {
                return;
            }
        }
        if let Some(chunk) = chunker.flush() {
            let _send_result = send_chunk(&chunk_sender, chunk, &overloaded);
        }
    })
}

fn send_chunk(
    sender: &SyncSender<AudioChunk>,
    chunk: AudioChunk,
    overloaded: &AtomicBool,
) -> Result<(), ()> {
    match sender.try_send(chunk) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            overloaded.store(true, Ordering::Release);
            Err(())
        }
        Err(TrySendError::Disconnected(_)) => Err(()),
    }
}

fn samples_from_buffer(sample: &CMSampleBuffer) -> Result<Vec<f32>> {
    let buffers = sample
        .audio_buffer_list()
        .context("ScreenCaptureKitの音声バッファが空です")?;
    let buffer = buffers
        .buffer(0)
        .context("ScreenCaptureKitの先頭音声チャンネルがありません")?;
    bytes_to_samples(buffer.data())
}

fn bytes_to_samples(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(size_of::<f32>()) {
        bail!("音声バッファのバイト数がFloat32境界と一致しません");
    }
    Ok(bytes
        .chunks_exact(size_of::<f32>())
        .map(|chunk| f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

fn record_error(slot: &Mutex<Option<String>>, message: String) {
    match slot.lock() {
        Ok(mut error) => {
            if error.is_none() {
                *error = Some(message);
            }
        }
        Err(mut poisoned) => {
            **poisoned.get_mut() = Some(message);
        }
    }
}

fn permission_help() -> &'static str {
    "システム音声を取得できません。システム設定 > プライバシーとセキュリティ > 画面収録とシステムオーディオでTapTextを許可し、コマンドを再実行してください"
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::bytes_to_samples;

    #[test]
    fn converts_native_endian_float_audio() -> Result<()> {
        // GIVEN
        let expected = vec![0.25_f32, -0.5_f32];
        let bytes = expected
            .iter()
            .flat_map(|sample| sample.to_ne_bytes())
            .collect::<Vec<_>>();

        // WHEN
        let actual = bytes_to_samples(&bytes)?;

        // THEN
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn rejects_misaligned_float_audio() {
        // GIVEN
        let bytes = [0_u8; 3];

        // WHEN
        let actual = bytes_to_samples(&bytes);

        // THEN
        assert!(actual.is_err());
    }
}
