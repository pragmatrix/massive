# Keyboard shortcuts

Which keys the massive desktop, the shell and the terminal client use, which ones are free,
and what comparable applications use them for. This list is for picking a key for a new
command without colliding with a habit users already have.

Status: 2026-10-15. WezTerm's defaults were checked against `wezterm/wezterm-gui/src/commands.rs`
and winit's menu against winit 0.30. The other applications are listed from their documented
defaults on macOS.

**Tags:** mac = macOS convention · Term = Terminal.app · iT = iTerm2 · Gh = Ghostty · Wz =
WezTerm · Kt = Kitty · VS = VS Code · Ob = Obsidian · Br = Safari and Chrome · Fi = Finder · MC =
Mission Control and macOS window tiling · Aero = AeroSpace

† marks a key that is hard to reach outside the US and UK layouts: its character needs Shift
or Option, or is a dead key, on common European layouts (see [Layouts](#layouts)). Every key in
use or planned today is reachable on all of them.

## In use

Desktop shortcuts are matched in `desktop/src/desktop_system/keyboard_shortcuts.rs` and
delivered top down; the level that consumes one decides its meaning (ADR 0020).

| Key | Owner | Action | Elsewhere |
|---|---|---|---|
| Enter | desktop (project, launcher) | Enter the focused project; start an instance from the focused launcher | — |
| Shift+Enter | desktop (launcher) | Start an assistant instance | — |
| Cmd+Enter | desktop (project, launcher, instance) | Zoom into the focused target; reaches the application when the instance is fully zoomed in | full screen (iT, Gh) · new window/split (Kt) · insert line below (VS) |
| Cmd+T | desktop (instance, launcher) | Start an instance next to the focused one | new tab (mac, all terminals, Br, Fi, Ob) |
| Cmd+Shift+T | desktop (instance, launcher) | Start an assistant instance | reopen closed tab (Br, VS, Ob) |
| Cmd+W | desktop (instance) | Close the instance | close tab or window (mac, everywhere) |
| Cmd+Shift+W | desktop (launcher, project) | Remove the focused launcher or nested project, but only if no instance runs anywhere below it | close window (Br, VS, terminals) |
| Cmd+N | desktop | Add a launcher right of the focused slot and focus it | new window (mac, all terminals, Br, Fi) · new note (Ob) |
| Cmd+G | desktop | Wrap the selected slots, or else the focused slot, in a new project at their place and focus the project | find next (mac, Term, Br, VS, Gh) · group (Fg, Sketch, Illustrator) · graph view (Ob) |
| Cmd+Shift+G | desktop (project) | Move the slots of the focused project, or of the project whose slots are all selected, into its parent's matrix, remove it, and select the ungrouped slots | find previous (mac, Br, VS, Gh) · ungroup (Fg, Sketch, Illustrator) |
| Cmd+←/→/↑/↓ | desktop | Navigate between targets; repeats when held | line start/end, document top/bottom (mac text) · previous/next tab (iT) · back/forward (Safari) · previous/next prompt (Gh ↑/↓, Term marks) · enclosing folder/open (Fi ↑/↓) |
| Cmd+Shift+←/→/↑/↓ | desktop | Extend the slot selection by one placement; repeats when held | select to line start/end, document top/bottom (mac text) |
| Cmd+A | desktop (project) | Select all slots of the project whose matrix holds the focused launcher or project; inside an instance, the application receives it | select all (mac, everywhere) |
| Esc | desktop (launcher, project) | Collapse an extended slot selection; otherwise leave the focused launcher or project by one level. In an instance, the application receives it | deselect (Fg, Kn, Sketch) · leave a group (Sketch, Kn, Illustrator) · collapse multiple cursors (VS, Zd) |
| Cmd+Esc | desktop | Like Esc; in an instance, leave it to its launcher | — |
| Cmd+Ctrl+←/→ | desktop | Navigate | resize split (Gh) · move editor to the neighboring group (VS) |
| Cmd+Ctrl+↑ | desktop | Zoom in | resize split (Gh) |
| Cmd+Ctrl+↓ | desktop | Zoom out | resize split (Gh) |
| Cmd+Ctrl+F | shell (View menu) | Toggle full screen | full screen (mac) |
| Cmd+Q | shell (winit app menu) | Quit | quit (mac) |
| Cmd+H | shell (winit app menu) | Hide the app | hide (mac) |
| Cmd+Opt+H | shell (winit app menu) | Hide others | hide others (mac) |
| Cmd+C | terminal client | Copy the selection | copy (mac) |
| Cmd+V | terminal client | Paste | paste (mac) |
| Cmd+click | terminal client | Open a hyperlink; links are highlighted while Cmd is held | open link (iT, Gh, VS; Wz and Kt use a plain click) · open in new tab (Br, Ob) |

Inside an instance, Cmd+Shift+W is not a desktop shortcut; it passes to the application. Only a
focused launcher or project consumes it. The terminal client drops
every Cmd chord it doesn't bind, except for applications that enabled the Kitty keyboard protocol,
which receive it.

## Planned

From the terminal client's find proposal, not implemented:

| Key | Action | Elsewhere |
|---|---|---|
| Cmd+F | Open find, or select the query | find (everywhere) |
| Enter / Shift+Enter in the find field | Older / newer match | find next / previous in the field (VS, Br); Cmd+G and Cmd+Shift+G, used for this elsewhere, group and ungroup on the desktop |
| Cmd+E | Search for the selection | use selection for find (mac, VS) · toggle edit/preview (Ob) · eject (Fi) |
| Cmd+Opt+C / Cmd+Opt+R | Toggle case and regex in the find field | case and regex toggles in find (VS) |

## Free: Cmd + letter

| Key | Usually |
|---|---|
| Cmd+B | bold (mac text, Ob) · toggle sidebar (VS) · bookmarks (Br) |
| Cmd+D | split pane (Term, iT, Gh) · bookmark (Br) · add next occurrence (VS) · duplicate (Fi) |
| Cmd+I | get info (Fi) · italic (Ob) · inspector (Term) · inline chat (VS) |
| Cmd+J | toggle panel (VS) · view options (Fi) · downloads (Chrome) |
| Cmd+K | clear scrollback (Term, iT, Gh, Wz, Kt) · insert link (Ob) · chord prefix (VS) · connect to server (Fi) |
| Cmd+L | address bar (Br) · select line (VS) |
| Cmd+M | minimize (mac, Wz); the shell has no Window menu, so nothing minimizes today |
| Cmd+O | open (mac, Fi) · quick switcher (Ob) |
| Cmd+P | print (mac) · quick open (VS) · command palette (Ob) |
| Cmd+R | reload (Br) · reload configuration (Wz) · show original (Fi) |
| Cmd+S | save (mac) |
| Cmd+U | view source (Chrome) · undo cursor (VS) |
| Cmd+X | cut (mac) |
| Cmd+Y | history (Br) · Quick Look (Fi) |
| Cmd+Z | undo (mac) |

## Free: Cmd + digit and punctuation

| Key | Usually |
|---|---|
| Cmd+1 … Cmd+8 † | go to tab N (terminals, Br, Ob) · focus editor group N (VS) · view mode (Fi, 1–4) |
| Cmd+9 † | last tab (Br, Wz) |
| Cmd+0 † | reset font size or zoom (Wz, Kt, Gh, Br) · focus sidebar (VS) |
| Cmd+= † / Cmd+- | bigger / smaller font or zoom (mac, terminals, Br, VS) |
| Cmd+, | settings (mac, Gh, Kt, VS, Ob) |
| Cmd+. † | cancel (mac dialogs) · quick fix (VS) |
| Cmd+/ † | toggle comment (VS) · find cursor (iT) |
| Cmd+[ / Cmd+] † | back / forward (Br, Fi) · previous / next split (iT, Gh) · outdent / indent (VS) |
| Cmd+\ † | split editor (VS) |
| Cmd+; † | autocomplete (iT) |
| Cmd+` † | cycle the app's windows (mac, provided by a Window menu) |

## Free: common variants

| Key | Usually |
|---|---|
| Cmd+Shift+[ / ] † | previous / next tab (mac, terminals, Br, VS) |
| Cmd+Shift+D | split down (iT, Gh) |
| Cmd+Shift+Enter | zoom split (iT, Gh) · insert line above (VS) |
| Cmd+Shift+F | search everything (VS, Ob) |
| Cmd+Shift+N | new folder (Fi) · private window (Br) |
| Cmd+Shift+P | command palette (VS, Gh) |
| Cmd+Shift+Z | redo (mac) |
| Cmd+Shift+. † | show hidden files (Fi) |
| Cmd+Opt+←/→ | previous / next tab (Chrome, VS) · back / forward (Ob) |
| Cmd+Opt+arrows | go to split (iT, Gh) |
| Cmd+Opt+1 … 9 † | go to window N (iT) |
| Cmd+Opt+I | developer tools (Br) · inspector (Gh) |

## Selection

How comparable applications extend a selection and select several things at once. The desktop
selects only a rectangle of slots, with Cmd+Shift+arrows (ADR 0025) or all of a project's slots
with Cmd+A (ADR 0030); this is the vocabulary users bring. Checked
2026-10-19: WezTerm against `wezterm-gui/src/inputmap.rs`, Kitty against
`kitty/options/definition.py`, Ghostty against `src/surface_mouse.zig`, Zed against
`assets/keymaps/default-macos.json`; the rest from documented defaults.

Extra tags for this section: Sb = Sublime Text · JB = JetBrains IDEs · Xc = Xcode · Zd = Zed ·
Fg = Figma · Kn = Keynote, Pages and Freeform · text = macOS text views (TextEdit, Notes, every
`NSTextView`)

### Extending a text selection

The same everywhere text is edited, terminal emulators excepted, which have no keyboard selection
outside a copy or vi mode.

| Key | Extends by |
|---|---|
| Shift+←/→ / Shift+↑/↓ | character / line |
| Shift+Opt+←/→ | word |
| Shift+Opt+↑/↓ | paragraph (text); VS and JB instead move the line or copy it |
| Shift+Cmd+←/→ | to line start / end |
| Shift+Cmd+↑/↓ | to document start / end |
| Shift+click | to the clicked position (text, editors, all terminals; Kt also right click) |
| double / triple click | word / line or paragraph (everywhere, all terminals) |
| Cmd+L | select the line, repeat to extend (VS, Sb, Zd) · address bar (Br) |

### Expanding by structure

Grow the selection to the enclosing word, expression, block, and back.

| Tool | Expand | Shrink |
|---|---|---|
| VS | Ctrl+Shift+Cmd+→ | Ctrl+Shift+Cmd+← |
| Zd | Cmd+Ctrl+→, or Ctrl+Shift+→ | Cmd+Ctrl+←, or Ctrl+Shift+← |
| JB | Opt+↑ | Opt+↓ |
| Sb | Cmd+Shift+Space (scope); Ctrl+Shift+M (brackets), Cmd+Shift+A (tag), Cmd+Shift+J (indentation) | soft undo (Cmd+U) |
| Fg | Shift+Enter (select the parent) | Enter (select the children) |
| Xc | no default | — |

### Several selections in text

| Action | VS | Zd | Sb | JB | Xc |
|---|---|---|---|---|---|
| Add the next occurrence | Cmd+D | Cmd+D | Cmd+D | Ctrl+G | Opt+Cmd+E (from a blog, not Apple's list) |
| Skip it, take the next | Cmd+K Cmd+D | Cmd+K Cmd+D | Cmd+K Cmd+D | Ctrl+Shift+G, then Ctrl+G | — |
| Select all occurrences | Cmd+Shift+L, Cmd+F2 | Cmd+Shift+L, Cmd+F2 | Ctrl+Cmd+G | Ctrl+Cmd+G | Ctrl+Cmd+E (edit all in scope) |
| Add a cursor above / below | Opt+Cmd+↑/↓ | Opt+Cmd+↑/↓ | Ctrl+Shift+↑/↓ | Opt Opt (held) +↑/↓ | Ctrl+Shift+↑/↓ |
| Add a cursor by clicking | Opt+click | Opt+click (`multi_cursor_modifier`) | Cmd+click | Opt+Shift+click | Ctrl+Shift+click |
| One cursor per selected line | Shift+Opt+I | — | Cmd+Shift+L | Opt+Shift+G | — |

macOS text views have their own, older form: Cmd+drag or Cmd+double-click adds a separate range
to the selection.

### Rectangle selection

| Tool | Gesture |
|---|---|
| Term, Gh, Wz, text, Sb, JB, Xc | Opt+drag |
| iT | Cmd+Opt+drag |
| Kt | Ctrl+Opt+drag |
| VS | Shift+Opt+drag; Shift+Opt+Cmd+arrows from the keyboard |

Wz extends a rectangle with Shift+Opt+click; JB also toggles a keyboard column mode with
Cmd+Shift+8. Gh used Cmd+Opt+drag, like iT, before it moved to Opt+drag to match Term and Wz.

### Several objects

| Context | Range | Toggle one | Marquee | All |
|---|---|---|---|---|
| Lists and tables (Fi list and column view, Mail, `NSTableView`) | Shift+click, Shift+↑/↓ | Cmd+click | — | Cmd+A |
| Icon grids (Fi icon view, Photos) | — | Shift+click or Cmd+click | drag; Cmd+drag toggles what it covers | Cmd+A |
| Kn | — | Shift+click or Cmd+click | drag | Cmd+A |
| Fg | — | Shift+click; Cmd+click selects inside a group instead | drag; Shift+drag adds | Cmd+A; Opt+Cmd+A matching layers |
| Tabs (Br, VS, Ob) | Shift+click | Cmd+click | — | — |

### Where tools disagree

- **Shift+click** selects a range in lists and text but toggles a single object in icon grids and
  canvases (Fi icon view, Kn, Fg). Which meaning applies depends on whether the items have an
  order the user reads as a line.
- **Cmd+click** toggles an item in lists, but opens a link in the terminal client, terminals and
  browsers, adds a cursor in Sb, and selects inside a group in Fg. Inside an instance it already
  opens hyperlinks; toggling a target would have to happen on the desktop level, where no link
  competes.
- **Adding a cursor** has no common key: Opt+click (VS, Zd), Cmd+click (Sb), Opt+Shift+click
  (JB), Ctrl+Shift+click (Xc). Cmd+D for the next occurrence is the one binding VS, Zd and Sb
  share; JB uses Ctrl+G.
- **Expanding by structure** differs in every editor. Zd's Cmd+Ctrl+←/→ collides with the
  desktop's Cmd+Ctrl+arrows (navigate); VS's Ctrl+Shift+Cmd+→ adds Shift to the same keys. JB's
  Opt+↑/↓ takes paragraph movement from macOS text.
- **Rectangle selection** is Opt+drag in most tools, with iT (Cmd+Opt) and Kt (Ctrl+Opt) the
  outliers.

## Layouts

Where the † keys sit on common Mac layouts other than US and UK. Letters move too (QWERTZ swaps
Y and Z; AZERTY swaps A/Q and Z/W) but stay one key press away.

| Key | Where it is |
|---|---|
| 1 … 0 | French and Belgian (AZERTY) only: Shift + the top row; the unshifted keys type `& é " ' ( § è ! ç à`. macOS layouts likely carry a Cmd layer that reports Cmd+`ç` as Cmd+9, which is why Cmd+digit works there in other apps (not verified). The desktop matches shortcuts by the character winit reports, so whether a Cmd+digit binding works on AZERTY depends on whether winit reports `9` or `ç` with Cmd held. Untested; if it reports `ç`, match digits by physical key |
| = | German, Spanish, Italian, Nordic: Shift+0 |
| . | French: Shift+; |
| / | German, Spanish, Italian, Nordic: Shift+7; French: Shift+: |
| ; | German, Spanish, Italian, Nordic: Shift+, |
| [ ] | German: Opt+5 / Opt+6; Nordic: Opt+8 / Opt+9; French, Spanish, Italian, Swiss: other Option combinations |
| \ | German, Nordic: Opt+Shift+7; most other layouts: an Option combination |
| ` | a dead key on German, French, Spanish, Italian and Nordic layouts |

## Taken by the system

These never reach the application, or reach it only when the user disabled them. A new binding should avoid
them.

| Key | System use |
|---|---|
| Cmd+Space | Spotlight, or the launcher that replaces it (Raycast, Alfred) |
| Cmd+Opt+Space | Finder search window |
| Ctrl+Space / Ctrl+Opt+Space | previous / next input source, with more than one enabled; it then never reaches a tmux prefix or Emacs's set-mark |
| Cmd+Tab | app switcher |
| Cmd+Shift+3 / 4 / 5 | screenshots |
| Cmd+Ctrl+Q | lock screen |
| Cmd+Ctrl+Space | character viewer |
| Cmd+Opt+Esc | force quit |
| Cmd+Opt+D | toggle Dock hiding |
| Ctrl+←/→ | previous / next Space (MC) |
| Ctrl+↑ / Ctrl+↓ | Mission Control / App Exposé (MC) |
| Ctrl+1 … 9 | switch to Space N (MC, off by default) |
| Globe+Ctrl+arrows, Globe+Ctrl+F | tile the window, fill the screen (MC, macOS 15) |
| Opt+H/J/K/L, Opt+1 … 9, Opt+Shift+… | focus, workspaces, moves (Aero, when installed) |

Stage Manager has no default shortcuts. AeroSpace's Opt bindings take keys that otherwise
compose characters (Opt+C types `ç`), so its users won't type those in the terminal client either.

## Collisions with habits

- **Cmd+Enter** zooms on the desktop, but means full screen in iTerm2 and Ghostty. Users coming from those
  terminals will expect full screen; Cmd+Ctrl+F covers it.
- **Cmd+arrows** navigate the desktop, so shells don't get the line start/end that Ghostty
  sends, and Cmd+↑/↓ can't jump between prompts as in Ghostty and Terminal.app.
- **Cmd+N** opens a new window in every comparable application. On the desktop, the closest
  meaning is a new launcher next to the focused slot.
- **Cmd+1 … 9, Cmd+K, Cmd+, and Cmd+=/-** are free, yet nearly every comparable application
  binds them the same way. A user will try them first; they are the strongest candidates for go
  to instance N, clearing scrollback, settings and font size.
  Cmd+= needs Shift on most European layouts, where other apps accept Cmd++ instead; Cmd+1 … 9
  may need care on AZERTY (†, see [Layouts](#layouts)).
- **Cmd+D** splits a pane in three of the terminals. On the desktop, the closest meaning is
  starting an instance next to the focused one, which Cmd+T already does.
- **Cmd+M** is free only because the shell has no Window menu; macOS users expect it to minimize.
  For a single-window app it adds little over Cmd+H, which hides the whole app and comes back
  with Cmd+Tab, while a minimized window does not. Keyboard users mostly hide; Cmd+M is often hit
  by accident next to Cmd+N. (An impression, not measured.)
