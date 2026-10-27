# gluon

Mail, calendar, tasks and Claude in one terminal app, part of the
[Fe₂O₃ suite](https://isene.org/fe2o3/).

gluon runs [kastrup](https://github.com/isene/kastrup),
[tock](https://github.com/isene/tock) and
[deck](https://github.com/isene/deck) (a front for Claude Code) as
parts of one screen, so every key you know from them works the same here.

| Tab | What it is | Files |
|---|---|---|
| Home | unread mail, the agenda and the tasks, over a Claude pane | `~/.gluon/` |
| Mail | kastrup: email, RSS and chat | `~/.kastrup/` |
| Calendar | tock: calendar with ephemeris | `~/.tock/` |
| Tasks | your task list, coloured as HyperList | `~/.tasks/todo.hl` |

Home has four parts:
- top left, the unread mail in kastrup's views 1, 2 and 3, newest first,
  each in its view's colour (other views: put their keys in
  `~/.gluon/views`, as `1 2 4`);
- top right, what is left of today and the coming 30 days in tock;
- under it, the top of your task list;
- the bottom half, one Claude Code session that stays, told where your
  mail, calendar and tasks are. **Ctrl+Z** gives it the whole screen and
  back.

**Ctrl+O** moves to the next tab. **Ctrl+A** jumps Home and back; Claude
gets what the tab you came from had on screen (the message, the week, the
task list) with your next prompt. A tab out of view goes on fetching
mail, sending scheduled messages, ringing alarms and taking Claude's
answer, which shows a box when it is done.

Home draws again only when something changed: the mail database, the
minute, or the task file.

Claude Code starts at the first letter you type at Home, since an idle
one takes more power than the rest of gluon. The session's id stays in
`~/.gluon/session`, so the next start picks up the same conversation, and
`~/.gluon/CLAUDE.md` tells it where your mail, calendar and tasks are.
At Home, C-D ends gluon, and C-T shows the tool calls in full.

With [cc-sessions](https://github.com/isene/CC-sessions), gluon bookmarks
the session as #gluon, so `cc gluon` and fleet find it.

In the Tasks tab, `j`/`k` scroll, `e` or Enter opens the list in your
editor (`$EDITOR`, or scribe when it is not set), and `q` ends gluon.

Run gluon instead of kastrup and tock, not beside them: tock's calendar
opens to one program at a time, and two kastrups would both poll the same
sources.

## Coming

- An agenda with list, week, month and board views, and dates read from
  messages and tasks.

## Build

```bash
git clone https://github.com/isene/gluon
# beside it: crust, glow, highlight, mail, feed, ferrite, kastrup, tock, deck
cargo build --release
```

## License

[Unlicense](https://unlicense.org/): public domain.
