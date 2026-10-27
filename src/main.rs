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
Mail (kastrup), Calendar (tock) and Tasks. The user moves between them
with Ctrl+O, and Ctrl+A jumps Home and back. When Ctrl+A comes from
another tab, the next prompt starts with an <on-screen> block saying what
that tab showed. Ctrl+Z gives this pane the whole screen and back.

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
        let editor = std::env::var("EDITOR").ok().filter(|e| !e.is_empty())
            .unwrap_or_else(|| "scribe".to_string());
        if let Some(dir) = self.path.parent() { let _ = std::fs::create_dir_all(dir); }
        Crust::cleanup();
        let _ = std::process::Command::new(&editor).arg(&self.path).status();
        Crust::init();
        Crust::set_app_identity("gluon");
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

/// The Home tab: unread mail in kastrup's views at the top left, the
/// agenda over the top of the task list at the top right, and Claude in
/// the bottom half, or on the whole screen after Ctrl+Z.
struct Home {
    bar: Pane,
    mail: Pane,
    agenda: Pane,
    tasks: Pane,
    /// The keys of the views to show, as "1 2 3", in ~/.gluon/views.
    views: PathBuf,
    todo: PathBuf,
    /// What the panes were made from, so a wake with nothing new draws
    /// nothing: kastrup's database files, the minute, todo.hl's time.
    sig: Option<(u64, u128, u128)>,
    minute: i64,
    mtime: Option<std::time::SystemTime>,
    full: bool,
}

impl Home {
    fn new(home: &Path) -> Self {
        let p = || Pane::new(1, 1, 1, 1, 252, 0);
        let mut h = Home {
            bar: p(),
            mail: p(),
            agenda: p(),
            tasks: p(),
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

    /// Rows above Claude's half: the bar and the top panes.
    fn split() -> (u16, u16) {
        let (_, rows) = Crust::terminal_size();
        let top = (rows.saturating_sub(1) / 2).max(1);
        (top + 1, rows.saturating_sub(top + 1))
    }

    fn layout(&mut self) {
        let (cols, _) = Crust::terminal_size();
        let h = Self::split().0 - 1;
        let lw = cols / 2;
        let rw = cols.saturating_sub(lw + 1);
        let ah = h / 2;
        self.bar = Pane::new(1, 1, cols, 1, 255, 236);
        self.mail = Pane::new(1, 2, lw, h, 252, 0);
        self.agenda = Pane::new(lw + 2, 2, rw, ah, 252, 0);
        self.tasks = Pane::new(lw + 2, 2 + ah, rw, h - ah, 252, 0);
        for p in [&mut self.bar, &mut self.mail, &mut self.agenda, &mut self.tasks] {
            p.wrap = false;
            p.scroll = false;
        }
    }

    /// The whole tab, after another tab had the screen or it was resized.
    fn repaint(&mut self, mail: &kastrup::app::App, cal: &tock::app::App, claude: &mut Claude) {
        if let (true, Claude::On(c)) = (self.full, &mut *claude) {
            c.set_tab_line(Some(tab_line(HOME) + &style::fg("^Z half ", 240)));
            c.set_area(None);
            return;
        }
        Crust::clear_screen();
        self.layout();
        self.paint(mail, cal, true);
        let (top, rows) = Self::split();
        let (cols, _) = Crust::terminal_size();
        match claude {
            Claude::On(c) => {
                c.set_tab_line(Some(String::new()));
                c.set_area(Some((top, rows)));
            }
            other => {
                let why = match other {
                    Claude::Failed(e) => format!("Claude could not start: {e}"),
                    _ => "Type to talk to Claude; it starts at the first key.".into(),
                };
                Pane::new(1, top + 1, cols, 1, 255, 234).say(&style::fg(" Claude", 39));
                Pane::new(2, top + 3, cols.saturating_sub(2), rows.saturating_sub(2), 245, 0).say(&why);
                Cursor::hide();
            }
        }
    }

    /// The bar and the top panes, each drawn only when its text changed,
    /// unless `force`. True when something was drawn.
    fn paint(&mut self, mail: &kastrup::app::App, cal: &tock::app::App, force: bool) -> bool {
        if self.full {
            return false;
        }
        let mut drew = false;
        let sig = mail.db_signature();
        if force || self.sig != Some(sig) {
            self.sig = Some(sig);
            let text = self.unread(mail);
            drew |= show(&mut self.mail, text, force);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        if force || now / 60 != self.minute {
            self.minute = now / 60;
            let w = self.agenda.w as usize;
            let lines = cal.agenda_lines((self.agenda.h as usize).saturating_sub(1), w);
            let text = if lines.is_empty() {
                head("Agenda") + "\n" + &style::fg("  Nothing in the next 30 days", 240)
            } else {
                head("Agenda") + "\n" + &lines.join("\n")
            };
            drew |= show(&mut self.agenda, text, force);
            let cols = self.bar.w as usize;
            let right = style::fg(&format!("^Z Claude full · {} ", clock(now, "%a %e %b · week %V")), 245);
            let left = tab_line(HOME);
            let room = cols.saturating_sub(crust::display_width(&left) + crust::display_width(&right));
            drew |= show(&mut self.bar, format!("{left}{}{right}", " ".repeat(room)), force);
        }
        let mtime = std::fs::metadata(&self.todo).and_then(|m| m.modified()).ok();
        if force || mtime != self.mtime {
            self.mtime = mtime;
            let text = std::fs::read_to_string(&self.todo).unwrap_or_default();
            let n = (self.tasks.h as usize).saturating_sub(1);
            let w = self.tasks.w as usize;
            let top: Vec<&str> = text.lines().take(n).collect();
            let coloured = highlight::highlight_hyperlist(&top.join("\n"), usize::MAX).replace('\t', "   ");
            let lines: Vec<String> = coloured.lines().map(|l| crust::truncate_ansi(l, w)).collect();
            drew |= show(&mut self.tasks, head("Tasks") + "\n" + &lines.join("\n"), force);
        }
        drew
    }

    /// The unread messages in the views, newest first, each in its view's
    /// colour.
    fn unread(&self, mail: &kastrup::app::App) -> String {
        let file = std::fs::read_to_string(&self.views).unwrap_or_default();
        let mut keys: Vec<&str> = file.split(|c: char| c.is_whitespace() || c == ',').filter(|k| !k.is_empty()).collect();
        if keys.is_empty() {
            keys = vec!["1", "2", "3"];
        }
        let w = self.mail.w as usize;
        let n = (self.mail.h as usize).saturating_sub(1);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let today = clock(now, "%F");
        let list = mail.unread_in_views(&keys, n);
        let mut t = head("Unread");
        if list.is_empty() {
            t += &style::fg(&format!("\n  Nothing unread in views {}", keys.join(" ")), 240);
        }
        for u in list {
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
            t += "\n";
            t += &crust::truncate_ansi(&line, w);
        }
        t
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

fn head(name: &str) -> String {
    style::bold(&style::fg(&format!(" {name}"), 250))
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
        println!("  Home      unread mail in kastrup's views 1-3 (or those in ~/.gluon/views),");
        println!("            the agenda, the top of the tasks, and Claude below them");
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
        let next = match key.as_deref() {
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
                Some(HOME)
            }
            _ => None,
        };
        if let Some(n) = next {
            if n == HOME {
                back = tab;
            }
            tab = n;
            if let Some(c) = claude.app() {
                c.set_hidden(tab != HOME);
            }
            if tab != HOME {
                Cursor::hide();
            }
            match tab {
                MAIL => mail.repaint(),
                CALENDAR => cal.repaint(),
                TASKS => tasks.repaint(),
                _ => home.repaint(&mail, &cal, &mut claude),
            }
            continue;
        }
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
        // Home's panes, when their mail, agenda or tasks changed, or when
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
