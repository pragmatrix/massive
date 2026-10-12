# Out-of-process applications plan

Status: **draft, not approved**. Decisions were taken in a session discussion
(2026-10-11); deferred decisions are listed under "Open questions and follow-ups".

## Goal

Let massive applications run in their own OS processes and still meet the
in-process performance bar. The motivation is:

- **Crash and hang isolation.** A panicking or stuck application (e.g. `mt` under an
  output burst) must not take the desktop down or stall its frames.
- **Independent lifecycles.** Applications are built, versioned, and launched
  separately from the desktop. An application binary built against an older
  protocol keeps working against newer desktops without a recompile.
- **Dev loop.** An application can be restarted, or started with `cargo run`, into a
  running desktop.

Not goals: non-Rust clients, sandboxing of untrusted applications, Windows, and wasm.

In-process instances stay supported. Both modes sit behind one boundary, and
application code is the same in either mode.

## Performance bar

Measured with the `metrics` feature, `mt` running out of process under a sustained
output burst (e.g. `cat` of a large file) at display refresh rate:

- no added frame of latency between `ApplyAnimations(tick)` and the submission
  appearing on screen, compared with in-process;
- desktop frame time unaffected while the application is saturated;
- input→echo latency within ~1 ms of in-process;
- the above holds with 10 concurrent `mt` instances.

## Current situation

- Instances already run on dedicated threads (ADR 0009) and talk to the desktop
  only through messages. They submit a `ChangeSet<InstanceChange>` (scene changes
  interleaved with view and configuration changes, ADR 0008) plus `RenderPacing`
  through `InstanceEnvironment::submission_sender`
  (`applications/src/instance_environment.rs`). They receive a coalesced
  `ApplicationMessage` stream: view events, `ApplyAnimations(PresentationTick)`,
  fullscreen, and shutdown (`applications/src/application_event.rs`).
- Applications are registered in-process as closures:
  `Application { name, runtime_kind, run: Fn(InstanceContext) -> Future }`
  (`desktop/src/application_registry.rs`). Each instance gets a desktop-owned
  `view_parent: Ref<Location>` that its views hang under.
- The payloads are not wire-ready:
  - `Visual` shares shapes and glyph runs through `Arc`.
  - `Shape::Custom` is a type-erased `SmallBox<dyn CustomShape>`. No application
    produces it, and the renderer skips it (`renderer/src/shape_renderer.rs`).
  - `GlyphRun` glyphs carry `FaceId(u64)`, which is local to the process: a
    sequential number over the producing engine's font database, resolved through
    that process's `FontRegistry` at rasterization time.
  - Scene `Id`s are dense `u32`s from one global allocator per process
    (`TypeIdGenerator`) and serve as renderer row indices, so two processes would
    hand out colliding ids.
  - `ViewEvent` carries winit types (`DeviceId`, `KeyEvent`, `Key`).
- Shaping and animation run inside the instance (`TaskContext`, `ShapingContext`),
  driven by desktop vblank ticks.
- `ViewChange::SetCursor` carries a winit `CursorIcon` (`applications/src/view.rs`), so
  outbound view changes contain winit types too.

## Feasibility (measured 2026-10-11)

Feasibility study: can `mt` run remotely without dropping performance? The result:
the process boundary fits the performance bar. The blockers are missing
infrastructure, not transport cost.

Setup: Apple M4, two real processes over a Unix socketpair, release build. The test
frame is a full 200×60 terminal screen with every line new, which is what a `cat` burst
produces. `mt` keeps one `Visual` per line and scrolls through transforms, but during a
burst every visible line gets new content each frame. Today's `RunGlyph` is 40 bytes
of plain integers, so the frame holds 394 KB of glyph data.

| Moving one 394 KB frame | Time | Throughput |
|---|---|---|
| `memcpy` | 9.4 µs | ~42 GB/s |
| Socket, macOS default 8 KB buffers | 178 µs | 2.2 GB/s |
| Socket, 1 MB `SO_SNDBUF`/`SO_RCVBUF` | 19.7 µs | 20 GB/s |

| Encoding the same frame with postcard | Encode | Decode |
|---|---|---|
| Per-glyph keys (394 KB encoded) | 326 µs | 178 µs |
| Run-level keys (46 KB encoded) | 47 µs | 17 µs |

A small message round trip (tick, keystroke) takes 6.7 µs at p50 and 8.4 µs at p99.

Consequences:

- **Tick → submission and input → echo add tens of µs.** Both are well within the
  bar. `mt` submits on every pty notification, not only per vblank, and that rate is
  fine at these message costs.
- **Every connection sets 1 MB socket buffers.** macOS defaults Unix stream sockets
  to 8 KB (`net.local.stream.sendspace`), which splits a frame into ~50 small
  transfers, each a syscall plus a wakeup. With 1 MB buffers the socket runs at
  about 2× a memcpy (the price of copying twice through the kernel).
- **Bulk glyph arrays go on the wire as fixed-layout bytes.** Per-field serde
  encoding, not the socket, dominated the naive numbers. `scene::v1` defines a
  `#[repr(C)]` glyph wire struct and copies whole slices, so encoding, transfer, and
  decoding cost about 10 + 20 + 10 µs per full screen. That is in the same range as
  run-level keys, with no clever encoding. Postcard stays for small, structured
  messages.
- **Font ids are translated per glyph on the desktop.** It is a loop over the
  decoded slice with one table lookup per glyph (Q9), estimated at 10–20 µs per full
  screen; not measured.
- **The bandwidth fits.** 10 bursting terminals move about 4 MB per frame, about
  470 MB/s at 120 Hz, well within an M4's memory bandwidth. The desktop's share stays
  under ~0.5 ms per frame.
- **Run-level glyph keys are an optional size optimization, not a requirement.** A
  running glyph state with per-glyph change flags (face, size, weight, clip, y)
  would cut bytes about 10×, losslessly, including fallback faces, wide cells, and
  ligature clip widening. Revisit only if bandwidth or instance count shows up at the
  performance gate.
- **The expensive work stays in the application process.** Shaping and terminal
  parsing never cross the boundary.
- **Thread priority is a latency risk the benchmark cannot show.**
  `interactive-thread-priority` only raises threads inside the desktop process. Remote
  application threads need the same treatment; otherwise macOS may schedule them on
  efficiency cores under load, at a cost far above the socket's.
- **Blockers for a remote `mt`:**
  - winit types in `ViewEvent` and in `ViewChange::SetCursor`;
  - `FaceId` is local to the process, while `mt` uses system font fallback;
  - the `mt` binary is the desktop (`shell::run` → `DesktopEnvironment::run_desktop`
    in `src/main.rs`), and there is no client entry point;
  - not verified: `arboard` (clipboard) and `opener` (hyperlinks) are expected to
    work unchanged from `mt`'s own process, because they only need the user session.

## Decisions (taken)

### Boundary and process model

- **The boundary is the scene protocol.** Applications send scene changes and the
  desktop keeps rendering. Text stays crisp under camera zoom and 3D surfaces
  (ADR 0003, 0004), and applications need no GPU device. Shared GPU surfaces are a
  possible later extension, not part of this plan.
- **One process per application.** Instances stay threads inside it, as in ADR 0009.
  The font database and shaping caches are shared across an application's
  instances. A crash takes down all instances of that application, but nothing else.
- **The code boundary is a new crate, `massive-remote`, with two halves:**
  - `client`, used in the application process, builds a real `InstanceContext`
    whose submissions and events flow over the connection, so application code such
    as `mt` is unchanged;
  - `host`, used in the desktop, registers a remote application as an ordinary
    `Application` whose `run` future proxies one instance over the connection. The
    desktop core is unaware of remoteness.

### Payloads

- **Fonts are content-addressed.** The client announces each face once, lazily, the
  first time a run uses it: font file path + face index + content hash, or a
  shared-memory blob for in-memory fonts. The host loads the face into the
  desktop's registry and maps the client's `FaceId` to its own. Because the
  announcement comes earlier in the same ordered stream, the host has resolved the
  face before any glyph that references it.
- **Scene ids are remapped per connection.** The host keeps a dense
  `remote Id → desktop Id` table per connection and translates on receive. The
  renderer's dense-row invariant is unaffected.
- **`Shape::Custom` is not part of the wire.** Submitting one on a remote
  connection is rejected at the client (fail-fast), not skipped.
- **`ViewEvent` becomes winit-free.** It is a prerequisite refactor of the
  in-process API, not a wire-only mirror. The in-process event types themselves
  become serializable domain types. `DeviceId` maps to a small desktop-assigned
  integer.
- **Animations stay app-driven.** The tick → interpolate → submit round trip
  crosses the process boundary once per vblank per application. A late submission
  lands in the next frame, as it does today with a slow in-process instance.
  Desktop-side interpolation is a fallback, decided only at the performance gate.

### Transport

- **A Unix-domain socket with 1 MB buffers, carrying length-prefixed frames.** A
  shared-memory ring is a go/no-go at the performance gate, not a starting point.
- **Two encodings, split by shape of data:**
  - **`zerocopy` for bulk arrays and fixed-size records** (glyph arrays, transforms,
    location headers). Encoding is `as_bytes()`, and decoding is a checked cast of the
    receive buffer, so the cost is at most a memcpy. The wire structs are our own
    `#[repr(C)]` types in the frozen version module, with explicit padding fields
    (enforced by the `zerocopy` derives). They are converted from domain types such as
    `RunGlyph`; they are not the domain types themselves. Golden fixtures pin their
    layout, and no third-party format version can change it.
  - **Postcard for small, nested messages** (handshake, view events, configuration
    requests). Its wire format has been frozen since 1.0.
  - Rejected: `rkyv`, whose archived layout belongs to rkyv and changed between 0.7
    and 0.8, which conflicts with frozen protocol versions. Also rejected: Cap'n
    Proto and FlatBuffers, which bring code generation and a versioning model that
    overlaps with the frozen modules.
- **One ordered stream of tagged items.** A submission frame is an atomic batch: a
  sequence of items, each prefixed by a one-byte protocol tag, followed by pacing.
  Ordering across protocols is required (a face announcement before the glyph run
  that uses it, view creation interleaved with scene changes), so protocols cannot
  be separate streams. The host decodes each item through its protocol's version
  module and pushes the converted domain change into one `ChangeSet` in stream
  order. The desktop downstream never sees protocols.
- **The outbound path never blocks the desktop.** Each connection has its own host
  writer task. Outbound messages that coalesce in-process (`Resized`,
  `CursorMoved`, `ApplyAnimations` per presentation) also coalesce in the host's
  per-connection queue, so a stalled client costs bounded memory and never stalls a
  desktop frame.

### Protocols and compatibility

- **One protocol per concern, each versioned independently:**
  - `session`: handshake, lifecycle, shutdown, liveness;
  - `scene`: locations, transforms, visuals, shapes;
  - `fonts`: face announcement;
  - `input`: view events;
  - `views`: view creation, extents, configuration requests, pacing.

  `session` can never change incompatibly, because it carries the negotiation.
- **The handshake fixes exactly one version per protocol for the lifetime of the
  connection.** The client offers one version for each protocol it speaks, and the
  host accepts if it supports each one. Versions must match exactly; there is no
  feature negotiation inside a version. The tag alone then selects the decoder.
- **The handshake declares the client's byte order.** `zerocopy` wire structs use
  native byte order, so the client states its byte order in the handshake, and the
  host refuses a mismatch with an error naming both. Both processes share one machine
  today, so a mismatch cannot occur; the declaration keeps that assumption explicit and
  checked rather than silent if a connection ever crosses machines. The byte-order
  field is part of `session` and therefore can never change.
- **Frozen version modules.** Each `(protocol, version)` is a module of wire types
  (`scene::v1`, `scene::v2`, …) that is never edited after release. Golden byte
  fixtures per version fail CI on any accidental change.
- **Inbound conversion always upgrades.** The host converts every supported version
  explicitly to the current domain types. That always succeeds while the domain
  model only grows.
- **Outbound downgrade rule:**
  - **Projection is allowed.** Anything the client's version cannot express is not
    sent. This means a whole event (a new `Pinch` for an `input` v1 client), or a
    new field, when the event keeps its old meaning without it.
  - **Substitution is never allowed.** The host does not invent stand-ins (pinch →
    scroll + modifier). That would fabricate input and bury UI policy in protocol
    code.
  - **New obligations** (messages the client must act on or acknowledge) cannot be
    projected away. Either the host handles them on the old client's behalf (e.g.
    it never suspends a client that predates suspension), or the feature cannot be
    backported and the old version is a candidate for sunset.
- **Versions are added freely and sunset rarely.** Sunset is reserved for an
  intrinsic security problem, or a design issue that cannot be backported.
  - A version is first marked *deprecated* in the handshake reply. The client
    library logs a warning, the desktop shows a one-time notice for that
    application, and removal follows in a later release.
  - A security sunset may remove a version immediately.
  - A client offering only a removed version fails the handshake with an error
    naming the protocol and the version.

### Lifecycle, discovery, and embedding

- **Launching.**
  - The desktop spawns remote applications registered in code
    (`Application::remote(name, command)`) and hands them a pre-connected socket.
  - Applications can also attach through a per-user socket in a `0700` runtime
    directory. Same-user processes are trusted.
  - An attaching application registers its name in the handshake.
- **Discovery.**
  - The desktop sets `MASSIVE_SOCKET` (socket path) and `MASSIVE_ORIGIN` (an opaque
    token naming the instance whose process tree this is) in every process it
    spawns. `mt` passes both through to its shell, so anything started from a
    terminal inside the desktop connects to that desktop.
  - Without `MASSIVE_SOCKET`, the client falls back to the well-known per-user
    socket and arrives without an origin.
- **Placement of live-embedded instances.**
  - With a live origin: in the origin instance's project, in the slot after the
    origin (ADR 0011).
  - Without an origin, or when the origin has gone: in the focused project.
  - Interim focus behaviour: the new instance takes keyboard focus via the usual
    command path (ADR 0002). The real policy is open (see below).
- **Without a reachable desktop, each binary has one fixed behaviour, chosen at
  build time.** Applications that also ship in-process (`mt`) run standalone with
  the in-process shell and their own window. Remote-only applications fail with a
  clear error. There is no runtime fallback heuristic.
- **Crash and hang.**
  - When an application process dies, its instances keep their last scene, frozen
    and marked as crashed. The user can restart them manually through
    `CreationMode::Restore`. There is no automatic restart, which avoids crash loops.
  - An application counts as hung when ticks go unanswered for ~2 s while it has
    pending input. It is shown as hung, and it never stalls desktop frames.

## Sequencing

Each phase can ship and be measured on its own.

0. **Baseline.** Measure the performance bar in-process with `metrics`, including
   the 10-instance case, so remote has a reference.
1. **Winit-free `ViewEvent` and view changes.** Replace winit types in the in-process
   event API and in outbound view changes (`CursorIcon`) with own serializable domain
   types. Convert from and to winit at the shell edge, and update `mt` and the
   examples.
2. **Serializable domain.**
   - Make the wire-path payloads serializable without `Arc` sharing, with glyph
     arrays as fixed-layout bytes on the wire and 1 MB socket buffers (see
     "Feasibility").
   - Introduce content-addressed face identity (font path/blob + index + hash)
     alongside `FaceId`.
   - Reject `Shape::Custom` on remote connections.
3. **`massive-remote` over an in-process loopback.**
   - Build the client and host halves, the frozen v1 modules for all five
     protocols, golden fixtures, id remapping, face announcement, and outbound
     coalescing.
   - Implement the `zerocopy` wire structs and postcard messages of the v1 modules.
   - Run `mt` through the loopback. This proves the protocol semantics with no
     process boundary, separating "the protocol is correct" from "the boundary is
     fast".
4. **Real processes.** Spawn with a pre-connected socket, the attach socket, the
   handshake (including the deprecation flag and byte order), liveness, and the crash/hang
   presentation and manual restart. Add a client entry point for `mt`, raise the
   priority of application-process instance threads as `interactive-thread-priority`
   does in-process, and verify clipboard and hyperlink opening from the remote `mt`.
5. **Performance gate.** Check the bar with 10 out-of-process `mt` instances. Two
   go/no-go decisions follow:
   - a shared-memory ring for bulk payloads;
   - desktop-side animation interpolation, as a separate plan if needed.
6. **Discovery and live embedding.** `MASSIVE_SOCKET` / `MASSIVE_ORIGIN`
   propagation, the placement rule, and standalone-or-fail per binary.

## Validation

- Golden fixtures per `(protocol, version)` pin the frozen wire formats.
- One test per downgrade conversion pins projection: a new event is not sent, a new
  field is dropped, and no substitute is produced.
- Loopback runs of `mt` (phase 3) match in-process behaviour.
- Phase 0 and phase 5 measurements are recorded in this document next to the bar.

## Open questions and follow-ups

- **Focus on arrival.** Whether an embedded instance takes keyboard focus, or
  appears as an assisting instance next to its origin, depends on the applications'
  use cases. Candidates are an app-declared arrival role, a per-app user override in
  the desktop configuration, or both. Interim: take focus.
- **Persisting ad-hoc instances.** Whether instances started from a shell are saved
  and relaunched (command line + working directory) on desktop restart. This is a
  policy and safety question. Interim: ephemeral.
- **Remote applications in the desktop configuration.** An application entry gains
  an optional `command`, so remote applications can be declared without code.
- **Shared GPU surfaces** as an optional visual for applications that must render
  themselves (video, browser).
- **Shared-memory ring** and **desktop-side animation**, if the performance gate
  fails.
- **Run-level glyph keys** (see "Feasibility"), if the performance gate shows
  bandwidth or instance count as a bottleneck.

## ADR candidates

To be written when the corresponding phase lands:

- the scene protocol is the out-of-process boundary;
- one process per application, instances stay threads;
- per-concern protocols with frozen versions, the projection-only downgrade rule,
  and rare sunset through deprecation;
- environment-based discovery and origin-relative embedding.

## Not part of this plan

Non-Rust clients, sandboxing and hostile-input hardening, Windows and wasm, and
shared GPU surfaces.
