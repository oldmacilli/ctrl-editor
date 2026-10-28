# ctrl-monitor: one MIDI monitor for every editor, the mesh and midi-tools

Status: design, for review. Nothing here is implemented yet.

## 1. Goal

One monitor component, written once, used in four places:

| Where | What it gets |
| --- | --- |
| `ctrl-monitor` (this repo) | the data model, the parser, the real-time tap, the store, the describers, one view model, a Slint component and a ratatui widget |
| `midi-tools` (new repo) | two apps, **Monitor** and **Librarian**, each with GUI and TUI at feature parity |
| the device editors (KORG, DB-1, TEC, UC-33) | the monitor pane; KORG's current monitor is replaced and enhanced |
| LightBow (zigtact) | the Mesh Monitor's model becomes `ctrl-monitor`; the mesh is one producer among several |

It takes the best of three existing monitors:

- **MIDI Monitor / SysEx Librarian** (krevis/MIDIApps, BSD-3, macOS only): sources as a
  tree with "From X" / "To X", spying on other programs' output, full type and
  channel filters, details with a hex view, save as `.syx`, the librarian's paced
  sending (bytes/s, pause between messages, buffer size).
- **The Mesh Monitor** (zigtact `network-midi` + `lightbow/meshmon.rs`): the monitor
  answers questions instead of listing traffic. Musical labels before hex, clock and
  CC become chips and curves rather than rows, an alert strip for exceptions only,
  pinning a stream dims the others, pause freezes the view without losing data.
- **The KORG monitor**: the control that sent a message lights up on the panel;
  scene changes are recognised.

And adds what none of them has: descriptors. A message that matches a CDL descriptor
is shown by its name and fields ("nanoKONTROL Studio: scene write, scene 3, 560 bytes")
instead of as hex.

## 2. Layers

```
producers ──► RT tap (lock-free ring) ──► collector ──► store ──► view model ──► Slint / ratatui / egui
 ctrl-midi ports   fixed-size slots       parse          bounded     filter, fold,
 editor sends      + SysEx byte ring      describe       ring +      columns, chips,
 HID reports       drops counted          annotate       SysEx arena alerts, details
 network peers
 thru proxy
```

- **Producers** never allocate and never block. Each owns a `Tap` handle.
- **Collector** (one thread per store) joins, parses and describes. It is the only
  place that allocates.
- **Store** keeps everything that passed capture; display filters never lose data.
- **View model** is plain data, shared by every front end, so TUI and GUI cannot drift
  (the same rule as the editors: one `View`, one `Command` list, parity tests).

### Crates

| Crate | Content | Depends on |
| --- | --- | --- |
| `ctrl-monitor` | model, tap, parser, store, describers, view model, capture files | `ctrl-codec`, `ctrl-descriptors` (optional feature `cdl`), `rtrb` |
| `ctrl-monitor-midi` | producers on `ctrl-midi`: ports, passive taps, virtual ports, thru | `ctrl-midi` |
| `ctrl-monitor-slint` | the Slint component (`.slint` library + Rust glue) | `slint` |
| `ctrl-monitor-tui` | the ratatui widget and its key map | `ratatui` |

Network producers (Bome Network, mesh UDP) stay in zigtact `network-midi` and depend
on `ctrl-monitor`, never the other way round. An RFC 6295 / AppleMIDI producer is
written from the public RFC and can live here (§6.4).

## 3. Data model

### 3.1 Ports

Every event points at a port by a small index into the store's port table:

```rust
pub struct PortRef {
    pub kind: PortKind,      // Alsa, Jack, CoreMidi, WinMm, WinMidiServices, Hid,
                             // Virtual, Thru, Network { protocol }, Editor, File
    pub id: String,          // stable: ALSA "client:port" name, JACK full name,
                             // CoreMIDI unique id, peer address, …
    pub name: String,        // what people read: "nanoKONTROL Studio CTRL"
    pub device: Option<DeviceMatch>, // descriptor that identified it, if any
}
```

The editors' own sends are a port of kind `Editor`, so "what the editor sent" and
"what the device answered" appear in one list.

### 3.2 Events

```rust
pub struct Event {
    pub seq: u64,            // per store, monotonic; a gap means a drop
    pub at: Stamp,
    pub port: PortIdx,
    pub dir: Dir,            // In (from the port), Out (to the port), Thru
    pub msg: Msg,
    pub notes: NoteIdx,      // annotations, see 3.4; 0 = none
}

pub struct Stamp {
    pub host_ns: u64,        // monotonic, from the producer (the backend's own
                             // packet time when it has one, not the poll time)
    pub wall_ms: i64,        // for "12:03:44.120" and saved captures
    pub domain: Domain,      // Host, Sample(u64), LinkBeat(f64), DeviceZero
}

pub enum Msg {
    Voice { status: u8, d1: u8, d2: u8 },   // 80..EF, running status resolved
    Common { status: u8, d1: u8, d2: u8 },  // F1 F2 F3 F6
    Realtime(u8),                           // F8..FF
    SysEx { blob: BlobIdx, len: u32, end: SysExEnd }, // Complete, Cut(status), Timeout, Truncated
    Ump([u32; 4], u8),                      // MIDI 2.0, word count
    Hid { report: BlobIdx },                // TEC and other HID devices
    Invalid { blob: BlobIdx },
}
```

`Event` is `Copy` and fixed-size. SysEx, HID and invalid bytes live in a separate byte
arena, bounded by bytes, not by count, so a 1 MB dump does not push out the last ten
minutes of notes; the arena reports what it evicted.

### 3.3 Real-time tap

Taken from the LightBow VST3 MIDI log, the one existing tap that is proven
allocation-free:

- an `rtrb` single-producer ring of fixed slots (`Event` minus indexes, plus up to
  12 inline bytes) and a second byte ring for long payloads;
- a full ring drops the message and counts it; the count is shown (§5.5), never hidden;
- one tap per producer thread; the collector drains all of them.

Thresholds: slot ring 4096, byte ring 1 MB, per producer, settable.

### 3.4 Annotations

Annotations explain an event. They are optional and come from describers (§4) or
producers:

| Annotation | From |
| --- | --- |
| `Described { name, fields }` | a CDL descriptor match |
| `Control { label, id }` | a device annotator (KORG panel control that sent it) |
| `Scene(n)`, device state changes | a device annotator |
| `Rule { gate, action, rule }` | a MeshGate / LBMT rule |
| `Path { hops, tier, latency_us }` | the mesh producer |
| `Joined { pieces }` | the collector, when a SysEx came in pieces |

The Mesh Monitor's path fields (`paradigm`, `tier`, `journal_ok`, `deduped`, `hops`,
`link_beat`) become the `Path` annotation and `Domain::LinkBeat`, so the core needs no
mesh types.

## 4. Parsing and describing

### 4.1 Parser

One parser for every byte stream:

- running status (MIDIApps does not have it; ALSA rawmidi, serial and network
  streams need it);
- real-time bytes anywhere, including inside SysEx;
- SysEx across packets; a status byte inside SysEx ends it as `Cut`; a SysEx without
  F7 ends after a timeout (default 1 s) as `Timeout`; progress is reported every
  256 bytes so a view can show "receiving 12 KB…";
- Note On with velocity 0 is kept as received and *shown* as Note Off unless the
  view shows raw messages;
- UMP: decoded with the codec from the LBMT MIDI 2.0 branch (`lbmt::wire`), which the
  MIDI 2.0 spec notes identify as the correct reference.

### 4.2 Describers, in order

1. **Descriptor** (`cdl` feature): `ctrl-codec` identifies the message against the
   descriptors of the matched device, or all descriptors: name, fields, status
   (measured/decoded/inferred). Joined long SysEx (`sysex-limit`) is described once.
2. **Device annotator** (trait, implemented by an editor): KORG lights the control,
   names the scene change; DB-1 names the drawbar.
3. **MIDI 1.0 / 2.0 generic**: type names as in MIDI Monitor; note names (C4 = 60 or
   C3 = 60, sharps); CC names (coarse/fine, pedals, sound controllers, RPN/NRPN,
   channel mode); RPN/NRPN and 14-bit CC pairs folded into one value; manufacturer
   names for SysEx (1-byte and 3-byte ids); universal SysEx (identity request/reply,
   MTC full frame, MMC).

The CC and manufacturer tables are taken from SnoizeMIDI's `ControllerNames.plist`
and `ManufacturerNames.plist` (BSD-3, credited in `NOTICE`), converted to a Rust
table at build time.

## 5. Views

Every view is a function of the store plus a `ViewState` (filters, pins, pause,
selection). Slint and ratatui draw the same rows with the same colours and the same
key bindings.

### 5.1 List (the default)

Columns: time (clock, Δ to previous, Δ to selection, host ns, beat), direction
glyph (← in, → out, ⇄ thru), port, channel, message, value, description.

- colour by class: voice green/blue/orange by type (as now in KORG), system common
  violet, real-time grey, SysEx teal, invalid and drops red; optional tint by channel;
- **folding** (Mesh Monitor rule): repeated Clock and Active Sense collapse into a
  chip "Clock 120.0 BPM ×384"; runs of one CC collapse into one row with a spark line
  "CC 7 Volume 12→98 ×37"; a click expands;
- SysEx rows expand in place: decoded fields, then the hex/ASCII view with offsets;
  "Save .syx", "Copy hex", "Send again" (editors only, behind their write gate);
- follows the newest row only while scrolled to the end; pause freezes the view,
  capture goes on, a badge counts what arrived meanwhile;
- newest at the bottom in both front ends;
- copy rows as tab-separated text.

### 5.2 Streams

One lane per port × channel: notes on a piano-roll axis (`piano-stage::PitchAxis` in
LightBow), CC curves, pitch bend, pressure. Pinning a lane dims the others. This is
the Mesh Monitor's "what is playing".

### 5.3 Map

Ports, peers and thru routes as a graph: connected or not, messages/s, bytes/s,
drops, last seen. The Mesh Monitor's "is it connected".

### 5.4 Journey

For events with a `Path` annotation: hops with Δt each. Mesh only; hidden otherwise.

### 5.5 Alert strip

Exceptions only: dropped events (tap or arena), SysEx cut or timed out, stuck notes
(Note On without Off after a port closed), port disconnected, a descriptor reply that
failed its range check, a gate that blocked. Grouped by kind with a count.

### 5.6 Filters

Two stages, because both monitors were right about something:

- **capture filter** (MIDI Monitor): what is never stored, e.g. Active Sense at
  300/s. Type mask (19 types as in SnoizeMIDI), channel mask, per port.
- **view filter** (Mesh Monitor): what is hidden now but kept: the same masks plus
  "only this device's messages", "only described", "only exceptions", text search
  over descriptions.

No filter language: masks, pins and search are enough.

## 6. Sources: seeing traffic between other programs and a device

The monitor can watch the device's input, output, or both, without the editor being
part of the conversation. How depends on the platform:

| | Device → programs | Programs → device |
| --- | --- | --- |
| **Linux, ALSA sequencer** | passive subscription to the device's port | passive subscription to the *program's* output port (only if the program uses the sequencer; programs on raw ALSA MIDI bypass it) |
| **Linux, JACK / PipeWire** | connect a monitor input to the device's capture port | connect a monitor input to the program's output port; JACK and PipeWire allow any number of readers |
| **macOS, CoreMIDI** | passive: sources fan out to every client | needs a thru port (§6.3) or a CoreMIDI driver plug-in that enables monitoring (later, see §9) |
| **Windows, Windows MIDI Services** | multi-client endpoints allow a second reader | thru port via a loopback endpoint pair |
| **Windows, WinMM** | usually fails: most drivers open ports exclusively | thru port only, needs a virtual-port driver |
| **Network** | the monitor is a peer in the session | thru: programs connect to the monitor's network endpoint, which forwards to the device |

### 6.1 Passive taps (`ctrl-midi`)

`ctrl-midi` gains a `watch(port)` call: on ALSA a read subscription, on JACK a second
connection, on CoreMIDI a plain input. It never sends.

### 6.2 Virtual ports

"Act as a destination for other programs" from MIDI Monitor: a virtual input named
"ctrl-monitor" (ALSA, JACK, CoreMIDI, Windows MIDI Services).

### 6.3 Thru

The one method that works everywhere for both directions: the monitor creates a
virtual port pair "<device> (monitored)", the program is pointed at it, and the
monitor forwards to and from the real device, recording both directions with one
clock. The added latency is measured and shown in the Map view.

### 6.4 Network thru

Network MIDI is the cross-platform way to put the monitor between a program and a
device on any operating system, including a device on another computer:

- **RFC 6295 / AppleMIDI** (session protocol and recovery journal from the public
  RFC): macOS has it built in (Audio MIDI Setup, Network), Windows and Linux programs
  use rtpMIDI-compatible drivers. A `ctrl-monitor-midi` producer is a session peer and
  can act as thru between a session and a local port.
- **Bome Network** and the **LightBow mesh**: producers in zigtact `network-midi`,
  feeding the same tap. Their peer identity (address, session name) becomes the
  `PortRef`, their routing becomes the `Path` annotation.

## 7. Capture files

- **`.ctrlcap`**: the native file. A KDL header (ports, settings, filters, the
  descriptor revision) followed by the event stream and the byte arena, zstd
  compressed. Open, append, and "save the last N minutes".
- **Export**: `.syx` (selected SysEx, each forced to F0…F7 or as received),
  Standard MIDI File (`midly`, real timing), tab-separated text.
- MIDI Monitor's `.midimonitor` (NSKeyedArchiver) is not imported.

## 8. Librarian (midi-tools)

SysEx Librarian's feature set, cross-platform, on the same core:

- **Library**: a list of files with cached name, manufacturer, message count, size,
  program number. Stored as KDL in the data directory; each entry keeps the path and a
  SHA-256, so a moved file is found again by content.
- **Formats**: `.syx` (F0 to the next status byte; a cut-off last message is kept),
  `.mid` (all tracks, `midly`), export to both.
- **Record**: one message, or many until Done; all inputs or chosen inputs; the
  monitor's parser with its SysEx timeout; progress in bytes.
- **Play**: to a port, a virtual source, or a network peer. Pacing per port:
  bytes/s (10–3125), pause between messages (0–5000 ms, default 150), chunk size
  (rounded to a multiple of 3 for USB MIDI cables that fail on short packets). If a
  descriptor matches the device, its `sysex-limit` and `segment-delay` are the
  defaults, so the Linux 4 KB limit is handled the same way the editors handle it.
  Progress "message i of N, bytes", cancel.
- **Describe**: an entry that matches a descriptor shows what it is ("nanoKEY Studio
  scene set, 5 scenes").
- **Program change trigger**: a virtual input that plays the entry with the matching
  program number.
- **Find missing**, rename on disk, delete or move to trash, details with hex view and
  checksums.

The Librarian's send and record show up in the Monitor as port `Editor` traffic when
both run in one process, and as ordinary traffic otherwise.

## 9. What the editors and LightBow change

- **KORG**: `korg-app/live.rs` and `monitor.rs` lose the parser, the message type and
  the ring; KORG implements the device annotator (scene change, lit controls, global
  channel) and its own sends become `Editor` events. The pane becomes the shared
  component: the list with KORG descriptions, the lit control's name in a column. The
  lighting/expiry code that is now copied into both front ends moves into the view
  model.
- **DB-1, TEC, UC-33**: the same pane; UC-33's learn mode reads from a monitor store;
  TEC events are `Hid` with descriptor descriptions.
- **LightBow**: `network-midi` records into a `ctrl-monitor` tap instead of
  `MonitorTap`; `meshd` writes `.ctrlcap` instead of the JSON feed; the egui pane draws
  the shared view model (Map, Streams, Journey) and the serde mirror goes away, which
  also ends its drift (the gate fields it lacks).

## 10. Milestones

| # | Content | Done when |
| --- | --- | --- |
| M1 | `ctrl-monitor` core: model, tap, parser, store, generic describers, tables | parser property tests (running status, real-time inside SysEx, split SysEx); tap allocation test; store bounds |
| M2 | descriptor describer; view model: list, filters, folding, alerts, details | each descriptor's vectors come back described |
| M3 | `ctrl-monitor-tui` and `ctrl-monitor-slint`: list, details, filters, alert strip | parity tests over the view model; headless render on Weston (tools/linux-check) |
| M4 | KORG adopts it | KORG's monitor tests pass on the shared component; editor sends shown |
| M5 | `midi-tools` Monitor: source tree, passive taps (ALSA, JACK/PipeWire, CoreMIDI), virtual port, several windows, `.ctrlcap`, exports | the linux-check container watches a JACK and an ALSA conversation between two other clients |
| M6 | `midi-tools` Librarian | paced send measured on the loopback; `.syx`/`.mid` round trip |
| M7 | thru (virtual pairs), Windows MIDI Services | both directions recorded on macOS and Linux |
| M8 | AppleMIDI producer and network thru | interoperates with macOS Network MIDI |
| M9 | Streams and Map views; LightBow mesh producer; egui pane on the view model | Mesh Monitor tests pass on the new model |
| M10 | UMP | MIDI 2.0 channel voice described and shown |

DB-1, TEC and UC-33 adopt it after M4, each in its own PR.

## 11. Open questions

1. App names in `midi-tools`: "Monitor" and "Librarian", or names of their own?
2. A macOS CoreMIDI driver plug-in for spying without thru: worth it later, or is
   thru enough?
3. Windows: Windows MIDI Services only (Windows 11), or WinMM with a virtual-port
   driver too?
4. Should the LightBow egui pane move to Slint, or stay egui on the shared view model?
