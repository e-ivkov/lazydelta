# Spec Delta

## Purpose

Vim/Helix-style modal keybindings for the review surface: single-key navigation
in a default normal mode, a space-prefixed leader menu for review actions, and
a proper insert mode when editing a line — so vim users never fight the
keymap while reviewing.

## ADDED Requirements

### Requirement: Modal navigation in normal mode

The review surface SHALL default to a normal mode in which single keystrokes
navigate: `j`/`k` move focus to the next/previous added line, `g g` and `G`
focus the first/last added line, and `] h` / `[ h` jump to the next/previous
hunk. Arrow keys SHALL remain as aliases. Navigation at the boundaries SHALL
be a visible no-op (focus stays, brief feedback), never an error or wrap.

#### Scenario: Hunk jump with no hunks left

- **WHEN** the reviewer presses `] h` on the last hunk
- **THEN** focus stays on the current line and a transient hint confirms there is no next hunk

### Requirement: Leader menu for review actions

Review actions SHALL be reachable via a space-prefixed two-key sequence:
`space a` accept hunk, `space r` reject, `space u` restore, `space c` comment
on the focused line, `space s` send feedback, `space e` jump to the external
editor. When `space` is pressed, the UI SHALL show a pending-leader hint; an
unmatched second key cancels the sequence with visible feedback and performs
nothing.

#### Scenario: Abandoned leader sequence

- **WHEN** the reviewer presses `space` and then `z`
- **THEN** no action runs and the hint clears, indicating the sequence was cancelled

### Requirement: Insert mode for line editing

On a focused added line, `i` or `a` SHALL enter insert mode (line editing);
keystrokes that are normal-mode bindings SHALL NOT fire while inserting, and
typed characters SHALL be inserted into the line. `Esc` SHALL commit the edit
and return to normal mode.

#### Scenario: Typing a binding letter while inserting

- **WHEN** the reviewer is inserting and types `r`
- **THEN** the character `r` is inserted into the line and the hunk is not rejected

#### Scenario: Esc commits

- **WHEN** the reviewer presses `Esc` while inserting
- **THEN** the edit is recorded (human-fix provenance applies) and the surface returns to normal mode

### Requirement: Mode state is always visible

The surface SHALL display the current mode (`NORMAL` / `INSERT`) and the
pending-leader state, and the footer key hints SHALL reflect the active mode.

#### Scenario: Mode indicator follows state

- **WHEN** the reviewer presses `i` on a focused line and then `Esc`
- **THEN** the indicator shows `INSERT` while editing and `NORMAL` after `Esc`
