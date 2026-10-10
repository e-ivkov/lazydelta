# lazydelta

An agent-first review surface with an **editable diff** — the open-source take on
the UX [Zed's Delta](https://delta.dev/) points at: threads anchored to code,
review as conversation, and *your manual fixes becoming part of the changeset
and the critique at the same time*.

The differentiating idea:

> When you hand-edit a line while reviewing an agent's hunk, that edit is
> stored as a **hunk with provenance** — it is (a) part of the materialized
> changeset and (b) feedback the agent can learn from ("I rewrote your line to
> X; match this style").

## Workspace

| Crate | What it is |
|---|---|
| [`crates/core`](crates/core) | Provenance-aware changeset model: hunks (`agent` / `human` / `human-fix`), statuses (`proposed`/`accepted`/`rejected`), anchored comments, materialization policies, feedback serialization. |
| [`crates/acp`](crates/acp) | Minimal blocking [ACP](https://agentclientprotocol.com/) stdio client stub (`initialize` → `session/new` → `session/prompt`, buffering `session/update`). |
| [`crates/app`](crates/app) | GPUI demo app: diff renderer whose added lines are editable in place, per-hunk accept/reject, comment anchoring, feedback preview. |

## Try it

```bash
cargo run            # demo changeset, no agent required
```

Keys: `↑/↓` focus added line · `a` accept · `r` reject · `u` restore ·
`e` edit focused line (type, `enter` commits, `esc` cancels) · `c` comment ·
`s` serialize feedback (also printed to stdout).

## Status

Skeleton. The demo renders a hardcoded changeset; wiring real agents, threads,
worktrees, and persistence is planned in `openspec/changes/` (see
`openspec/` for the spec-driven workflow).

## Patches

`patches/xattr` carries a one-line fix for [xattr 0.2.3][xattr] referencing
`libc::ENOATTR`, which newer `libc` removed on Linux (`zed-async-tar`, pulled
in by gpui 0.2.2, pins `xattr ^0.2`). Drop the `[patch.crates-io]` entry in
the workspace `Cargo.toml` once the tree moves to gpui with xattr ≥ 1.

[xattr]: https://github.com/Stebalien/xattr

## License

MIT
