# Releasing c4tui

Phase 6 distribution is driven by `.github/workflows/release.yml`.

## Required secrets

Configure these repository secrets before pushing a release tag:

- `CARGO_REGISTRY_TOKEN` — crates.io API token with publish permission for `c4tui`.
- `HOMEBREW_TAP_TOKEN` — GitHub token that can create and push to `scshafe/homebrew-tap`.

## Release process

1. Ensure `Cargo.toml` has the intended version.
2. Run local gates:

   ```sh
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   cargo package
   ```

3. Push a semver tag:

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

The release workflow will:

- Build archives for:
  - `aarch64-apple-darwin`
  - `x86_64-apple-darwin`
  - `x86_64-unknown-linux-gnu`
  - `aarch64-unknown-linux-gnu`
- Attach archives and SHA-256 files to the GitHub Release.
- Publish `c4tui` to crates.io.
- Create/update `scshafe/homebrew-tap` so `brew install scshafe/tap/c4tui` works.

Manual `workflow_dispatch` runs only verify packaging and do not publish.
