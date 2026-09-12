//! Observed ECU value vocabulary. Duplicate streak counters belong to actor runtime.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObservedBool {
    #[default]
    Unknown,
    Off,
    On,
}

impl From<bool> for ObservedBool {
    fn from(value: bool) -> Self {
        if value { Self::On } else { Self::Off }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationDisposition {
    Initial,
    Changed { completed_duplicates: u64 },
    Duplicate { current_duplicates: u64 },
    Lifecycle,
}

impl ObservedBool {
    /// Classify a new observation against the currently stored value.
    ///
    /// Task 2 compares previous vs next only. Streak counters are actor runtime
    /// state (Task 3), so `Changed` / `Duplicate` carry `0` here.
    pub fn classify(self, next: bool) -> ObservationDisposition {
        match self {
            Self::Unknown => ObservationDisposition::Initial,
            current if current == Self::from(next) => ObservationDisposition::Duplicate {
                current_duplicates: 0,
            },
            _ => ObservationDisposition::Changed {
                completed_duplicates: 0,
            },
        }
    }
}
