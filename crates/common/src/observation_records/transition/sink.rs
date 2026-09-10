//! Transition-record sink abstraction (L4-facing emission plumbing).
//!
//! The actor projects each pure [`RawTransitionRecord`](crate::fsm::RawTransitionRecord) into a
//! serializable, `Instant`-free [`PublishedTransitionRecord`] (see [`super`]) and emits it through
//! this interface. Any further formatting, enrichment, persistence, or transport mapping happens
//! in sink implementations / receivers, not in the actor.

use super::PublishedTransitionRecord;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionSinkError {
    Closed,
}

pub trait TransitionRecordSink: Send + Sync {
    fn emit(&self, record: PublishedTransitionRecord) -> Result<(), TransitionSinkError>;
}

#[derive(Clone)]
pub struct TokioMpscTransitionRecordSink {
    tx: mpsc::UnboundedSender<PublishedTransitionRecord>,
    closed: Arc<AtomicBool>,
}

impl TokioMpscTransitionRecordSink {
    pub fn new(downstream: mpsc::Sender<PublishedTransitionRecord>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let closed = Arc::new(AtomicBool::new(false));
        let closed_by_forwarder = Arc::clone(&closed);
        tokio::spawn(async move {
            while let Some(record) = rx.recv().await {
                if downstream.send(record).await.is_err() {
                    closed_by_forwarder.store(true, Ordering::Release);
                    break;
                }
            }
        });
        Self { tx, closed }
    }
}

impl TransitionRecordSink for TokioMpscTransitionRecordSink {
    fn emit(&self, record: PublishedTransitionRecord) -> Result<(), TransitionSinkError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(TransitionSinkError::Closed);
        }
        self.tx
            .send(record)
            .map_err(|_| TransitionSinkError::Closed)
    }
}
