# Tasks

## 1. Core: worktree derivation

- [ ] 1.1 Add `ChangeSet::from_directory` that reads working-tree files and builds the proposed side; pure function over contents (inject paths+contents, testable without fs)
- [ ] 1.2 Add base resolution via `git show HEAD:<path>` (+ changed-file listing via `git status --porcelain`) in a small `git` module in `lazydelta-core`, feature-complete for modified files
- [ ] 1.3 Snapshot fallback: capture `base/` copies at thread creation for non-git directories; loader reads them back
- [ ] 1.4 Carry review state across refresh by matching hunks on (path, old_start, old_len); unit tests for match, ambiguity, and no-match paths
- [ ] 1.5 Unit tests: derivation from fixtures (git repo fixture via `git` CLI in a tmpdir; snapshot dir), write-back of preview and accepted-only policies

## 2. Thread persistence

- [ ] 2.1 Define `ThreadDocument` (schema_version, changeset, messages, queue) with serde roundtrip tests
- [ ] 2.2 Thread store: load all threads from `.lazydelta/threads/`, save with tmp+rename, unknown-schema loud failure (error names file + versions)
- [ ] 2.3 Crash-safety test: truncated/absent tmp file never breaks load; rename semantics verified on save
- [ ] 2.4 Wire store into the app: threads reload on open, state (statuses/comments/edits) intact

## 3. ACP runner

- [ ] 3.1 Agent runner thread: owns `AcpAgent`, drains outbound prompt queue, forwards `session/update` values over mpsc; FIFO ordering behind in-flight prompt
- [ ] 3.2 Config: project-local agent list (name, exe, args) with spawn-failure surfaced to the thread; agent picker when >1
- [ ] 3.3 Timeout per prompt (default 10 min): error event, drop head, keep queue; session-death error keeps thread state, allows new session
- [ ] 3.4 Stub ACP agent script (`scripts/stub-agent`) echoing canned `session/update` stream; integration test drives runner end-to-end against it

## 4. App: close the loop

- [ ] 4.1 Thread view additions: agent messages panel (streaming), queue-depth indicator, error banner with retry/new-session actions
- [ ] 4.2 Rewire `s` to enqueue `feedback_markdown()` (with thread summary prefix) to the runner; keep preview panel showing what was sent
- [ ] 4.3 Project open flow: pick directory, derive changeset (D2/D3), refresh action `R` with state carry-over and ambiguity warning
- [ ] 4.4 Write-back action: materialize preview / accepted-only to the working tree, with confirmation when overwriting unreviewed external changes
- [ ] 4.5 Keep `--demo` mode working (current hardcoded changeset path)
- [ ] 4.6 Markdown-subset renderer for agent messages (headings, bold/italic/inline-code, fenced blocks, lists, links as text); unit tests over pulldown-cmark events → elements
- [ ] 4.7 Collapsible thread panel: toggle action, per-thread persisted collapse state (default expanded), queue-depth/error indicators still visible when collapsed

## 5. Review surface: external editor handoff

- [ ] 5.1 Editor config with `{file}`/`{line}` template + presets (zed, nvim, code); unset-config error path
- [ ] 5.2 `J` jump action: new-side line mapping (removed-only lines → hunk `new_start`), detached spawn, no-editor-config error surfaced
- [ ] 5.3 Content guarantee: compare working tree vs preview materialization before jump; mismatch → confirm write-back first, cancel aborts
- [ ] 5.4 Refresh-after-return flow: external edits appear as human-provenance hunks, prior review state preserved (reuses task 1.4 matching)

## 6. Review surface: modal bindings

- [ ] 6.1 Keymap table + contexts: NORMAL navigation (`j`/`k`, `g g`, `G`, `] h` / `[ h`, arrow aliases, boundary no-op feedback) bound on the review context; remove bare-letter action bindings
- [ ] 6.2 Leader state machine: `space` → pending context + footer indicator, single-key dispatch (`a`/`r`/`u`/`c`/`s`/`e`), unmatched key and 1 s timeout cancel with feedback
- [ ] 6.3 Insert mode: `i`/`a` on focused added line enter editing, `Esc` commits + returns focus to normal mode; verify no normal binding fires while inserting
- [ ] 6.4 Mode indicator (`NORMAL` / `INSERT`) + leader-pending state in the header/footer; footer hints reflect active mode
- [ ] 6.5 Update README key guide to the modal scheme (space-menu table)

## 7. Verification

- [ ] 7.1 `cargo test --workspace` green including new integration tests; `cargo clippy` clean
- [ ] 7.2 Manual end-to-end: real ACP agent (e.g. claude-code-acp) edits a file in a scratch repo; reviewer edits a line, comments, rejects a hunk, sends feedback, agent replies in-thread (rendered, collapsible); jump to Zed/Nvim at focused line, edit externally, refresh, review the external edit; drive the whole flow with modal keys only (j/k/gg/G/]h, space menu, i/Esc); restart app, state intact
- [ ] 7.3 Update README (try-it steps with a real agent, config example, editor presets, modal keymap) and record the v1 limitations (hunk granularity, refresh-not-watch, no IME, markdown subset, no counts)

## Workflow follow-up

- After all tasks: run `/opsx-archive` to fold deltas into `openspec/specs/` and archive the change.
