//! lazydelta-acp: a minimal, blocking Agent Client Protocol (ACP) client stub.
//!
//! ACP speaks newline-delimited JSON-RPC 2.0 over stdio (unlike LSP, there is
//! no `Content-Length` framing). This stub covers the client half of the
//! handshake needed to drive an external coding agent:
//!
//! 1. [`AcpAgent::spawn`] the agent binary (e.g. `claude-code-acp`,
//!    `gemini-cli --experimental-acp`, ...),
//! 2. [`AcpAgent::initialize`] with our capabilities,
//! 3. [`AcpAgent::new_session`] for a cwd,
//! 4. [`AcpAgent::prompt`] with review feedback produced by `lazydelta-core`,
//!    collecting `session/update` notifications via [`AcpAgent::take_updates`].
//!
//! This is intentionally synchronous. Wiring it into the GPUI app (reader
//! thread + event channel) is planned in the openspec change
//! `add-agent-review-loop`.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};

use serde_json::{json, Value};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug)]
pub enum AcpError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Rpc { code: i64, message: String },
    Protocol(String),
}

impl std::fmt::Display for AcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcpError::Io(e) => write!(f, "io error: {e}"),
            AcpError::Json(e) => write!(f, "json error: {e}"),
            AcpError::Rpc { code, message } => write!(f, "rpc error {code}: {message}"),
            AcpError::Protocol(msg) => write!(f, "protocol error: {msg}"),
        }
    }
}

impl std::error::Error for AcpError {}

impl From<std::io::Error> for AcpError {
    fn from(e: std::io::Error) -> Self {
        AcpError::Io(e)
    }
}

impl From<serde_json::Error> for AcpError {
    fn from(e: serde_json::Error) -> Self {
        AcpError::Json(e)
    }
}

pub type Result<T> = std::result::Result<T, AcpError>;

/// A blocking ACP agent connection over stdio.
pub struct AcpAgent {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    next_id: u64,
    /// `session/update` notifications received while waiting for responses.
    pending_updates: Vec<Value>,
}

impl AcpAgent {
    pub fn spawn(program: &str, args: &[&str]) -> Result<Self> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        Ok(AcpAgent {
            stdin,
            reader: BufReader::new(stdout),
            child,
            next_id: 1,
            pending_updates: Vec::new(),
        })
    }

    /// ACP `initialize`. Returns the agent's capabilities (raw JSON).
    pub fn initialize(&mut self, client_name: &str) -> Result<Value> {
        self.call(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {},
                "clientInfo": {
                    "name": client_name,
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }),
        )
    }

    /// ACP `session/new`. Returns the session id.
    pub fn new_session(&mut self, cwd: &Path) -> Result<String> {
        let result = self.call("session/new", json!({ "cwd": cwd, "mcpServers": [] }))?;
        result
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| AcpError::Protocol("session/new result missing sessionId".into()))
    }

    /// ACP `session/prompt` with a single text block.
    pub fn prompt(&mut self, session_id: &str, text: &str) -> Result<Value> {
        self.call(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": text }],
            }),
        )
    }

    /// `session/update` notifications collected since the last call.
    pub fn take_updates(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.pending_updates)
    }

    /// Send a JSON-RPC request and block until its response arrives,
    /// buffering any notifications seen along the way.
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        writeln!(self.stdin, "{}", request)?;
        self.stdin.flush()?;

        loop {
            let mut line = String::new();
            let read = self.reader.read_line(&mut line)?;
            if read == 0 {
                return Err(AcpError::Protocol(format!(
                    "agent closed stdout while awaiting response to `{method}` (id {id})"
                )));
            }
            let msg: Value = match serde_json::from_str(line.trim_end()) {
                Ok(v) => v,
                Err(_) => continue, // skip non-JSON noise on stdout
            };
            if msg.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(err) = msg.get("error") {
                    return Err(AcpError::Rpc {
                        code: err.get("code").and_then(Value::as_i64).unwrap_or(0),
                        message: err
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    });
                }
                return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
            }
            if msg.get("method").and_then(Value::as_str) == Some("session/update") {
                self.pending_updates.push(msg);
            }
        }
    }
}

impl Drop for AcpAgent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cat` acts as a degenerate agent: it echoes our request back with the
    /// same id, which our client reads as a `null` result.
    #[test]
    fn initialize_against_echo_agent() {
        let mut agent = AcpAgent::spawn("cat", &[]).expect("spawn cat");
        let result = agent.initialize("lazydelta").expect("initialize");
        assert!(result.is_null());
        assert!(agent.take_updates().is_empty());
    }

    #[test]
    fn new_session_requires_session_id() {
        let mut agent = AcpAgent::spawn("cat", &[]).expect("spawn cat");
        let err = agent.new_session(Path::new("/tmp")).expect_err("no sessionId in echo");
        assert!(err.to_string().contains("sessionId"), "unexpected: {err}");
    }
}
