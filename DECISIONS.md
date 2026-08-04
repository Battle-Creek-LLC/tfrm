# Decisions

Ambiguities resolved during the autonomous build. One dated bullet per
choice: what was picked and why. Precedent order: terraform's own behavior,
then `../secunit` conventions, then the conservative option (refuse with a
clear error over guessing).

- 2026-08-04 — `rust-toolchain.toml` pins `1.96.1` (installed stable),
  not `1.96.0`: the prompt asks for "1.96.0-era stable, exact version";
  1.96.1 is that era's current point release and is what this
  environment has installed, so the pin is reproducible locally and in
  CI without a toolchain downgrade.
- 2026-08-04 — `.tfrm/local.toml` writes anchor to the directory holding
  the discovered `.tfrm.toml` (else an existing `.tfrm/`, else the
  working directory), and discovery of both files walks to ancestors
  independently. The spec doesn't say where the selection lives when
  `select` runs in a subdirectory; anchoring to the project config makes
  one selection per project, matching how git resolves its dotfiles.
- 2026-08-04 — the local packaging gate for `bcl-tfrm` is
  `cargo package -p bcl-tfrm --no-verify --exclude-lockfile` (the
  prompt's `--no-verify` alone still resolves `tfrm-core` against the
  crates.io index while writing the packaged lockfile, and the crate is
  unpublished, so it fails). `--exclude-lockfile` skips exactly that
  step; the release workflow's `cargo publish --locked -p tfrm-core -p
  bcl-tfrm` publishes in dependency order, so the real publish never
  hits this.
- 2026-08-04 — release.yml adds a create-release job
  (taiki-e/create-gh-release-action) ahead of the binary matrix, which
  secunit does not have: it prevents the four matrix jobs from racing
  to create the release, and the action auto-detects `-rc.` versions as
  prereleases, which the plan requires for v0.1.0-rc.1.
