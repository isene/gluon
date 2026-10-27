//! gluon: mail, calendar, tasks and Claude in one terminal app. The Home
//! tab shows unread mail, the agenda and the tasks over a Claude pane
//! (deck); the Mail tab is kastrup and the Calendar tab is tock, all run
//! as libraries; the Tasks tab shows ~/.tasks/todo.hl. Ctrl+O moves to
//! the next tab, and Ctrl+A jumps Home and back.

use crust::{style, Crust, Cursor, Input, Pane};
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
  running tock imports.
- Tasks: ~/.tasks/todo.hl, a HyperList file indented with tabs. Edit it
  in place; the Tasks tab reads it again each time it is shown.

Home lists the unread mail in the kastrup views whose keys are in
~/.gluon/views, as \"1 2 3\" (the default), each in its view's colour.

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

/// The Tasks tab: the todo file, coloured as HyperList, edited in the
/// editor (scribe unless $EDITOR names another).
struct Tasks {
    top: Pane,
    body: Pane,
    bottom: Pane,
    path: PathBuf,
    quit: bool,
}

impl Tasks {
    fn new(home: &Path) -> Self {
        let mut t = Tasks {
            top: Pane::new(1, 1, 1, 1, 255, 236),
            body: Pane::new(1, 2, 1, 1, 255, 0),
            bottom: Pane::new(1, 3, 1, 1, 252, 236),
            path: home.join(".tasks").join("todo.hl"),
            quit: false,
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

    fn load(&mut self) {
        let text = std::fs::read_to_string(&self.path)
            .unwrap_or_else(|_| format!("No tasks yet: {} does not exist. Press e to start it.", self.path.display()));
        let ix = self.body.ix;
        // The highlighter reads the indent from the tabs; on screen a
        // level is three columns, as in scribe, not crust's eight.
        let coloured = highlight::highlight_hyperlist(&text, usize::MAX).replace('\t', "   ");
        self.body.set_text(&coloured);
        self.body.ix = ix;
    }

    fn repaint(&mut self) {
        Crust::clear_screen();
        self.layout();
        self.load();
        self.top.say(&tab_line(TASKS));
        self.body.full_refresh();
        self.bottom.say(&style::fg(" j/k scroll  e edit  Ctrl+O next tab  Ctrl+A Home  q quit", 245));
    }

    /// For Claude: which file is on screen, and from where.
    fn claude_context(&self) -> String {
        format!(
            "The task list {} is on screen, from line {}. It is HyperList.",
            self.path.display(), self.body.ix + 1
        )
    }

    fn edit(&mut self) {
        edit(&self.path, self.body.ix as i64 + 1);
        self.repaint();
    }

    fn key(&mut self, key: &str) {
        match key {
            "j" | "DOWN" => { self.body.linedown(); }
            "k" | "UP" => { self.body.lineup(); }
            " " | "PgDOWN" => { self.body.pagedown(); }
            "PgUP" => { self.body.pageup(); }
            "g" | "HOME" => { self.body.top(); }
            "G" | "END" => { self.body.bottom(); }
            "e" | "ENTER" => { self.edit(); }
            "q" => { self.quit = true; }
            "RESIZE" => { self.repaint(); }
            _ => {}
        }
    }
}

/// The task list in the editor (scribe unless $EDITOR names another), at
/// `line`.
fn edit(path: &Path, line: i64) {
    let editor = std::env::var("EDITOR").ok().filter(|e| !e.is_empty())
        .unwrap_or_else(|| "scribe".to_string());
    if let Some(dir) = path.parent() { let _ = std::fs::create_dir_all(dir); }
    Crust::cleanup();
    let _ = std::process::Command::new(&editor).arg(format!("+{line}")).arg(path).status();
    Crust::init();
    Crust::set_app_identity("gluon");
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

/// Claude, started at the first key in the Home tab or at Ctrl+A: an
/// idle Claude Code process takes about 33 mW, more than the rest of gluon.
enum Claude {
    Off,
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

    fn open(&mut self, home: &Path) {
        if let Claude::Off = self {
            *self = match start_claude(home) {
                Ok(a) => {
                    Crust::set_app_identity("gluon");
                    Claude::On(a)
                }
                Err(e) => Claude::Failed(e),
            };
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
                if focused && i == self.sel && k.is_some() {
                    let p = crust::strip_ansi(t);
                    let pad = " ".repeat(w.saturating_sub(crust::display_width(&p)));
                    style::styled(&(p + &pad), Some(255), Some(238), "")
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
enum Act {
    /// Message `id` opened in the Mail tab, then the key, if any, for it.
    Mail(i64, Option<String>),
    /// The Calendar on an event, by its id and start.
    Event(i64, i64),
    /// The task list in the editor, at a line.
    Edit(i64),
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
}

impl Home {
    fn new(home: &Path) -> Self {
        let mut h = Home {
            bar: Pane::new(1, 1, 1, 1, 255, 236),
            boxes: [
                Frame::new(" Enter opens · r replies · Space scrolls, S-← back · Esc Claude "),
                Frame::new(" Enter opens it in Calendar · Esc Claude "),
                Frame::new(" Enter edits at this line · Esc Claude "),
            ],
            preview: Frame::new(""),
            shown: None,
            previewing: false,
            focus: CLAUDE,
            views: home.join(".gluon").join("views"),
            todo: home.join(".tasks").join("todo.hl"),
            sig: None,
            minute: 0,
            mtime: None,
            full: false,
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
                c.set_tab_line(Some(claude_label(self.focus == CLAUDE)));
                c.set_area(Some(Self::split()));
                c.set_focus(self.focus == CLAUDE);
            }
            other => self.claude_off(other),
        }
    }

    /// Claude's rows before it has started, or after it failed to.
    fn claude_off(&self, claude: &Claude) {
        let (top, rows) = Self::split();
        let (cols, _) = Crust::terminal_size();
        let why = match claude {
            Claude::Failed(e) => format!("Claude could not start: {e}"),
            _ => "Type to talk to Claude; it starts at the first key.".into(),
        };
        Pane::new(1, top + 1, cols, 1, 255, 236).say(&claude_label(self.focus == CLAUDE));
        Pane::new(2, top + 3, cols.saturating_sub(2), rows.saturating_sub(2), 245, 0).say(&why);
        Cursor::hide();
    }

    /// The keys to box `f`, or to Claude.
    fn focus_on(&mut self, f: usize, claude: &mut Claude) {
        self.focus = f;
        match claude {
            Claude::On(c) => {
                c.set_tab_line(Some(claude_label(f == CLAUDE)));
                c.set_focus(f == CLAUDE);
            }
            other => self.claude_off(other),
        }
    }

    /// A key while a box has the keys, or Tab while Claude has them. The
    /// box moves its pick itself; what goes beyond it comes back.
    fn key(&mut self, k: &str, claude: &mut Claude) -> Option<Act> {
        match k {
            "TAB" => self.focus_on((self.focus + 1) % 4, claude),
            "S-TAB" => self.focus_on((self.focus + 3) % 4, claude),
            "ESC" => self.focus_on(CLAUDE, claude),
            _ => {
                let b = self.boxes.get_mut(self.focus)?;
                let page = b.pane.h as isize;
                let all = b.rows.len() as isize;
                // kastrup moves with the arrows alone, and its letters act
                // on the picked message.
                let vi = self.focus != BOX_MAIL;
                match k {
                    "DOWN" => b.step(1),
                    "UP" => b.step(-1),
                    "j" if vi => b.step(1),
                    "k" if vi => b.step(-1),
                    "PgDOWN" => b.step(page),
                    "PgUP" => b.step(-page),
                    "HOME" => b.step(-all),
                    "END" => b.step(all),
                    "g" if vi => b.step(-all),
                    "G" if vi => b.step(all),
                    " " | "S-RIGHT" if !vi => self.preview.pane.pagedown(),
                    "S-LEFT" if !vi => self.preview.pane.pageup(),
                    _ => {
                        let (id, start) = b.picked()?;
                        let one = k.chars().count() == 1;
                        return match (self.focus, k) {
                            (BOX_MAIL, "ENTER") => Some(Act::Mail(id, None)),
                            // q would end gluon, and a digit would leave
                            // the message for another view.
                            (BOX_MAIL, _) if one && k != "q" && !k.starts_with(|c: char| c.is_ascii_digit()) => {
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
        if force || now / 60 != self.minute {
            self.minute = now / 60;
            self.load_agenda(cal);
            let cols = self.bar.w as usize;
            let right = style::fg(&format!("Tab next box · ^Z Claude full · {} ", clock(now, "%a %e %b · week %V")), 245);
            let left = tab_line(HOME);
            let room = cols.saturating_sub(crust::display_width(&left) + crust::display_width(&right));
            drew |= show(&mut self.bar, format!("{left}{}{right}", " ".repeat(room)), force);
        }
        let mtime = std::fs::metadata(&self.todo).and_then(|m| m.modified()).ok();
        if force || mtime != self.mtime {
            self.mtime = mtime;
            self.load_tasks();
        }
        // While the mail box has the keys, the picked message takes the
        // place of the agenda and the tasks.
        let preview = self.focus == BOX_MAIL;
        let swap = preview != self.previewing;
        self.previewing = preview;
        drew |= self.boxes[BOX_MAIL].draw(preview, force);
        if preview {
            drew |= self.draw_preview(mail, force || swap);
        } else {
            for i in [BOX_AGENDA, BOX_TASKS] {
                drew |= self.boxes[i].draw(self.focus == i, force || swap);
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

    /// The unread messages in the views, newest first, each in its view's
    /// colour.
    fn load_mail(&mut self, mail: &kastrup::app::App) {
        let file = std::fs::read_to_string(&self.views).unwrap_or_default();
        let mut keys: Vec<&str> = file.split(|c: char| c.is_whitespace() || c == ',').filter(|k| !k.is_empty()).collect();
        if keys.is_empty() {
            keys = vec!["1", "2", "3"];
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let today = clock(now, "%F");
        let list = mail.unread_in_views(&keys, UNREAD);
        let more = if list.len() == UNREAD { "+" } else { "" };
        let mut rows = Vec::new();
        if list.is_empty() {
            rows.push((None, style::fg(&format!(" Nothing unread in views {}", keys.join(" ")), 240)));
        }
        for u in &list {
            let when = clock(u.time, if clock(u.time, "%F") == today { "%H:%M" } else { "%e %b" });
            let light = lighter(u.colour);
            let from = crust::truncate_ansi(&plain(&u.from), 18);
            let pad = " ".repeat(18usize.saturating_sub(crust::display_width(&from)));
            let line = format!(
                "{} {when:>6}  {}  {}",
                style::styled(&format!(" {} ", u.view), Some(255), Some(u.colour), ""),
                style::fg(&(from + &pad), light),
                style::fg(&plain(&u.subject), light),
            );
            rows.push((Some((u.id, 0)), line));
        }
        let b = &mut self.boxes[BOX_MAIL];
        b.title = format!(" Unread · {}{more} ", list.len());
        b.set_rows(rows);
    }

    /// The coming 30 days, with a rule between one day and the next.
    fn load_agenda(&mut self, cal: &tock::app::App) {
        let b = &mut self.boxes[BOX_AGENDA];
        let w = b.inner();
        let mut rows = Vec::new();
        for a in cal.agenda(UNREAD, w) {
            if a.new_day && !rows.is_empty() {
                rows.push((None, style::fg(&"─".repeat(w), 237)));
            }
            rows.push((Some((a.id, a.start)), a.text));
        }
        if rows.is_empty() {
            rows.push((None, style::fg("  Nothing in the next 30 days", 240)));
        }
        b.title = " Agenda ".into();
        b.set_rows(rows);
    }

    /// The task list, coloured as HyperList, one row per line of the file.
    fn load_tasks(&mut self) {
        let text = std::fs::read_to_string(&self.todo).unwrap_or_default();
        let coloured = highlight::highlight_hyperlist(&text, usize::MAX).replace('\t', "   ");
        let mut rows: Vec<_> = coloured.lines().enumerate().map(|(i, l)| (Some((i as i64 + 1, 0)), l.to_string())).collect();
        if rows.is_empty() {
            rows.push((Some((1, 0)), style::fg(" No tasks yet; Enter starts the list", 240)));
        }
        let b = &mut self.boxes[BOX_TASKS];
        b.title = " Tasks ".into();
        b.set_rows(rows);
    }
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

/// A lighter shade of a colour from the 6x6x6 cube, for text on black:
/// a view's colour is made for a bar's background, and some are too dark
/// to read as text.
fn lighter(c: u8) -> u8 {
    if !(16..232).contains(&c) {
        return c;
    }
    let up = |v: u8| v + (5 - v) / 2;
    let i = c - 16;
    16 + 36 * up(i / 36) + 6 * up(i / 6 % 6) + up(i % 6)
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
        println!("            (Esc comes back), and kastrup's keys act on a picked message");
        println!("  Mail      kastrup, with its own keys and ~/.kastrup/");
        println!("  Calendar  tock, with its own keys and ~/.tock/");
        println!("  Tasks     ~/.tasks/todo.hl as HyperList; e edits it");
        println!();
        println!("Claude is deck: one Claude Code session that stays, in ~/.gluon/. Ctrl+Z");
        println!("gives it the whole screen and back. Ctrl+O moves to the next tab. Ctrl+A");
        println!("jumps Home with what is on screen, and back. q in Mail, Calendar or");
        println!("Tasks, or C-D at Home, ends gluon.");
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
    let Some(mut cal) = tock::app::start() else { return };
    let Some(mut mail) = kastrup::app::start() else {
        tock::app::finish(cal);
        return;
    };
    let mut claude = Claude::Off;
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

    while mail.running() && cal.running() && !tasks.quit && claude.running() {
        // Claude's 1 s clock while it works is for its own bar alone.
        let mut wait = match tab {
            MAIL => mail.before_wait().min(unseen.run(|| cal.before_wait())),
            CALENDAR => cal.before_wait().min(unseen.run(|| mail.before_wait())),
            _ => unseen.run(|| mail.before_wait().min(cal.before_wait())),
        };
        if let (HOME, Claude::On(c)) = (tab, &claude) {
            wait = wait.min(c.before_wait());
        }
        let key = Input::getchr(Some(wait));
        let mut act = None;
        let mut next = match key.as_deref() {
            Some("C-O") => Some((tab + 1) % TABS.len()),
            Some("C-A") if tab == HOME => Some(back),
            Some("C-A") => {
                let seen = match tab {
                    MAIL => mail.claude_context().map(|t| ("kastrup", t)),
                    CALENDAR => Some(("tock", cal.claude_context())),
                    _ => Some(("tasks", tasks.claude_context())),
                };
                claude.open(&dir);
                if let (Some(c), Some((what, text))) = (claude.app(), seen) {
                    unseen.run(|| c.attach(what, &text));
                }
                home.focus = CLAUDE;
                Some(HOME)
            }
            // Esc in the tab a box at Home opened goes back Home; kastrup
            // takes it first, to drop the one-message list.
            Some("ESC") if from_home && tab != HOME => {
                if tab == MAIL {
                    mail.after_wait(key.clone());
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
                                    _ => tasks.key("RESIZE"),
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
                    TASKS => tasks.key(k),
                    _ if k == "C-Z" => {
                        claude.open(&dir);
                        if claude.app().is_some() {
                            home.full = !home.full;
                        }
                        home.repaint(&mail, &cal, &mut claude);
                    }
                    _ if k == "C-D" && claude.app().is_none() => break,
                    // A box at Home has the keys, or Tab moves them on.
                    _ if !home.full && (home.focus != CLAUDE || k == "TAB" || k == "S-TAB") => {
                        match home.key(k, &mut claude) {
                            Some(Act::Edit(line)) => {
                                edit(&home.todo, line);
                                home.repaint(&mail, &cal, &mut claude);
                            }
                            Some(a @ Act::Mail(..)) => (act, next) = (Some(a), Some(MAIL)),
                            Some(a @ Act::Event(..)) => (act, next) = (Some(a), Some(CALENDAR)),
                            None => {}
                        }
                    }
                    // A typed letter starts Claude; Esc or an arrow does not.
                    _ if k.chars().nth(1).is_some() && matches!(claude, Claude::Off) => {}
                    _ => {
                        if let Claude::Off = claude {
                            claude.open(&dir);
                            home.repaint(&mail, &cal, &mut claude);
                        }
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
                }
            }
        }
        if let Some(n) = next {
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
                    if let Some(Act::Mail(id, k)) = act {
                        mail.goto_message(id);
                        if k.is_some() {
                            mail.after_wait(k);
                        }
                    }
                }
                CALENDAR => {
                    if let Some(Act::Event(id, start)) = act {
                        cal.show_event(id, start);
                    }
                    cal.repaint();
                }
                TASKS => tasks.repaint(),
                _ => home.repaint(&mail, &cal, &mut claude),
            }
            continue;
        }
        // Home's boxes, when their mail, agenda or tasks changed, or when
        // a popup or the editor of Claude's drew over them.
        if tab == HOME {
            let over = claude.app().is_some_and(|c| c.take_overdraw());
            if home.paint(&mail, &cal, over) {
                if let Some(c) = claude.app() {
                    c.place_cursor();
                }
            }
        }
    }

    if let Claude::On(c) = claude {
        keep(&dir, c.session_id());
        deck::app::finish(c);
    }
    tock::app::finish(cal);
    kastrup::app::finish(mail);
}
