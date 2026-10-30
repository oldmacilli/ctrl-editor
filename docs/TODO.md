# Living TODO: merging the three editors

Concept: [CONCEPT.md](CONCEPT.md) · Language: [CDL.md](CDL.md)

How to keep this file alive:

- Tick an item in the same commit that finishes it, and append the commit
  hash: `- [x] … (abc1234)`.
- New work goes under the phase it belongs to, with an owner.
- A blocked item says what it waits for after `blocked:`.
- Remove items that are no longer wanted; do not leave them stale.

Owners: **platform** = the ctrl-editor lane (w5:p1) · **korg** = KORG
editor lane (OMP, w5:p2) · **db1** / **tec** = whoever owns those repos ·
**zigtact** = the LBMT owners · **user** = needs the owner's decision or
hardware.

Last updated: 2026-10-28 (CDL gaps closed, long SysEx, UC-33, Faderfox UC4).

## P0 Agree the concept

- [x] user: GitHub repo `oldmacilli/ctrl-editor` created and pushed
  (9d89ac2); public since 2026-10-28
- [ ] user: approve the concept
- [ ] korg: review CONCEPT §3 and §4 (crate moves out of the KORG repo)
- [ ] db1, tec: review CONCEPT and their descriptors
  (`descriptors/oceanbeach/db1.ctrl.kdl`, `descriptors/tecontrol/breath.ctrl.kdl`)
- [ ] zigtact: agree to `lbmt` as a pinned git dependency and to the
  wide-integer request (P3)

## P1 CDL toolchain

- [x] platform: CDL 0.1 draft (`docs/CDL.md`)
- [x] platform: example descriptors for DB-1, TEC, nanoKONTROL Studio and a
  firmware template; all four parse as KDL 2.0 (`tools/cdl-parse`, now
  `ctrl-cli check`)
- [x] platform: the DB-1 LBMT example parses, compiles and runs in
  zigtact's `lbmt` crate
- [x] platform: KORG lane review applied: model id `00 01 37`, device id
  from the identity reply, preferred transport, procedure IDs aligned,
  driver-owned bytes on record (`owns`), path grammar for `verify` (CDL
  §4.4), protocol-regression risk
- [x] platform: `ctrl-cdl` crate: KDL → typed IR (serde), with errors that
  point at line and column; includes merge (`*.inc.kdl`); templates, paths
  and ranges parsed; all descriptors load (3ae75df)
- [x] platform: slice placeholders `{curve[6] pad=7F}`; the TEC curve pages
  are CDL again, no driver step (1d62935)
- [x] platform: validator (`ctrl_cdl::validate`): references and paths,
  sizes and ranges, `status` rules (CDL §4.1), write policy (§11), port
  overlap (§5), SysEx data bytes, HID frame sizes, scan messages (1d62935)
- [x] platform: CDL §5.1 `scan` and `listen` for DIN-only devices; DB-1
  found by scanning with its revision request (1d62935)
- [x] platform: CDL expressions (§10.1), parsed by the validator; LBMT stays
  for compute and translators (7b765c8)
- [x] platform: `ctrl-codec`: encode and decode `send`/`reply` (CDL §6),
  checksums (§6.1), lengths, `midi-id`, `packed7`, HID frame padding, reply
  captures through expressions; test-vector runner in `cdl-parse` (7b765c8)
- [x] platform: KORG framing from generic parts (CDL §8.1): `const`,
  `midi-id`, `{0x40 | device-id}`; 10 KORG vectors byte for byte with
  `korg-sysex`'s captures (7b765c8)
- [x] platform: CDL versatility, from the KORG lane's proposal: send-only
  messages (exchange on one is an error), reply variants with first match,
  named patterns, replies into blocks with exact size (`ctrl_codec::fill`),
  `selected` needs a config transport (8a9e2a2)
- [x] korg: scene-set-info reply variants, per-device `scene-data`
  patterns for read-block, a scene-read reply vector from a capture, then
  the procedures off `korg-measured` (d9aa7fd)
- [x] platform+db1: DB-1 vectors from `db1-protocol` tests, the app's
  frames and the hardware ping reply: 10; drawbar `arg` index 0..=8 (0c98374)
- [x] platform+tec: TEC vectors from the HID captures: 16, and every
  transaction in the 10 TECcontrol traces decodes to one message (37195a4)
- [x] platform: KORG and TEC descriptors in one style: `arg` ranges (CDL
  §6.5), named ack patterns, reply variants, evidence on every message and
  procedure; TEC `read-name` and the `51` reply added (37195a4)
- [ ] platform: a vendor-encoding registry entry only when no generic part
  fits; none needed so far
- [x] platform: `ctrl-cli check | explain | trace` (CDL §16); replaces
  `cdl-parse` (aec51ef)
- [ ] platform: `ctrl-cli check` compiles LBMT once `lbmt` is a dependency
- [x] platform: tree-sitter queries (highlights, LBMT injection) on
  tree-sitter-kdl, checked against the grammar and the node table (fc696e8)
- [ ] zigtact: a tree-sitter grammar for LBMT, so the injection highlights
- [x] platform: `cdl-schema.md`: one entry per node with every property,
  generated from the converter's node table (`schema.rs`, `ctrl-cli
  schema`); a test fails when it is stale (5cdb9a0)
- [x] platform: CDL gaps closed (214eee4): `arg` ranges on reply
  captures (§6.5); status and real-time bytes in MIDI templates (§6.6);
  `u21le`/`u21be` and `varlen7` lengths (§8.1); KORG native body written
  as `{len:u21le} {command} …`, so both Studios share one scene pattern
- [x] platform: long SysEx (§8.2, 214eee4): transport `sysex-limit` and
  `segment-delay` split host writes, `SysexJoiner` rejoins input, a
  message's `segment` names a device's own multi-part form, `Codec::plan`.
  KORG transports carry korg-midi-midir's measured 256 bytes / 5 ms
- [x] platform: USB side of HID in CDL (§9, 214eee4): `report-descriptor`
  (parsed, checked against `frame`), `poll`, `pace`; TEC filled in
- [x] platform: `ctrl-descriptors`: every descriptor embedded, for the
  editors' conformance tests through a pinned git dependency
- [x] korg, db1, tec: conformance test in each editor repo, pinned to
  414ecb3: KORG_Kontrol_Editor PR #2, OceanBeach_DB-1 PR #27, TECcontrol
  PR #4 (open, for the owner to merge)
- [x] platform: `arg type=` (CDL §6.5): typed arguments, so polarity 2,
  channel 16 or an unknown status fail as in db1-protocol (found by DB-1
  PR #27). The DB-1 descriptor types its set and reply values
- [x] db1: bump DB-1 PR #27's pin and un-ignore its typed-value tests
  (DB-1 8f70768, merged)
- [ ] korg: `korg_sysex::build_ack` builds the first-generation `5F
  <status>` ack; the Studios send `{len:u21le} <status>` (KORG PR #2,
  ignored test). Unused outside tests: fix or retire it
- [x] user: history rewritten on 2026-10-28 before wider use: binary-analysis
  details (routines, addresses, resource names and hashes, local paths) and
  the generator left this repository; the generator lives in
  KORG_Kontrol_Editor (`tools/gen-korg-cdl`). Commit ids changed; the ids in
  this file are the new ones
- [x] user: ctrl-editor made public on 2026-10-28, as it is (vendor
  pictures and KORG label lists included, by the owner's decision), so the
  editors' packages fetch the shared crates without credentials (KORG PR
  #2/#3 cargo-sources.json)
- [ ] tec: descriptor poll interval 52 ms (original app) vs tec-device's
  50 ms default; align one of them
- [ ] platform: ctrl-midi from the KORG lane's PR #3: split only a
  complete SysEx (F0…F7), reuse long-lived clients for port scans (KORG
  45c6f56 keeps two CoreMIDI clients for its 1 s scan), and open ports by
  identity, not by name alone (two ports of one name open the first)
- [ ] platform: `ctrl-transport` applies `sysex-limit` and `pace` when it
  writes, from `Codec::plan`
- [x] platform: `ctrl-midi`: run-time MIDI backend (ALSA sequencer, JACK
  via libjack loaded at run time, so jackd2 or PipeWire; CoreMIDI), ALSA
  writes split by `sysex-limit`, input SysEx rejoined; `docs/LINUX.md`
- [x] platform: `tools/linux-check`: JACK loopback (CC + 6002-byte SysEx)
  under jackd2 and PipeWire's libjack; KORG, DB-1, TEC and UC-33 GUIs start
  on Wayland (headless Weston, GL and software renderers)
- [ ] korg, db1: move the MIDI layer to `ctrl-midi` and add the backend
  switch (GUI, TUI, CLI), as UC33-Editor did
- [ ] user: on a Linux machine with a device, the hand checks in
  docs/LINUX.md §8 (ALSA run, PipeWire Midi-Bridge port names, a dump)
- [x] platform: guides (CDL §17): `hands`, guide steps in procedures,
  `recipe`, `edit` rules and tours; `ctrl_engine::edit_guide`,
  `Operator::guide`/`ask` (defaults keep old operators working),
  `ctrl-cli guide`. UC-33 and UC4 descriptors carry them
- [x] platform: shortest guides (`edit_guide_at`: skips what the device
  holds, an open element, a selected item; cheapest `one-of`), `kinds`
  for the editor's type choice, `edit read=` and `changes_between` for
  the status quo (CDL §17.4, §17.6)
- [ ] platform: GUI and TUI cards for `Operator::guide` (key labels as
  chips, `shows` and `hint` beside them, a "done" button; an `await` card
  closes itself when the dump arrives)
- [ ] platform: labels per language for `hands` keys and guide texts
  (§13.1); today they are English

## P2 Prove CDL on the DB-1; KORG stays intact

Order (CONCEPT §4, revised after the KORG lane's review): new small crates
first, DB-1 on them, KORG behind adapters. No rename of working crates.

- [ ] platform: `ctrl-transport`: MIDI trait (shape of `korg-midi` and
  `db1-midi`), `HidDevice` trait (shape of `tec-device`), mocks that replay
  captures
- [ ] platform: `ctrl-engine`: procedure runner (exchange, repeat, verify
  with paths, gates, restore points, tx log from `db1-session`). First
  slice in place (send, exchange, await, gate, restore-point, verify,
  call, repeat over ranges, delay, ok-when, require; editor and device
  documents with addressed memory); missing: read-into, from=, drivers,
  expect-identity, HID, the tx log
- [ ] platform+db1: DB-1 runs on `ctrl-engine` from its descriptor; every
  `db1-protocol` test vector reproduced byte for byte
- [ ] platform+korg: adapters so the engine and front ends can drive
  `korg-core` unchanged (`EditorController`, `MeasuredSession`, `run_job`)
- [ ] korg: first KORG device on CDL (nanoKONTROL Studio): its CDL vectors
  reproduce `korg-sysex` and `MeasuredSession` output byte for byte; hardware
  checklist on USB and BLE
- [ ] korg+platform: generalise `EditorController`/`ViewModel` into
  `ctrl-core` only after that. **API change: korg owner acks first.**
- [ ] platform: `ctrl-tui` from `korg-tui`, with no device-specific code
- [ ] korg: `ctrl-slint` from `korg-slint` (at `c98ded8`: tabs, browse,
  copy/paste, New from factory, prefs, measured jobs)
- [ ] platform: `scripts/check-platform-deps.sh` and `ci-check.sh` carried
  over

## P3 LBMT integration

- [ ] platform: `ctrl-lbmt`: sandboxed run of `compute` and `translator`
  snippets (no key, mouse or timer actions in transforms; step limit)
- [ ] zigtact: wide-integer mode in `lbmt` (Bome uses 32-bit signed; today
  every result is clamped to 0..127). Needed for 14-bit values and
  checksums in LBMT. In review: SPEC-lbmt-001, zigtact PR #138 (branch
  `claude/lbmt-midi2`, head bb0d32534): i64 wrapping registers,
  `lbmt::wire::{encode_midi1, encode_ump}` clamp per protocol, `midi2`
  keyword with `group=`, `MidiOutputEvent` with raw i64 fields plus a
  `Domain`. Bump the pin at its merge commit.
- [ ] zigtact: parser gaps from the spec examples: `THEN` parses; missing
  field names, `IF` guarding more than one op and `IF` with a non-number
  right-hand side go to a separate parser spec (zigtact owner)
- [ ] platform: translator panel in GUI and TUI (enable, edit, monitor)

## P4 Device packs

### KORG

- [ ] korg: nanoKONTROL Studio descriptor complete: procedures that use the
  CDL messages (identity, requests, scene write, commit, global write, acks;
  in place since 7b765c8) instead of the `korg-measured` driver; data reply
  pattern for the receive (header and packed payload)
- [ ] korg: Strings XML from the vendor application (KORG repository)
  → control lists and value labels for all 14 devices
- [x] platform: `gen-korg-cdl` (now in KORG_Kontrol_Editor): a generated base for all 16 devices
  (device, decoded model id, scene and global block sizes, file types,
  thumbnails, a MIDI transport found by identity scan); the Studios' hand
  files override it; shared facts split into `common.inc.kdl` (decoded) and
  `studio.inc.kdl` (measured) (a932023)
- [x] platform: labels, enums, panel pictures, thumbnails and list icons in
  the generator, from the resource mapping: names come from the vendor's resource table; 13 devices with labels and panels, 16 with images (0f09311)
- [ ] korg: turn generated value-label lists into enums once getters give
  the wire codes (display order is not code order; X14802 jog: 4 labels, 7
  codes, found by the KORG lane in d9aa7fd)
- [ ] korg: parameter schemas for the 12 devices without one (getters)
- [ ] korg: exact click areas from the component code
- [x] korg: nanoKEY Studio (X14800) global write measured: same header
  `14 01 00 51`, ack 23, no commit; write and re-receive byte-identical;
  scene-set write verified unchanged (KORG aaa4f36). Descriptor
  `descriptors/korg/nanokey-studio.ctrl.kdl`; shared facts in
  `descriptors/korg/common.inc.kdl`
- [x] user+korg: global write captured over BLE (data header `14 01 00 51`,
  ack 23, no commit) and implemented in korg-app, Slint, TUI and CLI;
  byte-identical round trip on X14802 (KORG eba516b)
- [x] user+korg: nanoKEY Studio over USB verified with a cable: input
  "nanoKEY Studio KEYBOARD/CTRL", output "nanoKEY Studio CTRL"; receive and
  unchanged write byte-identical (KORG fbf74bc; ctrl-editor 8c31b8d). CDL
  got `port-match-in`/`port-match-out`. The identity device id is 0x0F
  over USB too: a stored setting, not tied to the transport
- [ ] user+korg: check whether the nanoKONTROL Studio's USB input port also
  has its own name; its descriptor uses one `port-match` today
- [x] platform: overlapping port matches are a validation error (CDL §5)
  (1d62935)
- [ ] korg: nanoKEY Studio calibration (`X14800_Calib`). blocked: capture
- [x] korg: `docs/rewrite/72-feature-parity.md` updated with the Slint
  state (KORG eba516b)
- [ ] korg: global schema; the editors show the global block's raw length
  only

### DB-1

- [ ] db1: descriptor reviewed against `db1-protocol`; `db1-cli` test
  vectors pass through `ctrl-cli check`
- [ ] db1: the instrument enum (0..12) named in the descriptor from
  `state.rs`
- [ ] db1: restore points and tx log move to `ctrl-engine`
- [ ] db1: hardware checklist through the platform (read, write, restore)
  equal to the `db1-cli` run in `docs/HANDOVER.md`
- [ ] db1: retire `db1-tui` (REPL) in favour of `ctrl-tui`

### Evolution UC-33

- [x] platform: `descriptors/evolution/uc33.ctrl.kdl` (214eee4), from the
  UC33-Editor repo: original UC-33, USB 0a4d:00f5 release 1.01, no
  identity reply, no SysEx dump; messages classify what the controls send
- [ ] uc33: bump UC33-Editor's ctrl-editor pin and drop its channel filter
  workaround now that reply ranges exist
- [ ] uc33: replace `uc33-core/src/recipe.rs` with `edit_guide` on the
  descriptor's `edit "preset"` / `edit "global"`, and run the `tour`
  procedure for the learn pass; then drop `labels "en" list "keys"`
- [ ] user: check the UC-33 guide and tour on the unit (the rules come from
  the UC-33e Advanced Guide, `inferred`)

### Faderfox UC4

- [x] platform: `descriptors/faderfox/uc4.ctrl.kdl` from the vendor manual
  and firmware 02.05 (setup dump, stored layout, value codes decoded; no
  unit measured). CDL got `field … bits=` and the `nib3` records (§4.3,
  §8.1, 63487b5)
- [x] user: evidence in the private repository Faderfox_UC4 (local; the
  owner creates the GitHub remote)
- [x] user: USB ids 2256:2004, port "Faderfox UC4"; SndA and Sndc captured
  on firmware 02.03: the dump format and the layout are measured
- [x] platform: dumps as data, no driver: `records` (addressed chunks in
  one SysEx), `derive` with `x.sum` (the 16-bit sum), block `address` and
  `chunk` (CDL §6.7, §8.3); both UC4 dumps from the unit decode and
  re-encode byte for byte (b71345f)
- [x] user+platform: write measured on 02.03: a one-setup dump (kind 2)
  into SE16, read back, restored; final dump byte-identical. A one-chunk
  kind-3 dump was not stored (`dump-out-all` stays decoded)
- [x] platform: UC4 `write-setup` (kind 2) as CDL: `await` for dumps the
  user starts, `records-from` builds the records from blocks, backup as
  restore point, verify against both copies; `ctrl-engine` (first slice)
  and `ctrl-cli run`; tests rebuild the measured file byte for byte
- [x] user+platform: `write-setup` run end to end on the unit through
  `ctrl-cli run` (SE16 renamed and restored, read-backs byte-identical)
- [x] user+platform: a full kind-3 write measured (430 chunks, `pace=20`):
  SE16 renamed and the backup restored byte for byte; `restore-all`
  procedure (block `dump-area`) builds the same bytes
- [x] user+platform: `restore-all` run on the unit through `ctrl-cli run`
  with `--load dump=<backup>`; read-back byte-identical. A write whose
  `records-from` data is not loaded now stops before its first step
- [x] user: the group-name character table, codes 0..38 (display order on
  the unit, four codes confirmed by SndA)
- [ ] analysis: the firmware page check value (the bootloader is not in the
  update file); needed only for a modified image
- [x] user: firmware update 02.03 → 02.05 with the vendor's file
  (2026-10-28); setups kept, dumps differ only in the version tag
- [x] user+platform: `write-setup` repeated on 02.05 (SE16 dAtA and back,
  read-backs byte-identical; the send carries the version the unit reports)
- [x] user+platform: `restore-all` repeated on 02.05 (read-back
  byte-identical to the backup)
- [x] user+platform: pacing vs completeness for kind 3: on 02.05 one chunk
  (paced or not) and a full unpaced dump are all stored; the 02.03
  one-chunk failure is the old firmware's receiver
- [ ] user: check the UC4 edit-mode guide on the unit (`edit "controls"`,
  `"fader9"`, `"names"`, `"global"`, from the manual's Edit mode and Setup
  mode sections, `inferred`); the guided read and write procedures keep
  their measured steps

### TEC

- [x] korg (OMP): TEC USB gaps closed from the analysis and a read-only
  trace (TECcontrol fb97e8b, findings); applied to
  the descriptor (1d62935)
- [x] platform: TEC descriptor: USB ids with bcd from HANDOVER.md, 8-byte
  read replies, curve pages driver-owned (3ae75df)
- [ ] tec: `ctrl-hid-iokit` from `tec-device/src/macos.rs` (Seize, report
  id 0, 8-byte frames)
- [ ] tec: Linux configuration over hidraw (`ctrl-hid-hidapi`, hidapi
  Linux-native backend), udev rule `70-tecontrol-breath.rules` (uaccess),
  `tec-midi` on `ctrl-midi`; first write staged with a usbmon capture
  (docs/LINUX.md §6)
- [ ] tec: hub (one thread per device, keyed by serial) in `ctrl-engine`
- [ ] tec: `tec-curve` driver; byte-exact against the captured curve
  tables
- [ ] tec: `arm` gate replaces the `unsafe-commands` runtime arm; the Cargo
  feature stays as a build-time guard
- [ ] tec: live meters (poll at about 19 Hz) in GUI and TUI
- [ ] user: first platform write to a TEC controller, with the tracer
  running

## P5 Firmware update

- [ ] db1: decode the `.obd` image (`DB-1 findings` §5, §8) and
  capture the vendor updater on hardware. blocked: user, hardware with a
  spare unit
- [ ] platform: `firmware` runner with the CDL §12 checks (image, version,
  recovery text, arm)
- [ ] tec: a debug-interface firmware read is out of scope unless the user
  asks for it

## P6 Retire the old front ends

- [ ] each lane: the old editor's hardware checklist passes on the platform
- [ ] each lane: old product crates removed from their repos; the repos keep
  analysis, findings and captures

## PM Shared MIDI monitor ([MONITOR.md](MONITOR.md))

- [x] platform: M1 `ctrl-monitor` core: tap, parser, store, collector,
  generic names (b1211e3)
- [x] platform: M2 describers (CDL by descriptor, reply to the last
  request), view model (filters, folding, chips, alerts, details), capture
  filter
- [x] platform: M3 `ctrl-monitor-tui` (ratatui pane, key map, demo) and
  `ctrl-monitor-slint` (pane, window, demo, headless test on the Slint
  testing backend, Weston in `tools/linux-check`). The Slint crate needs
  Rust 1.92 (slint 1.18); the rest of the workspace keeps 1.85
- [x] platform: view-model gaps the Slint pane found, fixed: alerts counted
  once, `Scroll` for the wheel, the selection kept in a smaller window,
  `SetTypes`, `SetTime`, stable `Row::stripe`
- [x] platform: the pane is embeddable (owner's choice: inline, not a
  separate window): `monitor_glue!()` expands the glue in the editor's
  crate, `@ctrl-monitor/monitor.slint` through `DEP_CTRL_MONITOR_SLINT_UI`
  (`links` metadata); `tools/monitor-embed-check` proves it. One pane per
  window while the bridge is a global
- [x] platform: `ctrl-monitor-midi` and `ctrl_midi::watch` /
  `virtual_input` (§6.1, §6.2): raw, unjoined bytes into a tap; an
  editor's sends recorded (`RecordedOutput`). Proved on JACK (jackd2 and
  PipeWire, `tools/linux-check midi`: a third client watches two others)
  and on CoreMIDI (virtual input). The container has no ALSA sequencer:
  the ALSA watch is untested there
- [x] korg: M4, the KORG monitor on the shared component, inline in both
  front ends; the panel control and scene as a describer; editor sends
  recorded (KORG PR #5)
- [ ] platform: gaps from M4: `CdlDescriber` cannot be rebound once the
  collector owns it (no lookup from a descriptor `class` to its
  `DeviceIdx`); `MonitorController` has `stop()` but no restart (a hidden
  pane keeps refreshing); panel lighting stays in the editor (MONITOR.md
  §9 said view model): decide or reword
- [ ] user: create the private repo `lb-midi-tools`; M5 lb-monitor, M6
  lb-librarian
- [ ] platform: M7 thru (virtual pairs), Windows MIDI Services backend in
  `ctrl-midi`
- [ ] platform: M8 RFC 6295 / AppleMIDI producer and network thru
- [ ] zigtact: M9 `network-midi` produces into `ctrl-monitor` (mini-spec
  pending the owner's approval); the egui Mesh Monitor pane is retired
- [ ] platform: M10 UMP
- [ ] platform: later, spying without thru: CoreMIDI driver plug-in, Linux
  usbmon producer (§6.5)
