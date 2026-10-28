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
| 2. SysEx patching (dumps, set/ack chains, packing, checksums, long SysEx) | §8 | KORG scenes, DB-1 config |
| 3. USB parameter patching (HID or bulk frames, report descriptor, polling) | §9 | TEC breath controller |
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
const "model" 0x137                       // named numbers for templates (§8.1)
types { … }                               // enums and value types (§4.2)
block "drawbar" { … }                     // stored data and its fields (§4)
panel { … }                               // controls and bindings (§13)
midi-map { … }                            // classic MIDI patching (§7)
sysex-frame "korg-native" { … }           // vendor SysEx framing (§8.1)
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
    image "assets/breath/thumbnail.png" role="thumbnail"   // a file next to the descriptor
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
`u21le`, `u21be` (three 7-bit bytes), `s7` (offset 64), `u16le`/`u16be`, `bits(n)` inside a byte, `ascii(n)`
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

A MIDI transport may say how much SysEx the host takes per write
(`sysex-limit`, §8.2); a HID transport carries its report descriptor and
poll (§9). `pace` is the shortest time between two frames on any transport.

Messages and procedures name the transport they use. `role="config"` marks
the port that the write policy guards. When a device has more than one
config transport (KORG over USB and BLE), one carries `preferred=#true`,
and the user's port choice overrides it. Without a preferred transport the
engine asks the user to choose and never picks one silently.
`transport="selected"` on a message means that chosen config transport; a
device that uses it needs at least one `role="config"` transport.

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
| `{name}` | one value, encoded with the field's storage type; without a type it is `u7` on MIDI transports (must fit `0..=127`) and `u8` on USB transports |
| `{name:type}` | encode with a storage type, e.g. `{value:u14le}` → 2 bytes |
| `{name:midi-id}` | a MIDI ID: 1 or 3 bytes (§8.1) |
| `{name[n]}`, `{name[n]:type pad=7F}` | `n` elements of an array, from the step's `from=` slice; elements past the array's end are sent as `pad` (without `pad`, an error) |
| `{name:packed7}` | bytes, 8→7 bit packed (§8.1) |
| `{name:raw}` | bytes as they are |
| `{name:ascii(n)}` | text, padded with spaces to `n` bytes |
| `{len:type}` | the number of body bytes after this field, up to the first checksum or the end of the body; `type` is a storage type (`u21le` for KORG Studios) or `varlen7` (§8.1) |
| `{sum:roland(from,to)}` | checksum over a range of the body (§6.4) |
| `{expr}`, `{=expr}` | a computed byte, a CDL expression (§10.1) |
| `..` | in a reply pattern: any one byte |
| `{name*}` | in a reply pattern: capture the rest up to what follows |

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
`errors "status" { value 7 "Non-volatile write error" }`. A status can be
named with a `const` (`const "write-complete" 0x21`, then
`ok-when "status == write-complete"`).

A message without a `reply` is send-only: the engine does not wait. The
validator rejects `exchange` on it and warns about `send` on a message that
has a reply, which would ignore it. `..` and `{x*}` in a reply never count
as unknown bytes for the status rules (§4.1); they only match.

A placeholder whose content is a name (letters, digits, `_`, `-`, `.`, `$`;
a letter, `_` or `$` first) is a field or argument; anything else, such as
`{0x40 | device-id}`, is a CDL expression (§10.1), the same as `{=…}`. `..`
and `{name*}` are allowed in replies only.

A message's bytes are the frame's `prefix`, the message's own template (the
**body**), then the frame's `suffix`. A name resolves to the step's
arguments, the transport's captures and the reply's earlier captures, then
to the device's `const` nodes.

Matching a reply, a placeholder whose name is already known (an argument, a
capture, a `const`) is checked; an unknown one is captured. An expression is
checked when all its names are known. Otherwise it captures its one unknown
name if it has one of the forms `x`, `x + k`, `k + x`, `x - k`, `k | x` or
`x | k`, with `k` known; for `|` every bit of `k` must be set in the byte, and
`x` takes the remaining bits. So `{0x40 | device-id}` captures the KORG
device id from `4F`, and rejects `3F`. A variable-width piece (`{x*}`,
`packed7`, `raw`) takes the bytes up to the fixed-width pieces that follow
it; one such piece per message. On a fixed-size HID or USB frame, sends are
padded with `00` and zeros after the pattern are padding.

### 6.1 Reply variants

Some devices answer one request with different frames, a payload or an
ack. Each shape is a `reply` with a `variant` name; the engine tries them
in order and takes the first that matches. A variant may carry its own
`ok-when`, which replaces the message's:

```kdl
message "scene-set-info" frame="korg-native" transport="selected" {
    send "01 00 00 12"
    reply "02 00 00 42 {count}" variant="info"
    reply pattern="korg-ack" variant="ack" ok-when="status == data-received"
}
```

A reply that matches no variant is an error naming every variant's
mismatch. A `reply=#true` test vector may say which `variant` must match.

### 6.2 Named patterns

`pattern "name" "<template>"` defines a template once; `send pattern=…` and
`reply pattern=…` use it. Patterns resolve after includes merge, so a
shared message can use a pattern that each device defines with its own
bytes:

```kdl
// common: the message
message "read-block" frame="korg-native" { send "01 00 00 10"; reply pattern="scene-data" }
// per device: the header differs
pattern "scene-data" "01 05 00 40 {data:packed7}"
```

### 6.3 Replies into blocks

A step with `into="scene[scene]"` stores the reply's captures in that
target: captures named after its fields go to those fields, and a single
byte-array capture (`{data:packed7}`, `{data:raw}`) fills the whole target.
That capture must be exactly the target's size; a short or long frame is a
protocol error, not a partial block (`ctrl_codec::fill`).

### 6.4 Checksums

Built in: `sum7` and `roland` (both `(128 - sum % 128) % 128`), `xor7` (the
XOR of the bytes, low 7 bits) and `fnv1a32` (files only). `from` and `to`
index the message body, not the whole message: `from` is included, `to` is
not, and a negative index counts from the body's end. Checksum bytes count
as `00` while the sum is computed. So a Roland-style body
`12 {address:u14be} {value} {sum:roland(1,-1)}` sums the address and the
value. Anything else is a `compute` LBMT snippet or a driver.

### 6.5 Argument ranges

An index argument says which values it takes, with `arg`. The value may
be one the message sends or one a reply captures:

```kdl
message "select-scene" frame="korg-native" transport="selected" {
    arg "scene" index="scene"          // 0..count-1 of block "scene"
    send "02 00 00 14 {scene}"
}
message "write-curve-page" transport="hid" kind="write" {
    arg "s" index="sensor"             // 0..=3
    arg "start" range="0..=30"
    send "{0x40 + s} {start} {curve[6] pad=7F}"
}
```

`index=` takes the range from the block's `count`, so a shared KORG message
gets 0..=4 on the nanoKONTROL Studio and 0..=7 on the nanoKEY Studio.
`range=` is inclusive. An `arg` must appear in `send`. The codec refuses to
encode a value outside the range, and a frame whose decoded value falls
outside does not match. Without it, `{0x10 + s}` matches any byte: a trace
read the TEC read-meta request `10 00 …` as write-meta with `s = -16`.

On a reply the range does the same for what the device sends by itself.
A UC-33 note-on and its CC look alike to `{0x90 + ch}`: `B5` is `0x90 +
37`. The range rejects that reading, so `B5 07 57` is a CC only:

```kdl
message "note-on" transport="controls" {
    arg "ch" range="0..=15"
    reply "{0x90 + ch} {note} {velocity}"
}
```

### 6.6 Channel and real-time messages

A MIDI template starts with one status byte; every other byte is data
(`0..=7F`). SysEx runs from `F0` to `F7`. A first piece that is an
expression (`{0xB0 + ch}`) is the status byte. A real-time byte (`F8..FF`:
clock, Active Sensing `FE`) is a message by itself, `reply "FE"`, and the
validator rejects bytes after it.

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
(§11). The vendor's framing is written once, as a `sysex-frame`, and each
message names it with `frame=`.

### 8.1 Vendor framing from generic parts

Most vendors' SysEx is built from the same parts. CDL has one generic tool
per part, so a frame needs no vendor code:

| Part | Examples | CDL |
|---|---|---|
| start, end | `F0` … `F7` | literal bytes in `prefix` and `suffix` |
| manufacturer ID | `42` (KORG), `00 20 29` (3-byte IDs) | `const "manufacturer" 0x42`, `{manufacturer:midi-id}` |
| device ID in a byte with fixed bits | `4d` (KORG), `1n` | an expression, `{0x40 \| device-id}`; the value comes from a transport `capture` of the identity reply |
| model ID | `00 01 37` (KORG), fixed bytes | `const "model" 0x137` with `{model:midi-id}`, literal bytes, or `{model:u14be}` |
| command, address | `01 00 00 12` | literal bytes, fields, `{address:u14be}` |
| byte count | before the data | `{len:u14be}`, `{len:u21le}`, `{len:varlen7}` (§6) |
| 8-bit data in 7-bit bytes | KORG 7-in-8 | `{data:packed7}` |
| plain data, text | | `{data:raw}`, `{name:ascii(16)}`, slices `{v[6]}` |
| checksum | Roland | `{sum:roland(1,-1)}` (§6.4) |

**`midi-id`**: a value below `0x80` is one byte; any other is three bytes
`00 hi lo` with `hi = v >> 8` and `lo = v & 0xFF`, both below `0x80`
(`0x137` → `00 01 37`, `0x2029` → `00 20 29`). Manufacturer IDs and KORG
model IDs use it. Decoding reads one byte, or three when the first is `00`.

**`packed7`**: data goes in groups of up to seven bytes, each preceded by
one byte that holds their top bits: bit `i` is the top bit of the group's
byte `i`. A short last group still gets its leading byte, so `n` bytes pack
into `n + ceil(n / 7)`, and a packed region of `m` bytes unpacks into
`m - ceil(m / 8)`. `80 01` packs to `01 00 01`.

Names in a frame come from the device, not from message arguments: each is
a `const` or a transport `capture`. `ctrl_cdl::validate` checks this.

**`varlen7`**: a length with an escape, as KORG's first generation writes
it: one byte below `7F`; from `7F` up, `7F`, a byte count, then that many
7-bit bytes, high first, at least two (`200` is `7F 02 01 48`).

#### KORG native SysEx

`descriptors/korg/common.inc.kdl` draws it; in short:

```text
F0 42 4d [model id] | [length] [command] [arguments or data] | F7
          frame     |            body                      | frame
```

```kdl
const "manufacturer" 0x42
sysex-frame "korg-native" status="measured" {
    prefix "F0 {manufacturer:midi-id} {0x40 | device-id} {model:midi-id}"
    suffix "F7"
}
// Studio generation (measured): a 3-byte length, low first, counts the
// body bytes after it. 02 00 00 14 03 is length 2, command 14, scene 3.
message "select-scene" frame="korg-native" transport="selected" {
    arg "scene" index="scene"
    send "{len:u21le} 14 {scene}"
}
pattern "korg-ack" "{len:u21le} {status}"                   // 01 00 00 23
pattern "scene-data" "{len:u21le} 40 {data:packed7}"        // marker 40, packed scene
// First generation (decoded, not yet seen on hardware): command 7F,
// a varlen7 length, marker 51, packed data.
pattern "korg-gen1-data" "7F {len:varlen7} 51 {data:packed7}"
```

Each device adds `const "model" 0x137`. The transports `capture
"device-id" from="identity"`; the identity reply checks `model`, so another
model's reply does not match. The frame then gives `F0 42 4F 00 01 36 02 00
00 14 03 F7` for scene 3 on a nanoKEY Studio with device id `0F`, byte for
byte the capture that `korg-sysex` tests against. Because the length is
computed, the two Studios share one scene pattern: `01 05 00` (641) and
`14 02 00` (276) are the lengths of a 560- and a 240-byte scene, packed,
plus the marker.

Other vendors' framing has the same shape. These sketches show the parts,
not evidence; a real descriptor needs captures:

```kdl
// Roland-style: device id byte, model id, command 12, address, data, checksum
sysex-frame "roland" { prefix "F0 41 {device-id} {model:midi-id} 12"; suffix "F7" }
message "set" frame="roland" { send "{address:u14be} {value} {sum:roland(0,-1)}" }
// Yamaha-style: 1n device byte, a byte count before the data
sysex-frame "yamaha" { prefix "F0 43 {0x10 | device-id}"; suffix "F7" }
message "bulk" frame="yamaha" { send "7A {len:u14be} {data:raw} {sum:sum7(1,-1)}" }
```

A device pack may register its own encoding (`{x:vendor-name}`, a Rust
`VendorEncoding` in `ctrl-codec`) only when no generic part fits, and only
with test vectors. Dump messages carry a block with `{data:packed7}`; the
engine splits and joins multi-message dumps when the block says `chunk=…`.

### 8.2 Long SysEx

Some host stacks carry only so much SysEx per write: Linux ALSA rawmidi
buffers hold 4096 bytes by default, and some USB-MIDI drivers stop at a few
KiB. CDL handles it on two layers, both declared in the descriptor:

1. **The host write.** `sysex-limit` on a MIDI transport is the longest
   message the engine hands to the host in one write. A longer message
   goes out in consecutive pieces of at most that many bytes, with
   `segment-delay` milliseconds between them. The device still receives
   one SysEx from `F0` to `F7`; only the writes are cut. On input the
   engine joins pieces back into whole messages (`ctrl_codec::SysexJoiner`:
   real-time bytes inside a SysEx are passed on, a status byte before `F7`
   reports the message as broken). This works for every device on every
   platform.

   ```kdl
   transport "usb" kind="midi" port-match="nanoKONTROL Studio CTRL" role="config" sysex-limit=4096
   ```

2. **The device's own parts.** A device whose protocol has a multi-message
   form of a long transfer names it with `segment`. When the whole message
   would be longer than `threshold` (default: the transport's
   `sysex-limit`), the engine sends the `segment` message once per
   `size`-byte piece of the byte array `over`, with the arguments `part`
   (0-based), `parts` and `offset`; each piece then goes through layer 1.

   ```kdl
   message "dump" transport="usb" kind="write" {
       send "{len:u21le} 40 {data:packed7}"
       segment message="dump-part" over="data" size=2000
   }
   message "dump-part" transport="usb" kind="write" {
       send "{len:u21le} 41 {part} {parts} {data:packed7}"
   }
   ```

   The validator checks that `over` is a byte array of both messages, that
   the part message carries `{part}` or `{offset}`, that both have the same
   `kind`, and that a threshold exists. The part message is an ordinary
   message: its `status` and evidence rules apply, so no device gets a
   multi-part form that was not measured.

`Codec::plan` returns what the engine writes: the messages, and for each
the host writes with their delays. Every KORG transport carries
`sysex-limit=4096`; no KORG transfer measured so far is longer (the largest
is a 641-byte scene).

## 9. USB parameter patching

HID and bulk frames use the same messages as SysEx, with 8-bit bytes. The
differences are in the transport, not in the messages:

| | SysEx (KORG) | HID (TEC) |
|---|---|---|
| framing | `sysex-frame`: `F0 42 {0x40 \| device-id} {model:midi-id}` … `F7` | none: the transport's `frame=8` report; sends are padded with `00` |
| addressing | the device id, captured from the identity reply | the USB device (`identify usb vid= pid= bcd=`), one per `usb-serial` |
| command | literal bytes after the frame (`02 00 00 14`) | byte 0: an opcode plus the sensor index, `{0x10 + s}` |
| index | `arg "scene" index="scene"` | `arg "s" index="sensor"` |
| ack | `pattern "korg-ack" "01 00 00 {status}"`, `ok-when` on the status | `pattern "tec-ack" "00 00 00 00 00 00 00 00"` |
| reply pairing | one reply per request; acks share a pattern | one input report per output report |

The TEC descriptor (`descriptors/tecontrol/breath.ctrl.kdl`):

```kdl
transport "hid" kind="hid" usage-page=0x0C frame=8 report-id=0 seize=#true role="config" status="measured"
pattern "tec-ack" "00 00 00 00 00 00 00 00"
message "read-meta" transport="hid" status="measured" evidence="TECcontrol findings" {
    arg "s" index="sensor"
    send "{0x10 + s}"                         // 13 00 00 00 00 00 00 00 for s = 3
    reply "{cc} {channel} {message} {param} {attack} {decay} 00 00" variant="meta" timeout=200
    ok-when "param == 9"
}
message "write-meta" transport="hid" kind="write" status="measured" {
    arg "s" index="sensor"
    send "{0x20 + s} {cc} {channel} {message} {param} {attack} {decay} 00"
    reply pattern="tec-ack" variant="ack" timeout=200
}
```

`{0x20 + s}` is a computed byte (an expression in braces, §10.1). The
transport sets the report ID (`report-id=0`: hidapi strips it, so the wire
frame is the 8 bytes). A send must fit the frame, and a reply must fill
it.

The USB side of a HID transport is data too:

```kdl
transport "hid" kind="hid" usage-page=0x0C frame=8 report-id=0 seize=#true role="config" {
    report-descriptor "05 0C 09 00 A1 02 09 00 95 08 75 08 81 00 09 00 95 08 75 08 91 00 C0"
    poll "poll" interval=52            // live values at about 19 Hz
}
```

| Node or property | Meaning | Checked |
|---|---|---|
| `report-descriptor` | the HID report descriptor as the device reports it | parsed (HID 1.11 §6.2.2); `frame` must equal the input and the output report of `report-id` |
| `poll "msg" interval=` | a read the engine repeats while the device is shown | the message exists, is a read and has a reply |
| `pace=` | shortest time between two frames, ms | |
| `seize` | open the device exclusively (IOKit) | |
| `identify usb vid= pid= bcd=` | which device and firmware | `bcd` pins the firmware the bytes were measured on |
| `key "usb-serial"` | tells several units apart (one hub thread each) | |

An array goes out in pages with a slice placeholder. The TEC curve is 32
values in 6 pages of 6, and the last page is padded with 7F:

```kdl
message "write-curve-page" transport="hid" kind="write" {
    arg "s" index="sensor"
    arg "start" range="0..=30"
    send "{0x40 + s} {start} {curve[6] pad=7F}"
    reply pattern="tec-ack" variant="ack" timeout=200
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

Placeholders (`{0x20 + s}`), `ok-when`, `visible-when`, `require`, path
indexes and `{…}` arguments use **CDL expressions**, a small integer
language. They are not LBMT: LBMT binds `& | ^ << >>` tighter than `+`, has
comparisons only in `IF`, and its names cannot contain `-`. LBMT stays for
`compute` blocks and translators (§10.2, §10.3).

- Values are 64-bit integers: decimal, `0x` hex, `_` separators. A
  comparison or `&&`, `||`, `!` gives 0 or 1; any non-zero value is true.
- Names: letters, digits, `_`, `$`, `.`, and `-` between two such characters
  when a letter follows (`device-id`, `image.crc`). `v-1` and `v - 1` are
  both `v` minus 1.
- Operators bind as in C, tightest first: unary `- ! ~`; `* / %`; `+ -`;
  `<< >>`; `< <= > >=`; `== !=`; `&`; `^`; `|`; `&&`; `||`.
- An arithmetic operator (`+ - * / %`) and a bitwise one (`& | ^ << >>`) may
  not be operands of each other without parentheses: write `(a + b) | c`.
  This keeps an expression from meaning one thing here and another in LBMT.
- Overflow, division by zero, a shift outside `0..64` and an unknown name
  are errors. `&&` and `||` stop early.

The validator parses every expression; the codec evaluates them. `v` is the
value at hand in `display` and `parse`:

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
        repeat "chunk" image-chunks size=… { exchange "write-chunk" data="{chunk}" }
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
panel picture="550x329" image="assets/nanokontrol-studio/panel.png" source="editor panel picture, measured by eye" {
    control "jog" kind="jog" label="Jog Wheel" group="Jog Wheel" box="36,199,92,92" bind="scene.jog"
    repeat "n" "1..=8" {
        control "knob{n}" kind="knob" group="Knob" box="{135.5 + 44.5 * n},146,28,28"
    }
}
```

`bind` names a block, group or field; an unbound control shows "not decoded
yet". `kind`: `knob`, `fader`, `button`, `pad`, `key`, `touchpad`, `jog`,
`meter` (live value from `midi-map` or a poll).

Image paths (`image` on `device` and `panel`) are relative to the file that
names them, below its directory (no `..`). `ctrl-cli check` reports a
missing file.

### 13.1 Labels

`labels` holds UI texts for one language as numbered lists: the names of
controls, parameters and edit pages, and the texts of value lists. A label
list's numbers are display positions, not wire codes; an `enum` (§4.2)
gives the codes, from a getter or a capture, and may take its texts from a
list. The KORG jog list shows why: 4 labels in display order against 7
measured codes.

```kdl
labels "en" source="KORG KONTROL EDITOR Strings XML, resource …" {
    list "controller-names" {
        item 0 "Common"
        item 1 "Keyboard"
    }
}
```

Item numbers need not be contiguous. Lists merge by name, items by number
(§2), so a hand-written descriptor can correct one label of a generated
list.

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
test-vector "scene-write" message="scene-write" status="decoded" {
    args device-id=0
    data "data" "AA 55 80 01"                  // a byte-array argument
    bytes "F0 42 40 00 01 37 01 05 00 40 05 2A 55 00 01 F7"
}
test-vector "commit-ack" message="commit-ack" reply=#true {
    args device-id=0 status=0x21
    bytes "F0 42 40 00 01 37 01 00 00 21 F7"
}
test-vector "scene-1" block="scene" file="the local capture archive" sha256="…"   // external, opt-in
```

A send vector encodes `args` and `data` and compares the result to `bytes`,
then decodes `bytes` with the send template and compares back to the
arguments. A `reply=#true` vector decodes `bytes` with the reply pattern.
String arguments are text (`ascii`). `ctrl-codec` runs them
(`Codec::run_vectors`); `ctrl-cli check` prints the results. External vectors run only when their file exists, like KORG's
`KORG_PRODUCTS_DIR` tests.

## 16. Tooling

`docs/cdl-schema.md` lists every node with its arguments, properties and
children. It is printed from the converter's own node table
(`crates/ctrl-cdl/src/schema.rs`) by `ctrl-cli schema`; a test fails when
it is stale.

`tools/ctrl-cli`:

- `ctrl-cli check <file>…`: parse, validate, check that images exist, run
  the test vectors, and list every message with a `send` whose message,
  frame or constants are not `measured` or `decoded`. LBMT is not compiled
  yet: that waits for `lbmt` as a dependency (TODO P0); `check` says how
  many snippets it skipped. Fails on errors and failed vectors.
- `ctrl-cli explain <file> <message> name=value…`: a dry run. Prints the
  bytes, then each run of them with its region (`prefix`, `body`,
  `suffix`, `padding`) and the template piece and value that made it, then
  the reply variants with their `ok-when`. Values: `12`, `0x0F`, `-1`,
  `hex:AA55`, `text:Name`, `@file`. A missing value names what the
  message takes.
- `ctrl-cli trace <file> [--matched] <capture>…`: matches each frame
  against every `send` and reply (`Codec::identify`). A capture is text,
  one frame per line with an optional `>` (to the device) or `<` (from
  it), `#` comments; JSON lines with `hex` and `dir`; or a binary
  `.syx`/`.bin` MIDI stream, split into messages; and HID trace logs
  (`<time> OUT … data=…`, `IN … data=…`, as TECcontrol's tracer writes). Several SysEx messages on
  one line are split; other lines, such as HID reports, stay whole.
  - A reply pairs with the message sent just before it when that message
    accepts it. A second reply to the same request is shown as a repeat
    (a device may send one input report twice).
  - Otherwise the most specific matches win: the most literal bytes, then
    no negative values, then the fewest captured values. So `50` on the
    TEC is `commit`, not `{0x40 + s}` with `s = 16`. The others are
    counted, not shown. Matches that stay tied are printed and counted as
    ambiguous: KORG acks share one pattern, and their `ok-when` tells
    which request each one satisfies.
  - Transport captures (`device-id`) carry over from the first frame that
    shows them; a frame that only matches with another value says so.
  - Values print as decimal, and from 10 up also as hex (`35/0x23`).
- Tree-sitter queries for CDL on top of tree-sitter-kdl, with LBMT
  injected as language `lbmt`: `editors/tree-sitter/` (README there),
  checked by `cargo test -p cdl-queries`.
