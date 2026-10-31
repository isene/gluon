//! The bridge look for Home: a frame of coloured bars and rounded corners
//! in the style of a starship's screens, painted in pixels through glow.
//! The text shows through holes in the picture. With the look off, none
//! of this runs; with it on, a picture goes out only when the plan changed.

use crust::{style, Cursor};
use glow::{Canvas, Display};

pub type Rgb = (u8, u8, u8);
pub const ORANGE: Rgb = (255, 153, 0);
pub const PEACH: Rgb = (255, 204, 153);
pub const LILAC: Rgb = (204, 153, 204);
pub const PERI: Rgb = (153, 153, 255);
pub const RED: Rgb = (204, 102, 102);
pub const SKY: Rgb = (153, 204, 255);

/// The sidebar's width in columns.
pub const SIDE: u16 = 12;

/// The sidebar's buttons, one per place at Home that can have the keys.
const NAMES: [&str; 4] = ["MESSAGES", "AGENDA", "TASKS", "CLAUDE"];
const COLOURS: [Rgb; 4] = [PEACH, LILAC, PERI, SKY];

/// A button without the keys: its colour at three fifths, dark enough to
/// set the lit one apart and light enough for its black label.
fn dim((r, g, b): Rgb) -> Rgb {
    let d = |v: u8| (v as u16 * 3 / 5) as u8;
    (d(r), d(g), d(b))
}

/// Everything the picture shows, in cells from 0. The same plan draws
/// the same picture, so an unchanged plan sends nothing.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Plan {
    pub cols: u16,
    pub rows: u16,
    /// The row of the bar between Home and Claude.
    pub mid: u16,
    /// Sidebar parts: first row, rows, colour, label ("" for the clock,
    /// which is text alone, so its minutes send no picture). The first
    /// part is the top corner, the last the bottom corner, and the one
    /// between with no label is where the bar at `mid` crosses.
    pub parts: Vec<(u16, u16, Rgb, Option<&'static str>)>,
    /// The parts with the keys, which reach across the gap to their text.
    pub lit: Vec<usize>,
    /// A box's header bar: row, first column, columns, colour.
    pub bars: Vec<(u16, u16, u16, Rgb)>,
    /// Cells that show text: row, column, cells.
    pub holes: Vec<(u16, u16, u16)>,
}

/// One box's header, as Home placed it: row, column and columns of the
/// box (from 0), the title's width, and whether the box has the keys.
pub struct Head {
    pub row: u16,
    pub col: u16,
    pub cols: u16,
    pub title: u16,
    pub focused: bool,
}

/// The plan for the screen: Home above the bar at row `mid`, Claude
/// below it; the box with the keys lit (`focus` 0 to 3, 3 for Claude),
/// the boxes' headers, and the text spans in the top bar and the middle
/// one (column, cells).
pub fn plan(cols: u16, rows: u16, mid: u16, focus: usize, heads: &[Head], top: &[(u16, u16)], foot: &[(u16, u16)]) -> Plan {
    let mut p = Plan { cols, rows, mid, ..Plan::default() };
    // The buttons share what the corners leave above the bar, up to three rows each.
    let each = (mid.saturating_sub(6) / 4).clamp(1, 3);
    p.parts.push((0, 2, ORANGE, None));
    p.lit.push(1 + focus.min(3));
    for (i, name) in NAMES.iter().enumerate() {
        let c = if i == focus { COLOURS[i] } else { dim(COLOURS[i]) };
        p.parts.push((2 + i as u16 * each, each, c, Some(*name)));
    }
    let after = 2 + 4 * each;
    let join = mid.saturating_sub(2).max(after);
    if join > after {
        p.parts.push((after, join - after, RED, Some("")));
    }
    let below = (mid + 2).max(join + 1);
    p.parts.push((join, below - join, ORANGE, None));
    let bottom = rows.saturating_sub(3).max(below);
    if bottom > below {
        // Beside Claude, lit with the Claude button.
        if focus == 3 {
            p.lit.push(p.parts.len());
        }
        p.parts.push((below, bottom - below, if focus == 3 { SKY } else { dim(SKY) }, Some("COMMS")));
    }
    p.parts.push((bottom, rows.saturating_sub(bottom), ORANGE, None));
    for (row, rows, _, label) in &p.parts {
        if label.is_some() {
            p.holes.push((row + rows - 1, 0, SIDE));
        }
    }
    for h in heads {
        let c = if h.focused { ORANGE } else { PERI };
        p.bars.push((h.row, h.col, h.cols.saturating_sub(1), c));
        p.holes.push((h.row, h.col + 2, h.title.min(h.cols.saturating_sub(4))));
    }
    for &(col, n) in top {
        p.holes.push((0, col, n));
    }
    for &(col, n) in foot {
        p.holes.push((mid, col, n));
    }
    p
}

/// The sidebar's labels, black on their buttons, right-aligned on each
/// button's last row, at 1-based cursor positions; `clock` on the part
/// that has no name.
pub fn labels(p: &Plan, clock: &str) -> String {
    let mut s = String::new();
    for (row, rows, c, label) in &p.parts {
        if let Some(name) = label {
            let name = if name.is_empty() { clock } else { name };
            let text = format!("{name:>w$} ", w = SIDE as usize - 1);
            s += &(Cursor::at(1, row + rows) + &style::rgb(&text, Some((0, 0, 0)), Some(*c), "b"));
        }
    }
    s
}

/// The share of pixel (`x`, `y`) inside a rectangle from (`x0`, `y0`) to
/// (`x1`, `y1`), with its corners rounded by `r` (top left, top right,
/// bottom right, bottom left).
fn rect(x: f64, y: f64, (x0, y0, x1, y1): (f64, f64, f64, f64), r: [f64; 4]) -> f64 {
    let over = |a: f64, lo: f64, hi: f64| ((a + 1.0).min(hi) - a.max(lo)).clamp(0.0, 1.0);
    let base = over(x, x0, x1) * over(y, y0, y1);
    if base == 0.0 {
        return 0.0;
    }
    let (cx, cy) = (x + 0.5, y + 0.5);
    let corners = [
        (x0 + r[0], y0 + r[0], r[0], cx < x0 + r[0] && cy < y0 + r[0]),
        (x1 - r[1], y0 + r[1], r[1], cx > x1 - r[1] && cy < y0 + r[1]),
        (x1 - r[2], y1 - r[2], r[2], cx > x1 - r[2] && cy > y1 - r[2]),
        (x0 + r[3], y1 - r[3], r[3], cx < x0 + r[3] && cy > y1 - r[3]),
    ];
    for (ox, oy, rr, inside) in corners {
        if rr > 0.0 && inside {
            let d = ((cx - ox).powi(2) + (cy - oy).powi(2)).sqrt() - rr;
            return (0.5 - d).clamp(0.0, 1.0).min(base);
        }
    }
    base
}

/// The share of pixel (`x`, `y`) in the inside curve of a corner: the
/// square of side `r` from (`x0`, `y0`), less the circle about (`ox`, `oy`).
fn curve(x: f64, y: f64, x0: f64, y0: f64, r: f64, (ox, oy): (f64, f64)) -> f64 {
    let base = rect(x, y, (x0, y0, x0 + r, y0 + r), [0.0; 4]);
    if base == 0.0 {
        return 0.0;
    }
    let d = ((x + 0.5 - ox).powi(2) + (y + 0.5 - oy).powi(2)).sqrt();
    (d - r + 0.5).clamp(0.0, 1.0).min(base)
}

/// Paint `rgb` over the pixels from (`x0`, `y0`) to (`x1`, `y1`), each as
/// much as `cover` says.
fn fill(c: &mut Canvas, (x0, y0, x1, y1): (f64, f64, f64, f64), rgb: Rgb, cover: impl Fn(f64, f64) -> f64) {
    for y in y0.floor().max(0.0) as usize..(y1.ceil() as usize).min(c.h) {
        for x in x0.floor().max(0.0) as usize..(x1.ceil() as usize).min(c.w) {
            let a = cover(x as f64, y as f64);
            if a >= 1.0 {
                // Most pixels are covered whole: no blend to work out.
                let o = (y * c.w + x) * 4;
                c.rgba[o..o + 4].copy_from_slice(&[rgb.0, rgb.1, rgb.2, 255]);
            } else if a > 0.0 {
                c.blend(x as i64, y as i64, rgb, a);
            }
        }
    }
}

/// The picture for a plan. `cell` is None on the terminal, a size in tests.
pub fn picture(p: &Plan, cell: Option<(u16, u16)>) -> Canvas {
    let mut c = Canvas::sized(p.cols, p.rows, cell);
    c.see_through();
    let (cw, ch) = (c.cell_w(), c.cell_h());
    let (w, h) = (c.w as f64, c.h as f64);
    let gap = (ch / 6.0).round().max(2.0);
    let side = SIDE as f64 * cw;
    // The arms along the top and the bottom are as thick as a row; the
    // corners' outer curve is wider, the inner one smaller.
    let (arm, outer, inner) = (ch, (ch * 1.6).min(side / 2.0), (ch * 0.6).min(cw));
    let last = p.parts.len().saturating_sub(1);
    for (i, (row, rows, rgb, label)) in p.parts.iter().enumerate() {
        let y0 = *row as f64 * ch + if i > 0 { gap } else { 0.0 };
        let y1 = (*row + *rows) as f64 * ch;
        if i == 0 {
            let bar = (0.0, 0.0, w, arm);
            let block = (0.0, 0.0, side, y1);
            let cover = |x, y| {
                rect(x, y, block, [outer, 0.0, 0.0, 0.0])
                    .max(rect(x, y, bar, [outer, arm / 2.0, arm / 2.0, 0.0]))
                    .max(curve(x, y, side, arm, inner, (side + inner, arm + inner)))
            };
            // Only the bar runs the whole width; below it are the block and its curve.
            fill(&mut c, bar, *rgb, &cover);
            fill(&mut c, (0.0, arm.ceil(), side + inner, y1), *rgb, &cover);
        } else if i == last {
            let bar = (0.0, h - arm, w, h);
            let block = (0.0, y0, side, h);
            let cover = |x, y| {
                rect(x, y, block, [0.0, 0.0, 0.0, outer])
                    .max(rect(x, y, bar, [0.0, arm / 2.0, arm / 2.0, outer]))
                    .max(curve(x, y, side, h - arm - inner, inner, (side + inner, h - arm - inner)))
            };
            fill(&mut c, (0.0, y0, side + inner, (h - arm).floor()), *rgb, &cover);
            fill(&mut c, bar, *rgb, &cover);
        } else if label.is_none() {
            // The bar between Home and Claude crosses the sidebar here,
            // curving into it above and below.
            let ya = p.mid as f64 * ch;
            let bar = (0.0, ya, w, ya + arm);
            let block = (0.0, y0, side, y1);
            let cover = |x, y| {
                rect(x, y, block, [0.0; 4])
                    .max(rect(x, y, bar, [0.0, arm / 2.0, arm / 2.0, 0.0]))
                    .max(curve(x, y, side, ya - inner, inner, (side + inner, ya - inner)))
                    .max(curve(x, y, side, ya + arm, inner, (side + inner, ya + arm + inner)))
            };
            fill(&mut c, (0.0, y0, side + inner, ya.floor()), *rgb, &cover);
            fill(&mut c, bar, *rgb, &cover);
            fill(&mut c, (0.0, (ya + arm).ceil(), side + inner, y1), *rgb, &cover);
        } else {
            // A part with the keys reaches across the gap, its end rounded.
            let (x1, r) = if p.lit.contains(&i) { (side + cw, cw / 2.0) } else { (side, 0.0) };
            fill(&mut c, (0.0, y0, x1, y1), *rgb, |x, y| rect(x, y, (0.0, y0, x1, y1), [0.0, r, r, 0.0]));
        }
    }
    // A box's header: a bar a little thinner than its row, round at both ends.
    for &(row, col, cols, rgb) in &p.bars {
        let (y0, y1) = (row as f64 * ch + ch * 0.15, (row + 1) as f64 * ch - ch * 0.15);
        let (x0, x1) = (col as f64 * cw, (col + cols) as f64 * cw);
        let r = (y1 - y0) / 2.0;
        fill(&mut c, (x0, y0, x1, y1), rgb, |x, y| rect(x, y, (x0, y0, x1, y1), [r; 4]));
    }
    for &(row, col, n) in &p.holes {
        c.hole(row as usize, col as usize, n as usize);
    }
    c
}

/// The strips that hold all of the picture's colour, as row, column, rows
/// and columns from 0: the arms along the top, the middle and the bottom,
/// the sidebar between them, and the boxes' header bars. The terminal
/// blends a picture again over each row of text that changes under it,
/// so text outside the strips costs nothing extra.
pub fn pieces(p: &Plan) -> Vec<(u16, u16, u16, u16)> {
    // A lit button reaches one column past the sidebar.
    let (strip, last) = (SIDE + 1, p.rows.saturating_sub(1));
    let mut v = vec![
        (0, 0, 1, p.cols),
        (1, 0, p.mid.saturating_sub(1), strip),
        (p.mid, 0, 1, p.cols),
        (p.mid + 1, 0, last.saturating_sub(p.mid + 1), strip),
        (last, 0, 1, p.cols),
    ];
    for &(row, col, cols, _) in &p.bars {
        v.push((row, col, 1, cols));
    }
    v.retain(|&(_, _, rows, cols)| rows > 0 && cols > 0);
    v
}

/// Those cells of a canvas as a canvas of their own.
fn cut(c: &Canvas, (row, col, rows, cols): (u16, u16, u16, u16)) -> Canvas {
    let px = |n: u16, cell: f64, max: usize| ((n as f64 * cell).round() as usize).min(max);
    let (x0, x1) = (px(col, c.cell_w(), c.w), px(col + cols, c.cell_w(), c.w));
    let (y0, y1) = (px(row, c.cell_h(), c.h), px(row + rows, c.cell_h(), c.h));
    let mut rgba = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for y in y0..y1 {
        rgba.extend_from_slice(&c.rgba[(y * c.w + x0) * 4..(y * c.w + x1) * 4]);
    }
    Canvas { cols, rows, cell: c.cell, w: x1 - x0, h: y1 - y0, rgba }
}

/// The picture on the terminal, and the plan it was made from.
pub struct Bridge {
    display: Display,
    shown: Option<Plan>,
}

impl Bridge {
    /// None where the terminal cannot hold a picture under its text.
    pub fn new() -> Option<Bridge> {
        let display = Display::new();
        display.keeps_pictures().then_some(Bridge { display, shown: None })
    }

    /// Put the picture up, unless this plan is the one up already. True
    /// when it was sent.
    pub fn show(&mut self, plan: Plan) -> bool {
        if self.shown.as_ref() == Some(&plan) {
            return false;
        }
        let whole = picture(&plan, None);
        let cuts: Vec<_> = pieces(&plan).into_iter().map(|q| (cut(&whole, q), q.0, q.1)).collect();
        let list: Vec<_> = cuts.iter().map(|(c, row, col)| (c, col + 1, row + 1)).collect();
        self.display.swap_canvases(&list);
        self.shown = Some(plan);
        true
    }

    /// Send it again at the next show, after the screen was cleared.
    pub fn forget(&mut self) {
        self.shown = None;
    }

    pub fn hide(&mut self) {
        self.shown = None;
        self.display.drop_canvas();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Plan {
        let heads = [
            Head { row: 1, col: 13, cols: 80, title: 14, focused: true },
            Head { row: 1, col: 93, cols: 81, title: 8, focused: false },
            Head { row: 17, col: 93, cols: 81, title: 7, focused: false },
        ];
        plan(174, 51, 33, 0, &heads, &[(15, 20), (140, 30)], &[(150, 14)])
    }

    #[test]
    fn the_buttons_fit_between_the_corners() {
        let p = sample();
        assert_eq!(p.parts.len(), 9);
        assert_eq!(p.parts[1], (2, 3, PEACH, Some("MESSAGES")));
        assert_eq!(p.parts[2], (5, 3, (122, 91, 122), Some("AGENDA")));
        assert_eq!(p.lit, vec![1]);
        assert_eq!(p.parts[6], (31, 4, ORANGE, None), "the crossing");
        assert_eq!(p.parts.last().unwrap().0 + p.parts.last().unwrap().1, 51);
        // Short screens get one-row buttons, and the parts still fit.
        let small = plan(80, 16, 9, 3, &[], &[], &[]);
        assert!(small.parts.windows(2).all(|w| w[0].0 + w[0].1 <= w[1].0));
        assert_eq!(small.parts.last().unwrap().0 + small.parts.last().unwrap().1, 16);
    }

    #[test]
    fn text_shows_through_and_the_rest_is_see_through() {
        let c = picture(&sample(), Some((11, 21)));
        let alpha = |col: usize, row: usize| c.rgba[((row * 21 + 10) * c.w + col * 11 + 5) * 4 + 3];
        assert_eq!(alpha(2, 3), 255, "a button");
        assert_eq!(alpha(2, 4), 0, "its label");
        assert_eq!(alpha(50, 10), 0, "inside a box");
        assert_eq!(alpha(20, 1), 0, "a title");
        assert_eq!(alpha(60, 1), 255, "a header bar");
        assert_eq!(alpha(100, 33), 255, "the bar between Home and Claude");
        assert_eq!(alpha(100, 40), 0, "Claude");
        assert_eq!(alpha(2, 40), 255, "the sidebar beside Claude");
        assert_eq!(alpha(12, 3), 255, "the lit button reaches across the gap");
        assert_eq!(alpha(12, 6), 0, "the others stop at the sidebar");
        let piece = cut(&c, (33, 0, 1, 174));
        assert_eq!(piece.rgba[..], c.rgba[33 * 21 * c.w * 4..34 * 21 * c.w * 4], "a cut row is the row");
        if let Ok(path) = std::env::var("BRIDGE_DUMP") {
            std::fs::write(path, c.png()).unwrap();
        }
    }

    #[test]
    fn the_pieces_hold_all_the_colour_and_never_overlap() {
        let heads = [
            Head { row: 1, col: 13, cols: 80, title: 14, focused: true },
            Head { row: 17, col: 93, cols: 81, title: 7, focused: false },
        ];
        for focus in 0..4 {
            let p = plan(174, 51, 33, focus, &heads, &[(15, 20)], &[(150, 14)]);
            let c = picture(&p, Some((11, 21)));
            let mut owner = vec![0u8; 174 * 51];
            for (row, col, rows, cols) in pieces(&p) {
                for r in row..row + rows {
                    for k in col..col + cols {
                        owner[r as usize * 174 + k as usize] += 1;
                    }
                }
            }
            assert!(owner.iter().all(|&n| n <= 1), "pieces overlap");
            for y in 0..c.h {
                for x in 0..c.w {
                    if c.rgba[(y * c.w + x) * 4 + 3] > 0 {
                        assert_eq!(owner[y / 21 * 174 + x / 11], 1, "colour outside the pieces at {x},{y}, focus {focus}");
                    }
                }
            }
            let covered = owner.iter().filter(|&&n| n == 1).count();
            assert!(covered * 5 < owner.len(), "the pieces cover {covered} of {} cells", owner.len());
        }
    }
}
