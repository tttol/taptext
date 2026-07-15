use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
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

use crate::{
    audio::{SAMPLE_RATE, UtteranceJob, UtteranceKind, VadSegmenter, VoiceActivityDetector},
    vad::SileroVad,
};

const RAW_AUDIO_QUEUE_CAPACITY: usize = 256;
const UTTERANCE_QUEUE_CAPACITY: usize = 4;

pub(crate) struct CaptureSession {
    stream: Option<SCStream>,
    jobs: Receiver<UtteranceJob>,
    worker: Option<JoinHandle<()>>,
    overloaded: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
}

impl CaptureSession {
    pub(crate) fn start(vad_model_path: &Path) -> Result<Self> {
        let detector = SileroVad::load(vad_model_path)?;
        let (audio_sender, audio_receiver) = mpsc::sync_channel(RAW_AUDIO_QUEUE_CAPACITY);
        let (job_sender, jobs) = mpsc::sync_channel(UTTERANCE_QUEUE_CAPACITY);
        let overloaded = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let worker = spawn_vad_worker(
            audio_receiver,
            job_sender,
            Arc::clone(&overloaded),
            Arc::clone(&error),
            detector,
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
            jobs,
            worker: Some(worker),
            overloaded,
            error,
        })
    }

    pub(crate) fn receive(&self, timeout: Duration) -> Result<UtteranceJob, RecvTimeoutError> {
        self.jobs.recv_timeout(timeout)
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

    pub(crate) fn stop_capture(&mut self) -> Result<()> {
        self.stream
            .take()
            .map(|stream| {
                let result = stream.stop_capture();
                drop(stream);
                result
            })
            .transpose()
            .context("システム音声の取得を停止できませんでした")?;
        Ok(())
    }

    pub(crate) fn drain_jobs(
        &self,
        mut process: impl FnMut(&UtteranceJob) -> Result<()>,
    ) -> Result<()> {
        let mut first_error = None;
        for job in &self.jobs {
            if first_error.is_none()
                && let Err(cause) = process(&job)
            {
                first_error = Some(cause);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(crate) fn finish_worker(&mut self) -> Result<()> {
        self.worker
            .take()
            .map(JoinHandle::join)
            .transpose()
            .map_err(|_| anyhow::anyhow!("音声分割スレッドが異常終了しました"))?;
        Ok(())
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        let _stop_result = self.stop_capture();
        let _drain_result = self.drain_jobs(|_| Ok(()));
        let _finish_result = self.finish_worker();
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

fn spawn_vad_worker<D>(
    audio_receiver: Receiver<Vec<f32>>,
    job_sender: SyncSender<UtteranceJob>,
    overloaded: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    detector: D,
) -> JoinHandle<()>
where
    D: VoiceActivityDetector + Send + 'static,
{
    thread::spawn(move || {
        let mut segmenter = VadSegmenter::new(detector);
        for samples in audio_receiver {
            let jobs = match segmenter.push(&samples) {
                Ok(jobs) => jobs,
                Err(cause) => {
                    record_error(&error, cause.to_string());
                    return;
                }
            };
            let send_result = jobs
                .into_iter()
                .try_for_each(|job| send_job(&job_sender, job, &overloaded));
            if send_result.is_err() {
                return;
            }
        }
        if let Some(job) = segmenter.flush() {
            let _send_result = send_job(&job_sender, job, &overloaded);
        }
    })
}

fn send_job(
    sender: &SyncSender<UtteranceJob>,
    job: UtteranceJob,
    overloaded: &AtomicBool,
) -> Result<(), ()> {
    match job.kind {
        UtteranceKind::Final => sender.send(job).map_err(|_| ()),
        UtteranceKind::Partial => match sender.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                overloaded.store(true, Ordering::Release);
                Err(())
            }
            Err(TrySendError::Disconnected(_)) => Err(()),
        },
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
    use std::sync::{Arc, Mutex, atomic::AtomicBool, mpsc};

    use anyhow::Result;

    use crate::audio::{UtteranceJob, UtteranceKind};

    use super::{CaptureSession, bytes_to_samples, send_job};

    fn job(kind: UtteranceKind) -> UtteranceJob {
        UtteranceJob {
            id: 0,
            kind,
            start_sample: 0,
            samples: Vec::new(),
            deduplicate_prefix: false,
        }
    }

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

    #[test]
    fn drains_queued_partials_before_joining_worker_and_preserves_final() -> Result<()> {
        // GIVEN
        let (sender, jobs) = mpsc::sync_channel(1);
        let overloaded = Arc::new(AtomicBool::new(false));
        let worker_overloaded = Arc::clone(&overloaded);
        let worker = std::thread::spawn(move || {
            assert!(send_job(&sender, job(UtteranceKind::Partial), &worker_overloaded).is_ok());
            assert!(send_job(&sender, job(UtteranceKind::Final), &worker_overloaded).is_ok());
        });
        let mut session = CaptureSession {
            stream: None,
            jobs,
            worker: Some(worker),
            overloaded,
            error: Arc::new(Mutex::new(None)),
        };
        let expected = vec![UtteranceKind::Partial, UtteranceKind::Final];
        let mut actual = Vec::new();

        // WHEN
        session.stop_capture()?;
        session.drain_jobs(|job| {
            actual.push(job.kind);
            Ok(())
        })?;
        session.finish_worker()?;

        // THEN
        assert_eq!(actual, expected);
        assert!(!session.is_overloaded());
        Ok(())
    }
}
