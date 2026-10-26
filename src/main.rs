//! gluon: mail, calendar and tasks in one terminal app. The Mail tab is
//! kastrup and the Calendar tab is tock, both run as libraries; the
//! Tasks tab shows ~/.tasks/todo.hl. Ctrl+O moves to the next tab.

use crust::{style, Crust, Input, Pane};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

const TABS: [&str; 3] = ["Mail", "Calendar", "Tasks"];
const MAIL: usize = 0;
const CALENDAR: usize = 1;
const TASKS: usize = 2;

/// The tab names for the first line, the one in view marked.
fn tab_line(active: usize) -> String {
    let mut line = String::from(" ");
    for (i, name) in TABS.iter().enumerate() {
        if i == active {
            line.push_str(&style::bold(&style::fg(name, 156)));
        } else {
            line.push_str(&style::fg(name, 245));
        }
        line.push_str(if i + 1 < TABS.len() { "  " } else { " " });
    }
    line
}

/// A tab out of view still does its idle work (new mail, scheduled
/// sends, alarms), with what it draws thrown away so it does not paint
/// over the tab in view. Two dup2 calls a turn; /dev/null stays open.
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
    fn new() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
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
        self.bottom.say(&style::fg(" j/k scroll  e edit  Ctrl+O next tab  q quit", 245));
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

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    if arg == "-h" || arg == "--help" {
        println!("gluon — mail, calendar and tasks in one terminal app (Fe2O3 suite)");
        println!();
        println!("Usage: gluon");
        println!();
        println!("  Mail      kastrup, with its own keys and ~/.kastrup/");
        println!("  Calendar  tock, with its own keys and ~/.tock/");
        println!("  Tasks     ~/.tasks/todo.hl as HyperList; e edits it");
        println!();
        println!("Ctrl+O moves to the next tab. q in any tab ends gluon.");
        return;
    }
    if arg == "-v" || arg == "--version" {
        println!("gluon {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // tock first: its store opens to one program at a time, and it ends
    // the program when another tock has it. Nothing of kastrup has
    // started by then.
    let Some(mut cal) = tock::app::start() else { return };
    let Some(mut mail) = kastrup::app::start() else {
        tock::app::finish(cal);
        return;
    };
    Crust::set_app_identity("gluon");
    mail.set_tab_line(Some(tab_line(MAIL)));
    cal.set_tab_line(Some(tab_line(CALENDAR)));
    let unseen = Unseen::new();
    let mut tasks = Tasks::new();
    let mut tab = MAIL;
    mail.repaint();

    while mail.running() && cal.running() && !tasks.quit {
        let wait = match tab {
            MAIL => mail.before_wait().min(unseen.run(|| cal.before_wait())),
            CALENDAR => cal.before_wait().min(unseen.run(|| mail.before_wait())),
            _ => unseen.run(|| mail.before_wait().min(cal.before_wait())),
        };
        let key = Input::getchr(Some(wait));
        if key.as_deref() == Some("C-O") {
            tab = (tab + 1) % TABS.len();
            match tab {
                MAIL => mail.repaint(),
                CALENDAR => cal.repaint(),
                _ => tasks.repaint(),
            }
            continue;
        }
        let idle = key.is_none();
        match tab {
            MAIL => mail.after_wait(key),
            CALENDAR => cal.after_wait(key),
            _ => if let Some(k) = key { tasks.key(&k) },
        }
        // The tabs out of view get their idle turn when no key came.
        if idle {
            if tab != MAIL { unseen.run(|| mail.after_wait(None)); }
            if tab != CALENDAR { unseen.run(|| cal.after_wait(None)); }
        }
    }

    tock::app::finish(cal);
    kastrup::app::finish(mail);
}
