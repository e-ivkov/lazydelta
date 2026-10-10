# Spec Delta

## Purpose

Driving an external coding agent over ACP from the review surface: configuring
agents, keeping one ordered session per thread, and turning review artifacts
(comments + human edits) into agent feedback with visible, non-lossy error
handling.

## ADDED Requirements

### Requirement: Agent configuration

The app SHALL let the user configure one or more external ACP agent commands
(name + executable + args) in a project-local settings file, and SHALL show a
clear error at thread start if a configured agent binary cannot be spawned.

#### Scenario: Agent binary missing

- **WHEN** the user starts a thread with an agent whose executable is not on `PATH`
- **THEN** the thread shows an error identifying the agent and the spawn failure, and no session is created

#### Scenario: Multiple agents configured

- **WHEN** more than one agent is configured and the user creates a thread
- **THEN** the user can choose which agent the thread uses

### Requirement: One ordered session per thread

The app SHALL maintain at most one ACP session per thread, created on first use,
and SHALL deliver prompts to it in the order the reviewer sent them. While a
prompt is in flight, further sends SHALL be queued (not dropped) and the queue
depth SHALL be visible in the thread.

#### Scenario: Send while busy

- **WHEN** the reviewer sends feedback while the agent is still responding to the previous one
- **THEN** the new feedback is queued and sent after the current response completes, and the thread indicates that it is queued

#### Scenario: Session dies mid-response

- **WHEN** the agent process exits while responding
- **THEN** the thread shows an error, keeps all prior review state and messages, and the reviewer can start a new session that continues the same thread

### Requirement: Review feedback is delivered as a prompt

When the reviewer sends feedback, the app SHALL serialize the thread's review
artifacts — accepted/rejected summary, comments, and human edits as explicit
before/after pairs — into a single prompt and deliver it to the thread's agent
session.

#### Scenario: Human edits become critique

- **WHEN** the reviewer edited an added line during review and then sends feedback
- **THEN** the prompt contains the agent's original line and the reviewer's replacement, asking the agent to match the change

### Requirement: Agent replies stream into the thread

The app SHALL display agent messages for the thread's session as they arrive
(while the response is in flight), and SHALL persist them as part of the thread
once the response completes.

#### Scenario: Streamed reply

- **WHEN** the agent is responding to a prompt
- **THEN** the thread view shows message content incrementally, and after completion the full message is stored with the thread
