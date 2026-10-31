//! The bridge look for Home: where its bars, buttons and holes go, and the
//! buttons' labels. deck's bridge module draws the picture and sends it.

use crust::{style, Cursor};
pub use deck::bridge::*;

/// The sidebar's buttons, one per place at Home that can have the keys.
const NAMES: [&str; 4] = ["MESSAGES", "AGENDA", "TASKS", "CLAUDE"];
const COLOURS: [Rgb; 4] = [PEACH, LILAC, PERI, SKY];

/// A button without the keys: its colour at three fifths, dark enough to
/// set the lit one apart and light enough for its black label.
fn dim((r, g, b): Rgb) -> Rgb {
    let d = |v: u8| (v as u16 * 3 / 5) as u8;
    (d(r), d(g), d(b))
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

/// The sidebar's rows beside Claude, from and to: below the crossing and
/// above the bottom corner. Also the buttons' rows and where the red part
/// ends, as (each, join, below, bottom).
fn rows_of(rows: u16, mid: u16) -> (u16, u16, u16, u16) {
    // The buttons share what the corners leave above the bar, up to three rows each.
    let each = (mid.saturating_sub(6) / 4).clamp(1, 3);
    let join = mid.saturating_sub(2).max(2 + 4 * each);
    let below = (mid + 2).max(join + 1);
    (each, join, below, rows.saturating_sub(3).max(below))
}

/// The parts beside Claude, which deck labels: the session, the model,
/// the context, the week's limit, and the state in sky blue, lit with the
/// Claude button (`lit`).
pub fn comms(rows: u16, mid: u16, lit: bool) -> Vec<Part> {
    let (_, _, below, bottom) = rows_of(rows, mid);
    if bottom > below {
        info(below, bottom, if lit { SKY } else { dim(SKY) })
    } else {
        Vec::new()
    }
}

/// The plan for the screen: Home above the bar at row `mid`, Claude
/// below it; the box with the keys lit (`focus` 0 to 3, 3 for Claude),
/// the boxes' headers, and the text spans in the top bar and the middle
/// one (column, cells).
pub fn plan(cols: u16, rows: u16, mid: u16, focus: usize, heads: &[Head], top: &[(u16, u16)], foot: &[(u16, u16)]) -> Plan {
    let mut p = Plan { cols, rows, mid, ..Plan::default() };
    let (each, join, below, bottom) = rows_of(rows, mid);
    p.parts.push((0, 2, ORANGE, None));
    p.lit.push(1 + focus.min(3));
    for (i, name) in NAMES.iter().enumerate() {
        let c = if i == focus { COLOURS[i] } else { dim(COLOURS[i]) };
        p.parts.push((2 + i as u16 * each, each, c, Some(*name)));
    }
    let after = 2 + 4 * each;
    if join > after {
        p.parts.push((after, join - after, RED, Some("")));
    }
    p.parts.push((join, below - join, ORANGE, None));
    let side = comms(rows, mid, focus == 3);
    // The last of them, the state, is lit with the Claude button.
    if focus == 3 && !side.is_empty() {
        p.lit.push(p.parts.len() + side.len() - 1);
    }
    p.parts.extend(side);
    p.parts.push((bottom, rows.saturating_sub(bottom), ORANGE, None));
    p.holes = label_holes(&p.parts);
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
/// that has no name. deck writes the labels beside Claude.
pub fn labels(p: &Plan, clock: &str) -> String {
    let mut s = String::new();
    for (row, rows, c, label) in &p.parts {
        if let Some(name) = label.filter(|n| NAMES.contains(n) || n.is_empty()) {
            let name = if name.is_empty() { clock } else { name };
            let text = format!("{name:>w$} ", w = SIDE as usize - 1);
            s += &(Cursor::at(1, row + rows) + &style::rgb(&text, Some((0, 0, 0)), Some(*c), "b"));
        }
    }
    s
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
        assert_eq!(p.parts.len(), 13);
        assert_eq!(p.parts[1], (2, 3, PEACH, Some("MESSAGES")));
        assert_eq!(p.parts[2], (5, 3, (122, 91, 122), Some("AGENDA")));
        assert_eq!(p.lit, vec![1]);
        assert_eq!(p.parts[6], (31, 4, ORANGE, None), "the crossing");
        // Beside Claude: deck's four buttons of a row each, then the state.
        assert_eq!(p.parts[7], (35, 1, LILAC, Some("tag")));
        assert_eq!(p.parts[11], (39, 9, dim(SKY), Some("state")));
        assert!(p.holes.contains(&(47, 0, SIDE)) && p.holes.contains(&(41, 0, SIDE)) && !p.holes.contains(&(40, 0, SIDE)));
        assert_eq!(comms(51, 33, false), p.parts[7..12].to_vec(), "deck gets the parts gluon draws");
        assert!(!labels(&p, "03:11").contains("tag"), "deck writes those labels");
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
