//! Transition-record sink abstraction (L4-facing emission plumbing).
//!
//! The actor projects each pure [`RawTransitionRecord`](crate::fsm::RawTransitionRecord) into a
//! serializable, `Instant`-free [`PublishedTransitionRecord`] (see [`super`]) and emits it through
//! this interface. Any further formatting, enrichment, persistence, or transport mapping happens
//! in sink implementations / receivers, not in the actor.

use super::PublishedTransitionRecord;
use async_trait::async_trait;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionSinkError {
    Closed,
}

#[async_trait]
pub trait TransitionRecordSink: Send + Sync {
    async fn emit(&self, record: PublishedTransitionRecord) -> Result<(), TransitionSinkError>;
}

#[derive(Clone)]
pub struct TokioMpscTransitionRecordSink {
    tx: mpsc::Sender<PublishedTransitionRecord>,
}

impl TokioMpscTransitionRecordSink {
    pub fn new(tx: mpsc::Sender<PublishedTransitionRecord>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl TransitionRecordSink for TokioMpscTransitionRecordSink {
    async fn emit(&self, record: PublishedTransitionRecord) -> Result<(), TransitionSinkError> {
        self.tx
            .send(record)
            .await
            .map_err(|_| TransitionSinkError::Closed)
    }
}
