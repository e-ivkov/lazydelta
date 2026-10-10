# Proposal

## Why

The skeleton proves the review surface (editable diff, provenance hunks, anchored
comments, feedback serialization) against a hardcoded demo changeset. The product
only becomes real when the loop closes: a live agent producing the changes, real
files on disk, and review state that survives restarts.

## What Changes

- The app can drive an external ACP agent (e.g. `claude-code-acp`): spawn, session,
  send prompt, stream replies into the thread view, surface errors.
- The `s` (send feedback) action delivers `feedback_markdown()` to the agent
  session instead of only previewing it; replies are appended to the thread.
- Changesets derive from a real directory: base = `git HEAD` (or a snapshot for
  non-git dirs), proposed = working tree; the materialized accepted state can be
  written back to the working tree.
- Threads (changeset + statuses + comments + human edits + agent messages) are
  persisted as JSON under the project and reloaded on restart.
- Agent messages render with a simple markdown subset in a collapsible thread
  panel.
- The reviewer can jump from a focused diff line into a full external editor
  (Zed, Neovim, VS Code, ...) opened at that exact line, with external edits
  flowing back through refresh.
- The review surface gains vim/helix-style modal keybindings: single-key
  navigation, a space-prefixed leader menu for review actions, and an explicit
  insert mode when editing lines (replacing the current one-letter action
  bindings, which stay available as space-menu entries).

## Capabilities

### New Capabilities

- `agent-feedback-loop`: driving an external ACP agent from the review surface —
  configuration, per-thread sessions, ordered prompt delivery, streamed replies,
  and turning review artifacts into agent feedback.
- `thread-persistence`: durable storage of review threads (changeset, review
  state, conversation) with atomic, human-readable files that reload on restart.
- `worktree-changesets`: deriving changesets from a real directory (git-aware
  base detection) and materializing review outcomes back to the working tree.
- `external-editor-handoff`: jumping from a focused review line into a
  configured external editor showing the reviewed content, and folding
  external edits back into review.
- `modal-review-bindings`: vim/helix-style modal keybindings for the review
  surface — normal-mode navigation, leader-menu actions, insert mode.

### Modified Capabilities

(none — no prior specs exist)

## Impact

- `crates/core`: add changeset derivation from disk/git (no new deps; `git` via
  CLI subprocess), keep existing API stable.
- `crates/acp`: unchanged public API; concurrency wrapper lives in the app crate.
- `crates/app`: new agent runner (std thread + channel), thread store, agent
  picker + thread view additions; `s` rewired from preview to send; small
  markdown-subset renderer for agent messages; external editor launcher.
- New dependency: `pulldown-cmark` (markdown events → GPUI elements); no other
  new external dependencies beyond what gpui already pulls in.
