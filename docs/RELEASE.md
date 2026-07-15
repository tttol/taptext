# Releasing TapText

TapText releases use Semantic Versioning and lightweight Git tags in the `X.Y.Z` format.

## Create a release

1. Ensure the intended commit is the current `main` commit and the `Test` workflow is green.
2. Create and push a lightweight tag:

   ```sh
   git tag X.Y.Z
   git push origin X.Y.Z
   ```

3. Wait for the `Release` workflow to finish. It updates `Cargo.toml` and `Cargo.lock`, moves the tag to that version-bump commit, validates the project, and publishes the GitHub Release.
4. Download and extract `taptext-aarch64-apple-darwin.tar.gz` from the release, then verify it:

   ```sh
   ./taptext --version
   ```

## If a release fails

Fix the cause on `main`, then rerun the failed `Release` workflow for the same tag. Do not move the tag or create the version-bump commit manually.

The release workflow is idempotent: after it creates the version-bump commit, rerunning it reuses that commit and the existing tag.
