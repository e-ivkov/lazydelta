//! Agent runner (design D1): one OS thread owns the blocking [`AcpAgent`],
//! drains a FIFO prompt queue, and forwards events over an mpsc channel.
//! Per-prompt timeout is enforced by a detached watchdog that SIGKILLs the
//! agent process; the next prompt lazily respawns a fresh session.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use std::{process::Command, thread};

use lazydelta_acp::AcpAgent;

use crate::config::AgentConfig;

#[derive(Debug)]
pub enum RunnerCmd {
    Prompt { text: String },
    Stop,
}

#[derive(Debug, Clone)]
pub enum RunnerEvent {
    /// The agent process spawned and completed the ACP handshake.
    SessionStarted { agent: String, session_id: String },
    /// A streamed chunk of the agent's reply.
    Message { text: String },
    /// The in-flight prompt completed successfully.
    PromptDone,
    /// The in-flight prompt failed (timeout, protocol, or session death).
    /// The queue head was dropped; the rest of the queue is kept. The next
    /// prompt respawns a fresh session automatically.
    PromptError { error: String },
    /// Unrecoverable (spawn/initialize failed). The runner thread exited.
    Fatal { error: String },
}

pub struct RunnerHandle {
    pub tx: Sender<RunnerCmd>,
    /// Shared so the UI poll task and the handle can both read events.
    pub rx: Arc<Mutex<Receiver<RunnerEvent>>>,
}

impl RunnerHandle {
    pub fn prompt(&self, text: String) {
        let _ = self.tx.send(RunnerCmd::Prompt { text });
    }

    pub fn stop(&self) {
        let _ = self.tx.send(RunnerCmd::Stop);
    }

    pub fn recv_timeout(&self, d: Duration) -> Result<RunnerEvent, std::sync::mpsc::RecvTimeoutError> {
        self.rx.lock().expect("runner receiver poisoned").recv_timeout(d)
    }

    pub fn try_recv(&self) -> Result<RunnerEvent, std::sync::mpsc::TryRecvError> {
        self.rx.lock().expect("runner receiver poisoned").try_recv()
    }
}

/// Spawn the runner thread for `agent`, reviewing project `cwd`.
pub fn spawn_runner(agent: AgentConfig, cwd: PathBuf, prompt_timeout: Duration) -> RunnerHandle {
    let (tx_cmd, rx_cmd) = channel::<RunnerCmd>();
    let (tx_ev, rx_ev) = channel::<RunnerEvent>();
    thread::Builder::new()
        .name(format!("acp-runner-{}", agent.name))
        .spawn(move || run(agent, cwd, prompt_timeout, rx_cmd, tx_ev))
        .expect("spawn runner thread");
    RunnerHandle {
        tx: tx_cmd,
        rx: Arc::new(Mutex::new(rx_ev)),
    }
}

fn run(
    agent_cfg: AgentConfig,
    cwd: PathBuf,
    prompt_timeout: Duration,
    rx_cmd: Receiver<RunnerCmd>,
    tx_ev: Sender<RunnerEvent>,
) {
    let mut queue = std::collections::VecDeque::new();
    let mut session: Option<(AcpAgent, String)> = None;

    'outer: loop {
        // Drain commands; block briefly so Stop is responsive.
        loop {
            match rx_cmd.recv_timeout(Duration::from_millis(50)) {
                Ok(RunnerCmd::Prompt { text }) => queue.push_back(text),
                Ok(RunnerCmd::Stop) => break 'outer,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break 'outer,
            }
        }

        if queue.is_empty() {
            continue;
        }

        // Lazy (re)spawn: on session death the next prompt gets a fresh one.
        if session.is_none() {
            match start_session(&agent_cfg, &cwd) {
                Ok(s) => {
                    let sid = s.1.clone();
                    let _ = tx_ev.send(RunnerEvent::SessionStarted {
                        agent: agent_cfg.name.clone(),
                        session_id: sid,
                    });
                    session = Some(s);
                }
                Err(e) => {
                    let _ = tx_ev.send(RunnerEvent::Fatal { error: e });
                    break 'outer;
                }
            }
        }

        let Some((agent, session_id)) = session.as_mut() else { unreachable!() };
        let text = queue.pop_front().expect("queue non-empty");

        // Watchdog: SIGKILL the agent if the prompt overruns the timeout.
        // Detached: on success the cancel flag makes it a no-op.
        let cancelled = Arc::new(AtomicBool::new(false));
        if let Some(pid) = agent.pid() {
            let flag = cancelled.clone();
            thread::spawn(move || {
                thread::sleep(prompt_timeout);
                if !flag.load(Ordering::SeqCst) {
                    let _ = Command::new("kill")
                        .arg("-9")
                        .arg(pid.to_string())
                        .output();
                }
            });
        }

        let result = agent.prompt(session_id, &text);
        cancelled.store(true, Ordering::SeqCst);

        match result {
            Ok(_) => {
                for update in agent.take_updates() {
                    if let Some(text) = extract_message(&update) {
                        let _ = tx_ev.send(RunnerEvent::Message { text });
                    }
                }
                let _ = tx_ev.send(RunnerEvent::PromptDone);
            }
            Err(e) => {
                // Session is unusable (killed, crashed, or protocol error):
                // drop the head, keep the queue, respawn on the next prompt.
                agent.kill();
                session = None;
                let _ = tx_ev.send(RunnerEvent::PromptError {
                    error: format!(
                        "prompt failed (agent: {}): {e}; session will restart on the next prompt, {} queued",
                        agent_cfg.name,
                        queue.len()
                    ),
                });
            }
        }
    }
}

fn start_session(cfg: &AgentConfig, cwd: &std::path::Path) -> Result<(AcpAgent, String), String> {
    let args: Vec<&str> = cfg.args.iter().map(String::as_str).collect();
    let mut agent = AcpAgent::spawn(&cfg.command, &args)
        .map_err(|e| format!("cannot spawn `{}`: {e}", cfg.command))?;
    agent
        .initialize("lazydelta")
        .map_err(|e| format!("initialize with `{}` failed: {e}", cfg.command))?;
    let session_id = agent
        .new_session(cwd)
        .map_err(|e| format!("session/new with `{}` failed: {e}", cfg.command))?;
    Ok((agent, session_id))
}

/// Pull the text out of an ACP `session/update` `agent_message_chunk`.
fn extract_message(update: &serde_json::Value) -> Option<String> {
    let params = update.get("params")?;
    if params.get("sessionUpdate")?.as_str()? != "agent_message_chunk" {
        return None;
    }
    let content = params.get("content")?;
    if content.get("type")?.as_str()? != "text" {
        return None;
    }
    content.get("text").and_then(|t| t.as_str()).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_agent_message_chunks_only() {
        let chunk = json!({
            "jsonrpc": "2.0", "method": "session/update",
            "params": {
                "sessionId": "s1", "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "partial"}
            }
        });
        assert_eq!(extract_message(&chunk).as_deref(), Some("partial"));

        let other = json!({
            "jsonrpc": "2.0", "method": "session/update",
            "params": {
                "sessionId": "s1", "sessionUpdate": "tool_call",
                "content": {"type": "text", "text": "ignored"}
            }
        });
        assert_eq!(extract_message(&other), None);
    }
}
