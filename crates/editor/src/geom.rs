//! Galley geometry helpers: char index ⇄ rows, rects for char ranges. Coordinates are
//! galley-relative; the caller adds the chunk origin and any per-row image shift.

use egui::text::{CCursor, CharIndex};
use egui::{Galley, Rect, pos2};

/// Char index where each row starts (rows include their implicit `\n` in the count).
pub(crate) fn row_starts(galley: &Galley) -> Vec<usize> {
    let mut out = Vec::with_capacity(galley.rows.len());
    let mut ci = 0usize;
    for row in &galley.rows {
        out.push(ci);
        ci += row.char_count_including_newline().0;
    }
    out
}

/// Total number of chars in the galley (including row newlines).
pub(crate) fn char_count(galley: &Galley) -> usize {
    galley.rows.iter().map(|r| r.char_count_including_newline().0).sum()
}

/// Row index of a char index (a caret at a wrap boundary goes to the next row).
pub(crate) fn row_of_char(galley: &Galley, ci: usize) -> usize {
    if galley.rows.is_empty() {
        return 0;
    }
    let lc = galley.layout_from_cursor(CCursor { index: CharIndex(ci), prefer_next_row: true });
    lc.row.min(galley.rows.len() - 1)
}

/// Caret rect (zero width) and row for char index `ci`.
pub(crate) fn caret_rect(galley: &Galley, ci: usize) -> (Rect, usize) {
    let ci = ci.min(char_count(galley));
    let row = row_of_char(galley, ci);
    let r = galley.pos_from_cursor(CCursor { index: CharIndex(ci), prefer_next_row: true });
    let r = if r.height() <= 0.0 || !r.is_finite() {
        // Fallback for empty galleys.
        let h = galley.rows.get(row).map_or(16.0, |r| r.height().max(1.0));
        Rect::from_min_max(pos2(r.min.x.max(0.0), 0.0), pos2(r.min.x.max(0.0), h))
    } else {
        r
    };
    (r, row)
}

/// Char index at galley x on `row`.
pub(crate) fn char_at_row_x(galley: &Galley, row: usize, x: f32) -> usize {
    let starts = row_starts(galley);
    let Some(r) = galley.rows.get(row) else { return char_count(galley) };
    starts[row] + r.char_at(x - r.pos.x).0
}

/// Start and end (excluding newline) char indices of `row`.
pub(crate) fn row_bounds(galley: &Galley, row: usize) -> (usize, usize) {
    let starts = row_starts(galley);
    match galley.rows.get(row) {
        Some(r) => (starts[row], starts[row] + r.char_count_excluding_newline().0),
        None => (char_count(galley), char_count(galley)),
    }
}

/// Rects (with row index) covering chars `[s, e)`. If `e` passes the end of a row that has a
/// newline (or the galley end, meaning the chunk's own final `\n`), the rect is extended by
/// `nl_w` so selected newlines are visible.
pub(crate) fn range_rects(galley: &Galley, s: usize, e: usize, nl_w: f32) -> Vec<(usize, Rect)> {
    let mut out = Vec::new();
    if e <= s {
        return out;
    }
    let mut ci = 0usize;
    let last = galley.rows.len().saturating_sub(1);
    for (ri, row) in galley.rows.iter().enumerate() {
        let n = row.char_count_excluding_newline().0;
        let nl = usize::from(row.ends_with_newline);
        let row_start = ci;
        let row_end = ci + n;
        let covers_nl = (nl == 1 || ri == last) && e > row_end;
        if s < row_end + 1 && e > row_start && (s < row_end || covers_nl) {
            let a = s.max(row_start) - row_start;
            let b = e.min(row_end) - row_start;
            let x0 = row.pos.x + row.x_offset(CharIndex(a));
            let mut x1 = row.pos.x + row.x_offset(CharIndex(b));
            if covers_nl {
                x1 += nl_w;
            }
            out.push((ri, Rect::from_min_max(pos2(x0, row.min_y()), pos2(x1, row.max_y()))));
        }
        ci += n + nl;
        if ci >= e {
            break;
        }
    }
    out
}

/// Vertical extent (min_y, max_y, first row, last row) of the rows touched by chars `[s, e)`.
pub(crate) fn rows_span(galley: &Galley, s: usize, e: usize) -> Option<(usize, usize)> {
    if galley.rows.is_empty() {
        return None;
    }
    let r0 = row_of_char(galley, s);
    let r1 = if e > s { row_of_char_end(galley, e) } else { r0 };
    Some((r0, r1.max(r0)))
}

/// Row containing the char just before `e` (end of a range).
fn row_of_char_end(galley: &Galley, e: usize) -> usize {
    let starts = row_starts(galley);
    let i = starts.partition_point(|&st| st < e);
    i.saturating_sub(1).min(galley.rows.len().saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::text::LayoutJob;
    use egui::{Color32, FontId};

    fn galley(ctx: &egui::Context, text: &str, wrap: f32) -> std::sync::Arc<Galley> {
        let job = LayoutJob::simple(text.to_owned(), FontId::proportional(14.0), Color32::WHITE, wrap);
        ctx.fonts_mut(|f| f.layout_job(job))
    }

    fn with_ctx(f: impl FnOnce(&egui::Context)) {
        let ctx = egui::Context::default();
        let mut f = Some(f);
        ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(f) = f.take() {
                f(ui.ctx());
            }
        })
        .drop_without_applying_deltas();
    }

    #[test]
    fn rows_and_rects() {
        with_ctx(|ctx| {
            let g = galley(ctx, "ab\ncde\n", f32::INFINITY);
            assert_eq!(g.rows.len(), 3);
            assert_eq!(row_starts(&g), vec![0, 3, 7]);
            assert_eq!(char_count(&g), 7);
            assert_eq!(row_of_char(&g, 0), 0);
            assert_eq!(row_of_char(&g, 2), 0);
            assert_eq!(row_of_char(&g, 3), 1);
            assert_eq!(row_of_char(&g, 7), 2);
            assert_eq!(row_bounds(&g, 1), (3, 6));
            let rr = range_rects(&g, 1, 5, 4.0);
            assert_eq!(rr.len(), 2);
            assert_eq!(rr[0].0, 0);
            assert_eq!(rr[1].0, 1);
            assert!(rr[0].1.width() > 0.0);
            assert_eq!(rows_span(&g, 0, 3), Some((0, 0)));
            assert_eq!(rows_span(&g, 0, 4), Some((0, 1)));
            let (c, row) = caret_rect(&g, 4);
            assert_eq!(row, 1);
            assert!(c.height() > 0.0);
            assert_eq!(char_at_row_x(&g, 1, -5.0), 3);
            assert_eq!(char_at_row_x(&g, 1, 1e6), 6);
            // Empty galley.
            let e = galley(ctx, "", f32::INFINITY);
            let (c, _) = caret_rect(&e, 3);
            assert!(c.height() > 0.0);
            assert!(range_rects(&e, 0, 1, 4.0).len() == 1);
        });
    }
}
