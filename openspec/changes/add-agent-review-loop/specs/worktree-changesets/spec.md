# Spec Delta

## Purpose

Deriving review changesets from a real working directory — git-aware base
detection, working tree as the proposed side — and writing review outcomes back
to the working tree.

## ADDED Requirements

### Requirement: Changesets derive from a real directory

When a thread is opened on a directory, the app SHALL build the changeset with
the base side taken from git `HEAD` when the directory is a git repository, or
from a snapshot captured at thread creation otherwise, and the proposed side
taken from the current file contents on disk. The user SHALL be able to refresh
the changeset to pick up external changes.

#### Scenario: Git worktree thread

- **WHEN** a thread is opened on a git repository with uncommitted modifications
- **THEN** the changeset's hunks reflect the difference between `HEAD` and the working tree, and the file's review shows those hunks

#### Scenario: Non-git directory

- **WHEN** a thread is opened on a directory that is not a git repository
- **THEN** the app snapshots the current contents as the base, and later edits appear as proposed hunks relative to that snapshot

### Requirement: Review outcomes can be written back

The app SHALL write the materialized files back to the working tree on demand,
applying hunks according to the review policy chosen (preview of all
non-rejected hunks, or accepted-only), including human edits, and excluding
additions of rejected hunks.

#### Scenario: Write back after rejecting

- **WHEN** the reviewer rejects a hunk and writes the preview state to the working tree
- **THEN** the affected file on disk contains the accepted/proposed changes with human edits applied, and the rejected hunk's additions are absent

### Requirement: Base detection must be explicit

If git metadata is unreadable in a directory that looks like a repository, the
app SHALL report the failure and ask whether to snapshot instead, rather than
silently treating the working tree as its own base (which would hide all
changes).

#### Scenario: Corrupt git metadata

- **WHEN** a directory contains a `.git` entry that git cannot read
- **THEN** the app shows an error and offers snapshot-based threading as a fallback
