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
    /// Classify one sample against the last stored value and update the streak.
    ///
    /// - first sample ever → `Initial`, store it, duplicate count 0
    /// - same as current → `Duplicate`, increment the count
    /// - different from current → `Changed`, report how many duplicates just ended, start a new streak
    ///
    /// `T: Copy` is required by *this* implementation, not by the idea of a streak.
    ///
    /// `match self.current` copies `Option<T>` so the Duplicate arm can compare and leave
    /// the stored sample in place. Without `Copy` that match *moves* `T` out; the Duplicate
    /// arm would drop the only copy and `self.current` would be uninitialized.
    /// `pending_summary` also returns `T` from `&self` (used at actor `post_stop`).
    ///
    /// Callers store `bool` (button pressed, beam ok/fail). `Clone` would let `String` /
    /// `Vec<u8>` compile and clone on every duplicate frame. `Copy` refuses those types.
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
