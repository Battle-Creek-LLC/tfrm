# tfrm — specification

A Rust CLI for HCP Terraform / Terraform Enterprise: select a workspace, list
its runs, view and diff plans with sensitive-value redaction, and apply
VCS-triggered runs from the terminal.

## Assumptions

- **A1** — Backend is the HCP Terraform / Terraform Enterprise v2 API. Runs
  are created by VCS pushes to the tracked branch; tfrm finds and confirms
  them via the API (`POST /runs/:id/actions/apply` — the same call the UI's
  "Confirm & Apply" uses; only CLI-driven runs are blocked on VCS workspaces).
- **A1a** — PR-triggered plans are speculative and can never be applied; only
  runs from the tracked branch are confirmable.
- **A2** — The API token is a **user or team token** (organization tokens are
  rejected for these endpoints) with: **admin-level access to the workspace**
  to read plan JSON output ([plans API][api-plans] — json-output requires
  admin, not just read), and "write" to apply. Where the token has write but
  not admin, show/diff degrade per R5.6/R6.7.
- **A3** — Plan rendering and diffing operate on Terraform's machine-readable
  plan JSON (format version 1.x), including the `before_sensitive` /
  `after_sensitive` masks.

## Goals

- **G1** — List and select workspaces; selection persists across commands.
- **G2** — View any run's plan in a readable form.
- **G3** — Diff two plans without ever printing a sensitive value; report only
  whether sensitive values differ.
- **G4** — Apply VCS-triggered runs from the CLI, always confirming against
  the fetched plan's change summary.
- **G5** — Authenticate standalone — no terraform binary required — while
  staying credential-compatible with it in both directions.

## Non-goals

- **N1** — Replacing `terraform` for init/validate/state operations.
- **N2** — Creating runs. Runs originate from VCS; tfrm inspects and confirms
  them.
- **N3** — Sentinel/OPA policy authoring. Policy check results are displayed
  (and optionally overridden), not managed.

## 1. Command surface

```
tfrm login [HOST]             browser OAuth (PKCE) with paste fallback; stores token
tfrm logout [HOST]            remove the stored token for HOST

tfrm workspace list           list the org's workspaces
tfrm workspace select <NAME>  set the current workspace (persisted)
tfrm workspace current        show the selection and its source

tfrm runs list                list recent runs
tfrm runs show <RUN_ID>       render one run's plan
tfrm runs diff <A> <B>        diff two plans
tfrm runs apply <RUN_ID>      confirm and apply a run awaiting confirmation
tfrm runs discard <RUN_ID>    reject a run awaiting confirmation
tfrm runs cancel <RUN_ID>     stop a run that is actively planning or applying
```

`apply`, `discard`, and `cancel` take `-m/--comment <text>`, sent as the
action's `comment` and shown in the run timeline.

Global flags: `--workspace/-w <name>`, `--org <name>`, `--format table|json`
(default `table`), `--no-color`, `--token <t>` (overrides env/config).

### Exit codes

| Code | Meaning |
|------|---------|
| 0 | success |
| 1 | unexpected error, or apply/discard ended in `errored` |
| 2 | usage error (including unresolvable workspace) |
| 3 | authentication/authorization failure |
| 4 | run, workspace, or plan not found |
| 6 | apply refused: run not confirmable (state, policy, queue) |

## 2. Configuration

- **R2.1** — Credential resolution is interoperable with the terraform CLI
  ([CLI config file][cli-config]): a machine where `terraform login` (or
  `TF_TOKEN_*`) already works needs zero tfrm-specific auth setup. Precedence,
  matching terraform's:
  1. `--token` flag (tfrm-only escape hatch)
  2. `TF_TOKEN_<host>` env var, host mangled per terraform's rules: dots →
     underscores, hyphens → double underscores, non-ASCII hosts → punycode
     (`app.terraform.io` → `TF_TOKEN_app_terraform_io`)
  3. `credentials "<host>"` blocks from the CLI config file (`~/.terraformrc`;
     Windows `%APPDATA%\terraform.rc`; `TF_CLI_CONFIG_FILE` override) and from
     `~/.terraform.d/credentials.tfrc.json` as written by
     [`terraform login`][tf-login]
  If none resolves, exit 3 with the hint "run `tfrm login <host>`".
- **R2.1a** — `credentials_helper` blocks are not executed in v0.1. If one is
  configured and no static credential matches the host, say so in the exit-3
  message instead of silently failing.

## 2b. `tfrm login` / `tfrm logout`

tfrm does not depend on the terraform binary; it implements the same login
protocol terraform does (verified against
`internal/command/cliconfig/credentials.go` and `internal/command/login.go` in
hashicorp/terraform).

- **R2b.1** — `login [HOST]` (default `app.terraform.io`) discovers the OAuth
  client config from the host's `/.well-known/terraform.json` (`login.v1`
  service: authz/token endpoints, port range). Exit 4 if the host does not
  advertise `login.v1`.
- **R2b.2** — Run OAuth2 authorization-code with PKCE (S256) as a public
  client: generate verifier + `state`, start a localhost callback listener on
  the advertised port range, open the authorize URL in the browser **and**
  print it (for remote/headless sessions).
- **R2b.3** — Accept whichever arrives first: the browser callback, or the
  user pasting into the CLI prompt either the full redirect URL or the bare
  authorization code. For a pasted URL, verify `state` matches; a bare code
  carries no state — accept it and note the check was skipped.
- **R2b.4** — Exchange the code (with the PKCE verifier) for a token, verify
  it with `GET /api/v2/account/details`, and print the account name on
  success.
- **R2b.5** — Store the token in `~/.terraform.d/credentials.tfrc.json`
  (create with mode 0600), preserving other hosts' entries. This is the same
  file terraform reads/writes, so credentials flow both ways — interop without
  dependency.
- **R2b.6** — `logout [HOST]` removes that host's entry from
  `credentials.tfrc.json`; no-op with a note if absent. Neither command
  touches `credentials` blocks in `.terraformrc` (read-only config, matching
  terraform's own behavior).
- **R2.2** — Read `org` and `hostname` (default `app.terraform.io`; TFE
  installs set their own) from `.tfrm.toml` in the working directory or
  nearest ancestor. Flags override the file.
- **R2.3** — Never write the token to any file or log output.
- **R2.4** — Workspace resolution precedence: `-w/--workspace` flag >
  selection persisted by `tfrm workspace select` > `workspace` in
  `.tfrm.toml`. If none resolves, exit 2 naming the three sources.

## 3. `tfrm workspace`

- **R3.1** — `list` shows the org's workspaces ([workspaces API][api-ws]):
  name, current run status, VCS repo (if connected), and `latest-change-at`.
  Mark the currently selected workspace. Paginate transparently
  (`page[number]`/`page[size]`, default page size 20).
- **R3.2** — `select <NAME>` verifies the workspace exists in the org (exit 4
  if not), then persists the selection to `.tfrm/local.toml` — user-local
  state, recommended for `.gitignore`.
- **R3.3** — `current` prints the resolved workspace and which source it came
  from (flag, selection, or config).

## 4. `tfrm runs list`

- **R4.1** — List the workspace's most recent runs ([runs API][api-run];
  default 20, `--limit <n>` paginating transparently, `--status <s>` mapping
  to `filter[status]`), newest first: run ID, status, created-at, commit SHA
  (via `include=configuration_version.ingress_attributes` — one request, no
  N+1), message, and a **source** column from the run's `source` attribute.
- **R4.1a** — The API excludes `plan_only` runs from listings by default;
  keep that default (they can never be applied, per N2/A1a).
- **R4.2** — Mark confirmable runs (`actions.is-confirmable = true`) with a
  distinct indicator.

## 5. `tfrm runs show`

- **R5.1** — Render, in order: run metadata (ID, workspace, status, source,
  commit SHA, message), the change summary (`+add ~change -destroy`),
  per-resource changes grouped by action (create / update / replace / delete /
  read), then output changes.
- **R5.2** — For `update` and `replace`, show attribute-level before → after
  values. Mark attributes forcing replacement.
- **R5.3** — Redact any attribute marked sensitive in `before_sensitive` or
  `after_sensitive`: print `(sensitive)` in place of the value. Apply the same
  redaction to sensitive outputs and inside `--format json` — emitted JSON must
  contain the marker, not the value.
- **R5.4** — Render unknown-after values (from `after_unknown`) as
  `(known after apply)`.
- **R5.5** — If the run's plan is still in progress, stream plan logs until it
  finishes, then render.
- **R5.6** — Plan JSON fetch ([plans API][api-plans]): `GET
  /runs/:id/plan/json-output` answers **307** with a temporary URL valid for
  one minute — follow it immediately and do not send the Authorization header
  to the redirect target (it is pre-signed). On 403 (token has write but not
  admin, A2), degrade: render the change summary from the plan record's
  resource-addition/change/destruction attributes plus raw log text, warn that
  attribute-level detail needs workspace admin, and exit 0.

## 6. `tfrm runs diff`

- **R6.1** — `diff <A> <B>` compares the plan JSON of two runs, keyed by
  resource address. `--against latest-applied` substitutes the workspace's
  most recent applied run, resolved as `GET
  /workspaces/:ws_id/runs?filter[status]=applied&page[size]=1` (listing is
  newest-first; the workspace `latest-run` relationship is deprecated and
  `current-run` is any-status — use neither).
- **R6.2** — Report four categories: resources only in A, only in B, in both
  with differing changes, and (suppressed by default, shown with `--all`) in
  both and identical.
- **R6.3** — For resources in both with differing changes, show
  attribute-level differences: attribute name, A's after-value, B's
  after-value.
- **R6.4** — Sensitive attribute rule: if an attribute is sensitive on either
  side, never print its value. If the underlying values are equal, treat the
  attribute as identical (omit it). If they differ, list the attribute as
  `(sensitive — differs)` with no values. Equality is computed in-process on
  the decoded JSON values; the values must not reach any output stream,
  including `--format json` and error messages.
- **R6.5** — Refuse to diff runs from different workspaces unless
  `--allow-cross-workspace` is passed.
- **R6.6** — Exit 0 whether or not differences exist; `--exit-code` makes the
  command exit 1 when differences exist (git-diff convention).
- **R6.7** — Diff requires plan JSON for both runs; R5.6's summary-only
  fallback is not enough. On 403, exit 3 stating that diff needs workspace
  admin on the token.

## 7. Run actions — `apply`, `discard`, `cancel`

- **R7.1** — Gate on the run's `actions.is-confirmable` attribute, not on
  status alone (a `planned` run can be blocked by a policy check or a queued
  run ahead of it). If false, exit 6 reporting the status and, when
  determinable, the blocking reason. Speculative and plan-only runs are never
  confirmable (A1a).
- **R7.2** — If the run is blocked by a failed soft-mandatory policy check
  ([policy checks API][api-policy]), report it and exit 6 by default. With
  `--override-policy`, first `POST /policy-checks/:id/actions/override`
  (empty body — the endpoint takes no comment), then proceed; the
  confirmation prompt (R7.3) must state that a policy is being overridden.
  Only soft-mandatory checks can be overridden, gated by the policy check's
  `can-override` permission — on refusal, exit 3 naming that permission.
  Hard-mandatory failures are final: exit 6.
- **R7.3** — Before applying, fetch the plan JSON and print the change summary
  and the run's commit SHA, then require the user to type the workspace name
  to confirm. `--auto-approve` skips the prompt. The summary shown is fetched
  at apply time — the user always confirms against the plan that will execute.
- **R7.4** — The apply action returns `202 Accepted` with an empty body;
  success means accepted, not applied. Stream apply logs to stdout and poll
  `GET /runs/:id` until `applied` (exit 0) or `errored` (exit 1). Surface
  TFC's own staleness/queue errors verbatim (TFC discards or refuses stale
  runs; tfrm does not duplicate that check).
- **R7.5** — On a 403, say the token lacks "write" on the workspace rather
  than printing a generic auth error.
- **R7.6** — `discard <RUN_ID>` — `POST /runs/:id/actions/discard`. Valid only
  for runs awaiting confirmation (`actions.is-discardable`); otherwise exit 6
  naming the run's status. No confirmation prompt (nothing executes), but
  print what was discarded.
- **R7.7** — `cancel <RUN_ID>` — `POST /runs/:id/actions/cancel`. Valid only
  while the run is actively planning or applying (`actions.is-cancelable`);
  otherwise exit 6 suggesting `discard` when the run is instead awaiting
  confirmation. `--force` uses `actions/force-cancel`, allowed only once the
  API reports `is-force-cancelable` (after the cooldown following a plain
  cancel).
- **R7.8** — `apply`, `discard`, and `cancel` accept `-m/--comment <text>`,
  sent as the action's `comment` body field. Optional; no default text.
- **R7.9** — A **409** from any action POST means the run's state changed
  between tfrm's check and the request (all four actions return 202 on
  success, 409 on state conflict); exit 6 with the API's error detail.

## 8. Output & errors

- **R8.1** — `--format json` on `workspace list`, `runs list`, `runs show`,
  and `runs diff` emits a stable, documented JSON shape (redaction rules of
  R5.3/R6.4 included) for scripting.
- **R8.2** — Write human progress/log streaming to stderr when `--format json`
  is active, so stdout stays machine-parseable.
- **R8.3** — API errors include the HTTP status and TFC error detail; never
  the request's Authorization header.
- **R8.4** — On **429**, honor the `Retry-After` header and retry; surface a
  note only if retries exceed 3.

## 9. Implementation notes (non-normative)

- Crates: `clap` (derive) for the command tree, `tokio` + `reqwest` for the
  API, `serde`/`serde_json` for plan JSON, `comfy-table` or hand-rolled
  rendering for tables.
- Model plan JSON with typed structs for the fields tfrm reads
  (`resource_changes`, `output_changes`, sensitivity/unknown masks) and
  `serde_json::Value` for attribute payloads — the diff walks values
  generically.
- Keep the redaction boundary in one module: decode → compare → render, where
  render is the only stage allowed to produce user-visible strings, and it
  receives already-redacted values for sensitive attributes.

## Open questions

- **Q1** — Resolve a run by commit SHA (`tfrm runs apply --commit <SHA>`)
  instead of run ID, matching the configuration version's commit among recent
  runs? Deferred; run IDs only in v0.1.
- **Q2** — Multi-workspace fan-out (list/diff/apply across N workspaces) —
  out of scope for v0.1.
- **Q3** — A soft "must have been viewed via `runs show`/`diff` first" gate on
  apply, restoring a review-before-apply invariant without the removed ledger.
  Deferred; R7.3's inline summary is the v0.1 safeguard.
- **Q4** — OS keychain storage for tokens instead of the plain-text
  `credentials.tfrc.json` (which is what terraform itself uses). Deferred:
  would break file-level interop unless done via a `credentials_helper`.

## API references

Verified against these pages 2026-08-04:

[api-ws]: https://developer.hashicorp.com/terraform/cloud-docs/api-docs/workspaces
[api-run]: https://developer.hashicorp.com/terraform/cloud-docs/api-docs/run
[api-plans]: https://developer.hashicorp.com/terraform/cloud-docs/api-docs/plans
[api-policy]: https://developer.hashicorp.com/terraform/cloud-docs/api-docs/policy-checks
[cli-config]: https://developer.hashicorp.com/terraform/cli/config/config-file
[tf-login]: https://developer.hashicorp.com/terraform/cli/commands/login

- Workspaces API: [developer.hashicorp.com/terraform/cloud-docs/api-docs/workspaces][api-ws]
- Runs API (list, get, apply/discard/cancel actions): […/api-docs/run][api-run]
- Plans API (json-output): […/api-docs/plans][api-plans]
- Policy checks API (override): […/api-docs/policy-checks][api-policy]
- Terraform CLI config file (credentials, `TF_TOKEN_*`): […/cli/config/config-file][cli-config]
- `terraform login`: […/cli/commands/login][tf-login]
