//! gluon: mail, calendar, tasks and Claude in one terminal app. The Home
//! tab shows unread mail, the agenda and the tasks over a Claude pane
//! (deck); the Mail tab is kastrup and the Calendar tab is tock, all run
//! as libraries; the Tasks tab is scribe on ~/.tasks/todo.hl, or the
//! same file as a board. Ctrl+O moves to
//! the next tab, and Ctrl+A jumps Home and back.

use crust::{style, Crust, Cursor, Input, Pane};
use std::collections::HashSet;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

const TABS: [&str; 4] = ["Home", "Mail", "Calendar", "Tasks"];
const HOME: usize = 0;
const MAIL: usize = 1;
const CALENDAR: usize = 2;
const TASKS: usize = 3;

/// Claude's CLAUDE.md, written to ~/.gluon once; edits there stay.
const BRIEF: &str = "# gluon's Claude pane

You run in the bottom half of the Home tab of gluon, a terminal app with
four tabs: Home (unread mail, the agenda and the tasks over this pane),
Mail (kastrup), Calendar (tock) and Tasks.

Ctrl+O moves to the next tab, Ctrl+A jumps Home and back, and Ctrl+Z
gives this pane the whole screen and back. When Ctrl+A comes from another
tab, the next prompt starts with an <on-screen> block saying what that
tab showed.

At Home, Tab moves the keys from box to box and on to this pane. Enter
in a box opens its item in its own tab, and Esc there comes back.

Where the user's things are:
- Mail, RSS and chat: kastrup's SQLite database, ~/.kastrup/kastrup.db.
  kastrup:<id> is a row in its messages table. Use the kastrup skill when
  there is one. A draft goes in ~/.kastrup/drafts/, and kastrup lists it
  on +.
- Calendar: tock, in ~/.tock/. gluon has tock's store open, so never
  write to it. Add an event the way the calendar skill says, as a file the
  running tock imports. tock:<id> is an event; the text pasted after it
  says what it is, since the store cannot be read while gluon runs.
- Tasks: ~/.tasks/todo.hl, a HyperList file indented with tabs. Edit it
  in place; the Tasks tab reads it again each time it is shown. todo:<n>
  is line n of it.

y in a box at Home copies the picked item as kastrup:<id>, tock:<id>
or todo:<n>, with its text. When the user moves from a box to
this pane, the next prompt starts with an <on-screen> block holding the
item that was picked there.

Home lists the unread mail in the kastrup views whose keys are in
~/.gluon/views, as \"1 2 3\" (the default), each with its view's key in
the view's colour.

To change a view or make one (\"a view 4 for coaching, dark amber, with
a box for new mail\"):
- A view is a row in the views table of kastrup.db: name, key_binding
  (the key that opens it), filters (JSON), created_at and updated_at (Unix
  seconds). kastrup reads a changed view within five seconds.
- filters, for example: {\"rules\": [{\"field\": \"folder\", \"op\": \"=\",
  \"value\": \"Work\"}], \"top_bg\": \"136\", \"notify\": true}
- All rules in a list apply at once. \"branches\": [{\"rules\": [...]}, ...]
  takes a message that any one branch takes.
- A rule's field is folder, sender, subject, source_id, source_type,
  platform, read or starred. sender and subject take \"a|b|c\": any of the
  words, anywhere, in any case. folder takes op \"=\" or \"like\".
- top_bg is the view's colour, a 256-colour number as a string; dark
  amber is 136.
- \"notify\": true shows a box at the top right for each new unread
  message in the view.
- Turn a loose request into fixed rules once: find the folders, senders
  and subject words in kastrup.db that cover it, say what the view will
  take, then write it. Never judge each message with a model.
- Add the new key to ~/.gluon/views when the user wants it at Home.

This pane sits beside the user's work: answer short.
";

/// The tab names for the first line, the one in view marked, then the
/// key that moves on.
fn tab_line(active: usize) -> String {
    let mut line = String::from(" ");
    for (i, name) in TABS.iter().enumerate() {
        if i == active {
            line.push_str(&style::bold(&style::fg(name, 156)));
        } else {
            line.push_str(&style::fg(name, 245));
        }
        line.push_str("  ");
    }
    line.push_str(&style::fg("^O ", 240));
    line
}

/// A tab out of view still does its idle work (new mail, scheduled
/// sends, alarms, Claude's answer), with what it draws thrown away so it
/// does not paint over the tab in view. Two dup2 calls a turn; /dev/null
/// stays open.
struct Unseen {
    null: std::fs::File,
    screen: i32,
}

impl Unseen {
    fn new() -> Self {
        let null = std::fs::OpenOptions::new().write(true).open("/dev/null")
            .expect("/dev/null");
        let screen = unsafe { libc::dup(1) };
        Unseen { null, screen }
    }

    fn run<T>(&self, f: impl FnOnce() -> T) -> T {
        std::io::stdout().flush().ok();
        unsafe { libc::dup2(self.null.as_raw_fd(), 1) };
        let out = f();
        std::io::stdout().flush().ok();
        unsafe { libc::dup2(self.screen, 1) };
        out
    }
}

/// The Tasks tab: scribe on the todo file, or the file as a board
/// (Ctrl+B switches).
struct Tasks {
    top: Pane,
    body: Pane,
    bottom: Pane,
    path: PathBuf,
    board: bool,
    /// scribe, opened the first time the tab shows it.
    ed: Option<scribe::App>,
    lanes: Vec<Lane>,
    /// The picked place on the board: lane, column and card in that cell.
    pick: (usize, usize, usize),
}

/// The board's columns, first to last, each with the box it writes in
/// front of a task in todo.hl. A task with no box is to do.
const COLUMNS: [(&str, &str); 4] = [("To do", ""), ("Doing", "[O] "), ("Waiting", "[-] "), ("Done", "[x] ")];

/// A lane of the board: a line at the top level of todo.hl, and its tasks,
/// the lines one level in. Each knows its line in the file.
struct Lane {
    line: usize,
    name: String,
    cards: Vec<Card>,
}

struct Card {
    line: usize,
    text: String,
    col: usize,
}

/// A task's column, from the box in front of it, and the task without it.
fn column_of(task: &str) -> (usize, &str) {
    for (b, c) in [("[_] ", 0), ("[ ] ", 0), ("[O] ", 1), ("[o] ", 1), ("[-] ", 2), ("[x] ", 3), ("[X] ", 3)] {
        if let Some(rest) = task.strip_prefix(b) {
            return (c, rest);
        }
    }
    (0, task)
}

fn depth(line: &str) -> usize {
    line.len() - line.trim_start_matches('\t').len()
}

fn lanes_of(text: &str) -> Vec<Lane> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(top) = lines.iter().filter(|l| !l.trim().is_empty()).map(|l| depth(l)).min() else {
        return Vec::new();
    };
    let mut lanes: Vec<Lane> = Vec::new();
    for (i, l) in lines.iter().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
        if depth(l) == top {
            lanes.push(Lane { line: i, name: l.trim().into(), cards: Vec::new() });
        } else if let (true, Some(lane)) = (depth(l) == top + 1, lanes.last_mut()) {
            let (col, text) = column_of(l.trim_start_matches('\t'));
            lane.cards.push(Card { line: i, text: text.trim_end().into(), col });
        }
    }
    lanes
}

/// Where the item on line `i` ends: after the last line below it that is
/// further in.
fn block_end(lines: &[String], i: usize) -> usize {
    let d = depth(&lines[i]);
    let mut end = i + 1;
    for (j, l) in lines.iter().enumerate().skip(i + 1) {
        if l.trim().is_empty() {
            continue;
        }
        if depth(l) <= d {
            break;
        }
        end = j + 1;
    }
    end
}

/// A task's line with the box of column `col` in front.
fn in_column(line: &str, col: usize) -> String {
    let tabs = depth(line);
    format!("{}{}{}", &line[..tabs], COLUMNS[col].1, column_of(&line[tabs..]).1)
}

/// The task on line `line`, with what is under it, moved to the end of the
/// lane on line `lane`.
fn to_lane(lines: &mut Vec<String>, line: usize, lane: usize) {
    let end = block_end(lines, line);
    let block: Vec<String> = lines.drain(line..end).collect();
    let lane = if lane > line { lane - block.len() } else { lane };
    let at = block_end(lines, lane);
    lines.splice(at..at, block);
}

/// The file with `f` done to its lines. Written in place, so a link or the
/// file's permissions stay.
fn rewrite(path: &Path, f: impl FnOnce(&mut Vec<String>)) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    f(&mut lines);
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    let _ = std::fs::write(path, out);
}

/// `s` in at most `n` lines of `w` columns, broken at spaces.
fn wrap(s: &str, w: usize, n: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if !cur.is_empty() && crust::display_width(&cur) + 1 + crust::display_width(word) > w {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    out.push(cur);
    if out.len() > n {
        out.truncate(n);
        out[n - 1].push_str(" …");
    }
    out.into_iter().map(|l| if crust::display_width(&l) > w { crust::truncate_ansi(&l, w) } else { l }).collect()
}

/// `s` cut or filled with spaces to `w` columns.
fn pad(s: &str, w: usize) -> String {
    let t = crust::truncate_ansi(s, w);
    let n = w.saturating_sub(crust::display_width(&t));
    t + &" ".repeat(n)
}

impl Tasks {
    fn new(home: &Path) -> Self {
        let mut t = Tasks {
            top: Pane::new(1, 1, 1, 1, 255, 236),
            body: Pane::new(1, 2, 1, 1, 255, 0),
            bottom: Pane::new(1, 3, 1, 1, 252, 236),
            path: home.join(".tasks").join("todo.hl"),
            board: false,
            ed: None,
            lanes: Vec::new(),
            pick: (0, 0, 0),
        };
        t.layout();
        t
    }

    fn layout(&mut self) {
        let (cols, rows) = Crust::terminal_size();
        self.top = Pane::new(1, 1, cols, 1, 255, 236);
        self.body = Pane::new(2, 2, cols.saturating_sub(2), rows.saturating_sub(2), 255, 0);
        self.body.scroll = true;
        self.bottom = Pane::new(1, rows, cols, 1, 252, 236);
    }

    /// scribe on the task list, at `line` (0 leaves the cursor where it
    /// was, or where scribe's session has it).
    fn editor(&mut self, line: usize) -> &mut scribe::App {
        if self.ed.is_none() {
            if let Some(dir) = self.path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let mut ed = scribe::App::open(self.path.clone(), line);
            ed.set_tab_line(Some(tab_line(TASKS) + &style::fg("^B board ", 240)));
            self.ed = Some(ed);
        }
        let ed = self.ed.as_mut().expect("opened above");
        ed.goto_line(line);
        ed
    }

    /// scribe at `line`, in view.
    fn edit_at(&mut self, line: usize) {
        self.board = false;
        self.editor(line);
        self.repaint();
    }

    /// Unsaved text is written when the tab goes out of view, so the board
    /// and Home show it.
    fn leave(&mut self) {
        if let (false, Some(ed)) = (self.board, self.ed.as_mut()) {
            ed.save_if_dirty();
        }
    }

    fn repaint(&mut self) {
        if !self.board {
            self.editor(0).repaint();
            return;
        }
        Crust::clear_screen();
        self.layout();
        self.top.say(&tab_line(TASKS));
        self.draw_board();
        self.bottom.say(&style::fg(
            " arrows pick  S-←→ column  S-↑↓ section  e edit  ^B scribe  Ctrl+O next tab  Ctrl+A Home  Q quit",
            245,
        ));
    }

    /// The lanes over each other, the columns side by side, and the
    /// picked card kept in view.
    fn draw_board(&mut self) {
        self.lanes = lanes_of(&std::fs::read_to_string(&self.path).unwrap_or_default());
        let n = COLUMNS.len();
        let cw = (self.body.w as usize).saturating_sub(n - 1) / n;
        let mut out = Vec::new();
        let heads: Vec<String> = COLUMNS.iter().enumerate().map(|(c, (name, _))| {
            let k = self.lanes.iter().flat_map(|l| &l.cards).filter(|x| x.col == c).count();
            pad(&format!("{} {}", style::bold(&style::fg(name, 255)), style::fg(&k.to_string(), 240)), cw)
        }).collect();
        out.push(heads.join(" "));
        out.push(vec![style::fg(&"─".repeat(cw), 238); n].join(" "));
        let mut at = 0;
        for (li, lane) in self.lanes.iter().enumerate() {
            let head = style::bold(&style::fg(&format!("▾ {}", lane.name), 156)) + &style::fg(&format!(" {} ", lane.cards.len()), 240);
            let rule = "─".repeat((self.body.w as usize).saturating_sub(crust::display_width(&head)));
            out.push(head + &style::fg(&rule, 237));
            let mut cells: Vec<Vec<String>> = vec![Vec::new(); n];
            for (c, cell) in cells.iter_mut().enumerate() {
                // An empty cell, picked, shows as a slot.
                if self.pick == (li, c, 0) && !lane.cards.iter().any(|x| x.col == c) {
                    at = out.len();
                    cell.push(style::styled(&" ".repeat(cw), None, Some(238), ""));
                }
                for (i, card) in lane.cards.iter().filter(|x| x.col == c).enumerate() {
                    let picked = self.pick == (li, c, i);
                    if picked {
                        at = out.len() + cell.len();
                    }
                    for (j, line) in wrap(&card.text, cw.saturating_sub(2), 3).iter().enumerate() {
                        let t = pad(&format!("{}{line}", if j == 0 { "• " } else { "  " }), cw);
                        cell.push(match (picked, c == n - 1) {
                            (true, _) => style::styled(&t, Some(255), Some(238), ""),
                            (_, true) => style::fg(&t, 240),
                            _ => style::fg(&t, 252),
                        });
                    }
                }
            }
            let h = cells.iter().map(Vec::len).max().unwrap_or(0).max(1);
            for r in 0..h {
                let row: Vec<String> = cells.iter().map(|cell| cell.get(r).cloned().unwrap_or_else(|| " ".repeat(cw))).collect();
                out.push(row.join(" "));
            }
            out.push(String::new());
        }
        if self.lanes.is_empty() {
            out.push(style::fg(&format!(" No sections in {}. Press e to start it.", self.path.display()), 245));
        }
        let h = self.body.h as usize;
        self.body.set_text(&out.join("\n"));
        if at < self.body.ix + 3 {
            self.body.ix = at.saturating_sub(3);
        } else if at + 3 > self.body.ix + h {
            self.body.ix = (at + 3).saturating_sub(h);
        }
        self.body.full_refresh();
    }

    /// The cards in one cell of the board.
    fn cell(&self, lane: usize, col: usize) -> Vec<&Card> {
        self.lanes.get(lane).map_or(Vec::new(), |l| l.cards.iter().filter(|x| x.col == col).collect())
    }

    fn picked(&self) -> Option<&Card> {
        let (l, c, i) = self.pick;
        self.cell(l, c).get(i).copied()
    }

    /// A key on the board: the arrows pick, the shifted arrows move the
    /// picked task to the next column (its box in todo.hl) or the next
    /// section.
    fn board_key(&mut self, key: &str) {
        let (l, c, i) = self.pick;
        let last = COLUMNS.len() - 1;
        match key {
            "LEFT" | "h" | "RIGHT" | "l" => {
                let c = if key == "LEFT" || key == "h" { c.saturating_sub(1) } else { (c + 1).min(last) };
                self.pick = (l, c, i.min(self.cell(l, c).len().saturating_sub(1)));
            }
            "UP" | "k" if i > 0 => self.pick = (l, c, i - 1),
            "UP" | "k" if l > 0 => self.pick = (l - 1, c, self.cell(l - 1, c).len().saturating_sub(1)),
            "DOWN" | "j" if i + 1 < self.cell(l, c).len() => self.pick = (l, c, i + 1),
            "DOWN" | "j" if l + 1 < self.lanes.len() => self.pick = (l + 1, c, 0),
            "S-LEFT" | "H" | "S-RIGHT" | "L" => {
                let Some(line) = self.picked().map(|x| x.line) else { return };
                let to = if key == "S-LEFT" || key == "H" { c.checked_sub(1) } else { Some(c + 1).filter(|&t| t <= last) };
                let Some(to) = to else { return };
                rewrite(&self.path, |lines| {
                    if let Some(s) = lines.get_mut(line) {
                        *s = in_column(s, to);
                    }
                });
                self.lanes = lanes_of(&std::fs::read_to_string(&self.path).unwrap_or_default());
                let i = self.cell(l, to).iter().position(|x| x.line == line).unwrap_or(0);
                self.pick = (l, to, i);
            }
            "S-UP" | "K" | "S-DOWN" | "J" => {
                let Some(line) = self.picked().map(|x| x.line) else { return };
                let to = if key == "S-UP" || key == "K" { l.checked_sub(1) } else { Some(l + 1).filter(|&t| t < self.lanes.len()) };
                let Some(to) = to else { return };
                let lane = self.lanes[to].line;
                rewrite(&self.path, |lines| to_lane(lines, line, lane));
                self.lanes = lanes_of(&std::fs::read_to_string(&self.path).unwrap_or_default());
                self.pick = (to, c, self.cell(to, c).len().saturating_sub(1));
            }
            "e" | "ENTER" => {
                let line = self.picked().map(|x| x.line).or(self.lanes.get(l).map(|x| x.line)).unwrap_or(0);
                self.edit_at(line + 1);
                return;
            }
            "RESIZE" => {
                self.repaint();
                return;
            }
            _ => return,
        }
        self.draw_board();
    }

    /// For Claude: which file is on screen, and from where.
    fn claude_context(&self) -> String {
        if self.board {
            let at = self.picked().map_or(String::new(), |x| format!(" The picked task is at line {}.", x.line + 1));
            return format!(
                "The task list {} is on screen as a board: its top-level lines are the lanes, the tasks under them \
                 the cards, in To do (no box), Doing ([O]), Waiting ([-]) or Done ([x]).{at} It is HyperList.",
                self.path.display()
            );
        }
        let line = self.ed.as_ref().map_or(1, |e| e.line());
        format!(
            "The task list {} is open in scribe, the cursor on line {line}. It is HyperList.",
            self.path.display()
        )
    }

    /// A key for the tab. True when scribe was quit, so the tab is done.
    fn key(&mut self, key: &str) -> bool {
        if key == "C-B" {
            if self.board {
                self.board = false;
            } else if self.editor(0).save_if_dirty() {
                // A failed write leaves scribe in view, saying why.
                self.board = true;
                self.lanes = lanes_of(&std::fs::read_to_string(&self.path).unwrap_or_default());
                if self.picked().is_none() {
                    let first = self.lanes.iter().enumerate().find_map(|(l, lane)| lane.cards.first().map(|x| (l, x.col)));
                    self.pick = first.map_or((0, 0, 0), |(l, c)| (l, c, 0));
                }
            }
            self.repaint();
            return false;
        }
        if self.board {
            self.board_key(key);
            return false;
        }
        let ed = self.editor(0);
        ed.after_wait(Some(key.into()));
        if ed.running() {
            return false;
        }
        ed.close();
        self.ed = None;
        true
    }
}

/// Claude: deck in ~/.gluon, the same session each time (its id
/// is kept in ~/.gluon/session), so Claude remembers what came before.
fn start_claude(home: &Path) -> Result<deck::app::App, String> {
    let dir = home.join(".gluon");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let brief = dir.join("CLAUDE.md");
    if !brief.exists() {
        let _ = std::fs::write(&brief, BRIEF);
    }
    let kept = std::fs::read_to_string(dir.join("session")).unwrap_or_default();
    let id = kept.trim();
    // A new session the first time, or when the old one has no transcript
    // (no prompt was ever sent in it).
    let app = if id.is_empty() {
        deck::app::start(None, dir.to_str())?
    } else {
        match deck::app::start(Some(id), None) {
            Err(e) if e.starts_with("No ") => deck::app::start(None, dir.to_str())?,
            other => other?,
        }
    };
    keep(home, app.session_id());
    Ok(app)
}

/// Keep the session id in ~/.gluon/session, and tag a new one #gluon.
fn keep(home: &Path, id: &str) {
    let dir = home.join(".gluon");
    let kept = std::fs::read_to_string(dir.join("session")).unwrap_or_default();
    if kept.trim() != id {
        let _ = std::fs::write(dir.join("session"), id);
        bookmark(home, &dir, id);
    }
}

/// Tag Claude's session #gluon in cc-sessions' bookmarks, where
/// fleet and `cc gluon` find it, and take the tag off the session before.
/// Without cc-sessions there is no file, and nothing to do.
fn bookmark(home: &Path, dir: &Path, id: &str) {
    let file = home.join(".cc-sessions/bookmarks.json");
    let Some(mut b) = std::fs::read_to_string(&file)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    else {
        return;
    };
    let Some(sessions) = b["sessions"].as_object_mut() else { return };
    sessions.retain(|_, e| !e["tags"].as_array().is_some_and(|t| t.len() == 1 && t[0] == "gluon"));
    for e in sessions.values_mut() {
        if let Some(t) = e["tags"].as_array_mut() {
            t.retain(|t| t != "gluon");
        }
    }
    sessions.insert(id.into(), serde_json::json!({ "path": dir, "tags": ["gluon"] }));
    let tmp = file.with_extension("json.tmp");
    if let Ok(text) = serde_json::to_string_pretty(&b) {
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &file);
        }
    }
}

/// Claude, started with gluon. An idle Claude Code process takes about
/// 33 mW, more than the rest of gluon.
enum Claude {
    On(deck::app::App),
    Failed(String),
}

impl Claude {
    fn app(&mut self) -> Option<&mut deck::app::App> {
        match self {
            Claude::On(a) => Some(a),
            _ => None,
        }
    }

    fn start(home: &Path) -> Claude {
        match start_claude(home) {
            Ok(a) => {
                Crust::set_app_identity("gluon");
                Claude::On(a)
            }
            Err(e) => Claude::Failed(e),
        }
    }

    fn running(&self) -> bool {
        match self {
            Claude::On(a) => a.running(),
            _ => true,
        }
    }
}

/// Home's boxes in Tab order; Claude's pane comes after them.
const BOX_MAIL: usize = 0;
const BOX_AGENDA: usize = 1;
const BOX_TASKS: usize = 2;
const CLAUDE: usize = 3;

/// The most unread messages the mail box lists.
const UNREAD: usize = 100;

/// kastrup's keys for one message, run from the mail box at Home: replies,
/// forward, edit, unsubscribe, triage, chat, attachments, the browser,
/// label, file, react, snooze, hide, star and unsee. Home comes
/// back when the key is done.
const BACK: &[&str] = &["r", "e", "g", "f", "E", "O", "z", "C", "v", "x", "X", "l", "s", "k", "h", "m", "M", "*", "-", "u", "U"];
/// Those whose answer shows in kastrup's own pane, so the Mail tab stays.
const STAY: &[&str] = &["I", "c"];

/// A box at Home: a frame with its title in the top edge and rows inside.
/// While it has the keys, one row is picked and the keys show in the
/// bottom edge.
struct Frame {
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    pane: Pane,
    title: String,
    keys: &'static str,
    /// What Enter acts on (none for a rule between days) and the text.
    rows: Vec<(Option<(i64, i64)>, String)>,
    sel: usize,
    top: usize,
    /// The rows changed since they were drawn.
    dirty: bool,
    /// The title and the focus the frame was drawn with.
    drawn: Option<(String, bool)>,
    /// Rows tagged for Claude (t), by their keys, on a dark grey.
    tags: HashSet<(i64, i64)>,
}

impl Frame {
    fn new(keys: &'static str) -> Self {
        Frame {
            x: 1, y: 1, w: 3, h: 3,
            pane: Pane::new(2, 2, 1, 1, 252, 0),
            title: String::new(),
            keys,
            rows: Vec::new(),
            sel: 0,
            top: 0,
            dirty: true,
            drawn: None,
            tags: HashSet::new(),
        }
    }

    fn place(&mut self, x: u16, y: u16, w: u16, h: u16) {
        (self.x, self.y, self.w, self.h) = (x, y, w.max(5), h.max(3));
        // A column of room inside each side.
        self.pane = Pane::new(x + 2, y + 1, self.w - 4, self.h - 2, 252, 0);
        self.pane.wrap = false;
        self.pane.scroll = false;
        self.drawn = None;
    }

    fn inner(&self) -> usize {
        self.w as usize - 4
    }

    /// New rows; the picked one stays picked while it is there.
    fn set_rows(&mut self, rows: Vec<(Option<(i64, i64)>, String)>) {
        let was = self.rows.get(self.sel).and_then(|r| r.0);
        let w = self.inner();
        self.rows = rows.into_iter().map(|(k, t)| (k, crust::truncate_ansi(&t, w))).collect();
        if let Some(i) = was.and_then(|k| self.rows.iter().position(|r| r.0 == Some(k))) {
            self.sel = i;
        }
        let keys: HashSet<(i64, i64)> = self.rows.iter().filter_map(|r| r.0).collect();
        self.tags.retain(|t| keys.contains(t));
        self.step(0);
        self.dirty = true;
    }

    /// The pick `d` rows on, past the rules, and kept in view.
    fn step(&mut self, d: isize) {
        if self.rows.is_empty() {
            (self.sel, self.top) = (0, 0);
            return;
        }
        let last = self.rows.len() as isize - 1;
        let dir = if d < 0 { -1 } else { 1 };
        let mut s = (self.sel as isize + d).clamp(0, last);
        while (0..=last).contains(&s) && self.rows[s as usize].0.is_none() {
            s += dir;
        }
        if !(0..=last).contains(&s) {
            s = (self.sel as isize).min(last);
        }
        self.sel = s as usize;
        let h = self.pane.h as usize;
        self.top = self.top.min(self.sel).min(self.rows.len().saturating_sub(h));
        if self.sel >= self.top + h {
            self.top = self.sel + 1 - h;
        }
        self.dirty = true;
    }

    fn picked(&self) -> Option<(i64, i64)> {
        self.rows.get(self.sel).and_then(|r| r.0)
    }

    /// t: the picked row tagged, or not, and the pick on to the next.
    fn tag(&mut self) {
        if let Some(k) = self.picked() {
            if !self.tags.remove(&k) {
                self.tags.insert(k);
            }
            self.step(1);
        }
    }

    /// The tagged rows, in the order they show, and no tags left.
    fn take_tags(&mut self) -> Vec<(i64, i64)> {
        if self.tags.is_empty() {
            return Vec::new();
        }
        let tags = std::mem::take(&mut self.tags);
        self.dirty = true;
        self.rows.iter().filter_map(|r| r.0).filter(|k| tags.contains(k)).collect()
    }

    /// The frame and the rows, each drawn when it changed, or both when
    /// `force`. True when something was drawn.
    fn draw(&mut self, focused: bool, force: bool) -> bool {
        let framed = self.drawn.as_ref().is_some_and(|(t, f)| *t == self.title && *f == focused);
        if !force && framed && !self.dirty {
            return false;
        }
        if force || !framed {
            self.frame(focused);
            self.drawn = Some((self.title.clone(), focused));
        }
        self.dirty = false;
        let w = self.inner();
        let text: Vec<String> = self.rows.iter().enumerate().skip(self.top).take(self.pane.h as usize)
            .map(|(i, (k, t))| {
                let tagged = k.is_some_and(|k| self.tags.contains(&k));
                let p = crust::strip_ansi(t);
                let pad = " ".repeat(w.saturating_sub(crust::display_width(&p)));
                if focused && i == self.sel && k.is_some() {
                    style::styled(&(p + &pad), Some(255), Some(if tagged { 240 } else { 238 }), "")
                } else if tagged {
                    // The grey again after each reset inside the row.
                    let bg = style::set_bg(236);
                    let again = format!("{}{bg}", style::RESET);
                    format!("{bg}{}{pad}{}", t.replace(style::RESET, &again), style::RESET)
                } else {
                    t.clone()
                }
            })
            .collect();
        show(&mut self.pane, text.join("\n"), force);
        true
    }

    fn frame(&self, focused: bool) {
        let c = if focused { 156 } else { 238 };
        let inner = self.w as usize - 2;
        let edge = |left: &str, text: String, right: &str| {
            let text = crust::truncate_ansi(&text, inner.saturating_sub(2));
            let rest = inner.saturating_sub(1 + crust::display_width(&text));
            style::fg(&format!("{left}─"), c) + &text + &style::fg(&("─".repeat(rest) + right), c)
        };
        let title = if focused { style::bold(&style::fg(&self.title, 255)) } else { style::fg(&self.title, 245) };
        let keys = if focused { style::fg(self.keys, 245) } else { String::new() };
        let mut s = Cursor::at(self.x, self.y) + &edge("╭", title, "╮");
        // The sides with the margin inside them, so nothing another box
        // drew in that column stays.
        for r in 1..self.h - 1 {
            s += &(Cursor::at(self.x, self.y + r) + &style::fg("│", c) + " ");
            s += &(Cursor::at(self.x + self.w - 2, self.y + r) + " " + &style::fg("│", c));
        }
        s += &(Cursor::at(self.x, self.y + self.h - 1) + &edge("╰", keys, "╯"));
        print!("{s}");
        let _ = std::io::stdout().flush();
    }
}

/// What a key in one of Home's boxes asks of the rest of gluon.
/// What Claude is asked on Ctrl+S at Home. The messages, the calendar and
/// the task list follow it.
const ASK: &str = r#"You help one person keep up with their mail, calendar and tasks. Below are their unread messages, their calendar for the coming 30 days and their task list (HyperList: a tab per level, [x] done, [O] doing). Propose at most 8 things they would want done:
- "task": something they must do, or were asked to do, that is not in the task list yet;
- "event": a meeting, appointment or deadline with a date, that is not on the calendar yet;
- "reply": an answer to a message that needs one, written as they would send it: short, in plain words and the language of the message, with no signature.
Leave out newsletters and notices that need nothing. Answer with a JSON array and nothing else:
[{"kind":"task","text":"Send Alice the contract","why":"Alice asked for it on Monday"},
 {"kind":"event","text":"Dentist","start":"YYYY-MM-DD HH:MM","end":"YYYY-MM-DD HH:MM","why":"..."},
 {"kind":"reply","id":123,"text":"Thursday works. See you at ten.","why":"..."}]
An event for a whole day has start as YYYY-MM-DD and no end. "id" is the number of the message. "why" is one short line. Answer [] when nothing needs doing."#;

/// One thing Claude proposes: a task for todo.hl, an event for the
/// calendar, or an answer to a message.
struct Idea {
    kind: String,
    text: String,
    /// An event's start and end, as "YYYY-MM-DD HH:MM" or a date alone.
    start: String,
    end: String,
    /// The message a reply answers.
    id: i64,
    why: String,
}

/// Claude's ideas, in the right column at Home after Ctrl+S. One
/// `claude -p` per press runs in a thread, so nothing waits on it, and
/// nothing runs between presses.
struct Ideas {
    frame: Frame,
    list: Vec<Idea>,
    /// The messages the ideas were made from, by id, as "sender: subject".
    msgs: std::collections::HashMap<i64, String>,
    job: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    open: bool,
    /// What happened last, at the top of the box.
    note: String,
}

impl Ideas {
    fn new() -> Self {
        Ideas {
            frame: Frame::new(" Enter does it · d drops · ^S asks again · Esc closes "),
            list: Vec::new(),
            msgs: Default::default(),
            job: None,
            open: false,
            note: String::new(),
        }
    }

    /// claude -p on `prompt`, with no tools and none of the user's
    /// settings but their writing rules in `rules` (~/.claude/CLAUDE.md),
    /// so the answer is the JSON alone. It wakes the key wait as deck's
    /// reader does: SIGWINCH, read as a RESIZE.
    fn start(&mut self, prompt: String, dir: PathBuf, rules: PathBuf) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let run = || -> Result<String, String> {
                let mut claude = std::process::Command::new("claude");
                claude.args(["-p", "--safe-mode", "--tools", "", "--no-session-persistence"]);
                if rules.is_file() {
                    claude.arg("--append-system-prompt-file").arg(&rules);
                }
                let mut child = claude
                    .current_dir(dir)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .map_err(|e| format!("claude did not start: {e}"))?;
                if let Some(mut i) = child.stdin.take() {
                    let _ = i.write_all(prompt.as_bytes());
                }
                let out = child.wait_with_output().map_err(|e| e.to_string())?;
                if !out.status.success() {
                    let err = String::from_utf8_lossy(&out.stderr);
                    return Err(err.lines().next().unwrap_or("claude failed").into());
                }
                Ok(String::from_utf8_lossy(&out.stdout).into_owned())
            };
            let _ = tx.send(run());
            unsafe {
                libc::kill(libc::getpid(), libc::SIGWINCH);
            }
        });
        self.job = Some(rx);
        self.list.clear();
        self.note = "Claude is reading the unread mail, the calendar and the tasks…".into();
        self.load();
    }

    /// True when Claude's answer came.
    fn poll(&mut self) -> bool {
        let Some(rx) = &self.job else { return false };
        let answer = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(_) => Err("the run stopped".into()),
            Ok(a) => a,
        };
        self.job = None;
        match answer.and_then(|a| parse_ideas(&a, &self.msgs)) {
            Ok(list) => {
                self.note = if list.is_empty() { "Nothing to propose.".into() } else { String::new() };
                self.list = list;
            }
            Err(e) => self.note = format!("No ideas: {e}"),
        }
        self.frame.sel = 0;
        self.frame.top = 0;
        self.load();
        true
    }

    /// The box's rows: a line for each idea, with its text and why under it.
    fn load(&mut self) {
        let w = self.frame.inner();
        let mut rows = Vec::new();
        if !self.note.is_empty() {
            for l in wrap(&self.note, w.saturating_sub(1), 3) {
                rows.push((None, style::fg(&format!(" {l}"), 245)));
            }
            rows.push((None, String::new()));
        }
        for (i, idea) in self.list.iter().enumerate() {
            if i > 0 {
                rows.push((None, String::new()));
            }
            let (label, c, head) = match idea.kind.as_str() {
                "task" => ("Task ", 156, idea.text.clone()),
                "event" => ("Event", 117, format!("{} {}", idea.start, idea.text)),
                _ => ("Reply", 222, format!("to {}", self.msgs.get(&idea.id).map_or("", |m| m.as_str()))),
            };
            rows.push((Some((i as i64, 0)), format!(" {} {head}", style::fg(label, c))));
            if idea.kind == "reply" {
                for l in wrap(&idea.text, w.saturating_sub(8), 6) {
                    rows.push((None, style::fg(&format!("       {l}"), 250)));
                }
            }
            for l in wrap(&idea.why, w.saturating_sub(8), 2) {
                rows.push((None, style::fg(&format!("       {l}"), 242)));
            }
        }
        self.frame.title = match (self.job.is_some(), self.list.len()) {
            (true, _) => " Ideas · thinking ".into(),
            (_, 0) => " Ideas ".into(),
            (_, n) => format!(" Ideas · {n} "),
        };
        self.frame.set_rows(rows);
    }

    /// The Ctrl+S hint on Home's bar.
    fn hint(&self) -> String {
        match (self.job.is_some(), self.list.len()) {
            (true, _) => "^S thinking…".into(),
            (_, 0) => "^S ideas".into(),
            (_, 1) if !self.open => "^S 1 idea".into(),
            (_, n) if !self.open => format!("^S {n} ideas"),
            _ => "^S ideas".into(),
        }
    }

    /// The picked idea, taken off the list.
    fn take(&mut self) -> Option<Idea> {
        let i = self.frame.picked()?.0 as usize;
        (i < self.list.len()).then(|| self.list.remove(i))
    }
}

/// Claude's answer as ideas: the first JSON array in it, with any text
/// around it skipped and what cannot be done left out (an event with no
/// date, a reply to a message it was not shown).
fn parse_ideas(answer: &str, msgs: &std::collections::HashMap<i64, String>) -> Result<Vec<Idea>, String> {
    let all: Vec<serde_json::Value> = answer
        .match_indices('[')
        .find_map(|(a, _)| serde_json::Deserializer::from_str(&answer[a..]).into_iter().next()?.ok())
        .ok_or("Claude's answer had no list")?;
    Ok(all
        .iter()
        .filter_map(|v| {
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
            let idea = Idea {
                kind: s("kind"),
                text: s("text"),
                start: s("start"),
                end: s("end"),
                id: v.get("id").and_then(|x| x.as_i64().or_else(|| x.as_str()?.parse().ok())).unwrap_or(0),
                why: s("why"),
            };
            let ok = !idea.text.is_empty()
                && match idea.kind.as_str() {
                    "task" => true,
                    "event" => ics_time(&idea.start).is_some(),
                    "reply" => msgs.contains_key(&idea.id),
                    _ => false,
                };
            ok.then_some(idea)
        })
        .take(8)
        .collect())
}

/// "YYYY-MM-DD HH:MM" as an ICS local time, "YYYYMMDDTHHMM00", or
/// "YYYY-MM-DD" as a date alone, "YYYYMMDD".
fn ics_time(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r.clone()).is_some_and(|d| d.iter().all(u8::is_ascii_digit));
    let date = digits(0..4) && b.get(4) == Some(&b'-') && digits(5..7) && b.get(7) == Some(&b'-') && digits(8..10);
    if !date {
        return None;
    }
    let d = format!("{}{}{}", &s[0..4], &s[5..7], &s[8..10]);
    match b.len() {
        10 => Some(d),
        16 if matches!(b[10], b' ' | b'T') && digits(11..13) && b[13] == b':' && digits(14..16) => {
            Some(format!("{d}T{}{}00", &s[11..13], &s[14..16]))
        }
        _ => None,
    }
}

/// An event for tock: an .ics in ~/.tock/incoming, and its day in
/// ~/.tock/goto, which makes tock read the file on its next turn and show
/// that day, as kastrup's Z does.
fn add_event(home: &Path, idea: &Idea) -> bool {
    let Some(start) = ics_time(&idea.start) else { return false };
    let tock = home.join(".tock");
    let incoming = tock.join("incoming");
    let _ = std::fs::create_dir_all(&incoming);
    let esc = |t: &str| t.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace('\n', "\\n");
    let kind = if start.len() == 8 { ";VALUE=DATE" } else { "" };
    let mut ics = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//gluon//EN\r\nBEGIN:VEVENT\r\nUID:gluon-{}-{}\r\nDTSTART{kind}:{start}\r\n",
        std::process::id(),
        crate::now(),
    );
    // With no end, tock makes it an hour, or the whole day.
    if let Some(end) = ics_time(&idea.end).filter(|e| e.len() == start.len() && *e > start) {
        ics += &format!("DTEND{kind}:{end}\r\n");
    }
    ics += &format!("SUMMARY:{}\r\nDESCRIPTION:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n", esc(&idea.text), esc(&idea.why));
    let file = incoming.join(format!("gluon_{}_{}.ics", crate::now(), start));
    std::fs::write(file, ics).is_ok() && std::fs::write(tock.join("goto"), &idea.start[..10]).is_ok()
}

/// A task at the end of the Inbox lane in todo.hl, or of the first lane,
/// or in a new Inbox lane when the file has none.
fn add_task(path: &Path, text: &str) {
    let text = text.replace('\n', " ");
    if !path.exists() {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(path, format!("Inbox\n\t{text}\n"));
        return;
    }
    rewrite(path, |lines| {
        let Some(top) = lines.iter().filter(|l| !l.trim().is_empty()).map(|l| depth(l)).min() else {
            lines.extend(["Inbox".to_string(), format!("\t{text}")]);
            return;
        };
        let lanes: Vec<usize> = (0..lines.len()).filter(|&i| !lines[i].trim().is_empty() && depth(&lines[i]) == top).collect();
        let lane = lanes
            .iter()
            .copied()
            .find(|&i| lines[i].trim().trim_end_matches(':').eq_ignore_ascii_case("inbox"))
            .unwrap_or(lanes[0]);
        let at = block_end(lines, lane);
        lines.insert(at, format!("{}{text}", "\t".repeat(top + 1)));
    });
}

/// Seconds since 1970.
fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

enum Act {
    /// Message `id` opened in the Mail tab, then the key, if any, for it.
    Mail(i64, Option<String>),
    /// Messages for kastrup to mark read, and to delete.
    Purge(Vec<i64>, Vec<i64>),
    /// The Calendar on an event, by its id and start.
    Event(i64, i64),
    /// The meeting of an event opened (J), by its id and start.
    Join(i64, i64),
    /// The task list in the editor, at a line.
    Edit(i64),
    /// An answer to message `id`, from Claude's ideas, for kastrup's r.
    Reply(i64, String),
    /// An event was put in tock's incoming folder.
    Import,
    /// Items in the boxes, by the box, the id and the start: the tagged
    /// ones, or else the picked one, for Claude when the keys went from a
    /// box to it.
    Context(Vec<(usize, i64, i64)>),
    /// The picked item, for the clipboard: its reference (y), or all of
    /// it (Ctrl+Y).
    Copy(usize, i64, i64, bool),
    /// kastrup's + in the Mail tab: the drafts, postponed and scheduled
    /// messages, or a new one.
    Compose,
}

/// Claude's name at the left of its bar, bright while it has the keys.
fn claude_label(focused: bool) -> String {
    if focused { style::bold(&style::fg(" ▸ Claude", 156)) } else { style::fg(" Claude", 240) }
}

/// The Home tab: boxes for the unread mail in kastrup's views (left), the
/// agenda and the task list (right), and Claude below them, or on the
/// whole screen after Ctrl+Z. Tab moves the keys from box to box and on
/// to Claude.
struct Home {
    bar: Pane,
    boxes: [Frame; 3],
    /// The message picked in the mail box, in the right column while the
    /// mail box has the keys; which one it shows; whether it was shown.
    preview: Frame,
    shown: Option<i64>,
    previewing: bool,
    /// Messages marked read or for deletion (d) in the mail box; they
    /// stay there until <. Then they are purged, and kept out of the box
    /// until kastrup's writer has caught up. A message is marked read the
    /// first time it is browsed (`seen`), and R toggles the mark.
    marked_read: HashSet<i64>,
    seen: HashSet<i64>,
    marked_del: HashSet<i64>,
    purged: HashSet<i64>,
    /// Lines of todo.hl marked for deletion (d in the tasks box), by line
    /// number (from 1) with their text, so < deletes only a line that is
    /// still the same.
    task_del: std::collections::HashMap<usize, String>,
    /// The box with the keys, or CLAUDE.
    focus: usize,
    /// The keys of the views to show, as "1 2 3", in ~/.gluon/views.
    views: PathBuf,
    todo: PathBuf,
    /// What the boxes were made from, so a wake with nothing new draws
    /// nothing: kastrup's database files, the minute, todo.hl's time.
    sig: Option<(u64, u128, u128)>,
    minute: i64,
    mtime: Option<std::time::SystemTime>,
    full: bool,
    /// Claude's ideas (Ctrl+S), in the right column while open; the
    /// column's owner as last drawn (agenda and tasks, message, ideas);
    /// the Ctrl+S hint on the bar as last drawn.
    ideas: Ideas,
    right: u8,
    hint: String,
    dir: PathBuf,
    /// What y copied, on the bar until the next key.
    note: String,
    /// The keys (?), in the right column while open.
    help: Frame,
    helping: bool,
    /// Events in the agenda with a meeting link, which J opens.
    joinable: HashSet<(i64, i64)>,
    /// The mail box lists the oldest first (i turns it, as in kastrup).
    oldest: bool,
}

impl Home {
    fn new(home: &Path) -> Self {
        let mut h = Home {
            bar: Pane::new(1, 1, 1, 1, 255, 236),
            boxes: [
                Frame::new(" Enter opens · R/A read · d delete · < clears · t tags · ? keys "),
                Frame::new(" Enter opens · J joins ▶ · t tags · y/^Y copies · ? keys "),
                Frame::new(" Enter edits · x done · d/< delete · t tags · y/^Y copies · ? keys "),
            ],
            preview: Frame::new(""),
            shown: None,
            previewing: false,
            marked_read: HashSet::new(),
            seen: HashSet::new(),
            marked_del: HashSet::new(),
            purged: HashSet::new(),
            task_del: Default::default(),
            focus: CLAUDE,
            views: home.join(".gluon").join("views"),
            todo: home.join(".tasks").join("todo.hl"),
            sig: None,
            minute: 0,
            mtime: None,
            full: false,
            ideas: Ideas::new(),
            right: 0,
            hint: String::new(),
            dir: home.to_path_buf(),
            note: String::new(),
            help: Frame::new(" Esc or ? closes "),
            helping: false,
            joinable: HashSet::new(),
            oldest: false,
        };
        h.layout();
        h
    }

    /// Rows above Claude: the bar and three fifths of the rest for the
    /// boxes.
    fn split() -> (u16, u16) {
        let (_, rows) = Crust::terminal_size();
        let top = (rows.saturating_sub(1) * 3 / 5).max(6);
        (top + 1, rows.saturating_sub(top + 1))
    }

    fn layout(&mut self) {
        let (cols, _) = Crust::terminal_size();
        let h = Self::split().0 - 1;
        let lw = cols / 2;
        let rw = cols.saturating_sub(lw);
        let ah = h / 2;
        self.bar = Pane::new(1, 1, cols, 1, 255, 236);
        self.bar.wrap = false;
        self.boxes[BOX_MAIL].place(1, 2, lw, h);
        self.boxes[BOX_AGENDA].place(lw + 1, 2, rw, ah);
        self.boxes[BOX_TASKS].place(lw + 1, 2 + ah, rw, h - ah);
        self.preview.place(lw + 1, 2, rw, h);
        self.preview.pane.wrap = true;
        self.preview.title = " Message ".into();
        self.shown = None;
        self.ideas.frame.place(lw + 1, 2, rw, h);
        self.ideas.load();
        self.help.place(lw + 1, 2, rw, h);
        self.help.title = " Keys at Home ".into();
        self.help.set_rows(HELP.iter().map(|l| (None, help_row(l))).collect());
    }

    /// The item picked in box `b`, its reference on the first line: the
    /// message, the event, or the task with the lines under it.
    fn item(&self, b: usize, id: i64, start: i64, mail: &kastrup::app::App, cal: &tock::app::App) -> String {
        match b {
            BOX_MAIL => {
                let text: String = crust::strip_ansi(&mail.message_text(id, 100)).chars().take(6000).collect();
                format!("kastrup:{id}\n{}\n", text.trim())
            }
            BOX_AGENDA => cal.event_text(id, start),
            _ => task_item(&self.task_lines(), id),
        }
    }

    /// The lines of todo.hl.
    fn task_lines(&self) -> Vec<String> {
        std::fs::read_to_string(&self.todo).unwrap_or_default().lines().map(String::from).collect()
    }

    /// Claude has the keys: its pane is focused and no ideas are open.
    fn claude_keys(&self) -> bool {
        self.focus == CLAUDE && !self.ideas.open
    }

    /// Ctrl+S: the ideas open, and Claude is asked for new ones when there
    /// are none, or when they were open already.
    fn ask(&mut self, mail: &kastrup::app::App, cal: &tock::app::App, claude: &mut Claude) {
        let again = self.ideas.open || self.ideas.list.is_empty();
        if self.ideas.job.is_none() && again {
            let file = std::fs::read_to_string(&self.views).unwrap_or_default();
            let unread: Vec<_> = mail
                .unread_in_views(&view_keys(&file), 20)
                .into_iter()
                .filter(|u| !self.purged.contains(&u.id) && !self.marked_del.contains(&u.id))
                .collect();
            let mut prompt = format!("{ASK}\n\nNow: {}\n\n# Unread messages\n", clock(now(), "%A %F %H:%M"));
            for u in &unread {
                let text: String = crust::strip_ansi(&mail.message_text(u.id, 100)).chars().take(1500).collect();
                prompt += &format!("\n## Message {}\n{}\n", u.id, text.trim());
            }
            prompt += "\n# Calendar, the coming 30 days\n";
            for a in cal.agenda(UNREAD, 100) {
                prompt += &format!("{} {}\n", clock(a.start, "%a %F"), crust::strip_ansi(&a.text).trim());
            }
            let tasks: String = std::fs::read_to_string(&self.todo).unwrap_or_default().chars().take(8000).collect();
            prompt += &format!("\n# Task list\n{tasks}\n");
            self.ideas.msgs = unread.iter().map(|u| (u.id, format!("{}: {}", plain(&u.from), plain(&u.subject)))).collect();
            self.ideas.start(prompt, self.dir.join(".gluon"), self.dir.join(".claude").join("CLAUDE.md"));
        }
        self.ideas.open = true;
        if self.full {
            self.full = false;
            self.repaint(mail, cal, claude);
        } else {
            self.focus_on(self.focus, claude);
        }
    }

    /// A key while the ideas are open. Enter does the picked one: a task
    /// goes in todo.hl and an event in tock, and both stay here; a reply
    /// opens in kastrup.
    fn idea_key(&mut self, k: &str, claude: &mut Claude) -> Option<Act> {
        let f = &mut self.ideas.frame;
        match k {
            "DOWN" | "j" => f.step(1),
            "UP" | "k" => f.step(-1),
            "PgDOWN" => f.step(f.pane.h as isize),
            "PgUP" => f.step(-(f.pane.h as isize)),
            "ESC" | "q" => {
                self.ideas.open = false;
                self.focus_on(self.focus, claude);
            }
            "d" => {
                self.ideas.take();
                self.ideas.note.clear();
                self.ideas.load();
            }
            "ENTER" => {
                let idea = self.ideas.take()?;
                let act = match idea.kind.as_str() {
                    "task" => {
                        add_task(&self.todo, &idea.text);
                        self.mtime = None;
                        self.ideas.note = format!("Added to the tasks: {}", idea.text);
                        None
                    }
                    "event" => {
                        let ok = add_event(&self.dir, &idea);
                        self.ideas.note = if ok {
                            format!("Added to the calendar: {} {}", idea.start, idea.text)
                        } else {
                            format!("Could not write the event to {}", self.dir.join(".tock/incoming").display())
                        };
                        ok.then_some(Act::Import)
                    }
                    _ => {
                        self.ideas.open = false;
                        self.ideas.note.clear();
                        self.focus_on(self.focus, claude);
                        Some(Act::Reply(idea.id, idea.text))
                    }
                };
                self.ideas.load();
                return act;
            }
            _ => {}
        }
        None
    }

    /// The whole tab, after another tab had the screen or it was resized.
    fn repaint(&mut self, mail: &kastrup::app::App, cal: &tock::app::App, claude: &mut Claude) {
        if let (true, Claude::On(c)) = (self.full, &mut *claude) {
            c.set_tab_line(Some(tab_line(HOME) + &style::fg("^Z half ", 240)));
            c.set_area(None);
            c.set_focus(true);
            return;
        }
        Crust::clear_screen();
        self.layout();
        self.paint(mail, cal, true);
        match claude {
            Claude::On(c) => {
                c.set_tab_line(Some(claude_label(self.claude_keys())));
                c.set_area(Some(Self::split()));
                c.set_focus(self.claude_keys());
            }
            other => self.claude_off(other),
        }
    }

    /// Claude's rows when it could not start.
    fn claude_off(&self, claude: &Claude) {
        let (top, rows) = Self::split();
        let (cols, _) = Crust::terminal_size();
        let why = match claude {
            Claude::Failed(e) => format!("Claude could not start: {e}"),
            _ => String::new(),
        };
        Pane::new(1, top + 1, cols, 1, 255, 236).say(&claude_label(self.claude_keys()));
        Pane::new(2, top + 3, cols.saturating_sub(2), rows.saturating_sub(2), 245, 0).say(&why);
        Cursor::hide();
    }

    /// The keys to box `f`, or to Claude.
    fn focus_on(&mut self, f: usize, claude: &mut Claude) {
        self.focus = f;
        let keys = self.claude_keys();
        match claude {
            Claude::On(c) => {
                c.set_tab_line(Some(claude_label(keys)));
                c.set_focus(keys);
            }
            other => self.claude_off(other),
        }
    }

    /// A key while a box has the keys, or Tab while Claude has them. The
    /// box moves its pick itself; what goes beyond it comes back.
    fn key(&mut self, k: &str, claude: &mut Claude) -> Option<Act> {
        if self.ideas.open {
            return self.idea_key(k, claude);
        }
        self.note.clear();
        // Any key closes the keys; Esc and ? do nothing else.
        if std::mem::take(&mut self.helping) && matches!(k, "ESC" | "?" | "q") {
            return None;
        }
        match k {
            "+" => return Some(Act::Compose),
            "TAB" | "S-TAB" | "ESC" => {
                let from = self.focus;
                let to = match k {
                    "TAB" => (from + 1) % 4,
                    "S-TAB" => (from + 3) % 4,
                    _ => CLAUDE,
                };
                self.focus_on(to, claude);
                // Claude gets the rows tagged in all three boxes, or else
                // the one picked in the box the keys came from.
                if to == CLAUDE && from != CLAUDE {
                    let mut items = Vec::new();
                    for (b, f) in self.boxes.iter_mut().enumerate() {
                        items.extend(f.take_tags().into_iter().map(|(id, s)| (b, id, s)));
                    }
                    if items.is_empty() {
                        let (id, start) = self.boxes[from].picked()?;
                        items.push((from, id, start));
                    }
                    return Some(Act::Context(items));
                }
            }
            _ => {
                let b = self.boxes.get_mut(self.focus)?;
                let page = b.pane.h as isize;
                let all = b.rows.len() as isize;
                // In the mail box, kastrup's letters act on the picked
                // message, but t and ? are Home's.
                let vi = self.focus != BOX_MAIL;
                match k {
                    "DOWN" => b.step(1),
                    "UP" => b.step(-1),
                    "j" if vi => b.step(1),
                    "k" if vi => b.step(-1),
                    "t" => b.tag(),
                    "?" => self.helping = true,
                    "PgDOWN" => b.step(page),
                    "PgUP" => b.step(-page),
                    "HOME" => b.step(-all),
                    "END" => b.step(all),
                    "g" if vi => b.step(-all),
                    "G" if vi => b.step(all),
                    "J" if self.focus == BOX_AGENDA => {
                        let key = b.picked()?;
                        if self.joinable.contains(&key) {
                            return Some(Act::Join(key.0, key.1));
                        }
                        self.note = " No meeting link in this event".into();
                    }
                    // Copied here, never in the Mail tab: going there and
                    // back redraws the whole screen for a line of text.
                    "y" | "C-Y" => {
                        let (id, start) = b.picked()?;
                        return Some(Act::Copy(self.focus, id, start, k == "C-Y"));
                    }
                    // At the end of the message, Space goes on to the next.
                    " " if !vi && self.previewing => {
                        let was = self.preview.pane.ix;
                        self.preview.pane.pagedown();
                        if self.preview.pane.ix == was {
                            b.step(1);
                        }
                    }
                    "S-RIGHT" if !vi && self.previewing => self.preview.pane.pagedown(),
                    // d marks a line for deletion, with the lines under it,
                    // and < deletes them.
                    "d" if self.focus == BOX_TASKS => {
                        let (line, _) = b.picked()?;
                        let lines: Vec<String> = std::fs::read_to_string(&self.todo).unwrap_or_default().lines().map(String::from).collect();
                        let i = line as usize - 1;
                        if i < lines.len() {
                            let on = !self.task_del.contains_key(&(i + 1));
                            for (j, l) in lines.iter().enumerate().take(block_end(&lines, i)).skip(i) {
                                if on {
                                    self.task_del.insert(j + 1, l.clone());
                                } else {
                                    self.task_del.remove(&(j + 1));
                                }
                            }
                        }
                        b.step(1);
                        self.mtime = None;
                    }
                    "<" if self.focus == BOX_TASKS => {
                        let marks = std::mem::take(&mut self.task_del);
                        rewrite(&self.todo, |lines| {
                            let mut n = 0;
                            lines.retain(|l| {
                                n += 1;
                                marks.get(&n) != Some(l)
                            });
                        });
                        self.mtime = None;
                    }
                    // x ticks a task done in todo.hl, or back to do.
                    "x" if self.focus == BOX_TASKS => {
                        let (line, _) = b.picked()?;
                        rewrite(&self.todo, |lines| {
                            let top = lines.iter().filter(|l| !l.trim().is_empty()).map(|l| depth(l)).min().unwrap_or(0);
                            if let Some(s) = lines.get_mut(line as usize - 1).filter(|s| depth(s) > top) {
                                let done = column_of(&s[depth(s)..]).0 == 3;
                                *s = in_column(s, if done { 0 } else { 3 });
                            }
                        });
                        self.mtime = None;
                    }
                    "S-LEFT" if !vi && self.previewing => self.preview.pane.pageup(),
                    "S-DOWN" if !vi && self.previewing => self.preview.pane.linedown(),
                    "S-UP" if !vi && self.previewing => self.preview.pane.lineup(),
                    // As in kastrup: d moves on to the next message.
                    "R" | "d" if !vi => {
                        let (id, _) = b.picked()?;
                        let set = if k == "R" { &mut self.marked_read } else { &mut self.marked_del };
                        if !set.remove(&id) {
                            set.insert(id);
                        }
                        if k == "d" {
                            b.step(1);
                        }
                        self.sig = None;
                    }
                    "i" if !vi => {
                        self.oldest = !self.oldest;
                        self.sig = None;
                    }
                    "A" if !vi => {
                        self.marked_read.extend(b.rows.iter().filter_map(|r| r.0.map(|k| k.0)));
                        self.sig = None;
                    }
                    "<" if !vi => {
                        let del: Vec<i64> = self.marked_del.drain().collect();
                        let read: Vec<i64> = self.marked_read.drain().filter(|i| !del.contains(i)).collect();
                        self.purged.extend(del.iter().chain(&read));
                        self.sig = None;
                        return Some(Act::Purge(read, del));
                    }
                    _ => {
                        let (id, start) = b.picked()?;
                        return match (self.focus, k) {
                            (BOX_MAIL, "ENTER") => Some(Act::Mail(id, None)),
                            (BOX_MAIL, _) if BACK.contains(&k) || STAY.contains(&k) => {
                                Some(Act::Mail(id, Some(k.into())))
                            }
                            (BOX_AGENDA, "ENTER") => Some(Act::Event(id, start)),
                            (BOX_TASKS, "ENTER" | "e") => Some(Act::Edit(id)),
                            _ => None,
                        };
                    }
                }
            }
        }
        None
    }

    /// The bar and the boxes, each drawn only when it changed, unless
    /// `force`. True when something was drawn.
    fn paint(&mut self, mail: &kastrup::app::App, cal: &tock::app::App, force: bool) -> bool {
        if self.full {
            return false;
        }
        let mut drew = false;
        let sig = mail.db_signature();
        if force || self.sig != Some(sig) {
            self.sig = Some(sig);
            self.load_mail(mail);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let hint = self.ideas.hint();
        let mark = format!("{hint}\n{}", self.note);
        if force || now / 60 != self.minute || mark != self.hint {
            if now / 60 != self.minute {
                self.load_agenda(cal);
            }
            self.minute = now / 60;
            let cols = self.bar.w as usize;
            let right = style::fg(
                &format!(
                    "{} · Tab next box · {hint} · ^Z Claude full · gluon {} ",
                    clock(now, "%a %e %b, week %V"),
                    env!("CARGO_PKG_VERSION")
                ),
                245,
            );
            self.hint = mark;
            let tabs = tab_line(HOME);
            let fit = cols.saturating_sub(crust::display_width(&tabs) + crust::display_width(&right) + 1);
            let left = tabs + &style::fg(&crust::truncate_ansi(&self.note, fit), 156);
            let room = cols.saturating_sub(crust::display_width(&left) + crust::display_width(&right));
            drew |= show(&mut self.bar, format!("{left}{}{right}", " ".repeat(room)), force);
        }
        let mtime = std::fs::metadata(&self.todo).and_then(|m| m.modified()).ok();
        if force || mtime != self.mtime {
            self.mtime = mtime;
            self.load_tasks();
        }
        // While the mail box has the keys, the picked message takes the
        // place of the agenda and the tasks; with no message, they stay.
        // Open ideas go over both.
        let (focus, open) = (self.focus, self.ideas.open);
        let keys = |i: usize| focus == i && !open;
        let preview = keys(BOX_MAIL) && self.boxes[BOX_MAIL].picked().is_some();
        if let (true, Some((id, _))) = (preview, self.boxes[BOX_MAIL].picked()) {
            if self.seen.insert(id) {
                self.marked_read.insert(id);
                self.load_mail(mail);
            }
        }
        let right = if self.ideas.open { 2 } else if self.helping { 3 } else { preview as u8 };
        let swap = right != self.right;
        self.right = right;
        self.previewing = preview;
        let keys = [keys(BOX_MAIL), keys(BOX_AGENDA), keys(BOX_TASKS)];
        drew |= self.boxes[BOX_MAIL].draw(keys[BOX_MAIL], force);
        if self.ideas.open {
            drew |= self.ideas.frame.draw(true, force || swap);
        } else if self.helping {
            drew |= self.help.draw(true, force || swap);
        } else if preview {
            drew |= self.draw_preview(mail, force || swap);
        } else {
            for i in [BOX_AGENDA, BOX_TASKS] {
                drew |= self.boxes[i].draw(keys[i], force || swap);
            }
        }
        drew
    }

    /// The picked message, read again when the pick moved (kastrup keeps
    /// it unread), and drawn whole when `force`.
    fn draw_preview(&mut self, mail: &kastrup::app::App, force: bool) -> bool {
        let id = self.boxes[BOX_MAIL].picked().map(|k| k.0);
        if !force && id == self.shown {
            return false;
        }
        let p = &mut self.preview;
        if force {
            p.frame(false);
        }
        if id != self.shown {
            let text = id.map_or(String::new(), |i| mail.message_text(i, p.pane.w as usize));
            p.pane.set_text(&text);
            p.pane.ix = 0;
            self.shown = id;
        }
        p.pane.full_refresh();
        true
    }

    /// The unread messages in the views, newest first (or oldest), each with its view's
    /// key in the view's colour.
    fn load_mail(&mut self, mail: &kastrup::app::App) {
        let file = std::fs::read_to_string(&self.views).unwrap_or_default();
        let keys = view_keys(&file);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let today = clock(now, "%F");
        let mut list = mail.unread_in_views(&keys, UNREAD);
        self.purged.retain(|id| list.iter().any(|u| u.id == *id));
        list.retain(|u| !self.purged.contains(&u.id));
        if self.oldest {
            list.reverse();
        }
        let more = if list.len() == UNREAD { "+" } else { "" };
        let mut rows = Vec::new();
        if list.is_empty() {
            rows.push((None, style::fg(&format!(" Nothing unread in views {}", keys.join(" ")), 240)));
        }
        for u in &list {
            let when = clock(u.time, if clock(u.time, "%F") == today { "%H:%M" } else { "%e %b" });
            // kastrup's colours: dark red for deletion, grey for read, and
            // else the colour of the message's kind.
            let (mark, c) = if self.marked_del.contains(&u.id) {
                (style::fg("D", 88), 88)
            } else if self.marked_read.contains(&u.id) {
                (style::fg("R", 245), 240)
            } else {
                (" ".into(), u.kind)
            };
            let from = crust::truncate_ansi(&plain(&u.from), 18);
            let pad = " ".repeat(18usize.saturating_sub(crust::display_width(&from)));
            let line = format!(
                "{}{mark} {when:>6} {} {}  {}",
                style::styled(&format!(" {} ", u.view), Some(255), Some(u.colour), ""),
                style::fg(&u.icon, u.kind),
                style::fg(&(from + &pad), c),
                style::fg(&plain(&u.subject), c),
            );
            rows.push((Some((u.id, 0)), line));
        }
        let r = list.iter().filter(|u| self.marked_read.contains(&u.id) && !self.marked_del.contains(&u.id)).count();
        let d = list.iter().filter(|u| self.marked_del.contains(&u.id)).count();
        let marks: String = [("R", r), ("D", d)].iter().filter(|m| m.1 > 0).map(|(k, n)| format!(" · {k} {n}")).collect();
        let b = &mut self.boxes[BOX_MAIL];
        let order = if self.oldest { " · oldest first" } else { "" };
        b.title = format!(" Unread · {}{more}{marks}{order} ", list.len());
        b.set_rows(rows);
    }

    /// The coming 30 days, with a rule between one day and the next. An
    /// event J can join has a green ▶ in front.
    fn load_agenda(&mut self, cal: &tock::app::App) {
        let b = &mut self.boxes[BOX_AGENDA];
        let w = b.inner();
        let mut rows = Vec::new();
        self.joinable.clear();
        for a in cal.agenda(UNREAD, w) {
            if a.new_day && !rows.is_empty() {
                rows.push((None, style::fg(&"─".repeat(w), 237)));
            }
            let text = match (a.join, a.text.strip_prefix(' ')) {
                (true, Some(rest)) => style::fg("▶", 78) + rest,
                _ => a.text,
            };
            if a.join {
                self.joinable.insert((a.id, a.start));
            }
            rows.push((Some((a.id, a.start)), text));
        }
        if rows.is_empty() {
            rows.push((None, style::fg("  Nothing in the next 30 days", 240)));
        }
        b.title = " Agenda ".into();
        b.set_rows(rows);
    }

    /// The task list, coloured as HyperList, one row per line of the file.
    /// A line marked for deletion shows a D, in kastrup's dark red.
    fn load_tasks(&mut self) {
        let text = std::fs::read_to_string(&self.todo).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        self.task_del.retain(|n, l| lines.get(n - 1) == Some(&l.as_str()));
        let coloured = highlight::highlight_hyperlist(&text, usize::MAX).replace('\t', "   ");
        let mut rows: Vec<_> = coloured.lines().enumerate().map(|(i, l)| {
            let row = if self.task_del.contains_key(&(i + 1)) {
                style::fg(&format!("D{}", crust::strip_ansi(l)), 88)
            } else {
                format!(" {l}")
            };
            // Keyed by the line's text too, so a tag never moves to
            // another task when lines come or go above it.
            (Some((i as i64 + 1, line_key(lines.get(i).unwrap_or(&"")))), row)
        }).collect();
        if rows.is_empty() {
            rows.push((Some((1, 0)), style::fg(" No tasks yet; Enter starts the list", 240)));
        }
        let b = &mut self.boxes[BOX_TASKS];
        b.title = match self.task_del.len() {
            0 => " Tasks ".into(),
            n => format!(" Tasks · D {n} "),
        };
        b.set_rows(rows);
    }
}

/// The keys at Home, as ? shows them: a heading, then a key and what it
/// does, split at the first two spaces.
const HELP: &[&str] = &[
    "Every box",
    "↑ ↓  pick a row; PgUp, PgDn, Home and End jump",
    "Enter  open it in its tab; Esc comes back",
    "t  tag it; Esc then hands Claude every tagged row",
    "y  copy as kastrup:<id>, tock:<id> or todo:<n>",
    "^Y  copy all of the message or event, or the task and its lines",
    "Tab  the next box; Shift+Tab back; Esc Claude",
    "+  kastrup's drafts, postponed and scheduled mail",
    "Mail",
    "Space  scroll the message, at its end the next one",
    "R A d  mark read or not, all read, for deletion",
    "<  mark the R ones read and delete the D ones",
    "i  oldest first or newest; r f s h * as in kastrup",
    "Agenda and Tasks",
    "J  join the meeting of an event marked ▶",
    "x d <  tick a task done; mark it for deletion; delete",
    "e  open the task list in scribe at that line",
    "Anywhere at Home",
    "^S  ideas from Claude     ^Z  Claude on the whole screen",
    "^A  back to the last tab  ^O  the next tab",
    "^L  draw it all again     Q   quit",
];

/// One line of HELP: a heading in bold, a key in green and the rest grey.
fn help_row(l: &str) -> String {
    match l.split_once("  ") {
        _ if l.is_empty() => String::new(),
        None => style::bold(&style::fg(l, 255)),
        Some((k, rest)) => format!(" {} {}", style::fg(&format!("{k:<10}"), 156), style::fg(rest, 250)),
    }
}

/// Task `n` of todo.hl (from 1) and the lines under it, as in the file.
fn task_block(lines: &[String], n: i64) -> &[String] {
    let i = (n - 1).max(0) as usize;
    match lines.get(i) {
        Some(l) if !l.trim().is_empty() => &lines[i..block_end(lines, i)],
        _ => &[],
    }
}

/// Task `n` for Claude: "todo:<n> text", then the lines under it.
fn task_item(lines: &[String], n: i64) -> String {
    let block = task_block(lines, n);
    let Some(first) = block.first() else { return String::new() };
    let mut out = format!("todo:{n} {}\n", first.trim());
    for l in &block[1..] {
        out += &format!("{l}\n");
    }
    out
}

/// Task `n` and the lines under it, moved out to the left (Ctrl+Y).
fn task_copy(lines: &[String], n: i64) -> String {
    let block = task_block(lines, n);
    let d = block.first().map_or(0, |l| depth(l));
    block.iter().map(|l| &l[depth(l).min(d)..]).collect::<Vec<_>>().join("\n")
}

/// A line of todo.hl as a number, so a tag stays with its text.
fn line_key(l: &str) -> i64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    l.hash(&mut h);
    h.finish() as i64
}

/// The keys of the views in ~/.gluon/views, as "1 2 4"; 1, 2 and 3 when
/// it names none.
fn view_keys(file: &str) -> Vec<&str> {
    let keys: Vec<&str> = file.split(|c: char| c.is_whitespace() || c == ',').filter(|k| !k.is_empty()).collect();
    if keys.is_empty() { vec!["1", "2", "3"] } else { keys }
}

/// A pane's text, drawn when it changed or when `force`.
fn show(p: &mut Pane, text: String, force: bool) -> bool {
    if !force && p.text() == text {
        return false;
    }
    p.set_text(&text);
    if force { p.full_refresh() } else { p.refresh() }
    true
}

/// One line of a sender or subject.
fn plain(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// The local time `t` in strftime's format.
fn clock(t: i64, fmt: &str) -> String {
    let Ok(fmt) = std::ffi::CString::new(fmt) else { return String::new() };
    let mut buf = [0u8; 64];
    let n = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&(t as libc::time_t), &mut tm);
        libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), &tm)
    };
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    if arg == "-h" || arg == "--help" {
        println!("gluon — mail, calendar, tasks and Claude in one terminal app (Fe2O3 suite)");
        println!();
        println!("Usage: gluon");
        println!();
        println!("  Home      boxes for the unread mail in kastrup's views 1-3 (or those in");
        println!("            ~/.gluon/views), the agenda and the tasks, and Claude below them.");
        println!("            Tab moves from box to box; Enter opens the picked item in its tab");
        println!("            (Esc comes back), and kastrup's keys act on a picked message;");
        println!("            one you browse is marked read (R undoes it, A marks all), d");
        println!("            marks it for deletion, and < does both. Ctrl+S asks Claude for");
        println!("            ideas: tasks to add, events and answers; Enter does the picked one.");
        println!("            t tags rows in any box, and Esc hands every tagged row to Claude;");
        println!("            ? shows all the keys at Home");
        println!("  Mail      kastrup, with its own keys and ~/.kastrup/");
        println!("  Calendar  tock, with its own keys and ~/.tock/");
        println!("  Tasks     ~/.tasks/todo.hl in scribe, or as a board (Ctrl+B switches)");
        println!();
        println!("Claude is deck: one Claude Code session that stays, in ~/.gluon/. Ctrl+Z");
        println!("gives it the whole screen and back. Ctrl+O moves to the next tab. Ctrl+A");
        println!("jumps Home with what is on screen, and back. Q ends gluon, except");
        println!("while Claude has the keys; there C-D ends it.");
        return;
    }
    if arg == "-v" || arg == "--version" {
        println!("gluon {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let dir = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();

    // tock first: its store opens to one program at a time, and it ends
    // the program when another tock has it. Nothing of kastrup has
    // started by then.
    // Neither paints: Home is the first screen drawn.
    let Some(mut cal) = tock::app::start(false) else { return };
    let Some(mut mail) = kastrup::app::start(false) else {
        tock::app::finish(cal);
        return;
    };
    let mut claude = Claude::start(&dir);
    Crust::set_app_identity("gluon");
    mail.set_tab_line(Some(tab_line(MAIL)));
    cal.set_tab_line(Some(tab_line(CALENDAR)));
    let unseen = Unseen::new();
    let mut tasks = Tasks::new(&dir);
    let mut home = Home::new(&dir);
    let mut tab = HOME;
    // Where Ctrl+A at Home goes back to.
    let mut back = MAIL;
    // The tab in view was opened from a box at Home, so Esc goes back.
    let mut from_home = false;
    let mut size = Crust::terminal_size();
    home.repaint(&mail, &cal, &mut claude);

    while mail.running() && cal.running() && claude.running() {
        // Claude's 1 s clock while it works is for its own bar alone.
        let mut wait = match tab {
            MAIL => mail.before_wait().min(unseen.run(|| cal.before_wait())),
            CALENDAR => cal.before_wait().min(unseen.run(|| mail.before_wait())),
            _ => unseen.run(|| mail.before_wait().min(cal.before_wait())),
        };
        if let (HOME, Claude::On(c)) = (tab, &claude) {
            wait = wait.min(c.before_wait());
        }
        // scribe wakes only to write unsaved text to its recovery file.
        if let Some(w) = tasks.ed.as_ref().and_then(|e| e.before_wait()) {
            wait = wait.min(w);
        }
        let key = Input::getchr(Some(wait));
        let mut act = None;
        // While Claude or scribe has the keys, Q and Esc are theirs.
        let to_claude = tab == HOME && (home.full || home.claude_keys()) && matches!(claude, Claude::On(_));
        let typing = to_claude || (tab == TASKS && !tasks.board);
        let mut next = match key.as_deref() {
            Some("C-O") => Some((tab + 1) % TABS.len()),
            // Q ends gluon. kastrup waits for a send in flight and says so
            // in the Mail tab.
            Some("Q") if !typing => {
                if tab == MAIL { mail.quit() } else { unseen.run(|| mail.quit()) }
                if !mail.running() || tab == MAIL {
                    continue;
                }
                Some(MAIL)
            }
            Some("C-A") if tab == HOME => Some(back),
            Some("C-A") => {
                let seen = match tab {
                    MAIL => mail.claude_context().map(|t| ("kastrup", t)),
                    CALENDAR => Some(("tock", cal.claude_context())),
                    _ => Some(("tasks", tasks.claude_context())),
                };
                if let (Some(c), Some((what, text))) = (claude.app(), seen) {
                    unseen.run(|| c.attach(what, &text));
                }
                Some(HOME)
            }
            // Esc in the tab a box at Home opened goes back Home; kastrup
            // takes it first, to drop the one-message list.
            Some("ESC") if from_home && tab != HOME && !typing => {
                if tab == MAIL {
                    unseen.run(|| mail.leave_message());
                }
                Some(HOME)
            }
            _ => None,
        };
        if next.is_none() {
            match key.as_deref() {
                // deck's reader sends SIGWINCH when Claude's output arrives,
                // so a RESIZE with the size unchanged is for Claude.
                Some("RESIZE") => {
                    let now = Crust::terminal_size();
                    let resized = now != size;
                    size = now;
                    match tab {
                        HOME => {
                            if resized {
                                home.repaint(&mail, &cal, &mut claude);
                            }
                            if let Some(c) = claude.app() {
                                c.after_wait(key);
                            }
                        }
                        _ => {
                            if resized {
                                match tab {
                                    MAIL => mail.after_wait(key),
                                    CALENDAR => cal.after_wait(key),
                                    _ => {
                                        tasks.key("RESIZE");
                                    }
                                }
                            }
                            if let Some(c) = claude.app() {
                                unseen.run(|| c.after_wait(None));
                            }
                        }
                    }
                }
                // Only deck asks for focus reports.
                Some("FOCUS_IN" | "FOCUS_OUT") => {
                    if let Some(c) = claude.app() {
                        if tab == HOME { c.after_wait(key) } else { unseen.run(|| c.after_wait(key)) }
                    }
                }
                Some(k) => match tab {
                    MAIL => mail.after_wait(key),
                    CALENDAR => cal.after_wait(key),
                    TASKS => {
                        if tasks.key(k) {
                            next = Some(HOME);
                        }
                    }
                    _ if k == "C-S" => home.ask(&mail, &cal, &mut claude),
                    // Home and Claude drawn again from the start.
                    _ if k == "C-L" => {
                        home.repaint(&mail, &cal, &mut claude);
                        if let Some(c) = claude.app() {
                            c.after_wait(key);
                        }
                    }
                    _ if k == "C-Z" => {
                        if claude.app().is_some() {
                            home.full = !home.full;
                        }
                        home.repaint(&mail, &cal, &mut claude);
                    }
                    // A box at Home has the keys, or Tab moves them on.
                    _ if !home.full && (!home.claude_keys() || k == "TAB" || k == "S-TAB") => {
                        match home.key(k, &mut claude) {
                            Some(a @ Act::Edit(..)) => (act, next) = (Some(a), Some(TASKS)),
                            Some(a @ (Act::Mail(..) | Act::Compose)) => (act, next) = (Some(a), Some(MAIL)),
                            Some(a @ Act::Event(..)) => (act, next) = (Some(a), Some(CALENDAR)),
                            Some(Act::Purge(read, del)) => unseen.run(|| mail.read_and_delete(read, del)),
                            Some(Act::Reply(id, text)) => {
                                mail.reply_with(text);
                                (act, next) = (Some(Act::Mail(id, Some("r".into()))), Some(MAIL));
                            }
                            // tock reads the event now, and the agenda again.
                            Some(Act::Import) => {
                                unseen.run(|| cal.after_wait(None));
                                home.minute = 0;
                            }
                            Some(Act::Context(items)) => {
                                let text: Vec<String> = items.iter()
                                    .map(|&(b, id, start)| home.item(b, id, start, &mail, &cal))
                                    .filter(|t| !t.is_empty())
                                    .collect();
                                let mut what: Vec<&str> = items.iter().map(|i| ["kastrup", "tock", "tasks"][i.0]).collect();
                                what.dedup();
                                let head = if items.len() == 1 { "this was picked" } else { "these were tagged" };
                                if let (Some(c), false) = (claude.app(), text.is_empty()) {
                                    c.attach(&what.join(" + "), &format!("At gluon's Home {head}:\n\n{}", text.join("\n")));
                                }
                            }
                            Some(Act::Join(id, start)) => home.note = format!(" {}", unseen.run(|| cal.join(id, start))),
                            Some(Act::Copy(b, id, start, whole)) => {
                                let text = home.item(b, id, start, &mail, &cal);
                                let text = match (whole, b) {
                                    (false, _) => text.lines().next().unwrap_or_default().to_string(),
                                    (true, BOX_TASKS) => task_copy(&home.task_lines(), id),
                                    _ => text.trim_end().to_string(),
                                };
                                if let Some(r) = text.lines().next() {
                                    crust::clipboard_copy(&text, "clipboard");
                                    let more = text.lines().count() - 1;
                                    home.note = match more {
                                        0 => format!(" Copied: {r}"),
                                        n => format!(" Copied: {} and {n} more lines", r.trim()),
                                    };
                                }
                            }
                            None => {}
                        }
                    }
                    _ => {
                        if let Some(c) = claude.app() {
                            c.after_wait(key);
                        }
                    }
                },
                // No key came: every tab gets its idle turn, the hidden ones
                // unseen. Claude's output wakes it by itself, so a hidden
                // Claude needs none.
                None => {
                    if tab == MAIL { mail.after_wait(None) } else { unseen.run(|| mail.after_wait(None)) }
                    if tab == CALENDAR { cal.after_wait(None) } else { unseen.run(|| cal.after_wait(None)) }
                    if let (HOME, Some(c)) = (tab, claude.app()) {
                        c.after_wait(None);
                    }
                    if let Some(e) = tasks.ed.as_mut().filter(|e| e.before_wait().is_some()) {
                        if tab == TASKS && !tasks.board { e.after_wait(None) } else { unseen.run(|| e.after_wait(None)) }
                    }
                }
            }
        }
        if let Some(n) = next {
            if tab == TASKS {
                tasks.leave();
            }
            if n == HOME {
                back = tab;
            }
            from_home = act.is_some();
            tab = n;
            if let Some(c) = claude.app() {
                c.set_hidden(tab != HOME);
            }
            if tab != HOME {
                Cursor::hide();
            }
            match tab {
                MAIL => {
                    mail.repaint();
                    // A key for kastrup from a box at Home; those done here
                    // go back Home.
                    // `one`: kastrup shows one message, and lets it go after.
                    let (done, one) = match act {
                        Some(Act::Mail(id, k)) => {
                            mail.goto_message(id, k.is_some());
                            let done = k.as_deref().is_some_and(|k| BACK.contains(&k));
                            if k.is_some() {
                                mail.after_wait(k);
                            }
                            (done, true)
                        }
                        // The drafts: kastrup's picker waits for its keys.
                        Some(Act::Compose) => {
                            mail.after_wait(Some("+".into()));
                            (true, false)
                        }
                        _ => (false, false),
                    };
                    if done && mail.running() {
                        if one {
                            unseen.run(|| mail.leave_message());
                        }
                        (back, tab, from_home) = (MAIL, HOME, false);
                        if let Some(c) = claude.app() {
                            c.set_hidden(false);
                        }
                        home.repaint(&mail, &cal, &mut claude);
                    }
                }
                CALENDAR => {
                    if let Some(Act::Event(id, start)) = act {
                        cal.show_event(id, start);
                    }
                    // tock checks its goto file and the poller once a
                    // minute; an invite kastrup just accepted shows now.
                    unseen.run(|| cal.after_wait(None));
                    cal.repaint();
                }
                TASKS => match act {
                    Some(Act::Edit(line)) => tasks.edit_at(line as usize),
                    _ => tasks.repaint(),
                },
                _ => home.repaint(&mail, &cal, &mut claude),
            }
            continue;
        }
        // Home's boxes, when their mail, agenda or tasks changed, or when
        // a popup or the editor of Claude's drew over them.
        if tab == HOME {
            let over = claude.app().is_some_and(|c| c.take_overdraw());
            // Claude's ideas came: the box shows them, or the bar counts them.
            home.ideas.poll();
            if home.paint(&mail, &cal, over) {
                if let Some(c) = claude.app() {
                    c.place_cursor();
                }
            }
        }
    }

    // A failed write keeps the recovery file.
    if let Some(mut e) = tasks.ed.take() {
        if e.save_if_dirty() {
            e.close();
        }
    }
    if let Claude::On(c) = claude {
        keep(&dir, c.session_id());
        deck::app::finish(c);
    }
    tock::app::finish(cal);
    kastrup::app::finish(mail);
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODO: &str = "\tInbox\n\tHome\n\t\tPaint\n\t\t[O] Bike\n\t\t\tChain\n\t\t[x] Milk\n\tWork\n\t\t[-] Ask\n";

    #[test]
    fn a_task_goes_to_claude_once_and_copies_moved_left() {
        let lines: Vec<String> = TODO.lines().map(String::from).collect();
        // Line 4 is "\t\t[O] Bike" with "\t\t\tChain" under it.
        assert_eq!(task_item(&lines, 4), "todo:4 [O] Bike\n\t\t\tChain\n");
        assert_eq!(task_copy(&lines, 4), "[O] Bike\n\tChain");
        assert_eq!(task_item(&lines, 99), "");
    }

    #[test]
    fn a_tag_stays_with_its_line_and_goes_to_claude_once() {
        let rows = |ls: &[&str]| ls.iter().enumerate().map(|(i, l)| (Some((i as i64 + 1, line_key(l))), l.to_string())).collect::<Vec<_>>();
        let mut f = Frame::new("");
        f.place(1, 1, 40, 10);
        f.set_rows(rows(&["a", "b", "c"]));
        f.step(1);
        f.tag();
        f.tag();
        // A line comes in above b and c, so their tags go, not to a and b.
        f.set_rows(rows(&["new", "a", "b", "c"]));
        assert!(f.tags.is_empty());
        f.set_rows(rows(&["a", "b", "c"]));
        f.sel = 0;
        f.tag();
        f.step(1);
        f.tag();
        assert_eq!(f.take_tags(), vec![(1, line_key("a")), (3, line_key("c"))]);
        assert!(f.take_tags().is_empty());
    }

    #[test]
    fn lanes_are_the_top_lines_and_cards_the_lines_under_them() {
        let lanes = lanes_of(TODO);
        let names: Vec<&str> = lanes.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Inbox", "Home", "Work"]);
        let home: Vec<(&str, usize)> = lanes[1].cards.iter().map(|c| (c.text.as_str(), c.col)).collect();
        assert_eq!(home, [("Paint", 0), ("Bike", 1), ("Milk", 3)]);
        assert_eq!((lanes[2].cards[0].text.as_str(), lanes[2].cards[0].col), ("Ask", 2));
    }

    #[test]
    fn a_column_is_the_box_in_front() {
        assert_eq!(in_column("\t\tPaint", 1), "\t\t[O] Paint");
        assert_eq!(in_column("\t\t[O] Bike", 3), "\t\t[x] Bike");
        assert_eq!(in_column("\t\t[x] Milk", 0), "\t\tMilk");
    }

    #[test]
    fn a_task_moves_to_another_lane_with_what_is_under_it() {
        let mut lines: Vec<String> = TODO.lines().map(String::from).collect();
        to_lane(&mut lines, 3, 0);
        assert_eq!(lines, ["\tInbox", "\t\t[O] Bike", "\t\t\tChain", "\tHome", "\t\tPaint", "\t\t[x] Milk", "\tWork", "\t\t[-] Ask"]);
        to_lane(&mut lines, 1, 6);
        assert_eq!(lines[3..], ["\t\t[x] Milk", "\tWork", "\t\t[-] Ask", "\t\t[O] Bike", "\t\t\tChain"]);
    }

    #[test]
    fn a_long_task_wraps_at_spaces_and_ends_in_dots() {
        assert_eq!(wrap("one two three four", 9, 3), ["one two", "three", "four"]);
        assert_eq!(wrap("one two three four", 9, 2), ["one two", "three …"]);
    }
    #[test]
    fn an_event_time_becomes_ics_and_a_bad_one_is_left_out() {
        assert_eq!(ics_time("2026-11-03 09:30").as_deref(), Some("20261103T093000"));
        assert_eq!(ics_time("2026-11-03").as_deref(), Some("20261103"));
        assert_eq!(ics_time("3 Nov"), None);
        assert_eq!(ics_time("2026-11-03 9:30"), None);
    }

    #[test]
    fn ideas_come_from_the_json_in_the_answer_and_bad_ones_drop() {
        let msgs = [(7, "Alice: Lunch".to_string())].into_iter().collect();
        let answer = r#"Here: [{"kind":"task","text":"Buy milk","why":"out"},
            {"kind":"event","text":"Dentist","start":"next week"},
            {"kind":"reply","id":"7","text":"Yes.","why":"Alice asks"},
            {"kind":"reply","id":8,"text":"No."}]"#;
        let ideas = parse_ideas(answer, &msgs).unwrap();
        assert_eq!(ideas.iter().map(|i| i.kind.as_str()).collect::<Vec<_>>(), ["task", "reply"]);
        assert_eq!(ideas[1].id, 7);
        assert!(parse_ideas("no list", &msgs).is_err());
        let fenced = "[a note] first:\n```json\n[{\"kind\":\"task\",\"text\":\"Buy milk\"}]\n```\nThen [more].";
        assert_eq!(parse_ideas(fenced, &msgs).unwrap().len(), 1);
    }

    #[test]
    fn a_task_goes_at_the_end_of_the_inbox_lane() {
        let path = std::env::temp_dir().join(format!("gluon-test-{}.hl", std::process::id()));
        std::fs::write(&path, "\tWork\n\t\tAsk\n\tInbox\n\t\tBike\n\t\t\tChain\n\tHome\n").unwrap();
        add_task(&path, "Buy milk");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "\tWork\n\t\tAsk\n\tInbox\n\t\tBike\n\t\t\tChain\n\t\tBuy milk\n\tHome\n");
        std::fs::write(&path, "").unwrap();
        add_task(&path, "Buy milk");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Inbox\n\tBuy milk");
        let _ = std::fs::remove_file(&path);
    }
}
