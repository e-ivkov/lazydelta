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
| [`crates/core`](crates/core) | Provenance-aware changeset model: hunks (`agent` / `human` / `human-fix`), statuses (`proposed`/`accepted`/`rejected`), anchored comments, materialization policies, feedback serialization, git-backed derivation (`git show HEAD:` base), snapshot fallback, review-state carry-over across refresh. |
| [`crates/acp`](crates/acp) | Minimal blocking [ACP](https://agentclientprotocol.com/) stdio client (`initialize` → `session/new` → `session/prompt`, buffering `session/update`). |
| [`crates/app`](crates/app) | GPUI desktop app: the modal review surface, agent runner thread, thread persistence, markdown thread rendering, external-editor jump. |

## Try it

```bash
cargo run                       # demo changeset, no agent required

# real project (git worktree: base = HEAD, proposed = working tree)
mkdir -p .lazydelta
cat > .lazydelta/config.json <<'EOF'
{
  "agents": [
    {"name": "claude", "command": "claude-code-acp", "args": []},
    {"name": "gemini", "command": "gemini", "args": ["--experimental-acp"]}
  ],
  "editor_preset": "nvim",
  "prompt_timeout_secs": 600
}
EOF
cargo run -- --project .
```

Non-git directories work too: the first open snapshots the current state as the
base (`.lazydelta/threads/<id>/base/`). If git metadata is unreadable, rerun
with `--snapshot`. Threads persist to `.lazydelta/threads/<id>/thread.json`
(atomic writes) and reload with all review state on restart.

Editor handoff: `editor_preset` is one of `zed`, `nvim`, `code`, or set an
explicit template:

```json
"editor": {"command": "zed", "args": ["{file}:{line}"]}
```

## Modal keymap (vim/helix-style)

| Keys | Action |
|---|---|
| `j` `k` (`↑↓`) | focus next/prev added line |
| `g g` / `G` | first / last added line |
| `] h` / `[ h` | next / prev hunk |
| `i` / `a` | insert mode on the focused added line (`Esc` commits) |
| `space a` / `r` / `u` | accept / reject / restore hunk |
| `space c` | comment on the focused line |
| `space s` | send feedback to the agent (thread summary + comments + your edits as before/after pairs) |
| `space e` / `J` | jump to the external editor at the focused line |
| `space w` / `W` | write preview / accepted-only state to the working tree |
| `R` | refresh the changeset from disk (carries review state over) |
| `space t` | collapse/expand the thread panel |
| `space x` | restart the agent session |
| `tab` | cycle configured agents |

The mode indicator (`NORMAL` / `INSERT`) and pending-sequence state (`g…`,
`]…`, `space _`) live in the header; boundary navigation is a visible no-op,
never an error.

## Status

v1 of the `add-agent-review-loop` change is implemented (see
`openspec/changes/`); verification against a real ACP agent from the GUI is the
remaining manual step. Known v1 limitations: hunk-level (not per-line)
accept/reject, refresh-not-watch, no IME in the editable line, a fixed markdown
subset for agent messages, no `5j` counts.

## Patches

`patches/xattr` carries a one-line fix for [xattr 0.2.3][xattr] referencing
`libc::ENOATTR`, which newer `libc` removed on Linux (`zed-async-tar`, pulled
in by gpui 0.2.2, pins `xattr ^0.2`). Drop the `[patch.crates-io]` entry in
the workspace `Cargo.toml` once the tree moves to gpui with xattr ≥ 1.

[xattr]: https://github.com/Stebalien/xattr

## License

MIT
