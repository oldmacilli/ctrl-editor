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

Last updated: 2026-10-20 (ctrl-cdl landed).

## P0 Agree the concept

- [x] user: GitHub repo `oldmacilli/ctrl-editor` (private) created and
  pushed (196eb94)
- [ ] user: approve the concept
- [ ] korg: review CONCEPT §3 and §4 (crate moves out of the KORG repo)
- [ ] db1, tec: review CONCEPT and their descriptors
  (`descriptors/oceanbeach/db1.ctrl.kdl`, `descriptors/tecontrol/breath.ctrl.kdl`)
- [ ] zigtact: agree to `lbmt` as a pinned git dependency and to the
  wide-integer request (P3)

## P1 CDL toolchain

- [x] platform: CDL 0.1 draft (`docs/CDL.md`)
- [x] platform: example descriptors for DB-1, TEC, nanoKONTROL Studio and a
  firmware template; all four parse as KDL 2.0 (`tools/cdl-parse`)
- [x] platform: the DB-1 LBMT example parses, compiles and runs in
  zigtact's `lbmt` crate
- [x] platform: KORG lane review applied: model id `00 01 37`, device id
  from the identity reply, preferred transport, procedure IDs aligned,
  driver-owned bytes on record (`owns`), path grammar for `verify` (CDL
  §4.4), protocol-regression risk
- [x] platform: `ctrl-cdl` crate: KDL → typed IR (serde), with errors that
  point at line and column; includes merge (`*.inc.kdl`); templates, paths
  and ranges parsed; all descriptors load (e44a6e1)
- [x] platform: slice placeholders `{curve[6] pad=7F}`; the TEC curve pages
  are CDL again, no driver step (COMMIT)
- [x] platform: validator (`ctrl_cdl::validate`): references and paths,
  sizes and ranges, `status` rules (CDL §4.1), write policy (§11), port
  overlap (§5), SysEx data bytes, HID frame sizes, scan messages (COMMIT)
- [x] platform: CDL §5.1 `scan` and `listen` for DIN-only devices; DB-1
  found by scanning with its revision request (COMMIT)
- [ ] platform: template codec: encode and decode `send`/`reply` (CDL §6),
  checksums (§6.1)
- [ ] platform: test-vector runner; DB-1 vectors from `db1-protocol` tests,
  TEC vectors from the HID captures
- [ ] platform: `ctrl-cli check | explain | trace` (CDL §16)
- [ ] platform: tree-sitter query file, LBMT injection
- [ ] platform: `cdl-schema.md`: one table per node with every property,
  generated from the IR types

## P2 Prove CDL on the DB-1; KORG stays intact

Order (CONCEPT §4, revised after the KORG lane's review): new small crates
first, DB-1 on them, KORG behind adapters. No rename of working crates.

- [ ] platform: `ctrl-transport`: MIDI trait (shape of `korg-midi` and
  `db1-midi`), `HidDevice` trait (shape of `tec-device`), mocks that replay
  captures
- [ ] platform: `ctrl-engine`: procedure runner (exchange, repeat, verify
  with paths, gates, restore points, tx log from `db1-session`)
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
  checksums in LBMT. blocked: zigtact owner
- [ ] zigtact: check the spec examples against the parser (`THEN` and
  nested translators in `lbmt-language-spec.md` may not parse)
- [ ] platform: translator panel in GUI and TUI (enable, edit, monitor)

## P4 Device packs

### KORG

- [ ] korg: nanoKONTROL Studio descriptor complete: SysEx exchanges as CDL
  messages instead of the `korg-measured` driver; identity reply as a CDL
  pattern that captures the device id
- [ ] korg: Strings XML from the vendor application (`the local capture archive`)
  → control lists and value labels for all 14 devices
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
  unchanged write byte-identical (KORG fbf74bc; ctrl-editor 8f9aaea). CDL
  got `port-match-in`/`port-match-out`. The identity device id is 0x0F
  over USB too: a stored setting, not tied to the transport
- [ ] user+korg: check whether the nanoKONTROL Studio's USB input port also
  has its own name; its descriptor uses one `port-match` today
- [x] platform: overlapping port matches are a validation error (CDL §5)
  (COMMIT)
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

### TEC

- [x] korg (OMP): TEC USB gaps closed from the analysis and a read-only
  trace (TECcontrol fb97e8b, findings); applied to
  the descriptor (COMMIT)
- [x] platform: TEC descriptor: USB ids with bcd from HANDOVER.md, 8-byte
  read replies, curve pages driver-owned (e44a6e1)
- [ ] tec: `ctrl-hid-iokit` from `tec-device/src/macos.rs` (Seize, report
  id 0, 8-byte frames)
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
