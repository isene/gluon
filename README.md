# gluon

Mail, calendar and tasks in one terminal app, part of the
[Fe₂O₃ suite](https://isene.org/fe2o3/).

gluon runs [kastrup](https://github.com/isene/kastrup) and
[tock](https://github.com/isene/tock) as tabs, so every key you know from
them works the same here. A third tab shows your tasks.

| Tab | What it is | Files |
|---|---|---|
| Mail | kastrup: email, RSS and chat | `~/.kastrup/` |
| Calendar | tock: calendar with ephemeris | `~/.tock/` |
| Tasks | your task list, coloured as HyperList | `~/.tasks/todo.hl` |

**Ctrl+O** moves to the next tab. A tab out of view goes on fetching mail,
sending scheduled messages and ringing alarms.

In the Tasks tab, `j`/`k` scroll, `e` or Enter opens the list in your
editor (`$EDITOR`, or scribe when it is not set), and `q` ends gluon.

Run gluon instead of kastrup and tock, not beside them: tock's calendar
opens to one program at a time, and two kastrups would both poll the same
sources.

## Coming

- An agenda with list, week, month and board views, and dates read from
  messages and tasks.
- A pane beside any item for a conversation with Claude about it.

## Build

```bash
git clone https://github.com/isene/gluon
# beside it: crust, glow, highlight, mail, feed, ferrite, kastrup, tock
cargo build --release
```

## License

[Unlicense](https://unlicense.org/): public domain.
