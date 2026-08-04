# tfrm — build-out plan

Jobs to take [`docs/spec.md`](spec.md) to a released binary. Every job names
its deliverable, the spec requirements it implements, how it is tested
(automated, runs in CI), and how it is verified (a command a human runs and
the exact observable result). A job is done only when both pass.

Release and CI conventions follow `../secunit` (workspace layout, `ci.yml`,
`release.yml`, changelog, crates.io + binstall); Phase 5 copies them
concretely.

## Repo layout

```
tfrm/
  Cargo.toml                 # workspace; shared version via workspace.package
  rust-toolchain.toml        # pinned channel, rustfmt + clippy, minimal profile
  .github/workflows/         # ci.yml, release.yml, dependency-review.yml, semgrep.yml
  crates/
    tfrm-core/               # API client, plan model, diff + redaction — library
    tfrm-cli/                # clap dispatch, rendering, prompts — binary (crate: bcl-tfrm)
  testdata/
    plans/                   # captured plan JSON fixtures (sanitized)
    cliconfig/               # terraformrc / credentials.tfrc.json fixtures
  docs/                      # spec.md, plan.md (this file), cli.md later
  CHANGELOG.md               # Keep a Changelog + SemVer, [Unreleased] section
```

Core/CLI split mirrors secunit: `tfrm-core` stays library-shaped so tests
drive it without going through clap. Published names follow secunit's
(`secunit-core` + `bcl-secunit`): `tfrm-core` + `bcl-tfrm`, binary `tfrm`,
`[package.metadata.binstall]` on the CLI crate.

## Test strategy (applies to every job)

- **Unit + integration tests** run against [`wiremock`](https://crates.io/crates/wiremock)
  HTTP mocks and `testdata/` fixtures — no network, no credentials, run in CI.
- **Redaction invariant**: fixture plans embed the sentinel value
  `SENTINEL-DO-NOT-PRINT`; a shared test helper asserts it appears in no
  stdout, stderr, `--format json`, or error output. Every rendering job wires
  this in.
- **Live smoke** (`verify` steps): env-gated `#[ignore]` tests plus manual
  commands against a real HCP Terraform workspace, run with
  `TFRM_E2E=1 TF_TOKEN_app_terraform_io=…`. Not in CI; each phase's exit
  criteria say which to run.

## Phases at a glance

| Phase | Goal | Jobs |
|---|---|---|
| 0 | Workspace, CI green, empty binary releases locally | J0.1–J0.2 |
| 1 | Credentials, HTTP client, config — the plumbing | J1.1–J1.3 |
| 2 | Read-only surface: workspace, runs list, show, diff | J2.1–J2.5 |
| 3 | Run actions: apply, discard, cancel | J3.1–J3.2 |
| 4 | Standalone auth: login / logout | J4.1–J4.2 |
| 5 | Release: workflows, changelog, crates.io, v0.1.0 | J5.1–J5.3 |

Dependencies are linear between phases except: Phase 4 needs only Phase 1;
it can run parallel to Phases 2–3.

---

## Phase 0 — Foundations

### J0.1 — Workspace scaffold + CI

Deliverable: cargo workspace (`tfrm-core`, `tfrm-cli`), `rust-toolchain.toml`
(current stable, pinned exact version like secunit's), `tfrm --version` and
`tfrm --help` working via clap derive with the full §1 command tree stubbed
(subcommands exist, print "not implemented", exit 1). `ci.yml` ported from
secunit minus the GUI/tauri jobs: `fmt`, `clippy -D warnings`, `test`,
`build-release` + smoke (`tfrm --version`), Swatinem/rust-cache + sccache,
actions pinned by SHA.

- Test: `cargo test` has one trycmd/assert_cmd case per stub asserting the
  "not implemented" exit code; clippy and fmt clean.
- Verify: push a branch; all four CI jobs green. `./target/release/tfrm
  --help` lists the §1 command tree exactly.

### J0.2 — Error/exit-code skeleton

Deliverable: one error enum in `tfrm-core` mapping onto the §1 exit-code
table (0/1/2/3/4/6); CLI converts every error path through it. R8.3 shape
(HTTP status + TFC error detail, never the Authorization header) defined
here.

- Test: unit tests assert each variant's exit code; a test greps the Display
  output of an error built from a request containing a token and asserts the
  token is absent.
- Verify: `tfrm runs show x` (no credentials) exits 3 with the R2.1 hint.

## Phase 1 — Plumbing

### J1.1 — Credential resolution (R2.1, R2.1a)

Deliverable: terraform-compatible credential lookup in `tfrm-core`:
`TF_TOKEN_<host>` env scan with terraform's mangling (`__`→`-`, `_`→`.`,
punycode tolerance), `credentials` blocks from `~/.terraformrc` /
`%APPDATA%\terraform.rc` / `TF_CLI_CONFIG_FILE` (HCL via the `hcl-rs`
crate), `credentials.tfrc.json`. Precedence: flag > env > file.

- Test: port the mangling cases from terraform's
  `credentials_test.go` (dots, double-underscore hyphens, punycode, case)
  as unit tests; fixture files in `testdata/cliconfig/` cover each source and
  the precedence order; a fixture with only a `credentials_helper` block
  asserts the R2.1a message.
- Verify: with only `terraform login`-produced credentials on disk (no env),
  `tfrm workspace list` (after J2.1) authenticates. Until then:
  `tfrm auth-debug` hidden subcommand prints which source resolved —
  matches expectation for env, file, and neither.

### J1.2 — API client core (R8.3, R8.4)

Deliverable: JSON:API client in `tfrm-core` (reqwest + tokio): bearer auth,
pagination iterator (`page[number]`/`page[size]`), 429 retry honoring
`Retry-After` (max 3), typed error mapping (401/403→exit 3, 404→4, 409→6),
redirect policy off by default (R5.6 handles its own 307).

- Test: wiremock cases — pagination across 3 pages; 429 then 200 (asserts
  the retry waited); each status→error mapping; assert the client never
  follows a redirect on its own.
- Verify: covered by J2.1's live smoke (first real API call).

### J1.3 — Config + workspace resolution (R2.2, R2.4, R3.2-persistence)

Deliverable: `.tfrm.toml` discovery (walk to ancestor), `.tfrm/local.toml`
read/write, resolution precedence flag > selection > config with source
tracking (for `workspace current`).

- Test: unit tests per precedence rank and for the exit-2 "none resolves"
  message naming all three sources; tempdir round-trip for local.toml.
- Verify: in a scratch dir with `.tfrm.toml`, `tfrm workspace current`
  names the config file as source; after `workspace select` it names the
  selection.

## Phase 2 — Read-only surface

### J2.1 — `workspace list` / `select` / `current` (R3.1–R3.3)

- Test: wiremock fixtures for the workspaces API (2 pages); asserts columns
  (name, current run status, VCS repo, `latest-change-at`), selected-marker,
  and `select` 404 → exit 4.
- Verify (live): `tfrm workspace list` against the real org shows the known
  workspaces; `tfrm workspace select <real-ws>` then `current` round-trips.

### J2.2 — `runs list` (R4.1, R4.1a, R4.2)

- Test: wiremock fixture with `include=configuration_version.ingress_attributes`
  echoed back — asserts exactly one request (no N+1), commit SHA column,
  source column, confirmable indicator, `--status` → `filter[status]`.
- Verify (live): `tfrm runs list` on a VCS workspace shows the runs visible
  in the UI with matching commit SHAs; a UI-visible plan-only run is absent
  (R4.1a).

### J2.3 — Plan JSON fetch (R5.6 fetch half)

Deliverable: `tfrm-core` fn: run → plan JSON. Handles the 307 to the
pre-signed URL (no Authorization forwarded, single immediate follow), 403 →
typed "needs admin" error, plan-record fallback summary struct.

- Test: wiremock 307 chain asserting the second request has **no**
  Authorization header; 403 path returns the fallback with summary counts
  from the plan record.
- Verify (live): with an admin token, fetch succeeds; with a write-only team
  token (if available), `runs show` prints the degraded summary + warning.

### J2.4 — `runs show` rendering (R5.1–R5.6)

Deliverable: renderer over plan JSON: metadata header, change summary,
grouped resource changes, before→after attributes, replace-forcing marks,
`(sensitive)` / `(known after apply)`, `--format json` with identical
redaction.

- Test: golden files against `testdata/plans/` fixtures (create, update,
  replace, delete, sensitive, unknown cases); the redaction sentinel
  invariant on every fixture; `--format json` snapshot re-parsed and walked
  for the sentinel.
- Verify (live): `tfrm runs show <run>` on a real run matches the UI's
  resource list; a known-sensitive attribute renders `(sensitive)`.

### J2.5 — `runs diff` (R6.1–R6.7)

Deliverable: plan-pair diff in `tfrm-core` keyed by resource address; the
four categories; attribute-level A/B values; R6.4 sensitive rule
(in-process equality, values never rendered); `latest-applied` resolution
via `filter[status]=applied&page[size]=1`; `--exit-code`; cross-workspace
refusal; R6.7 403 behavior.

- Test: fixture pairs per category; sensitive-equal (attribute omitted) and
  sensitive-differ (`(sensitive — differs)`, sentinel invariant) cases;
  identical-plans case exits 0 and prints "no differences"; `--exit-code`
  exits 1 on the differ pair; unit test that the resolver hits the applied
  filter, not `current-run`.
- Verify (live): diff two consecutive real runs; spot-check one intentional
  variable change appears and the workspace's sensitive var does not.

**Phase 2 exit criteria**: all live-smoke verifies above pass against the
real org; CI green; redaction sentinel wired into every renderer test.

## Phase 3 — Run actions

### J3.1 — `runs apply` (R7.1–R7.5, R7.9)

Deliverable: `is-confirmable` gate with blocking-reason report, policy
handling (`--override-policy`, empty-body override POST, `can-override`
gate, hard-mandatory refusal), R7.3 prompt (summary + commit SHA fetched at
apply time, type-workspace-name, `--auto-approve`), 202-then-poll to
`applied`/`errored`, apply-log streaming, 409 → exit 6, 403 → R7.5 message,
`-m` comment.

- Test: wiremock scenarios — happy path (checks body `comment`, polls
  through `applying`→`applied`, exit 0); not-confirmable → exit 6 with
  reason; soft-mandatory without flag → exit 6, with flag → override POST
  has empty body then apply proceeds; hard-mandatory + flag → exit 6; 409 on
  POST → exit 6; errored terminal → exit 1. Prompt tested via assert_cmd
  stdin (wrong workspace name typed → aborts, nothing POSTed).
- Verify (live): on a sandbox workspace, push a trivial change; `tfrm runs
  apply <run> -m "tfrm e2e"` → prompt shows the right summary, run applies,
  comment visible in the UI timeline. A second apply of the same run exits 6.

### J3.2 — `runs discard` / `cancel` (R7.6–R7.8)

- Test: wiremock — discard on `is-discardable` (with `-m` body); discard on
  an in-flight run → exit 6 suggesting `cancel`; cancel inverse suggesting
  `discard`; `--force` refused until `is-force-cancelable`, then POSTs
  force-cancel.
- Verify (live): discard a real pending run (comment in UI); cancel a real
  in-flight plan.

**Phase 3 exit criteria**: full loop on the sandbox workspace — push, list,
show, diff against previous, apply, see it applied in the UI.

## Phase 4 — Standalone auth (parallel to 2–3; needs Phase 1)

### J4.1 — `tfrm login` OAuth flow (R2b.1–R2b.4)

Deliverable: `login.v1` service discovery, PKCE S256 code flow as a public
client, localhost callback on the advertised port range, URL printed +
browser opened, race between callback and pasted URL/code (state verified
for URLs, skip-noted for bare codes), token verified via
`/api/v2/account/details`.

- Test: wiremock stands in for discovery + authorize + token endpoints —
  asserts `code_challenge`/`code_verifier` round-trip, `state` echo, exit 4
  when discovery lacks `login.v1`; pasted-URL path with wrong state refuses;
  bare-code path succeeds and prints the skip note; port-range exhaustion
  falls back to paste-only mode.
- Verify (live): `tfrm login` against app.terraform.io completes in-browser
  on a desktop; over SSH, the printed URL + pasted redirect completes;
  `account/details` name printed both times.

### J4.2 — Credential store write + `logout` (R2b.5–R2b.6)

- Test: tempdir round-trips — write preserves an existing foreign-host
  entry; file created 0600 (unix); logout removes only the target host;
  logout of absent host exits 0 with note; written file parses with J1.1's
  reader (self-interop).
- Verify: `tfrm login` then `terraform login` file inspection — terraform
  (if installed) or a JSON diff shows the same schema; `tfrm logout` then
  `tfrm workspace list` exits 3 with the login hint.

## Phase 5 — Release (follows secunit)

### J5.1 — JSON output contract + docs (R8.1, R8.2)

Deliverable: documented JSON shapes for `workspace list`, `runs list`,
`runs show`, `runs diff` in `docs/cli.md`; progress/logs to stderr when
`--format json`.

- Test: snapshot tests pinned to the documented shapes (a schema change
  fails the snapshot, forcing a deliberate update + doc edit); a test runs
  `runs show --format json` and asserts stdout parses as a single JSON
  document with streaming noise absent.
- Verify: `tfrm runs list --format json | jq .` works mid-plan-stream.

### J5.2 — Security + hygiene workflows

Deliverable: `dependency-review.yml` and `semgrep.yml` copied from secunit;
`CHANGELOG.md` seeded (Keep a Changelog, SemVer, `[Unreleased]`);
`cargo deny` or `cargo audit` job added to ci.yml.

- Test: CI runs the new jobs.
- Verify: open a PR bumping a dependency — dependency-review comments;
  semgrep job green.

### J5.3 — `release.yml` + crates.io + v0.1.0

Deliverable: secunit's release workflow adapted: trigger on `v*` tag +
`workflow_dispatch` with tag input; `taiki-e/upload-rust-binary-action`
(SHA-pinned) for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
`x86_64-apple-darwin`, `aarch64-apple-darwin`; `tar: unix`, `locked: true`,
`checksum: sha256`; `publish-crates` job **after** binaries (so binstall
always resolves an artifact): `cargo publish --locked -p tfrm-core -p
bcl-tfrm` with `CARGO_REGISTRY_TOKEN` from the `CRATES_IO_TOKEN` secret;
`[package.metadata.binstall]` on `bcl-tfrm` naming the `tfrm` binary.

- Test: dry run — push tag `v0.1.0-rc.1`; workflow attaches 4 archives +
  checksums to a prerelease; `cargo publish --dry-run -p tfrm-core -p
  bcl-tfrm` passes locally.
- Verify: after the real `v0.1.0` tag — download the linux archive, `sha256sum
  -c` the checksum, binary runs `tfrm --version` → `0.1.0`; on a clean
  machine `cargo binstall bcl-tfrm` installs the prebuilt `tfrm`;
  `cargo install bcl-tfrm --locked` compiles from crates.io. CHANGELOG
  `[0.1.0]` section dated; `workflow_dispatch` re-run against the tag
  re-uploads (idempotence check).

**Phase 5 exit criteria / definition of done for v0.1.0**: a user with no
terraform installation runs `cargo binstall bcl-tfrm`, `tfrm login`,
`tfrm workspace select`, `tfrm runs list`, `runs show`, `runs diff
--against latest-applied`, `runs apply` — the full spec §1 surface — against
a VCS-connected workspace, touching nothing but the released binary.
