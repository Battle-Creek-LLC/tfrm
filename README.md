# tfrm

A Rust CLI for HCP Terraform / Terraform Enterprise: select a workspace,
list its runs, view and diff plans with sensitive-value redaction, and
apply VCS-triggered runs from the terminal — no `terraform` binary
required.

## Install

Prebuilt binaries (fastest, needs [cargo-binstall](https://github.com/cargo-bins/cargo-binstall)):

```sh
cargo binstall bcl-tfrm
```

Or download a release archive directly (Linux x86_64 shown; also
`aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`):

```sh
curl -LO https://github.com/Battle-Creek-LLC/tfrm/releases/latest/download/tfrm-x86_64-unknown-linux-gnu.tar.gz
curl -LO https://github.com/Battle-Creek-LLC/tfrm/releases/latest/download/tfrm-x86_64-unknown-linux-gnu.sha256
sha256sum -c tfrm-x86_64-unknown-linux-gnu.sha256
tar xzf tfrm-x86_64-unknown-linux-gnu.tar.gz
sudo install tfrm /usr/local/bin/
```

Or build from source: `cargo install bcl-tfrm --locked`.

## Quickstart

```sh
# 1. Authenticate (skip if `terraform login` already ran on this machine —
#    tfrm reads the same credentials, and TF_TOKEN_app_terraform_io works too).
tfrm login

# 2. Point tfrm at your org (commit .tfrm.toml to the repo):
cat > .tfrm.toml <<'TOML'
org = "my-org"
# hostname = "tfe.example.com"   # defaults to app.terraform.io
TOML

# 3. Pick a workspace (persisted in .tfrm/local.toml — gitignore it):
tfrm workspace list
tfrm workspace select my-workspace

# 4. Inspect runs:
tfrm runs list
tfrm runs show run-abc123
tfrm runs diff run-abc123 --against latest-applied

# 5. Apply a run awaiting confirmation (VCS-triggered):
tfrm runs apply run-abc123 -m "reviewed via tfrm"
```

`--format json` on `workspace list`, `runs list`, `runs show`, and
`runs diff` emits stable documented JSON ([docs/cli.md](docs/cli.md)).

Sensitive values never reach any output stream: plans render them as
`(sensitive)`, and diffs report at most `(sensitive — differs)` without
values — including under `--format json`.

## Documentation

- [Specification](docs/spec.md) — behavior, exit codes, requirements.
- [CLI JSON reference](docs/cli.md) — the `--format json` shapes.
- [DECISIONS.md](DECISIONS.md) — recorded implementation choices.
