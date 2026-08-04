# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] — 2026-08-04

Promotes 0.1.0-rc.1. Everything under that release's Added list ships
here unchanged, plus:

### Fixed

- `runs list` and the `runs show` header now render only the commit
  subject line. The API returns the full multi-line commit message, and
  a Dependabot-style body previously spilled out of the table as raw
  lines after the row. `--format json` still carries the complete
  message.

## [0.1.0-rc.1] — 2026-08-04

### Added

- `tfrm login [HOST]` / `tfrm logout [HOST]` — browser OAuth (PKCE S256)
  with a paste fallback for headless sessions; tokens stored in
  terraform's own `~/.terraform.d/credentials.tfrc.json` (mode 0600,
  foreign hosts preserved), so credentials flow both ways with the
  terraform CLI without depending on it.
- `tfrm workspace list|select|current` — org workspace listing with
  current-run status, VCS repo, and latest-change-at; `select` verifies
  the name against the API before persisting to `.tfrm/local.toml`;
  `current` reports the resolved workspace and its source.
- `tfrm runs list` — recent runs with commit SHA (single-request
  include), source column, and a `>` marker on confirmable runs;
  `--status` and `--limit` filters.
- `tfrm runs show <RUN>` — full plan rendering from plan JSON: change
  summary, grouped resource changes, before→after attributes,
  replace-forcing marks, `(sensitive)` and `(known after apply)`
  redaction; degrades to summary counts with a warning when the token
  has write but not admin.
- `tfrm runs diff <A> [B]` — plan-pair diff by resource address with
  `--against latest-applied`, `--all`, `--exit-code`, and
  cross-workspace refusal. Sensitive attributes are compared in-process
  and never rendered: equal means omitted, different means
  `(sensitive — differs)`.
- `tfrm runs apply|discard|cancel` — confirmable-gated apply with a
  type-the-workspace-name prompt, apply-time plan summary, policy-check
  handling (`--override-policy` for soft-mandatory), log streaming, and
  `-m/--comment` on all three actions.
- `--format json` on the four read commands with a documented, pinned
  schema (`docs/cli.md`); progress moves to stderr under JSON.
- Terraform-compatible credential resolution: `--token` flag,
  `TF_TOKEN_<host>` env (terraform's exact host mangling), CLI config
  file and `credentials.tfrc.json`, with `credentials_helper` named in
  the error when configured but unsupported.
