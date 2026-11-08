//! What the worker does right now, in plain words, plus a short log.
//!
//! The worker writes to it; the app reads snapshots for the Activity view and
//! the status chips. A cancel request goes through the CancelToken of the
//! current job, so a model call stops at once, mid-stream.

use crate::util::now_ms;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Notify;

const LOG_MAX: usize = 300;

/// Shared stop signal for one job.
#[derive(Default)]
pub struct CancelToken {
    flag: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Completes when cancel() is called.
    pub async fn cancelled(&self) {
        loop {
            // A Notified future receives notify_waiters() from the moment it exists.
            let wait = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            wait.await;
            if self.is_cancelled() {
                return;
            }
        }
    }
}

/// One event from a streaming model call.
#[derive(Clone, Copy, Debug)]
pub enum StreamEvent {
    /// The request is sent; no answer bytes yet.
    Connecting,
    /// The server is alive but the request waits in its queue.
    KeepAlive,
    /// Reasoning text arrived (characters).
    Reasoning(usize),
    /// Answer text arrived (characters).
    Content(usize),
    /// Bytes arrived from a server that does not stream.
    Data,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Current {
    pub job_id: i64,
    /// "analyze_item" or "synthesize".
    pub kind: String,
    pub language_id: String,
    pub language_name: String,
    pub item_id: Option<String>,
    pub item_name: Option<String>,
    /// For example "Analyzing \"bracket.png\"".
    pub title: String,
    /// For example "Checking the answer".
    pub phase: String,
    /// Model call number within this job (0 before the first call).
    pub attempt: u32,
    pub max_attempts: u32,
    pub started_ms: i64,
    pub phase_ms: i64,
    /// "", "connecting", "queued", "thinking", "writing", "receiving".
    pub stream: String,
    pub call_started_ms: Option<i64>,
    pub last_data_ms: Option<i64>,
    pub reasoning_chars: u64,
    pub output_chars: u64,
    /// Output token limit of the current call (reasoning + answer).
    pub max_tokens: u64,
    pub cancel_requested: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct LogEntry {
    pub at_ms: i64,
    /// "info", "ok", "warn" or "error".
    pub level: String,
    pub language_id: Option<String>,
    pub text: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Snapshot {
    pub current: Option<Current>,
    pub log: Vec<LogEntry>,
    pub now_ms: i64,
}

struct State {
    current: Option<Current>,
    token: Arc<CancelToken>,
    log: VecDeque<LogEntry>,
}

pub struct Activity {
    state: Mutex<State>,
    /// Manual mode: true from "Run queue" until the queue is empty or you stop it.
    run_requested: AtomicBool,
}

impl Default for Activity {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                current: None,
                token: Arc::new(CancelToken::default()),
                log: VecDeque::new(),
            }),
            run_requested: AtomicBool::new(false),
        }
    }
}

impl Activity {
    fn s(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts a job. Returns its fresh cancel token.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        &self,
        job_id: i64,
        kind: &str,
        language_id: &str,
        language_name: &str,
        item_id: Option<&str>,
        item_name: Option<&str>,
        title: &str,
    ) -> Arc<CancelToken> {
        let now = now_ms();
        let token = Arc::new(CancelToken::default());
        let mut s = self.s();
        s.token = token.clone();
        s.current = Some(Current {
            job_id,
            kind: kind.into(),
            language_id: language_id.into(),
            language_name: language_name.into(),
            item_id: item_id.map(|x| x.to_string()),
            item_name: item_name.map(|x| x.to_string()),
            title: title.into(),
            phase: "Starting".into(),
            started_ms: now,
            phase_ms: now,
            ..Default::default()
        });
        token
    }

    pub fn set_run_requested(&self, on: bool) {
        self.run_requested.store(on, Ordering::SeqCst);
    }

    pub fn run_requested(&self) -> bool {
        self.run_requested.load(Ordering::SeqCst)
    }

    pub fn phase(&self, phase: &str) {
        let mut s = self.s();
        if let Some(c) = s.current.as_mut() {
            c.phase = phase.into();
            c.phase_ms = now_ms();
        }
    }

    /// Starts model call number `attempt`.
    pub fn call_begin(&self, attempt: u32, max_attempts: u32, phase: &str, max_tokens: u64) {
        let now = now_ms();
        let mut s = self.s();
        if let Some(c) = s.current.as_mut() {
            c.attempt = attempt;
            c.max_attempts = max_attempts;
            c.phase = phase.into();
            c.phase_ms = now;
            c.stream = "connecting".into();
            c.call_started_ms = Some(now);
            c.last_data_ms = None;
            c.reasoning_chars = 0;
            c.output_chars = 0;
            c.max_tokens = max_tokens;
        }
    }

    pub fn call_end(&self) {
        let mut s = self.s();
        if let Some(c) = s.current.as_mut() {
            c.stream.clear();
        }
    }

    pub fn stream(&self, ev: StreamEvent) {
        let now = now_ms();
        let mut s = self.s();
        let Some(c) = s.current.as_mut() else { return };
        match ev {
            StreamEvent::Connecting => {
                c.stream = "connecting".into();
                return;
            }
            StreamEvent::KeepAlive => {
                if c.reasoning_chars == 0 && c.output_chars == 0 {
                    c.stream = "queued".into();
                }
            }
            StreamEvent::Reasoning(n) => {
                c.reasoning_chars += n as u64;
                c.stream = "thinking".into();
            }
            StreamEvent::Content(n) => {
                c.output_chars += n as u64;
                c.stream = "writing".into();
            }
            StreamEvent::Data => {
                c.stream = "receiving".into();
            }
        }
        c.last_data_ms = Some(now);
    }

    pub fn end(&self) {
        self.s().current = None;
    }

    pub fn current(&self) -> Option<Current> {
        self.s().current.clone()
    }

    pub fn token(&self) -> Arc<CancelToken> {
        self.s().token.clone()
    }

    /// Stops the running job if its ID matches (or any job when `job_id` is None).
    /// Returns true when a running job got the request.
    pub fn cancel(&self, job_id: Option<i64>) -> bool {
        let mut s = self.s();
        let token = s.token.clone();
        match s.current.as_mut() {
            Some(c) if job_id.map(|j| j == c.job_id).unwrap_or(true) => {
                c.cancel_requested = true;
                c.phase = "Stopping".into();
                token.cancel();
                true
            }
            _ => false,
        }
    }

    /// Stops the running job if it belongs to this language.
    pub fn cancel_language(&self, language_id: &str) -> bool {
        let job = self
            .s()
            .current
            .as_ref()
            .filter(|c| c.language_id == language_id)
            .map(|c| c.job_id);
        match job {
            Some(j) => self.cancel(Some(j)),
            None => false,
        }
    }

    pub fn log(&self, level: &str, language_id: Option<&str>, text: impl Into<String>) {
        let mut s = self.s();
        s.log.push_front(LogEntry {
            at_ms: now_ms(),
            level: level.into(),
            language_id: language_id.map(|x| x.to_string()),
            text: text.into(),
        });
        while s.log.len() > LOG_MAX {
            s.log.pop_back();
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let s = self.s();
        Snapshot {
            current: s.current.clone(),
            log: s.log.iter().cloned().collect(),
            now_ms: now_ms(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_wakes_waiter() {
        let a = Arc::new(Activity::default());
        let token = a.begin(7, "synthesize", "l", "L", None, None, "Writing");
        let a2 = a.clone();
        let h = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            assert!(!a2.cancel(Some(99)));
            assert!(a2.cancel(Some(7)));
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), token.cancelled())
            .await
            .expect("cancelled() must complete");
        h.await.unwrap();
        assert!(a.current().unwrap().cancel_requested);
        // A new job gets a fresh token.
        let t2 = a.begin(8, "analyze_item", "l", "L", Some("i"), Some("a.png"), "Analyzing");
        assert!(!t2.is_cancelled());
    }

    #[test]
    fn stream_states() {
        let a = Activity::default();
        a.begin(1, "synthesize", "l", "L", None, None, "Writing");
        a.call_begin(1, 3, "Waiting for the model", 40_000);
        a.stream(StreamEvent::KeepAlive);
        assert_eq!(a.current().unwrap().stream, "queued");
        a.stream(StreamEvent::Reasoning(100));
        a.stream(StreamEvent::KeepAlive);
        assert_eq!(a.current().unwrap().stream, "thinking");
        a.stream(StreamEvent::Content(40));
        let c = a.current().unwrap();
        assert_eq!((c.stream.as_str(), c.reasoning_chars, c.output_chars), ("writing", 100, 40));
        a.log("info", Some("l"), "hello");
        assert_eq!(a.snapshot().log.len(), 1);
        a.end();
        assert!(a.current().is_none());
    }
}
