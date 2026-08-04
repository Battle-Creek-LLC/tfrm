# Live verification checklist

Deferred "Verify (live)" steps from `docs/plan.md`. This environment has no
HCP Terraform token and no terraform binary, so these run against a real
org by a human. Each item gives the exact command and the observable result
that counts as a pass.

Prereqs: a real HCP Terraform org with a VCS-connected sandbox workspace,
and either `terraform login`-produced credentials on disk or
`TF_TOKEN_app_terraform_io` exported.

## Phase 1 — plumbing

- [ ] **J1.1 credential interop** — on a machine where `terraform login`
  has already run (credentials in `~/.terraform.d/credentials.tfrc.json`,
  no `TF_TOKEN_*` set): `tfrm workspace list` authenticates and lists the
  org's workspaces. Also `tfrm auth-debug` prints
  `token for app.terraform.io: credentials file ~/.terraform.d/credentials.tfrc.json`.
