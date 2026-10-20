# CDL: Controller Description Language

Version 0.1, draft, 2026-10-20. Concept: [CONCEPT.md](CONCEPT.md).

CDL describes a hardware controller to the editor: how to find it, what it
stores, how its bytes are laid out, which messages read and write it, how to
patch it live over MIDI, and how to update its firmware. One file per
device, `descriptors/<vendor>/<model>.ctrl.kdl`.

CDL covers four areas:

| Area | Section | Example device |
|---|---|---|
| 1. Classic MIDI patching (CC, NRPN, RPN, program change, notes, live rules) | §7 | DB-1 drawbars, TEC sensors |
| 2. SysEx patching (dumps, set/ack chains, packing, checksums) | §8 | KORG scenes, DB-1 config |
| 3. USB parameter patching (HID or bulk frames) | §9 | TEC breath controller |
| 4. Firmware update procedure | §12 | template only; no device decoded yet |

## 1. Syntax: KDL v2

CDL is a schema on top of [KDL 2.0](https://kdl.dev). Reasons:

- Nodes are ordered, which procedures need. TOML has no ordered mixed
  sequences, and YAML has typing traps (`no`, `0x7F`, `1e3`).
- Multi-line and raw strings carry LBMT snippets without escaping
  (`#"…"#`, `"""…"""`).
- It has comments, which evidence notes need, and slashdash (`/-`) to
  disable a node.
- Type annotations such as `(u14)8192` fit wire values.
- There are Rust parsers (`kdl` crate, v2 support) and a tree-sitter
  grammar, so editors get highlighting.

```kdl
// node  argument   property=value   { children }
field "cc" offset=2 type="u7" status="measured"
```

Hex numbers (`0x7F`) are numbers. Byte strings for messages are quoted
templates (§6), because a bare `7F` is not a valid KDL value.

## 2. File structure

```kdl
cdl 0.1                                   // first node: language version
device "oceanbeach.db1" { … }             // identity, vendor, links
identify { … }                            // how to recognise it (§3)
transport "aux" kind="midi" { … }         // one or more (§5)
types { … }                               // enums and value types (§4.2)
block "drawbar" { … }                     // stored data and its fields (§4)
panel { … }                               // controls and bindings (§13)
midi-map { … }                            // classic MIDI patching (§7)
message "set-drawbar" { … }               // SysEx or HID message templates (§6)
procedure "read-all" { … }                // ordered exchanges (§11)
firmware { … }                            // update procedure (§12)
lbmt "organ-remap" { … }                  // live rules (§10)
driver "tec-curve"                        // Rust escape hatch (§14)
test-vector "set-drawbar-0" { … }         // bytes that must round-trip (§15)
```

`include "common.inc.kdl"` merges shared definitions, with the path
relative to the including file. Shared files are named `*.inc.kdl` and have
no `device` node; only `*.ctrl.kdl` files are device descriptors. Nodes merge by kind and name: a local node
with the same kind and name as an included one adds its properties and
children, and a child with the same kind and name replaces the included
child. So a device can extend a shared `driver` with its own `owns`
entries. Nodes without a name (`types`, `identify`, `send`, `vendor`) merge
by kind alone, and `value` by its number. A procedure or stage's steps are
an ordered list: local steps replace the included list as a whole. Two nodes with the same kind and name in one file are an error.

## 3. Identity

```kdl
device "tecontrol.breath" {
    vendor "TEControl"
    model "USB MIDI Breath Controller"
    aka "TECMidiBreathCtrl"
    manual "TEC manual v5"                         // a reference, not a file
}
identify {
    usb vid=0x2B06 pid=0x0018 bcd=0x0500 status="measured"   // bcd: firmware version
    midi-port match="Breath" direction="both"
    // the universal identity reply (MIDI 1.0), for devices that answer it
    sysex-identity reply="F0 7E {ch} 06 02 {manufacturer} {family:u14le} {member:u14le} {version*}"
    hid usage-page=0x0C report-size=8
}
```

The matchers combine with OR; the strongest match wins. A `hid` usage page
alone is weak (0x0C also matches media-key interfaces), so a HID device
names its USB ids. `bcd` pins the firmware version the descriptor was
measured on; on any other version the engine reads but does not write.
`key "usb-serial"` keys the device hub (several controllers at once).

## 4. Evidence, blocks and fields

### 4.1 Status

Every node may carry `status` and `evidence`:

| status | Meaning | The engine… |
|---|---|---|
| `measured` | seen on hardware or in a capture | uses it |
| `decoded` | read from the vendor binary or data, not yet seen on hardware | uses it; reads only, until measured |
| `inferred` | reasoned, not seen | shows it marked; sends it only after the user opts in per session |
| `unknown` | placeholder | never sends a message that depends on it |

`evidence` is free text pointing at the proof: a capture file, an analysed
address, a doc section. The rule "do not guess SysEx values" becomes a
check (`ctrl_cdl::validate`, run by `ctrl-cli check`):

- A node marked `unknown` is disabled and listed. A procedure, firmware
  stage or scan that is not `unknown` but sends an `unknown` message, frame
  or block path is an error: measure the dependency, or mark the node
  `unknown` too.
- A `write` or `commit` procedure that sends a message or frame that is only
  `decoded` is an error.
- Every message or frame on a send path, and every driver `owns` entry,
  carries a `status`.
- `inferred` dependencies are listed: they need the user's opt-in.

### 4.2 Types

```kdl
types {
    enum "polarity" { value 0 "Normal"; value 1 "Reversed" }
    enum "msg-type" { value 0 "CC"; value 1 "Aftertouch"; value 2 "Pitch Bend Up"
                      value 3 "Pitch Bend Down"; value 4 "Pitch Bend Both" }
    // display = stored + 1; the original shows channel 7 for a stored 6
    int "channel" storage="u7" range="0..=15" display="{v + 1}"
}
```

Built-in storage types: `u7`, `u8`, `u14le`, `u14be` (two 7-bit bytes),
`s7` (offset 64), `u16le`/`u16be`, `bits(n)` inside a byte, `ascii(n)`
(space padded), `bool`. Ranges are inclusive (`0..=127`). `display` and
`parse` are LBMT expressions (§10) or the built-ins `+1`, `-64`.

### 4.3 Blocks

A block is a stored data unit: a scene, a global area, one drawbar.

```kdl
block "scene" size=560 count=5 status="measured" evidence="korg-devices measured_layout(0x137)" {
    field "name" offset=0 type="ascii(16)"
    // offsets from korg-model JogConfig::decode
    group "jog" offset=512 {
        field "channel" offset=0 type="channel"
        field "type" offset=1 type="jog-type"
        field "acceleration" offset=2 type="jog-accel"
        field "cw-cc" offset=4 type="u7" visible-when=#"type <= 1"#
        field "ccw-cc" offset=5 type="u7" visible-when=#"type <= 1"#
    }
    array "knob" count=8 status="unknown"     // stride and offset not decoded
}
```

`array` repeats fields with a stride. `visible-when` hides fields as the
original editor does (the jog panel shows CW/CCW CC only for Inc/Dec). A
`field` without `offset` in a block whose size is unknown is allowed only
when the block is never written.

### 4.4 Paths

Procedures, `verify`, `restore-point` and bindings name data with paths:

```text
path   = block [ "[" index "]" ] { "." name [ "[" index "]" ] }
index  = expr | expr ".." expr          // a range is half-open
expr   = an LBMT expression over procedure arguments, e.g. i, s, p * 6
```

Examples: `scene[scene].jog.channel`, `sensor[s].shaping`,
`sensor[s].curve[p * 6 .. p * 6 + 6]`. A block without an index means all
of its elements. `compare=` takes paths separated by spaces; without
`compare`, `verify` compares everything the procedure wrote.

### 4.5 Parameters

Parameters are what the UI shows. Most map 1:1 to a field and need no extra
node. Add `param` when the label, group or UI differs:

```kdl
param "drawbar.*.cc" label="CC" group="Drawbar" ui="spin"
param "global.instrument" label="Instrument" ui="combo" min-firmware="x.10"
```

## 5. Transports

```kdl
transport "main" kind="midi" port-match="DB-1" role="performance"
transport "aux"  kind="midi" port-match="DB-1 AUX" role="config"
transport "hid"  kind="hid" usage-page=0x0C frame=8 seize=#true
transport "bulk" kind="usb-bulk" interface=1 ep-out=0x02 ep-in=0x81 status="unknown"
```

Messages and procedures name the transport they use. `role="config"` marks
the port that the write policy guards. When a device has more than one
config transport (KORG over USB and BLE), one carries `preferred=#true`,
and the user's port choice overrides it. Without a preferred transport the
engine asks the user to choose and never picks one silently.

A MIDI transport without a match is reached through a port the user picks,
and the choice is saved with the device: the DB-1 has 5-pin DIN ports only,
so its port is named after the user's interface.

`port-match` is a case-sensitive substring of the port name, used for both
directions. When the input and output ports have different names, use
`port-match-in` and `port-match-out` instead; a transport has either
`port-match` or both directional matches. The nanoKEY Studio over USB is
the case in point: input "nanoKEY Studio KEYBOARD/CTRL", output "nanoKEY
Studio CTRL".

```kdl
transport "usb" kind="midi" role="config" port-match-in="nanoKEY Studio KEYBOARD/CTRL" port-match-out="nanoKEY Studio CTRL"
```

The matches of one device's transports must not overlap, per direction: "nanoKEY Studio" also matches
"nanoKEY Studio Bluetooth", so the USB transport must name its port fully.
`ctrl-cli check` reports overlapping matches, and the engine refuses a port
that more than one transport matches.

A transport can capture values from the identity exchange for later
messages: `capture "device-id" from="identity-reply.device"`.

### 5.1 Finding a device without a port name: scan and listen

A device with 5-pin DIN ports only (the DB-1) sits behind any MIDI
interface or network session, so no port carries its name. `scan` names a
read message whose reply identifies the device:

```kdl
transport "aux" kind="midi" role="config" scan="revision"
```

The engine then:

1. sends the scan message on one MIDI output at a time, and listens on
   every input for its `reply` pattern, up to the message's `timeout`;
2. pairs that output with the input the matching reply arrived on, and
   checks `ok-when` (the DB-1 answers `product == 0`; 1 is its bootloader);
3. asks the user when several pairs answer, and saves the choice with the
   device, as for a port picked by hand;
4. skips ports already bound to a device, and scans only when the user
   starts it (a scan sends bytes to every device on every port).

The scan message must be a read (`kind` absent or `read`), measured or
decoded, with a `reply`. Prefer a message that other devices ignore: SysEx
with the vendor's manufacturer ID, or the universal identity request
`F0 7E 7F 06 01 F7`, which every device that implements it answers. On a
MIDI network with several devices of one model, the identity replies carry
each device's id (byte 2, `capture "device-id"`), so each reply becomes its
own device.

`listen` is passive: bytes the device sends by itself on an input, such as
Active Sensing (`FE`, about every 300 ms) or a known CC. It marks an input
as live, so the engine can scan it first or show it to the user. Active
Sensing carries no device id, so `listen` never identifies a device alone:

```kdl
identify { listen "FE" status="inferred" }
```

## 6. Message templates

A template is a string of bytes and placeholders:

| Element | Meaning |
|---|---|
| `F0`, `7F` | literal byte (hex, two digits) |
| `{name}` | one byte from a field or argument, encoded with the field's storage type; without a type it is `u7` on MIDI transports (must fit `0..=127`) and `u8` on USB transports |
| `{name:type}` | encode with a storage type, e.g. `{value:u14le}` → 2 bytes |
| `{name[n]}`, `{name[n]:type pad=7F}` | `n` elements of an array, from the step's `from=` slice; elements past the array's end are sent as `pad` (without `pad`, an error) |
| `{block:packed7}` | the whole block, 8→7 bit packed (KORG) |
| `{block:raw}` | the whole block as is |
| `{sum:roland(from,to)}` | checksum over a byte range (§6.1) |
| `{=lbmt expr}` | computed byte (§10) |
| `..` | in a reply pattern: any one byte |
| `{name*}` | in a reply pattern: capture the rest up to the terminator |

```kdl
message "set-drawbar" transport="aux" status="measured" evidence="db1-protocol/src/message.rs set_drawbar" {
    send "F0 09 00 00 00 02 03 {index} {polarity} {cc} {channel} F7"
    reply "F0 09 00 00 00 03 03 {status} F7" timeout=500
    ok-when #"status == 0"#
}
```

`send` encodes; `reply` decodes and captures. A reply that does not match
within `timeout` milliseconds is an error, and so is one that matches but
fails `ok-when`. Error texts come from an enum:
`errors "status" { value 7 "Non-volatile write error" }`.

A placeholder whose content is a name (letters, digits, `_`, `-`, `.`, `$`;
a letter, `_` or `$` first) is a field or argument; anything else, such as
`{0x40 | device-id}`, is an LBMT expression, the same as `{=…}`. `..` and
`{name*}` are allowed in replies only.

### 6.1 Checksums

Built in: `sum7` (two's complement of the sum, low 7 bits), `roland`
(`(128 - sum % 128) % 128`), `xor7`, `fnv1a32` (files only). Ranges are byte
indexes into the message, and negative values count from the end. Anything
else is a `compute` LBMT snippet or a driver.

## 7. Classic MIDI patching

`midi-map` states what the device sends and receives in performance use,
and which of those settings are themselves parameters:

```kdl
midi-map transport="main" {
    // drawbar n sends CC drawbar.n.cc on channel drawbar.n.channel
    source "drawbar" count=9 kind="cc" cc="{drawbar.$i.cc}" channel="{drawbar.$i.channel}"
        range="0..=127"
    source "button" kind="cc" cc="{button.cc}" channel="{button.channel}"
}
```

`kind`: `cc`, `cc14` (MSB/LSB pair), `nrpn`, `rpn`, `note`, `pc`, `pb`,
`at`, `poly-at`, `clock`, and `from-field` when the kind is itself a
setting named by `message=` (the TEC sensors send CC, aftertouch or pitch
bend by setting). The editor uses `midi-map` to:

- show live values (meters, the TEC sensor view);
- send live parameter changes when a device takes CC or NRPN for its
  settings (`sink` nodes, same syntax as `source`);
- run `lbmt` translators between the device and the computer (§10.3).

## 8. SysEx patching

SysEx is a set of `message` nodes (§6) plus the `procedure`s that use them
(§11). Two helpers make vendor framing short:

```kdl
// from korg-sysex: F0 42 4d, where d is the device id read from the
// identity reply (a stored device setting, not tied to the transport and
// not a MIDI channel), then the model id
// (one byte below 0x80, otherwise 00 hi lo, so 0x137 is 00 01 37), then
// the command
sysex-frame "korg-native" status="decoded" evidence="KORG crates/korg-sysex push_model, encode" {
    prefix "F0 42 {0x40 | device-id} {model:korg-model}"
    suffix "F7"
}
message "scene-dump-request" frame="korg-native" { send "{cmd.scene_data_request} {scene}" }
```

`korg-model` is a storage type of the `korg` device pack. Vendor pack types
like this one are allowed when they are pure functions with test vectors.
Dump messages carry a block with `{block:packed7}`; the engine splits and
joins multi-message dumps when the block says `chunk=…`.

## 9. USB parameter patching

HID and bulk frames use the same templates, with 8-bit bytes. Fixed-size
frames are padded with `00`:

```kdl
message "write-meta" transport="hid" status="measured" evidence="TECcontrol HANDOVER.md:399-434, hid-trace-2026-10-13" {
    send "{0x20 + sensor} {cc} {channel} {msg} 09 {attack} {decay} 00"
    reply "00 00 00 00 00 00 00 00"           // every write is acked with 8 zeros
}
message "poll" transport="hid" { send "00"; reply "{breath} {bite} {nod} {tilt} .. .. .. .." }
```

`{0x20 + sensor}` is a computed byte (an expression in braces, §10.1). HID
reports have no SysEx framing; the transport sets the report ID (none for
TEC). A send must fit the frame, and a reply must fill it.

An array goes out in pages with a slice placeholder. The TEC curve is 32
values in 6 pages of 6, and the last page is padded with 7F:

```kdl
message "write-curve-page" transport="hid" kind="write" {
    send "{0x40 + s} {start} {curve[6] pad=7F}"
    reply "00 00 00 00 00 00 00 00"
}
procedure "write-sensor" kind="write" {
    …
    repeat "p" "0..=5" {
        exchange "write-curve-page" start="{p * 6}" from="sensor[s].curve[p * 6 .. p * 6 + 6]"
    }
}
```

## 10. LBMT inside CDL

CDL embeds zigtact's LBMT. Snippets run in the `lbmt` interpreter, in a
sandbox: no `key`, `mouse` or `timer` actions inside transforms, and a step
limit.

### 10.1 Expressions

A short expression in braces or `#"…"#` is one LBMT expression. Field names
bind to LBMT globals by name; `v` is the value at hand:

```kdl
int "channel" storage="u7" range="0..=15" display="{v + 1}" parse="{v - 1}"
```

### 10.2 Compute and checksum blocks

```kdl
compute "upper-half" in="pp" out="qq" #"""
    qq = 0
    IF pp >= 64
    qq = pp - 64
"""#
```

A compute block holds LBMT rule statements; the engine wraps them in a
translator. `pp`..`zz` are locals: inputs bind to the locals listed in
`in=`, and the result is read from `out=`. As in Bome, `IF` guards the next
statement only. Until LBMT has wide integers (TODO P3), a
compute block must stay within 7-bit values; 14-bit and checksum work uses
the built-ins.

### 10.3 Translators: live MIDI rules

A `lbmt` node is a complete LBMT program that the engine runs between two
transports while the editor is open. The editor shows it, can enable it, and
saves it with the document:

```kdl
lbmt "organ-remap" from="main" to="virtual:DB-1 Organ" enabled=#false #"""
@preset "DB-1 organ remap"
@translator "drawbar to organ" enabled
  @trigger midi cc
  RULE:
    ss = qq
    IF qq < 9
    ss = qq + 12
  @action midi cc cc#=ss value=rr
@end
"""#
```

In a MIDI trigger, LBMT binds `pp` = channel, `qq` = note or CC number and
`rr` = velocity or value (`lbmt` interpreter). The example was parsed,
compiled and run with zigtact's `lbmt` crate: CC 3 comes out as CC 15, and
CC 20 passes through unchanged.

LBMT syntax is defined by zigtact `docs/notes/lbmt-language-spec.md` and the
`lbmt` crate's parser tests; CDL does not redefine it. `ctrl-cli check`
parses and compiles every snippet.

## 11. Procedures and the write policy

A procedure is an ordered list of steps:

| Step | Meaning |
|---|---|
| `send "msg" arg=…` | send a message, no reply expected |
| `exchange "msg" arg=…` | send and wait for its reply |
| `repeat "i" "0..=8" { … }` | loop with a counter (no other loops exist) |
| `read-into "block" from="capture"` | store reply captures into a block |
| `delay 20` | milliseconds |
| `verify "procedure" compare="path…"` | run the read procedure and compare the paths (§4.4); report the first differing byte |
| `gate "arm"` / `gate "confirm"` | write policy (below) |
| `restore-point "block…"` | snapshot before writing |
| `require #"firmware >= 0x0210"#` | precondition, LBMT expression |
| `call "procedure"` | sub-procedure |
| `driver "name" step="…"` | Rust step (§14) |

```kdl
procedure "write-all" kind="write" transport="aux" {
    gate "confirm" text="Write 9 drawbars, button and global settings to the DB-1?"
    restore-point "drawbar" "button" "global"
    repeat "i" "0..=8" { exchange "set-drawbar" index="{i}" from="drawbar[i]" }
    exchange "set-global" from="global"
    exchange "set-button" from="button"
    verify "read-all"
}
```

The write policy is the same for every device:

| Level | Applies to | Gates |
|---|---|---|
| read | `kind="read"` | none |
| write | `kind="write"` (RAM or settings) | dry run shows the bytes; `confirm`; restore point; `verify` when a read exists |
| commit | `kind="commit"` (flash, permanent) | the above plus `arm` (a runtime arm that expires after 10 minutes, as in TEC) |
| firmware | `firmware` | the above plus the image checks in §12; never on battery-low; no other job on the device |

`ctrl-cli` sends nothing without `--apply` (as `db1-cli` does). The GUI and
TUI show the confirm dialog with the bytes.

## 12. Firmware update procedure

No device in scope has a decoded update procedure. The DB-1 has a vendor
utility and an encoded `.obd` image; TEC has none. The node is defined now
so that a captured procedure can be written down without new syntax:

```kdl
firmware status="unknown" {
    image format="obd" magic="30 42 44 20" driver="db1-obd"   // decode in a driver
    version-from "revision-reply" field="major.minor"
    require #"battery_ok"#
    stage "enter-bootloader" { exchange "enter-bootloader"; expect-identity product=1 }
    stage "erase" kind="commit" { exchange "erase" timeout=5000 }
    stage "transfer" kind="commit" {
        repeat "chunk" image-chunks size=… { exchange "write-chunk" data="{chunk:packed7}" }
    }
    stage "verify" { exchange "image-crc"; ok-when #"crc == image.crc"# }
    stage "reboot" { send "reboot"; expect-identity product=0 timeout=10000 }
    recovery "If the device stays in the bootloader, run the update again; the bootloader is not erased."
}
```

Rules:

- `ctrl-cli check` refuses a `firmware` node whose stages contain
  `unknown` messages.
- The image is checked before the first byte is sent: format, magic, size,
  the driver's integrity check, and `version-from` against the device's
  reported version (no silent downgrade).
- `recovery` text is required and is shown before the arm gate.
- Each stage is logged in the tx log; a failure stops the update and shows
  the recovery text.

## 13. Panels

The panel is `korg_app::panel` written as data:

```kdl
panel picture="550x329" source="editor panel picture, measured by eye" {
    control "jog" kind="jog" label="Jog Wheel" group="Jog Wheel" box="36,199,92,92" bind="scene.jog"
    repeat "n" "1..=8" {
        control "knob{n}" kind="knob" group="Knob" box="{135.5 + 44.5 * n},146,28,28"
    }
}
```

`bind` names a block, group or field; an unbound control shows "not decoded
yet". `kind`: `knob`, `fader`, `button`, `pad`, `key`, `touchpad`, `jog`,
`meter` (live value from `midi-map` or a poll).

## 14. Drivers

```kdl
driver "tec-curve" provides="block:curve" from="param:bending param:symmetry"
driver "korg-nktrl-file" provides="file:nktrl_st_set file:nktrl_st_data"
```

`provides` names the procedures, blocks or files the driver implements,
using the same IDs as the descriptor (`procedure:receive-scene-set` needs a
`procedure "receive-scene-set"` node whose step calls the driver). A driver
that sends bytes the descriptor does not yet describe lists them as
`owns` children with their evidence, so the bytes are on record even before
they become CDL messages:

```kdl
driver "korg-measured" provides="procedure:write-scene" {
    owns "scene-write-header" bytes="01 05 00 40" status="measured" evidence="korg-devices measured_layout(0x137)"
}
```

A driver is a Rust type that implements `ctrl_engine::Driver`. It is
registered in the device pack's crate and named here, so the descriptor
stays the single list of what the device can do.

## 15. Test vectors

```kdl
test-vector "set-drawbar-0" message="set-drawbar" status="decoded" evidence="db1-protocol set_drawbar" {
    args index=0 polarity=0 cc=12 channel=0
    bytes "F0 09 00 00 00 02 03 00 00 0C 00 F7"
}
test-vector "scene-1" block="scene" file="the local capture archive" sha256="…"   // external, opt-in
```

`ctrl-cli check` encodes `args` and compares to `bytes`, and decodes `bytes`
back to `args`. External vectors run only when their file exists, like
KORG's `KORG_PRODUCTS_DIR` tests.

## 16. Tooling

- `ctrl-cli check <file>`: parse, validate the schema, compile LBMT, run
  the test vectors, list `unknown` and `inferred` items on send paths.
- `ctrl-cli explain <file> <message> k=v…`: print the bytes a message would
  send (dry run).
- `ctrl-cli trace <file> <capture>`: decode a capture with the descriptor.
- A tree-sitter query file for CDL on top of the KDL grammar, with
  injection of the LBMT grammar into `#"…"#` strings.
