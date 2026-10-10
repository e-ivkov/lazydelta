//! End-to-end runner test against the stub ACP agent (task 3.4).

use std::path::PathBuf;
use std::time::Duration;

use lazydelta_app::config::AgentConfig;
use lazydelta_app::runner::{RunnerEvent, spawn_runner};

fn stub_agent_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("scripts")
        .join("stub-agent")
}

fn start_stub_runner(extra_args: &[&str]) -> lazydelta_app::runner::RunnerHandle {
    let script = stub_agent_path();
    let mut args = vec![script.to_string_lossy().into_owned()];
    args.extend(extra_args.iter().map(|a| a.to_string()));
    let cfg = AgentConfig {
        name: "stub".into(),
        command: "python3".into(),
        args,
    };
    spawn_runner(cfg, std::env::temp_dir(), Duration::from_secs(30))
}

fn next_event(
    handle: &lazydelta_app::runner::RunnerHandle,
    pred: impl Fn(&RunnerEvent) -> bool,
    what: &str,
) -> RunnerEvent {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        match handle.recv_timeout(Duration::from_millis(200)) {
            Ok(ev) if pred(&ev) => return ev,
            Ok(_) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => panic!("runner channel closed while waiting for {what}"),
        }
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn runner_streams_replies_and_completes_prompts() {
    let handle = start_stub_runner(&[]);

    handle.prompt("first feedback".into());
    handle.prompt("second feedback".into());

    let mut messages = Vec::new();
    let mut dones = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while dones < 2 && std::time::Instant::now() < deadline {
        match handle.recv_timeout(Duration::from_millis(200)) {
            Ok(RunnerEvent::SessionStarted { .. }) => {}
            Ok(RunnerEvent::Message { text }) => messages.push(text),
            Ok(RunnerEvent::PromptDone) => dones += 1,
            Ok(other) => panic!("unexpected event: {other:?}"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        }
    }
    assert_eq!(dones, 2, "both queued prompts should complete (messages: {messages:?})");
    let joined = messages.join("");
    assert!(joined.contains("Got your feedback"), "messages: {messages:?}");
    assert!(joined.contains("match the style"), "messages: {messages:?}");
    // FIFO: two prompts against the stub each emit two chunks.
    assert!(messages.len() >= 4, "messages: {messages:?}");

    handle.stop();
}

#[test]
fn runner_survives_session_death_and_respawns() {
    // The stub exits after its first prompt; the runner only notices on the
    // next prompt (PromptError, queue head dropped) and then respawns a
    // fresh session transparently.
    let handle = start_stub_runner(&["--die-after-prompt"]);

    handle.prompt("go".into());
    let _ = next_event(&handle, |ev| matches!(ev, RunnerEvent::PromptDone), "first prompt done");

    // Session is dead now; the next prompt errors but keeps the thread alive.
    handle.prompt("again".into());
    match next_event(
        &handle,
        |ev| matches!(ev, RunnerEvent::PromptError { .. }),
        "prompt error after session death",
    ) {
        RunnerEvent::PromptError { error } => assert!(error.contains("restart"), "error: {error}"),
        other => panic!("{other:?}"),
    }

    // The following prompt respawns a session and completes.
    handle.prompt("more".into());
    let _ = next_event(
        &handle,
        |ev| matches!(ev, RunnerEvent::SessionStarted { .. }),
        "respawned session",
    );
    let _ = next_event(&handle, |ev| matches!(ev, RunnerEvent::PromptDone), "prompt after respawn");

    handle.stop();
}

#[test]
fn runner_times_out_hung_prompts() {
    let script = stub_agent_path();
    let cfg = AgentConfig {
        name: "stub".into(),
        command: "python3".into(),
        args: vec![
            script.to_string_lossy().into_owned(),
            "--hang".into(),
        ],
    };
    let handle = spawn_runner(cfg, std::env::temp_dir(), Duration::from_millis(500));
    handle.prompt("are you there".into());
    match next_event(
        &handle,
        |ev| matches!(ev, RunnerEvent::PromptError { .. }),
        "timeout PromptError",
    ) {
        RunnerEvent::PromptError { error } => {
            assert!(error.contains("restart"), "error: {error}")
        }
        other => panic!("{other:?}"),
    }
    // The queue head was dropped; a follow-up prompt respawns and also times
    // out (the stub still hangs), proving the runner kept servicing.
    handle.prompt("again".into());
    match handle.recv_timeout(Duration::from_secs(10)) {
        Ok(RunnerEvent::SessionStarted { .. }) => {}
        other => panic!("expected respawn, got {other:?}"),
    }
    handle.stop();
}

#[test]
fn runner_reports_spawn_failure_as_fatal() {
    let cfg = AgentConfig {
        name: "missing".into(),
        command: "lazydelta-definitely-not-a-binary".into(),
        args: vec![],
    };
    let handle = spawn_runner(cfg, std::env::temp_dir(), Duration::from_secs(5));
    handle.prompt("hi".into());
    match handle.recv_timeout(Duration::from_secs(10)) {
        Ok(RunnerEvent::Fatal { error }) => {
            assert!(error.contains("cannot spawn"), "error: {error}")
        }
        other => panic!("expected Fatal, got {other:?}"),
    }
}
