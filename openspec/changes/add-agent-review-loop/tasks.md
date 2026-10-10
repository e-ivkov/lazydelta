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

## 5. Verification

- [ ] 5.1 `cargo test --workspace` green including new integration tests; `cargo clippy` clean
- [ ] 5.2 Manual end-to-end: real ACP agent (e.g. claude-code-acp) edits a file in a scratch repo; reviewer edits a line, comments, rejects a hunk, sends feedback, agent replies in-thread; restart app, state intact
- [ ] 5.3 Update README (try-it steps with a real agent, config example) and record the v1 limitations (hunk granularity, refresh-not-watch, no IME)

## Workflow follow-up

- After all tasks: run `/opsx-archive` to fold deltas into `openspec/specs/` and archive the change.
