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

On the first launch, TapText asks before downloading the fixed, quantized English model and the Silero VAD model into `~/Library/Caches/taptext/models/`. Later runs are completely offline.

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
