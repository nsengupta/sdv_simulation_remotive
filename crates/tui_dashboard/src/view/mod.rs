//! Pure Dashboard presentation mapping (no I/O, no Ratatui).

mod driver;
mod engineer;
mod ledger_tail;
mod line;

pub use driver::{driver_pane, should_update_notice};
pub use engineer::engineer_pane;
pub use ledger_tail::LedgerTail;
pub use line::{LineRole, PaneLine, Segment, SegmentContent, SegmentStyle};

pub const MISSING: &str = "—";

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Truncate `s` to at most `width` **terminal columns**, appending `…` when shortened.
/// Never inserts a newline.
pub fn clip_line(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if s.width() <= width {
        return s.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }
    let target = width - 1; // reserve one column for ellipsis
    let mut used = 0usize;
    let mut end = 0usize;
    for (idx, ch) in s.char_indices() {
        let w = ch.width().unwrap_or(0);
        if used + w > target {
            break;
        }
        used += w;
        end = idx + ch.len_utf8();
    }
    format!("{}…", &s[..end])
}

/// Clip to `width` columns, then pad with spaces so prior-frame glyphs cannot bleed through.
pub fn fit_line(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let clipped = clip_line(s, width);
    let used = clipped.width();
    if used >= width {
        clipped
    } else {
        format!("{clipped}{}", " ".repeat(width - used))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_line_shortens_and_never_wraps() {
        let clipped = clip_line("abcdefghijklmnopqrstuvwxyz", 10);
        assert_eq!(clipped.width(), 10);
        assert!(clipped.ends_with('…'));
        assert!(!clipped.contains('\n'));
    }

    #[test]
    fn fit_line_pads_to_exact_width() {
        let fitted = fit_line("hi", 8);
        assert_eq!(fitted.width(), 8);
        assert!(fitted.starts_with("hi"));
    }

    #[test]
    fn clip_line_accounts_for_wide_arrows() {
        let s = format!("{}{}", "x".repeat(20), " → more text that is long");
        let clipped = clip_line(&s, 24);
        assert!(clipped.width() <= 24);
        assert!(!clipped.contains('\n'));
    }
}
