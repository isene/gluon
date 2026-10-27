//! gluon: mail, calendar, tasks and Claude in one terminal app. The Mail
//! tab is kastrup, the Calendar tab is tock and the Claude tab is deck,
//! all run as libraries; the Tasks tab shows ~/.tasks/todo.hl. Ctrl+O
//! moves to the next tab, and Ctrl+A jumps to Claude and back.

use crust::{style, Crust, Cursor, Input, Pane};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

const TABS: [&str; 4] = ["Mail", "Calendar", "Tasks", "Claude"];
const MAIL: usize = 0;
const CALENDAR: usize = 1;
const TASKS: usize = 2;
const CLAUDE: usize = 3;

/// The Claude tab's CLAUDE.md, written to ~/.gluon once; edits there stay.
const BRIEF: &str = "# gluon's Claude tab

You run in the Claude tab of gluon, a terminal app with four tabs: Mail
(kastrup), Calendar (tock), Tasks and this one. The user moves between
them with Ctrl+O, and Ctrl+A jumps here and back. When Ctrl+A comes from
another tab, the next prompt starts with an <on-screen> block saying what
that tab showed.

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

This tab is a pane beside the user's work: answer short.
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
        self.bottom.say(&style::fg(" j/k scroll  e edit  Ctrl+O next tab  Ctrl+A Claude  q quit", 245));
    }

    /// For the Claude tab: which file is on screen, and from where.
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

/// The Claude tab: deck in ~/.gluon, the same session each time (its id
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
    let _ = std::fs::write(dir.join("session"), app.session_id());
    Ok(app)
}

/// The Claude tab, started the first time it is shown: an idle Claude
/// Code process takes about 33 mW, more than the rest of gluon.
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
                Ok(mut a) => {
                    a.set_tab_line(Some(tab_line(CLAUDE)));
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

    fn repaint(&mut self) {
        match self {
            Claude::On(a) => a.repaint(),
            Claude::Failed(e) => claude_failed(e),
            Claude::Off => {}
        }
    }
}

/// What the Claude tab shows when deck could not start.
fn claude_failed(why: &str) {
    Crust::clear_screen();
    let (cols, rows) = Crust::terminal_size();
    Pane::new(1, 1, cols, 1, 255, 236).say(&tab_line(CLAUDE));
    Pane::new(2, 3, cols.saturating_sub(2), rows.saturating_sub(3), 252, 0)
        .say(&format!("Claude could not start: {why}"));
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    if arg == "-h" || arg == "--help" {
        println!("gluon — mail, calendar, tasks and Claude in one terminal app (Fe2O3 suite)");
        println!();
        println!("Usage: gluon");
        println!();
        println!("  Mail      kastrup, with its own keys and ~/.kastrup/");
        println!("  Calendar  tock, with its own keys and ~/.tock/");
        println!("  Tasks     ~/.tasks/todo.hl as HyperList; e edits it");
        println!("  Claude    deck: one Claude Code session that stays, in ~/.gluon/");
        println!();
        println!("Ctrl+O moves to the next tab. Ctrl+A jumps to Claude with what is on");
        println!("screen, and back. q in Mail, Calendar or Tasks, or C-D in Claude, ends gluon.");
        return;
    }
    if arg == "-v" || arg == "--version" {
        println!("gluon {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();

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
    let mut tasks = Tasks::new(&home);
    let mut tab = MAIL;
    // Where Ctrl+A in the Claude tab goes back to.
    let mut back = MAIL;
    let mut size = Crust::terminal_size();
    mail.repaint();

    while mail.running() && cal.running() && !tasks.quit && claude.running() {
        // Claude's 1 s clock while it works is for its own bar alone.
        let mut wait = match tab {
            MAIL => mail.before_wait().min(unseen.run(|| cal.before_wait())),
            CALENDAR => cal.before_wait().min(unseen.run(|| mail.before_wait())),
            _ => unseen.run(|| mail.before_wait().min(cal.before_wait())),
        };
        if let (CLAUDE, Claude::On(c)) = (tab, &claude) {
            wait = wait.min(c.before_wait());
        }
        let key = Input::getchr(Some(wait));
        let next = match key.as_deref() {
            Some("C-O") => Some((tab + 1) % TABS.len()),
            Some("C-A") if tab == CLAUDE => Some(back),
            Some("C-A") => {
                let seen = match tab {
                    MAIL => mail.claude_context().map(|t| ("kastrup", t)),
                    CALENDAR => Some(("tock", cal.claude_context())),
                    _ => Some(("tasks", tasks.claude_context())),
                };
                claude.open(&home);
                if let (Some(c), Some((what, text))) = (claude.app(), seen) {
                    unseen.run(|| c.attach(what, &text));
                }
                back = tab;
                Some(CLAUDE)
            }
            _ => None,
        };
        if let Some(n) = next {
            tab = n;
            if tab == CLAUDE {
                claude.open(&home);
            }
            if let Some(c) = claude.app() {
                c.set_hidden(tab != CLAUDE);
            }
            if tab != CLAUDE {
                Cursor::hide();
            }
            match tab {
                MAIL => mail.repaint(),
                CALENDAR => cal.repaint(),
                TASKS => tasks.repaint(),
                _ => claude.repaint(),
            }
            continue;
        }
        match key.as_deref() {
            // deck's reader sends SIGWINCH when Claude's output arrives,
            // so a RESIZE with the size unchanged is for the Claude tab.
            Some("RESIZE") => {
                let now = Crust::terminal_size();
                let resized = now != size;
                size = now;
                match (tab, &mut claude) {
                    (CLAUDE, Claude::On(c)) => c.after_wait(key),
                    (CLAUDE, _) => claude.repaint(),
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
                    if tab == CLAUDE { c.after_wait(key) } else { unseen.run(|| c.after_wait(key)) }
                }
            }
            Some(_) => match tab {
                MAIL => mail.after_wait(key),
                CALENDAR => cal.after_wait(key),
                TASKS => if let Some(k) = key { tasks.key(&k) },
                _ => if let Some(c) = claude.app() { c.after_wait(key) },
            },
            // No key came: every tab gets its idle turn, the hidden ones
            // unseen. Claude's output wakes it by itself, so a hidden
            // Claude tab needs none.
            None => {
                if tab == MAIL { mail.after_wait(None) } else { unseen.run(|| mail.after_wait(None)) }
                if tab == CALENDAR { cal.after_wait(None) } else { unseen.run(|| cal.after_wait(None)) }
                if let (CLAUDE, Some(c)) = (tab, claude.app()) {
                    c.after_wait(None);
                }
            }
        }
    }

    if let Claude::On(c) = claude {
        let _ = std::fs::write(home.join(".gluon").join("session"), c.session_id());
        deck::app::finish(c);
    }
    tock::app::finish(cal);
    kastrup::app::finish(mail);
}
