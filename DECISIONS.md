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
