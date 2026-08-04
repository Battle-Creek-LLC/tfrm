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
