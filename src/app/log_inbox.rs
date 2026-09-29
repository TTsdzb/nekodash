use nekodash_core::{SessionToken, models::Log};
use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard},
};

// Keep bursts bounded independently of the UI command queue.
const CAPACITY: usize = 4096;
#[derive(Default)]
pub(super) struct LogInbox(Mutex<Queue>);
#[derive(Default)]
struct Queue {
    token: Option<SessionToken>,
    logs: VecDeque<Log>,
    dropped: u64,
    gap: Option<String>,
}
pub(super) struct Batch {
    pub token: SessionToken,
    pub logs: VecDeque<Log>,
    pub dropped: u64,
    pub gap: Option<String>,
}
impl LogInbox {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        match self.0.lock() {
            Ok(queue) => queue,
            // Queue mutations preserve valid Rust values even if a caller unwinds.
            // Recover the bounded buffer instead of panicking on a poisoned lock.
            Err(poisoned) => poisoned.into_inner(),
        }
    }
    pub fn activate(&self, token: SessionToken) {
        *self.lock() = Queue {
            token: Some(token),
            ..Queue::default()
        };
    }
    pub fn push(&self, token: &SessionToken, log: Log) {
        let mut queue = self.lock();
        if queue.token.as_ref() != Some(token) {
            return;
        }
        if queue.logs.len() >= CAPACITY {
            queue.logs.pop_front();
            queue.dropped = queue.dropped.saturating_add(1);
        }
        queue.logs.push_back(log);
    }
    pub fn gap(&self, token: &SessionToken, message: String) {
        let mut queue = self.lock();
        if queue.token.as_ref() == Some(token) {
            queue.gap = Some(message);
        }
    }
    pub fn take(&self, limit: usize) -> Option<Batch> {
        let mut queue = self.lock();
        let token = queue.token.clone()?;
        if queue.logs.is_empty() && queue.gap.is_none() {
            return None;
        }
        while queue.logs.len() > limit {
            queue.logs.pop_front();
            queue.dropped = queue.dropped.saturating_add(1);
        }
        Some(Batch {
            token,
            logs: std::mem::take(&mut queue.logs),
            dropped: std::mem::take(&mut queue.dropped),
            gap: queue.gap.take(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn token(generation: u64) -> SessionToken {
        SessionToken {
            endpoint_id: "test".into(),
            generation,
        }
    }
    fn log(i: usize) -> Log {
        Log {
            level: "debug".into(),
            payload: i.to_string(),
        }
    }
    #[test]
    fn burst_keeps_latest_logs_and_accounts_for_loss() -> Result<(), Box<dyn std::error::Error>> {
        let inbox = LogInbox::default();
        inbox.activate(token(1));
        for i in 0..100_000 {
            inbox.push(&token(1), log(i));
        }
        let batch = inbox.take(1000).ok_or("missing batch")?;
        assert_eq!(batch.logs.len(), 1000);
        assert_eq!(batch.dropped, 99_000);
        assert_eq!(
            batch.logs.front().map(|v| v.payload.as_str()),
            Some("99000")
        );
        assert_eq!(batch.logs.back().map(|v| v.payload.as_str()), Some("99999"));
        assert!(inbox.take(1000).is_none());
        Ok(())
    }
    #[test]
    fn endpoint_switch_rejects_old_logs_and_gaps() -> Result<(), Box<dyn std::error::Error>> {
        let inbox = LogInbox::default();
        inbox.activate(token(1));
        inbox.push(&token(1), log(1));
        inbox.activate(token(2));
        inbox.push(&token(1), log(2));
        inbox.gap(&token(1), "old".into());
        assert!(inbox.take(1000).is_none());
        inbox.push(&token(2), log(3));
        inbox.gap(&token(2), "gap".into());
        let batch = inbox.take(1000).ok_or("missing batch")?;
        assert_eq!(batch.token, token(2));
        assert_eq!(batch.logs.len(), 1);
        assert_eq!(batch.gap.as_deref(), Some("gap"));
        assert!(inbox.take(1000).is_none());
        Ok(())
    }
}
