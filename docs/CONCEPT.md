# One editor for KORG, DB-1 and TEControl

Status: draft for review, 2026-10-20. Living plan: [TODO.md](TODO.md).
Descriptor language: [CDL.md](CDL.md).

## 1. Starting point

Three editors replace three dead or closed vendor tools. All three are Rust,
and they were built separately:

| | KORG Kontrol Editor | DB-1 Editor | TEC Ctrl Editor |
|---|---|---|---|
| Repo | `KORG_Kontrol_Editor` | `OceanBeach_DB-1` | `TECcontrol` |
| Devices | 16 KORG controllers (nanoKONTROL Studio, nanoKEY Studio, …) | Ocean Beach DB-1 drawbar controller | TEControl USB MIDI Breath Controller |
| Config transport | MIDI SysEx (Korg native, 7-bit packed dumps) | MIDI SysEx (`F0 09 00 00 00 type cmd … F7`, ack per step) | USB HID, 8-byte frames, no MIDI |
| Live transport | MIDI CC/notes from the device | MIDI CC on MAIN OUT | class-compliant USB MIDI from the firmware |
| Data model | descriptors and schemas generated from the vendor's tables (`analysis/`) | hard-coded Rust (`db1-protocol`) | hard-coded Rust (`tec-protocol`) |
| Core | `EditorController` (undo, view model, scenes) | `db1-session` (read/write chains, restore points, tx log) | session/hub actor per device, write policy, backups |
| GUI | Slint (`korg-slint`) | Slint planned (`db1-ui`) | Slint 1.18 (`tec-ui`) |
| TUI | ratatui (`korg-tui`) | line REPL (`db1-tui`) | none |
| MIDI I/O | midir | midir | CoreMIDI FFI |
| Safety | confirm, read-back verify | `--apply`, restore point | `unsafe-commands` feature and a 10-minute runtime arm |
| Firmware update | none | vendor tool, `.obd` image, not decoded | none exists |

The same layers show up three times: a protocol crate with no I/O, a
transport trait with a mock and a real backend, a session that runs request
and acknowledge chains, write safety, and thin front ends. The differences
are in data: message bytes, parameter layouts, value ranges, panels. That
split decides the design.

## 2. Goal

One codebase and one GUI and TUI. A device is added by writing a
**descriptor** (data). Code is written only where data cannot express the
behaviour; that code is a **driver**. The three current editors become
device packs on the shared platform. Their repos keep the analysis
evidence (`analysis/`, `findings/`, captures).

Out of scope for now: SQ-64 and the NTS kits (no data model yet),
Windows and Linux HID back ends beyond stubs, and executing Bome `.bmtp`
projects.

## 3. Architecture

```text
 front ends    ctrl-slint (GUI)            ctrl-tui (ratatui)             ctrl-cli
                     \                          |                          /
 application   ctrl-app: open/save, device list, jobs, panels, write policy
                     |
 core          ctrl-core: Document, EditorController (undo, view model), Session
                     |                       |
 engine        ctrl-engine: runs CDL procedures (send / expect / repeat / verify / gate)
                     |                       |                    |
 descriptors   ctrl-cdl: parse + validate    ctrl-codec: layouts,  ctrl-lbmt: embeds
               .ctrl.kdl into an IR          packing, checksums   zigtact `lbmt`
                     |
 transports    ctrl-transport traits: MidiPort, HidDevice, Clock (+ mocks, trace replay)
 adapters      ctrl-midi-midir · ctrl-midi-coremidi · ctrl-hid-iokit · (ctrl-hid-hidapi)
 drivers       korg-format (file codec) · tec-curve (32-point curve) · …  [Rust, behind `Driver`]
```

Rules carried over from the KORG workspace:

- The core and everything below it uses only `std`, `serde` and pure-Rust
  crates. Platform code lives in adapter crates.
  `scripts/check-platform-deps.sh` moves over unchanged.
- Front ends hold no protocol logic. Both draw the same `ViewModel` and the
  same `Panel` (`korg_app::panel`, already shared by `korg-slint` and
  `korg-tui`).
- Nothing is sent that a descriptor marks `unknown`. An `inferred` value
  needs the user's opt-in (CDL §4).

### 3.1 What comes from where

| Platform piece | Taken from | Change |
|---|---|---|
| `ctrl-core` EditorController, undo, view model | `korg-core` | later; first behind an adapter (§4), generalised only after DB-1 and one KORG device run on CDL |
| `ctrl-app` jobs, panels, document helpers | `korg-app` | `DeviceJob` becomes "run CDL procedure X" |
| `ctrl-tui` | `korg-tui` | no device-specific code; panels and fields come from CDL |
| `ctrl-slint` | `korg-slint` (tabs, browse, copy/paste, prefs) + `tec-ui` (live meters, curve view) | one window; device pack picks the tabs |
| Session: restore points, tx log | `db1-session` | restore point before every write, for all devices |
| Hub: one thread per device, keyed by serial | `tec-device` | needed for HID and for several controllers at once |
| Write gate | `tec-app` (arm) + `korg` (confirm, read-back) + `db1` (`--apply`) | one policy, CDL §11 |
| Fakes and trace replay | `MockTransport`, `MockDevice`, `FakeController`, hid traces | one `ctrl-transport::mock` that replays captures |
| MIDI | midir (KORG, DB-1) | TEC's CoreMIDI FFI stays as a second adapter until midir covers it |
| HID | `tec-device/src/macos.rs` (IOKit, Seize) | becomes `ctrl-hid-iokit` |
| Rule snippets | zigtact `crates/lbmt` | used as a pinned git dependency, not copied |

### 3.2 Descriptor first, driver second

A descriptor (`*.ctrl.kdl`, [CDL.md](CDL.md)) states what a device is, how
to find it, its data blocks and fields, its messages, and the procedures for
reading, writing and updating it. That covers the DB-1 almost completely,
and KORG's SysEx exchanges and scene layouts.

A **driver** is Rust code that a descriptor names (`driver "tec-curve"`) for
logic that is not data:

- the TEC response-curve generator, which must match the original exactly;
- the KORG `.nktrl_*` file codec, which re-encodes byte-identically;
- anything that needs state across many messages and cannot be written
  clearly in LBMT.

A driver implements a small trait: compute blocks from parameters, decode
vendor files, and optionally run a procedure step. It is tested with the
same test vectors as the descriptor.

### 3.3 LBMT

LBMT (Lightbow MIDI Translation, zigtact `crates/lbmt`) is Bome MIDI
Translator's rule model as a text DSL. It has a parser, compiler,
interpreter and debugger, and about 70 tests. CDL embeds it in three places
(CDL §10):

1. `transform`: value mapping between display, storage and wire values.
2. `checksum` and `compute`: derived bytes in a message.
3. `translator`: live MIDI rules, the "classic MIDI patching" layer. For
   example, the DB-1's drawbar CCs can be remapped to an organ's CC set
   while the editor runs, or a breath controller's CC can be curved and
   split.

Constraint: LBMT clamps arithmetic to 0..127 today. Bome uses 32-bit
signed integers, and checksums and 14-bit values need more than 7 bits.
Until LBMT has a wide-integer mode, CDL uses built-in checksum algorithms
(`sum7`, `roland`, `xor`) and LBMT only for 7-bit logic. The change belongs
to the zigtact lane (TODO P3).

## 4. Repository plan

A new repo, `ctrl-editor` (working name), holds the platform crates, the
front ends and `descriptors/<vendor>/`. The three existing repos keep
`analysis/`, `findings/`, captures, tools and vendor-file handling. Vendor
binaries and extracted content stay out of every repo, as now.

Order:

1. Prove CDL on the DB-1 first. Its protocol is small and fully measured.
   New, small platform crates (`ctrl-cdl`, `ctrl-engine`,
   `ctrl-transport`) run the DB-1 descriptor; `db1-session` brings
   restore points and the tx log. The KORG editor is not touched.
2. Put the KORG editor behind adapters, not a rename. `korg-core` and its
   `EditorController` stay as they are; adapters let the platform engine
   and front ends drive them, and the shared pieces that already exist
   (`korg_app::panel`, `run_job`) are reused. KORG devices then move to CDL
   one at a time, each behind its hardware checklist.
3. TEC third. It adds HID, the per-device hub and the arm gate, and is the
   first device with a driver (the curve).
4. The front ends: `ctrl-tui` and `ctrl-slint` grow out of `korg-tui` and
   `korg-slint` once two device families run on the engine.
5. Firmware update last, starting with a decoded, captured procedure. No
   device has one yet.

Each step keeps the old editor working until the new one passes the same
hardware checklist.

## 5. Risks

| Risk | Mitigation |
|---|---|
| Protocol regression while KORG code moves to the platform (framing, length style, device id, write header, commit) | No rename of working crates; KORG stays behind adapters (§4). Before a KORG device moves, its CDL test vectors must reproduce `korg-sysex` and `MeasuredSession` output byte for byte, and the hardware checklist (receive, write, read-back) must pass on USB and BLE |
| A generic engine hides device quirks that the hand-written code got right | Every descriptor ships test vectors from real captures; the old crate's tests must pass against the descriptor before the old crate is retired |
| The descriptor language grows into a programming language | Hard limit: no loops beyond `repeat`, no recursion; complex logic goes to LBMT or a driver |
| Bricking hardware (TEC incident 2026-10-13, firmware) | One write policy: dry run by default, confirm, arm for flash and firmware, restore point, read-back verify (CDL §11) |
| LBMT is still marked "Design" and owned elsewhere | Pin a revision; CDL needs only `parse`/`compile`/`Interpreter`; wide ints are requested, not patched locally |
| Three lanes edit the same code | Crate ownership in `AGENTS.md`; API changes to `ctrl-core` need the owner's ack, as today with `EditorController` |
