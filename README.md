# gluon

Mail, calendar, tasks and Claude in one terminal app, part of the
[Fe₂O₃ suite](https://isene.org/fe2o3/).

gluon runs [kastrup](https://github.com/isene/kastrup),
[tock](https://github.com/isene/tock),
[scribe](https://github.com/isene/scribe) and
[deck](https://github.com/isene/deck) (a front for Claude Code) as
parts of one screen, so every key you know from them works the same here.

| Tab | What it is | Files |
|---|---|---|
| Home | unread mail, the agenda and the tasks, over a Claude pane | `~/.gluon/` |
| Mail | kastrup: email, RSS and chat | `~/.kastrup/` |
| Calendar | tock: calendar with ephemeris | `~/.tock/` |
| Tasks | scribe on your task list, or the list as a board | `~/.tasks/todo.hl` |

Home has four parts, three boxes over Claude:
- top left, the unread mail in kastrup's views 1, 2 and 3, newest first,
  each with its view's key in the view's colour (other views: put their keys in
  `~/.gluon/views`, as `1 2 4`);
- top right, what is left of today and the coming 30 days in tock, with a
  line between one day and the next;
- under it, your task list;
- the bottom, one Claude Code session that stays, told where your mail,
  calendar and tasks are. **Ctrl+Z** gives it the whole screen and back.

**Tab** moves the keys from box to box and on to Claude (**Shift+Tab**
goes back, **Esc** jumps to Claude). In a box, the arrows pick a row
(Down on the last row goes round to the first, and Up the other way),
the box's keys show in its bottom edge, and `?` shows all the keys
at Home:
- Mail: the picked message shows on the right, in place of the agenda
  and the tasks, and stays unread; Space scrolls it and, at its end, goes
  on to the next message. Shift+Left scrolls back, and Shift+Down and
  Shift+Up scroll a line.
  Enter opens it in the Mail tab. A message you browse gets an R mark,
  which R takes off or puts back; A marks them all, and d marks one for
  deletion. Marked messages stay in the box until `<` marks the R ones
  read in kastrup and deletes the D ones; the box's title counts both
  marks. i lists the oldest first, and i again the newest, as in
  kastrup. y copies `kastrup:<id>`, and Ctrl+Y the whole message.
  kastrup's keys for one message work too (r replies, f
  forwards, s files, h snoozes, v lists the attachments, * stars, and so
  on), and Home comes back when they are done; I, c and V (the images)
  stay in the Mail tab to show their answer, and Esc comes back. Each row has the colour and icon of its kind,
  as in kastrup. While a mail you send is on its way, the box's title
  says "↑ Sending to …"; then the bar says "Sent to …", or why it
  failed, until the next key. A message you replied to gets kastrup's ←
  (→ forwarded, ↔ both).
- Agenda: Enter opens the event in the Calendar tab, and y copies it as
  `tock:<id>` with its title and time. Ctrl+Y copies all of the event.
  A green ▶ marks an event with a Teams, Zoom or Meet link, and J joins
  it, as J does in tock.
- Tasks: Enter opens the task list in scribe at that line, and x ticks
  the task done (`[x]`) or back. d marks a line for deletion, with the
  lines under it, and `<` deletes the marked lines. y copies the line as
  `todo:<n>` with its text, and Ctrl+Y copies the task with the lines
  under it.

t tags the picked row in any box, on a dark grey, and t again takes the
tag off. When the keys go from a box to Claude (Esc or Tab), Claude gets
every tagged row of all three boxes with your next prompt, or else the row
picked in that box: two messages, an event and three tasks, say, to talk
about together. The tags go with them.

**Ctrl+S** at Home asks Claude, in the pane below, what to do about the
unread mail, the agenda and the tasks, which go with the question. Claude
numbers at most 8 ideas (tasks to add, events for the calendar, answers to
messages, follow-ups you may miss), ends with one short reflection on the
days ahead, and has the keys, so you answer "do 1 and 3" or talk them over.

A task goes in todo.hl, an event on the calendar, and an answer in
kastrup's drafts, which `+` opens; nothing is sent without you. Each press
adds the unread mail, the agenda and the task list to the session, with
the mail you sent 5 to 30 days ago that nothing has answered.

To look wider, write the steps in `~/.gluon/sweep`, in your own words, and
each Ctrl+S adds them to the question: a search of your archive for what
waits on you, say. Without the file, nothing else is asked.

**Ctrl+Shift+S** turns timed ideas on and off, from any tab. While on,
gluon asks the Ctrl+S question by itself at 09:00 and 15:00, and an orange
`◷ ideas 09:00 15:00 21:00` on every tab's top line says so. A time from
18:00 on looks back instead: what got done, what moves to tomorrow, and
what you can let go. The times are in `~/.gluon/auto`.

A time that passed while the laptop slept is asked once on waking, the
same day. The answer waits in the pane below Home, and the keys stay where
they were; a desktop notice says it came while gluon was out of sight.
gluon wakes for it at those times alone.

`+` in any box opens kastrup's list of drafts, postponed and scheduled
messages, to send one or start a new one; Home comes back when you are
done.

Esc in the tab a box opened brings you back Home; in scribe, Esc is
scribe's, and Ctrl+A goes Home.

A digit at Home opens the Mail tab on that kastrup view, and Ctrl+digit
does the same from anywhere, even while Claude or scribe has the keys.
Ctrl+Shift+C opens the Calendar and Ctrl+Shift+T the Tasks, also from
anywhere. Ctrl+digit and Ctrl+Shift need a terminal with the kitty keyboard
protocol, such as glass or kitty; deck turns it on.

**Ctrl+O** moves to the next tab. **Ctrl+A** jumps Home and back, to the
box that had the keys; Claude gets what the tab you came from had on screen (the message, the week, the
task list) with your next prompt. A tab out of view goes on fetching
mail, sending scheduled messages, ringing alarms and taking Claude's
answer, which shows a box when it is done.

Home draws again only when something changed: the mail database, the
minute, or the task file.

Claude Code starts with gluon; an idle one takes about 33 mW, more than
the rest of gluon. The session's id stays in
`~/.gluon/session`, so the next start picks up the same conversation, and
`~/.gluon/CLAUDE.md` tells it where your mail, calendar and tasks are.

`Q` ends gluon from any tab, except while Claude or scribe has the keys;
in Claude, C-D ends it. Ctrl+L draws Home and Claude again. kastrup and tock no longer quit on `q` inside gluon. At
Home, C-T shows the tool calls in full.

With [cc-sessions](https://github.com/isene/CC-sessions), gluon bookmarks
the session as #gluon, so `cc gluon` and fleet find it.

The Tasks tab is scribe on the task list, with all of scribe's keys but
Ctrl+O, Ctrl+A and Ctrl+B, which stay gluon's. Unsaved text is written
when the tab goes out of view, and `:q` goes Home.

Ctrl+B shows the list as a board, and Ctrl+B again goes back to scribe. The lines at the
top level are the lanes (Inbox, Work, ...), and the tasks under them are
cards in four columns. The box in front of a task in todo.hl says its
column:

| Column | Box |
|---|---|
| To do | none, or `[_]` |
| Doing | `[O]` |
| Waiting | `[-]` |
| Done | `[x]` |

The arrows pick a card. Shift+Left and Shift+Right (or H and L) move it a
column, which rewrites its box. Shift+Up and Shift+Down (or K and J) move
it to the end of the lane above or below, with the lines under it. `e`
opens scribe at the picked card.

Run gluon instead of kastrup and tock, not beside them: tock's calendar
opens to one program at a time, and two kastrups would both poll the same
sources.

## Coming

- An agenda with list, week, month and board views, and dates read from
  messages and tasks.

## Build

```bash
git clone https://github.com/isene/gluon
# beside it: crust, glow, highlight, mail, feed, ferrite, kastrup, tock, scribe, deck
cargo build --release
```

## License

[Unlicense](https://unlicense.org/): public domain.
