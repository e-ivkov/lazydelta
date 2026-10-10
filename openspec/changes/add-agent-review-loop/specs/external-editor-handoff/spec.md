# Spec Delta

## Purpose

Handing a focused review location off to a full external editor (Zed, Neovim,
VS Code, ...) and flowing external edits back into the review, so the review
surface stays the hub while heavy editing happens in a real editor.

## ADDED Requirements

### Requirement: Editor is configurable

The app SHALL support configuring an external editor command with a
file/line-substituting invocation template, and SHALL ship working presets for
at least Zed, Neovim, and VS Code. Invoking the jump action with no editor
configured SHALL surface a clear error pointing at the setting.

#### Scenario: Preset selected

- **WHEN** the reviewer selects the Neovim preset and jumps
- **THEN** the configured command runs with the focused file's path and the target line substituted

### Requirement: Jump opens the reviewed content at the focused line

Invoking jump from a focused diff line SHALL open the external editor at the
line corresponding to the focused line, and the editor SHALL show the same
content the review surface shows for that file (the preview-materialized
state). If the working tree does not match the preview state, the app SHALL
first ask the reviewer to write the preview state back (or cancel), rather than
open mismatched content.

#### Scenario: Straight jump

- **WHEN** the working tree matches the preview state and the reviewer jumps from a focused added line
- **THEN** the editor opens that file at that line

#### Scenario: Preview not materialized

- **WHEN** the reviewer jumps while the working tree differs from the preview state
- **THEN** the app asks for confirmation to write the preview state to the working tree first, and only opens the editor on confirmation

### Requirement: External edits flow back through review

Edits made in the external editor SHALL be picked up by the changeset refresh,
appearing as proposed-side changes with human provenance that can be reviewed
like any other hunk.

#### Scenario: Edit in Neovim, review in lazydelta

- **WHEN** the reviewer edits the file in the external editor, saves, returns, and refreshes
- **THEN** the new edits appear as hunks attributed to the human, and prior review state for unaffected hunks is preserved
