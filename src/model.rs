use std::{
    env, fs,
    io::{self, BufReader, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

const MODEL_FILE_NAME: &str = "ggml-base.en-q5_1.bin";
const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/c521a4b02f422512d734391fdf08bb08c0862f68/ggml-base.en-q5_1.bin";
const MODEL_SHA256: &str = "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f";

pub(crate) fn ensure_model() -> Result<PathBuf> {
    let home = env::var_os("HOME").context("HOME環境変数が設定されていません")?;
    let model_path = PathBuf::from(home)
        .join("Library/Caches/taptext/models")
        .join(MODEL_FILE_NAME);
    if model_path.exists() {
        verify_model(&model_path)?;
        return Ok(model_path);
    }
    confirm_download()?;
    download_model(&model_path)?;
    verify_model(&model_path)?;
    Ok(model_path)
}

fn confirm_download() -> Result<()> {
    if !io::stdin().is_terminal() {
        bail!("認識モデルがありません。対話可能な端末からtaptextを再実行してください");
    }
    eprint!("英語認識モデル（約60MB）を初回ダウンロードしますか？ [y/N] ");
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

fn download_model(model_path: &Path) -> Result<()> {
    let parent = model_path
        .parent()
        .context("モデル保存先の親ディレクトリがありません")?;
    fs::create_dir_all(parent).context("モデル保存ディレクトリを作成できませんでした")?;
    let partial_path = model_path.with_extension("bin.part");
    let status = Command::new("/usr/bin/curl")
        .args(["--fail", "--location", "--progress-bar", "--output"])
        .arg(&partial_path)
        .arg(MODEL_URL)
        .status()
        .context("macOS標準のcurlを起動できませんでした")?;
    if !status.success() {
        match fs::remove_file(&partial_path) {
            Ok(()) | Err(_) => {}
        }
        bail!("モデルのダウンロードに失敗しました: {MODEL_URL}");
    }
    verify_model(&partial_path).inspect_err(|_| {
        let _cleanup_result = fs::remove_file(&partial_path);
    })?;
    fs::rename(&partial_path, model_path)
        .context("検証済みモデルをキャッシュへ配置できませんでした")
}

fn verify_model(path: &Path) -> Result<()> {
    let file = fs::File::open(path)
        .with_context(|| format!("モデルを開けませんでした: {}", path.display()))?;
    verify_sha256(BufReader::new(file), MODEL_SHA256)
        .with_context(|| format!("モデルが破損しています: {}", path.display()))
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
