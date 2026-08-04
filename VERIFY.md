# Live verification checklist

Deferred "Verify (live)" steps from `docs/plan.md`. This environment has no
HCP Terraform token and no terraform binary, so these run against a real
org by a human. Each item gives the exact command and the observable result
that counts as a pass.

Prereqs: a real HCP Terraform org with a VCS-connected sandbox workspace,
and either `terraform login`-produced credentials on disk or
`TF_TOKEN_app_terraform_io` exported.

## Phase 2 — read-only surface

- [ ] **J2.1 workspace list/select** — `tfrm workspace list` against the
  real org shows the known workspaces with run status, VCS repo, and
  latest-change-at columns. `tfrm workspace select <real-ws>` then
  `tfrm workspace current` round-trips: current prints the selected name
  with `selection` as the source. Also covers J1.2's first real API call.
- [ ] **J2.2 runs list** — `tfrm runs list` on a VCS-connected workspace
  shows the runs visible in the UI with matching commit SHAs and a `>`
  marker on any run awaiting confirmation. A PR-triggered (plan-only)
  run visible in the UI does NOT appear in the listing (R4.1a).
- [ ] **J2.3 plan fetch permissions** — with an admin token,
  `tfrm runs show <run>` renders full attribute detail. With a
  write-only team token (if available), the same command prints the
  degraded summary counts plus a warning that attribute-level detail
  needs workspace admin, and exits 0.
- [ ] **J2.4 runs show** — `tfrm runs show <run>` on a real run matches
  the UI's resource list (same addresses, same actions); a
  known-sensitive attribute renders `(sensitive)` and its value appears
  nowhere in the output, including `--format json`.
- [ ] **J2.5 runs diff** — `tfrm runs diff <newer> <older>` on two
  consecutive real runs: an intentional variable change between them
  appears as an attribute difference, and the workspace's sensitive
  variable does not appear anywhere (at most `(sensitive — differs)`).
  `tfrm runs diff <run> --against latest-applied` resolves the newest
  applied run.

**Phase 2 exit criteria** — all Phase 2 checks above pass against the
real org.

## Phase 3 — run actions

- [ ] **J3.1 runs apply** — on a sandbox workspace, push a trivial
  change; `tfrm runs apply <run> -m "tfrm e2e"` shows the correct
  change summary and commit SHA in the prompt, applies after typing the
  workspace name, and the comment appears in the run's UI timeline.
  A second `tfrm runs apply <same-run>` exits 6.
- [ ] **J3.2 discard/cancel** — `tfrm runs discard <pending-run> -m
  "tfrm e2e discard"` discards a real pending run and the comment shows
  in the UI; `tfrm runs cancel <in-flight-run>` stops a real in-flight
  plan.

**Phase 3 exit criteria** — the full loop on the sandbox workspace:
push, `runs list`, `runs show`, `runs diff --against latest-applied`,
`runs apply`, and the change is applied in the UI.

## Phase 4 — standalone auth

- [ ] **J4.1 login (browser)** — on a desktop, `tfrm login` against
  app.terraform.io opens the browser, completes via the localhost
  callback, and prints the account name from account/details.
- [ ] **J4.1 login (SSH/headless)** — over SSH, the printed authorize
  URL opened elsewhere plus the pasted redirect URL completes the same
  flow; the account name is printed.
- [ ] **J4.2 credential interop with terraform** — after `tfrm login`,
  inspect `~/.terraform.d/credentials.tfrc.json` (or run `terraform
  login` on another host and diff): the schema matches terraform's own
  file, and `terraform` (if installed) authenticates with the
  tfrm-written token. `tfrm logout` then `tfrm workspace list` exits 3
  with the login hint.

## Phase 1 — plumbing

- [ ] **J1.1 credential interop** — on a machine where `terraform login`
  has already run (credentials in `~/.terraform.d/credentials.tfrc.json`,
  no `TF_TOKEN_*` set): `tfrm workspace list` authenticates and lists the
  org's workspaces. Also `tfrm auth-debug` prints
  `token for app.terraform.io: credentials file ~/.terraform.d/credentials.tfrc.json`.
