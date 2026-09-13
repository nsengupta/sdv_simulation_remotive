//! Structured pane lines (semantic styles; no Ratatui).

use super::fit_line;
use common::vehicle_physics::{
    LUX_OFF_THRESHOLD, LUX_ON_THRESHOLD, SpeedBand, SpeedBarCell,
};
use unicode_width::UnicodeWidthStr;

/// Stable identity of a pane row (whole-line policy later).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineRole {
    Standby,
    Notice,
    Speed,
    Visibility,
    /// Retained for Phase I weather/wiper rows; unused in the Phase II active driver view.
    #[allow(dead_code)]
    Weather,
    EngineerState,
    EngineerEvent,
    EngineerHeading,
    EngineerAssembly,
    LedgerRow,
 /// Blank vertical rhythm between Driver segments (presentation only).
    Spacer,
}

/// Semantic style token — mapped to Ratatui colours only in `main`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentStyle {
    Default,
    Mute,
 /// Driver field labels (Notice / Speed / Visibility / Weather).
    Label,
    ZoneGreen,
    ZoneYellow,
    ZoneRed,
}

impl SegmentStyle {
    pub fn from_speed_band(band: SpeedBand) -> Self {
        match band {
            SpeedBand::Green => Self::ZoneGreen,
            SpeedBand::Yellow => Self::ZoneYellow,
            SpeedBand::Red => Self::ZoneRed,
        }
    }
}

/// Driver glyph vocabulary (not on the wire — presentation only).
/// Kept for compatibility; the Phase II active driver view does not emit icons.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverIcon {
    LuxDark,
    LuxHold,
    LuxBright,
    Dry,
    Raining,
    WiperOff,
    WiperOn,
}

impl DriverIcon {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LuxDark => "◼",
            Self::LuxHold => "▦",
            Self::LuxBright => "◻",
            Self::Dry => "☀",
            Self::Raining => "☁",
            Self::WiperOff => "x",
            Self::WiperOn => "≋",
        }
    }

    #[allow(dead_code)]
    pub fn for_ambient_lux(lux: u16) -> Self {
        if lux <= LUX_ON_THRESHOLD {
            Self::LuxDark
        } else if lux >= LUX_OFF_THRESHOLD {
            Self::LuxBright
        } else {
            Self::LuxHold
        }
    }
}

/// Inline content for one segment of a [`PaneLine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentContent {
    Text(String),
    SpeedBar { cells: Vec<SpeedBarCell> },
 /// Reserved: coloured visibility boxes (not emitted yet).
    #[allow(dead_code)]
    Swatch,
    #[allow(dead_code)]
    Icon(DriverIcon),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub style: SegmentStyle,
    pub content: SegmentContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneLine {
    pub role: LineRole,
    pub segments: Vec<Segment>,
}

impl PaneLine {
    pub fn plain(role: LineRole, text: impl Into<String>) -> Self {
        Self {
            role,
            segments: vec![Segment {
                style: SegmentStyle::Default,
                content: SegmentContent::Text(text.into()),
            }],
        }
    }

    pub fn plain_fitted(role: LineRole, text: &str, width: usize) -> Self {
        Self::plain(role, fit_line(text, width))
    }

 /// Blank spacer row occupying `width` columns.
    pub fn spacer(width: usize) -> Self {
        Self {
            role: LineRole::Spacer,
            segments: vec![Segment {
                style: SegmentStyle::Mute,
                content: SegmentContent::Text(" ".repeat(width)),
            }],
        }
    }

 /// Flatten to a single string (tests / width checks).
    pub fn text(&self) -> String {
        let mut out = String::new();
        for seg in &self.segments {
            match &seg.content {
                SegmentContent::Text(s) => out.push_str(s),
                SegmentContent::SpeedBar { cells } => {
                    for c in cells {
                        out.push(if c.filled { '|' } else { '.' });
                    }
                }
                SegmentContent::Swatch => {}
                SegmentContent::Icon(icon) => out.push_str(icon.as_str()),
            }
        }
        out
    }

    pub fn display_width(&self) -> usize {
        self.text().width()
    }

 /// Pad with trailing spaces so the line occupies exactly `width` columns.
    pub fn pad_to_width(mut self, width: usize) -> Self {
        if width == 0 {
            return self;
        }
        let used = self.display_width();
        if used < width {
            self.segments.push(Segment {
                style: SegmentStyle::Mute,
                content: SegmentContent::Text(" ".repeat(width - used)),
            });
        }
        self
    }
}
