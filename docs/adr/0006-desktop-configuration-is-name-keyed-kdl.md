# Desktop configuration is a name-keyed KDL file, edited surgically and persisted on change

The desktop configuration (projects, launchers, placements, startup profile) is
the only state that persists across sessions; everything else is runtime state
reconstructed per session. It lives in `~/.massive/desktop.kdl` keyed by name —
launchers get fresh UUIDs each startup, so the file cannot reference IDs.
Launcher names are unique among their project's siblings and project names
unique document-wide; the `startup` profile resolves a launcher by name across
all projects (first match wins, further duplicates are ignored with a warning).
A change that would introduce a duplicate name is renamed by appending ` 2`,
` 3`, ..., decided once when the change is created, so the live model and the
file cannot diverge. Every configuration change (`ProjectChange`) is mirrored as
a surgical edit to the parsed `KdlDocument` and written synchronously and
atomically, so hand-written comments and formatting survive every automated
write. We chose KDL over JSON because the file is human-maintained and
hand-edited (`kdl` is document-oriented like `toml_edit`), rejected automatic
JSON migration because no other users exist, and confirmed with a spike that
kdl nodes carry their comments across remove/insert moves, so node moves need
no extra format bookkeeping.