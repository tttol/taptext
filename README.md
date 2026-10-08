# TapText

TapText is an offline command-line application that transcribes all system audio playing on an Apple Silicon Mac. It captures audio with ScreenCaptureKit and runs the English Whisper `base.en-q5_1` model locally with Metal acceleration.

## Requirements

- Apple Silicon Mac
- macOS 26 or later
- Rust 1.96 or later
- Xcode Command Line Tools
- CMake at build time (`brew install cmake`)

## Build

```sh
cargo build --release
```

The executable is created at `target/release/taptext`.

## Usage

```sh
./target/release/taptext
./target/release/taptext --output transcript.txt
./target/release/taptext --version
```

## Install

Prebuilt binaries are available for Apple Silicon Macs running macOS 26 or later.

```sh
curl -LO https://github.com/tttol/taptext/releases/latest/download/taptext-aarch64-apple-darwin.tar.gz
tar -xzf taptext-aarch64-apple-darwin.tar.gz
./taptext --version
```

On the first launch, TapText asks before downloading the fixed, quantized English model (about 60 MB) and the Silero VAD model (about 1 MB) from the GitHub Release matching its version into `~/Library/Caches/taptext/models/`. It verifies both files with SHA-256. Later runs are completely offline. The installed application does not need access to Hugging Face.

If downloads are restricted, transfer `ggml-base.en-q5_1.bin` and `ggml-silero-v6.2.0.bin` from that release into the cache directory before launching TapText. Existing valid cached models are reused. Source builds with an empty cache need a published GitHub Release matching the version in `Cargo.toml`, or manually provisioned model files.

Model attribution and licenses are included in [docs/MODEL-LICENSES.txt](docs/MODEL-LICENSES.txt) and published alongside the model assets.

macOS asks for Screen & System Audio Recording permission on the first capture. Grant access to TapText in **System Settings > Privacy & Security > Screen & System Audio Recording**, then restart the command if macOS requests it.

Press `Ctrl+C` to stop. TapText finalizes any detected speech before closing the transcript. Existing output files are never overwritten.

Each line includes elapsed time:

```text
[00:00:05] Recognized English text.
```

TapText uses Silero VAD to detect complete utterances. While speech is active, it refreshes a stable partial transcript in an interactive terminal about once per second. Only the final utterance is appended to the UTF-8 text file, so provisional corrections do not create duplicate lines. Continuous speech is split after 15 seconds with a short boundary guard.

## Limitations

- English transcription only
- System audio only; microphone capture is not included
- Simultaneous applications are transcribed as one mixed stream
- DRM-protected audio that ScreenCaptureKit does not expose cannot be transcribed
- Local builds only; the binary is not signed or notarized
