# Design

## Context

The skeleton ships: `lazydelta-core` (changeset model with provenance, statuses,
anchored comments, materialization, feedback serialization — all pure and
tested), `lazydelta-acp` (blocking ndjson JSON-RPC client), and a GPUI demo app
rendering a hardcoded changeset with editable added lines. Nothing touches disk
or a real agent yet. gpui 0.2.2 is pinned from crates.io; the workspace avoids
async runtimes and git libraries.

## Goals / Non-Goals

**Goals:**

- Close the loop end-to-end with any ACP agent CLI, using only std threads and
  channels for concurrency.
- Keep `lazydelta-core` pure (no IO beyond the existing `write_to`).
- Persist threads in plain, diffable JSON inside the project.

**Non-Goals:**

- Multiplayer/sync (DeltaDB-style replication), CRDTs.
- Full editor features (syntax highlighting, multi-line editing, IME in the
  editable line) — tracked separately.
- Per-line accept/reject granularity (hunk-level only for now).
- Replacing git: git is used read-only via its CLI.

## Decisions

### D1: Agent runner = std thread + mpsc, drained on a UI timer

One OS thread owns each `AcpAgent`. A loop sends queued prompts and forwards
`session/update` values into an `std::sync::mpsc::Sender`. The GPUI side polls
the receiver on a repeating background timer and drains pending events, then
`cx.notify()`.
*Alternatives:* tokio/async (rejected: heavy dep for one subprocess);
gpui `BackgroundExecutor` + futures channels (viable later, more idiomatic but
couples the runner to gpui internals).

### D2: Base side via `git` CLI, snapshot fallback

Base text per file comes from `git show HEAD:<path>` (+ a `git status`-style
porcelain listing to know which files changed). Non-git directories capture a
`base/` snapshot under `.lazydelta/threads/<id>/` at creation.
*Alternative:* `git2` crate (rejected for now: large C dep; revisit when we need
index writes or blaming).

### D3: Thread store = one JSON file per thread, atomic rename

`.lazydelta/threads/<id>/thread.json` holds `{schema_version, changeset,
messages, queue}`. Saves go to `thread.json.tmp` then `rename(2)`. Schema
version mismatch → refuse + report (see spec). The changeset already serializes
via serde; agent messages are stored as raw ACP update values plus derived
display text.
*Alternative:* sqlite (rejected: not git-friendly/diffable for v1).

### D4: Feedback prompt is the core serializer, extended

Reuse `feedback_markdown()` as the wire format for v1, prefixed with a short
thread summary (accepted/rejected counts). One prompt per send; sends queue
behind the in-flight one (FIFO) with depth shown in the UI.

### D5: Refresh, not watch

The user refreshes the changeset explicitly (action `R`) by re-deriving from
disk; unreviewed review state (statuses/comments/edits) is carried over by
matching hunks on (path, old_start, old_len). Auto-watching (inotify) is a
follow-up.

## Risks / Trade-offs

- [ACP implementations drift / vary in handshake details] → probe with
  `initialize` at spawn, fail loudly with the agent's stderr; keep the stub
  agent script for CI.
- [Prompt queue can grow if the agent hangs] → timeout per prompt (configurable,
  default 10 min) then error + drop queue head, keep the rest.
- [Editing the working tree while a thread is open] → refresh-by-matching may
  mis-anchor comments on moved hunks; acceptable for v1, surface a warning when
  hunk matching is ambiguous.
- [Blocking runner thread leaks on app exit] → `AcpAgent::drop` kills the child;
  runner thread exits when the channel closes.

## Migration Plan

Pure addition; no existing state to migrate. The demo mode stays available via
`--demo`.

## Open Questions

- Should agent messages be rendered as raw markdown or stripped to plain text
  for v1? (Does not affect storage or the loop; decide during UI tasks.)
