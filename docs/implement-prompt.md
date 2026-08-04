# Prompt — implement tfrm end to end

Implement every job in [`docs/plan.md`](plan.md) (J0.1 through J5.3), in
order, until the repository holds a released `v0.1.0-rc.1` prerelease.
[`docs/spec.md`](spec.md) is normative for behavior; the plan is normative
for job scope and done-ness. Work autonomously start to finish.

## Hard constraints

1. **Never block on a human.** No credentials, approvals, or answers will
   arrive mid-run. When the spec is ambiguous, pick the behavior terraform
   itself or `../secunit` exhibits, implement it, and record the choice in
   `DECISIONS.md` (one dated bullet: choice + why). If neither has a
   precedent, pick the conservative option (refuse + clear error over
   guessing) and record it.
2. **The redaction invariant is non-negotiable.** No sensitive value may
   reach stdout, stderr, JSON output, or error text — this is the tool's
   core promise (spec R5.3/R6.4). The `SENTINEL-DO-NOT-PRINT` fixture test
   from the plan's test strategy must exist from J2.3 on and pass for every
   renderer. Do not mark any Phase 2+ job done without it.
3. **No live HCP Terraform calls.** This environment has no TFC token and no
   terraform binary; tests run against wiremock and `testdata/` fixtures
   only. Every "Verify (live)" step in the plan goes into `VERIFY.md` as a
   checkbox with the exact command and the observable result the human
   should see. Never invent a live result.
4. **No crates.io publish.** The `CRATES_IO_TOKEN` secret does not exist;
   `release.yml` must therefore keep the publish job from failing the
   workflow when the secret is absent (skip with a visible notice —
   secunit's workflow assumes the secret; adapt it). Local gate instead:
   `cargo publish --dry-run -p tfrm-core` must pass. (`bcl-tfrm` depends on
   the unpublished `tfrm-core`, so its dry-run cannot fully resolve; verify
   it with `cargo package -p bcl-tfrm --no-verify` and record the pair of
   commands in VERIFY.md.)
5. **A job is done only when its plan Test gate passes in CI.** Commit per
   job; push; watch the run (`gh run watch`); fix red before starting the
   next job. Never leave `main` red overnight-equivalent — the release tag
   at the end must cut from a green tip.

## Environment facts (verified 2026-08-04)

- `gh` is authenticated (account `jstockdi`, ssh protocol) — repo creation,
  pushes, secrets-free CI, and prerelease tagging all work.
- Rust 1.96.1 via rustup; pin `rust-toolchain.toml` to `1.96.0`-era stable
  exactly as secunit pins (exact version, rustfmt + clippy, minimal).
- Pattern references on disk: `../secunit` (workspace, ci.yml, release.yml,
  CHANGELOG conventions) and `../terraform` (credential mangling in
  `internal/command/cliconfig/credentials.go`, login flow in
  `internal/command/login.go`). Read the actual files when implementing
  J1.1 and J4.1; do not work from memory of them.
- The git repo exists locally with `docs/` committed on `main`; no remote.

## Setup (before J0.1)

1. `gh repo create Battle-Creek-LLC/tfrm --public --source . --push`
   (public — binstall downloads need unauthenticated release assets).
2. Create `DECISIONS.md` and `VERIFY.md` stubs; commit.

## Execution order

J0.1 → J0.2 → J1.1 → J1.2 → J1.3 → J2.1 → J2.2 → J2.3 → J2.4 → J2.5 →
J3.1 → J3.2 → J4.1 → J4.2 → J5.1 → J5.2 → J5.3. The plan allows Phase 4
after Phase 1; use that freedom only if a Phase 2/3 job is blocked and the
block is recorded.

Per job:

1. Re-read the job's entry in `docs/plan.md` and the spec sections it cites.
2. Implement in `tfrm-core` first, CLI wiring second — the plan's crate
   split exists so tests drive logic without clap.
3. Write the job's tests as specified (wiremock scenarios, fixtures, golden
   files, sentinel invariant). Tests the plan names are a floor, not a
   ceiling.
4. Run `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace` locally.
5. Append the job's live-verify items to `VERIFY.md`.
6. Commit (style below), push, `gh run watch` until green.

## Fixtures

Build `testdata/plans/` from Terraform's documented plan JSON format
(format version 1.x): hand-author minimal plans covering create, update,
replace, delete, sensitive (with `SENTINEL-DO-NOT-PRINT` as the sensitive
value), and unknown-after cases. Keep each fixture small enough to read in
one screen; realism beyond the fields tfrm reads is waste.

## Commit style

Imperative subject ≤50 chars, blank line, body wrapped at 72 explaining
*why* (the diff shows what). One job per commit; name the job id in the
body, not the subject. No AI attribution, no Co-Authored-By trailers —
authorship comes from the tooling.

## Definition of done

All of the following true, in this order:

1. CI green on `main` with jobs: fmt, clippy (`-D warnings`), test,
   build-release + smoke, dependency-review, semgrep, audit/deny.
2. `tfrm --help` shows exactly the spec §1 command tree; every subcommand
   implemented (no "not implemented" stubs remain).
3. `grep -r "SENTINEL-DO-NOT-PRINT" target` style leak check codified as a
   test, passing.
4. `CHANGELOG.md` has a dated `[0.1.0-rc.1]` section; `docs/cli.md`
   documents the R8.1 JSON shapes.
5. Tag `v0.1.0-rc.1` pushed from green `main`; the release workflow
   attaches 4 target archives + sha256 checksums to a GitHub **prerelease**;
   download one linux archive in-environment, verify the checksum, run
   `./tfrm --version` → `0.1.0-rc.1`.
6. `VERIFY.md` lists every deferred live check (Phases 2–4 verifies, the
   crates.io dry-run pair, and the post-secret publish steps) as runnable
   checkboxes for the human.
7. `README.md` quickstart: install (binstall + release download), login,
   select, list, show, diff, apply — copy-pasteable.

Stop after pushing the tag and confirming the release assets: the human
takes over at `VERIFY.md`. If the release workflow fails, fix and re-tag as
`v0.1.0-rc.2` (etc.) rather than force-moving a tag — moved tags break
binstall's checksum trust; record each re-tag in `DECISIONS.md`.

Reminders of the two constraints most likely to drift mid-run: never block
waiting for a human, and never let a sensitive value reach any output
stream.
