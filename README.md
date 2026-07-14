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
./target/release/taptext -w 5
./target/release/taptext --window-seconds 10
```

On the first launch, TapText asks before downloading the fixed, quantized English model into `~/Library/Caches/taptext/models/`. Later runs are completely offline.

macOS asks for Screen & System Audio Recording permission on the first capture. Grant access to TapText in **System Settings > Privacy & Security > Screen & System Audio Recording**, then restart the command if macOS requests it.

Press `Ctrl+C` to stop. TapText finishes any remaining audio of at least half a second before closing the transcript. Existing output files are never overwritten.

Each line includes elapsed time:

```text
[00:00:05] Recognized English text.
```

TapText uses a three-second recognition window by default and then transcribes once per second. Use `-w` or `--window-seconds` to select a window from 1 to 30 seconds. A longer window provides more speech context but increases the initial delay and inference cost. Each recognition result is written as one line without forced word-count wrapping.

## Limitations

- English transcription only
- System audio only; microphone capture is not included
- Simultaneous applications are transcribed as one mixed stream
- DRM-protected audio that ScreenCaptureKit does not expose cannot be transcribed
- Local builds only; the binary is not signed or notarized
