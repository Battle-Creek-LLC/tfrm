# tfrm CLI reference — JSON output (R8.1)

`--format json` on `workspace list`, `runs list`, `runs show`, and
`runs diff` emits exactly one JSON document on stdout. Progress and log
streaming move to stderr while `--format json` is active (R8.2), so
stdout always parses.

Redaction (R5.3/R6.4) applies inside JSON exactly as in tables: a
sensitive value is replaced by the string `"(sensitive)"`; a value not
known until apply is `"(known after apply)"`; a sensitive attribute
whose values differ between two plans appears in `runs diff` with
`"sensitive_differs": true` and **no value fields at all**.

These shapes are contractual for the 0.1 line: tests snapshot them, and
changing a field name or dropping a field is a breaking change that
requires editing this document and the pinned tests in the same commit.

## `tfrm workspace list --format json`

An array of workspace rows:

```json
[
  {
    "name": "networking",
    "current_run_status": "applied",
    "vcs_repo": "acme/networking",
    "latest_change_at": "2026-08-01T12:00:00Z",
    "selected": false
  }
]
```

- `current_run_status`, `vcs_repo`, `latest_change_at` — `null` when the
  workspace has no current run / VCS connection / recorded change.
- `selected` — true for the workspace the current selection resolves to.

## `tfrm runs list --format json`

An array of run rows, newest first:

```json
[
  {
    "id": "run-abc123",
    "status": "planned",
    "created_at": "2026-08-03T10:00:00Z",
    "commit_sha": "aaaa1111bbbb2222",
    "message": "add bucket",
    "source": "tfe-api",
    "confirmable": true
  }
]
```

- `commit_sha` — full SHA from the VCS ingress; `null` for non-VCS runs.
- `confirmable` — the run's `actions.is-confirmable` (R4.2).

## `tfrm runs show --format json`

One object:

```json
{
  "run": {
    "id": "run-abc123",
    "workspace": "platform",
    "status": "planned",
    "source": "tfe-api",
    "commit_sha": "aaaa1111bbbb2222",
    "message": "test message"
  },
  "summary": { "add": 1, "change": 0, "destroy": 0 },
  "resource_changes": [
    {
      "address": "aws_db_instance.main",
      "action": "update",
      "replace_forced_by": ["engine_version"],
      "attributes": [
        {
          "name": "password",
          "before": "(sensitive)",
          "after": "(sensitive)",
          "forces_replacement": true
        }
      ]
    }
  ],
  "output_changes": [
    { "name": "db_password", "action": "update",
      "before": "(sensitive)", "after": "(sensitive)" }
  ]
}
```

- `run.workspace`, `run.source`, `run.commit_sha`, `run.message` are
  omitted when unknown.
- `action` is one of `create | update | replace | delete | read`.
- `replace_forced_by` and `forces_replacement` are omitted when empty /
  false.
- `attributes[].before/after` hold the real JSON value, or the marker
  strings above.
- When the token has write but not admin (R5.6), the object carries
  `"degraded": true`, `summary` comes from the plan record, and both
  change arrays are empty.

## `tfrm runs diff A B --format json`

One object:

```json
{
  "run_a": "run-a",
  "run_b": "run-b",
  "only_in_a": [ { "address": "aws_s3_bucket.assets", "action": "create" } ],
  "only_in_b": [],
  "differing": [
    {
      "address": "aws_db_instance.main",
      "action_a": "update",
      "action_b": "update",
      "attributes": [
        { "name": "instance_type", "a": "t3.large", "b": "t3.2xlarge" },
        { "name": "password", "sensitive_differs": true }
      ]
    }
  ],
  "identical_count": 3,
  "identical": ["aws_eip.nat"]
}
```

- `identical` (the address list) is present only with `--all`;
  `identical_count` is always present (R6.2).
- In `attributes`, `a`/`b` are the after-values of each plan; for a
  sensitive attribute the entry is `{ "name": …, "sensitive_differs":
  true }` with no `a`/`b` keys (R6.4).

## Exit codes

See the table in [`docs/spec.md`](spec.md) §1; `--format json` does not
change exit-code behavior, including `runs diff --exit-code`.
