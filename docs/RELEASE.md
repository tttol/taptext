# Releasing TapText

TapText releases use Semantic Versioning and lightweight Git tags in the `X.Y.Z` format.

## Create a release

1. Ensure the intended commit is the current `main` commit and the `Test` workflow is green.
2. Create and push a lightweight tag:

   ```sh
   git tag X.Y.Z
   git push origin X.Y.Z
   ```

3. Wait for the `Release` workflow to finish. It updates `Cargo.toml` and `Cargo.lock`, moves the tag to that version-bump commit, validates the project, and publishes the GitHub Release. It also downloads the pinned upstream models from Hugging Face, verifies their SHA-256 hashes, and uploads both model files and `MODEL-LICENSES.txt` as release assets. Only the release runner needs Hugging Face access; TapText users download the models from GitHub.
4. Download and extract `taptext-aarch64-apple-darwin.tar.gz` from the release, then verify it:

   ```sh
   ./taptext --version
   ```

5. Confirm the release contains `ggml-base.en-q5_1.bin`, `ggml-silero-v6.2.0.bin`, and `MODEL-LICENSES.txt`. The released binary downloads the models from its own version's release, never from `latest`.

## Update the models

Keep the pinned upstream URLs and SHA-256 hashes in `.github/workflows/release.yml` synchronized with the filenames and hashes in `src/model.rs`. Update `docs/MODEL-LICENSES.txt` when attribution changes. Do not replace model assets on older releases with different files: older binaries require the original hashes. Model binaries are release assets and should not be committed to Git.

## If a release fails

Fix the cause on `main`, then rerun the failed `Release` workflow for the same tag. Do not move the tag or create the version-bump commit manually.

The release workflow is idempotent: after it creates the version-bump commit, rerunning it reuses that commit and the existing tag.
