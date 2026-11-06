//! Read-only display names from the control host's native session-alias CLI.

use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use codex_protocol::ThreadId;
use tokio::io::AsyncReadExt;
use tokio::process::Child;
use tokio::process::Command;
use tokio::task::JoinHandle;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::terminal_title::TitleEncoding;
use crate::terminal_title::sanitize_terminal_title;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_OUTPUT_BYTES: u64 = 4096;
const MAX_ALIAS_CHARS: usize = 60;

pub(crate) fn native_command() -> Option<PathBuf> {
    // Use the controller entrypoint, never a shared-fs script or a second registry.
    let command = PathBuf::from("/root/.local/bin/session-alias");
    command.is_file().then_some(command)
}

/// A small shared display value also retained by source-backed history headers.
#[derive(Clone, Debug, Default)]
pub(crate) struct SessionAliasName(Arc<Mutex<AliasDisplayValue>>);

#[derive(Debug, Default)]
struct AliasDisplayValue {
    name: Option<String>,
    revision: u64,
}

impl SessionAliasName {
    pub(crate) fn get(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .name
            .clone()
    }

    pub(crate) fn revision(&self) -> Option<u64> {
        let revision = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .revision;
        (revision > 0).then_some(revision)
    }

    pub(crate) fn update(&self, name: Option<String>) -> bool {
        let name = name.and_then(|name| parse_alias(name.as_bytes()).ok().flatten());
        let mut current = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current.name == name {
            return false;
        }
        current.name = name;
        current.revision = current.revision.saturating_add(1);
        true
    }
}

/// Owns one thread's alias lookup; replacing or dropping it stops the old reader.
pub(crate) struct SessionAlias {
    pub(crate) command: Option<PathBuf>,
    thread_id: Option<ThreadId>,
    name: SessionAliasName,
    task: Option<JoinHandle<()>>,
}

impl SessionAlias {
    pub(crate) fn new(command: Option<PathBuf>) -> Self {
        Self {
            command,
            thread_id: None,
            name: SessionAliasName::default(),
            task: None,
        }
    }

    pub(crate) fn set_thread(&mut self, thread_id: ThreadId, tx: AppEventSender) {
        if self.thread_id == Some(thread_id) {
            return;
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.thread_id = Some(thread_id);
        // Old history cells keep their own value, not the next thread's identity.
        self.name = SessionAliasName::default();
        if let Some(command) = self.command.clone() {
            let name = self.name.clone();
            self.task = Some(tokio::spawn(async move {
                while !tx.app_event_tx.is_closed() {
                    match lookup_alias(&command, thread_id).await {
                        Ok(value) => {
                            if name.update(value) {
                                tx.send(AppEvent::SessionAliasUpdated { thread_id });
                            }
                        }
                        Err(error) => {
                            tracing::debug!(%error, "session alias lookup unavailable");
                            if matches!(
                                error.kind(),
                                io::ErrorKind::NotFound
                                    | io::ErrorKind::PermissionDenied
                                    | io::ErrorKind::TimedOut
                            ) {
                                // A stuck or missing optional tool must not accumulate processes.
                                break;
                            }
                        }
                    }
                    tokio::time::sleep(REFRESH_INTERVAL).await;
                }
            }));
        }
    }

    pub(crate) fn name(&self) -> Option<String> {
        self.name.get()
    }

    pub(crate) fn reader(&self) -> SessionAliasName {
        self.name.clone()
    }
}

impl Drop for SessionAlias {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

struct AliasQuery(Child);

impl Drop for AliasQuery {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0.id() {
            // SAFETY: the unreaped child owns this group, including the CLI's awk reader.
            unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
        }
    }
}

async fn lookup_alias(command: &Path, thread_id: ThreadId) -> io::Result<Option<String>> {
    let mut command = Command::new(command);
    command
        .args(["name", &thread_id.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut query = AliasQuery(command.spawn()?);
    let stdout = query
        .0
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("session alias stdout unavailable"))?;
    let result = tokio::time::timeout(LOOKUP_TIMEOUT, async {
        let mut output = Vec::new();
        stdout
            .take(MAX_OUTPUT_BYTES + 1)
            .read_to_end(&mut output)
            .await?;
        if output.len() as u64 > MAX_OUTPUT_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "session alias output exceeded its limit",
            ));
        }
        if !query.0.wait().await?.success() {
            return Err(io::Error::other("session alias command failed"));
        }
        parse_alias(&output)
    })
    .await;
    result.unwrap_or_else(|_| {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "session alias lookup timed out",
        ))
    })
}

fn parse_alias(output: &[u8]) -> io::Result<Option<String>> {
    let value = std::str::from_utf8(output)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid session alias encoding"))?
        .trim();
    if value.lines().count() > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "session alias output was not a single name",
        ));
    }
    let name: String = sanitize_terminal_title(value, TitleEncoding::Unicode)
        .chars()
        .take(MAX_ALIAS_CHARS)
        .collect();
    Ok((!name.is_empty()).then_some(name))
}

#[cfg(test)]
#[path = "session_alias_tests.rs"]
mod tests;
