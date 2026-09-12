//! Per-signal duplicate classification owned by child actor runtime.

use crate::vehicle_state::ObservationDisposition;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationStreak<T> {
    current: Option<T>,
    duplicates: u64,
}

impl<T> Default for ObservationStreak<T> {
    fn default() -> Self {
        Self {
            current: None,
            duplicates: 0,
        }
    }
}

impl<T: Copy + Eq> ObservationStreak<T> {
    pub fn observe(&mut self, value: T) -> ObservationDisposition {
        match self.current {
            None => {
                self.current = Some(value);
                self.duplicates = 0;
                ObservationDisposition::Initial
            }
            Some(previous) if previous == value => {
                self.duplicates = self.duplicates.saturating_add(1);
                ObservationDisposition::Duplicate {
                    current_duplicates: self.duplicates,
                }
            }
            Some(_) => {
                let completed_duplicates = self.duplicates;
                self.current = Some(value);
                self.duplicates = 0;
                ObservationDisposition::Changed {
                    completed_duplicates,
                }
            }
        }
    }

    pub fn pending_summary(&self) -> Option<(T, u64)> {
        self.current.map(|value| (value, self.duplicates))
    }
}
