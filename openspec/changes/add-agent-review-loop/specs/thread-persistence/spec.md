# Spec Delta

## Purpose

Durable review threads: the changeset, review decisions, anchored comments,
human edits, and agent messages survive app restarts via atomic, human-readable
files stored inside the project.

## ADDED Requirements

### Requirement: Threads persist to the project

The app SHALL store each thread as a JSON document under a `.lazydelta/`
directory inside the opened project, one file per thread, with a schema version
field. Threads SHALL reload with all review state (statuses, comments, human
edits, agent messages) intact when the project is reopened.

#### Scenario: Review state survives restart

- **WHEN** the reviewer has accepted a hunk, commented on a line, and edited another line, then restarts the app on the same project
- **THEN** the thread reloads with that hunk accepted, both comments visible, and the human edit applied and still marked as a human fix

### Requirement: Writes are atomic

Saving a thread SHALL write to a temporary file in the same directory and rename
it over the target, so a crash mid-save can never leave a partially written
thread document.

#### Scenario: Crash during save

- **WHEN** the app is killed while saving a thread
- **THEN** the next load yields either the previous complete version or the new complete version, never a truncated document

### Requirement: Unknown schema versions fail loudly

When a thread document carries a schema version the app does not understand,
the app SHALL refuse to load that thread and SHALL surface which file and
version was rejected, rather than silently dropping state.

#### Scenario: Future schema version

- **WHEN** a thread file declares schema version 99 and the app supports 1
- **THEN** loading that thread fails with an error naming the file and the two versions, and other threads still load
