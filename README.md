# gluon

Mail, calendar, tasks and Claude in one terminal app, part of the
[Fe₂O₃ suite](https://isene.org/fe2o3/).

gluon runs [kastrup](https://github.com/isene/kastrup),
[tock](https://github.com/isene/tock) and deck (a front for Claude Code)
as tabs, so every key you know from them works the same here. One more
tab shows your tasks.

| Tab | What it is | Files |
|---|---|---|
| Mail | kastrup: email, RSS and chat | `~/.kastrup/` |
| Calendar | tock: calendar with ephemeris | `~/.tock/` |
| Tasks | your task list, coloured as HyperList | `~/.tasks/todo.hl` |
| Claude | one Claude Code session that stays, told where the rest is | `~/.gluon/` |

**Ctrl+O** moves to the next tab. **Ctrl+A** jumps to Claude and back;
Claude gets what the tab you came from had on screen (the message, the
week, the task list) with your next prompt. A tab out of view goes on
fetching mail, sending scheduled messages, ringing alarms and taking
Claude's answer, which shows a box when it is done.

Claude Code starts the first time you open its tab, since an idle one
takes more power than the rest of gluon. The session's id stays in
`~/.gluon/session`, so the next start picks up the same conversation, and
`~/.gluon/CLAUDE.md` tells it where your mail, calendar and tasks are.
In that tab, C-D ends gluon, and C-T shows the tool calls in full.

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
