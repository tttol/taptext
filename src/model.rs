use std::{
    env, fs,
    io::{self, BufReader, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
struct ModelSpec {
    label: &'static str,
    file_name: &'static str,
    url: &'static str,
    sha256: &'static str,
    approximate_size: &'static str,
}

const RECOGNITION_MODEL: ModelSpec = ModelSpec {
    label: "英語認識モデル",
    file_name: "ggml-base.en-q5_1.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/c521a4b02f422512d734391fdf08bb08c0862f68/ggml-base.en-q5_1.bin",
    sha256: "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f",
    approximate_size: "約60MB",
};
const VAD_MODEL: ModelSpec = ModelSpec {
    label: "VADモデル",
    file_name: "ggml-silero-v6.2.0.bin",
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/9ffd54a1e1ee413ddf265af9913beaf518d1639b/ggml-silero-v6.2.0.bin",
    sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
    approximate_size: "約1MB",
};

pub(crate) struct ModelPaths {
    pub(crate) recognition: PathBuf,
    pub(crate) vad: PathBuf,
}

pub(crate) fn ensure_models() -> Result<ModelPaths> {
    let home = env::var_os("HOME").context("HOME環境変数が設定されていません")?;
    let directory = PathBuf::from(home).join("Library/Caches/taptext/models");
    let targets = [RECOGNITION_MODEL, VAD_MODEL]
        .into_iter()
        .map(|spec| (spec, directory.join(spec.file_name)))
        .collect::<Vec<_>>();
    targets
        .iter()
        .filter(|(_, path)| path.exists())
        .try_for_each(|(spec, path)| verify_model(path, *spec))?;
    let missing = targets
        .iter()
        .filter(|(_, path)| !path.exists())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        confirm_download(&missing)?;
        missing
            .into_iter()
            .try_for_each(|(spec, path)| download_model(path, *spec))?;
    }
    Ok(ModelPaths {
        recognition: directory.join(RECOGNITION_MODEL.file_name),
        vad: directory.join(VAD_MODEL.file_name),
    })
}

fn confirm_download(missing: &[&(ModelSpec, PathBuf)]) -> Result<()> {
    let descriptions = missing
        .iter()
        .map(|(spec, _)| format!("{}（{}）", spec.label, spec.approximate_size))
        .collect::<Vec<_>>()
        .join("、");
    if !io::stdin().is_terminal() {
        bail!(
            "必要なモデルがありません（{descriptions}）。対話可能な端末からtaptextを再実行してください"
        );
    }
    eprint!("{descriptions}を初回ダウンロードしますか？ [y/N] ");
    io::stderr()
        .flush()
        .context("確認表示をflushできませんでした")?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("モデル取得の回答を読み取れませんでした")?;
    match answer.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(()),
        _ => bail!("モデルのダウンロードをキャンセルしました"),
    }
}

fn download_model(model_path: &Path, spec: ModelSpec) -> Result<()> {
    let parent = model_path
        .parent()
        .context("モデル保存先の親ディレクトリがありません")?;
    fs::create_dir_all(parent).context("モデル保存ディレクトリを作成できませんでした")?;
    let partial_path = model_path.with_extension("bin.part");
    let status = Command::new("/usr/bin/curl")
        .args(["--fail", "--location", "--progress-bar", "--output"])
        .arg(&partial_path)
        .arg(spec.url)
        .status()
        .context("macOS標準のcurlを起動できませんでした")?;
    if !status.success() {
        let _cleanup_result = fs::remove_file(&partial_path);
        bail!("{}のダウンロードに失敗しました: {}", spec.label, spec.url);
    }
    verify_model(&partial_path, spec).inspect_err(|_| {
        let _cleanup_result = fs::remove_file(&partial_path);
    })?;
    fs::rename(&partial_path, model_path)
        .with_context(|| format!("検証済み{}をキャッシュへ配置できませんでした", spec.label))
}

fn verify_model(path: &Path, spec: ModelSpec) -> Result<()> {
    let file = fs::File::open(path)
        .with_context(|| format!("{}を開けませんでした: {}", spec.label, path.display()))?;
    verify_sha256(BufReader::new(file), spec.sha256)
        .with_context(|| format!("{}が破損しています: {}", spec.label, path.display()))
}

fn verify_sha256(mut reader: impl Read, expected: &str) -> Result<()> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .context("SHA-256検証中に読み取りに失敗しました")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != expected {
        bail!("SHA-256が一致しません（expected: {expected}, actual: {actual}）");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::verify_sha256;

    #[test]
    fn accepts_matching_sha256() -> Result<()> {
        // GIVEN
        let input = b"abc".as_slice();
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

        // WHEN
        let actual = verify_sha256(input, expected);

        // THEN
        assert!(actual.is_ok());
        Ok(())
    }

    #[test]
    fn rejects_mismatched_sha256() {
        // GIVEN
        let input = b"abc".as_slice();
        let expected = "incorrect";

        // WHEN
        let actual = verify_sha256(input, expected);

        // THEN
        assert!(actual.is_err());
    }
}
