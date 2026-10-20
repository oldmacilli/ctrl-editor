# CDL schema

Generated from `crates/ctrl-cdl/src/schema.rs` by `ctrl-cli schema`; do
not edit. The converter checks every node against the same table. A
path `a/b` is node `b` inside `a`; `block … field` is a field at any
depth of a block; `step x` is a step in a procedure, a firmware stage
or a repeat. Section numbers refer to `docs/CDL.md`.

Every node also takes:

| Property | Meaning |
|---|---|
| `status` | `measured`, `decoded`, `inferred` or `unknown` |
| `evidence` | where the fact comes from: a capture or code |
| `note` | free text |

## `cdl`

First node of every file. (§2)

- Arguments: 1: the CDL version, `"0.1"`
- Children: none
- Properties: none besides the common ones

## `device`

Who the device is. (§3)

- Arguments: 1: the device id, `vendor.model`
- Children: yes
- Properties: none besides the common ones

## `device/vendor`, `model`, `class`, `file-key`, `manual`, `replaces`, `aka`

One text each; `aka` may repeat. (§3)

- Arguments: 1: text
- Children: none
- Properties: none besides the common ones

## `device/image`

A picture of the device. (§13)

- Arguments: 1: a path below the file's directory
- Children: none

| Property | Meaning |
|---|---|
| `role` | `thumbnail`, `list-icon`, … |

## `const`

A named number for templates and expressions. (§8.1)

- Arguments: 2: name, integer
- Children: none
- Properties: none besides the common ones

## `pattern`

A reply shape several messages share (`reply pattern=…`). (§6.2)

- Arguments: 2: name, reply template
- Children: none
- Properties: none besides the common ones

## `identify`

How the device is recognised; every matcher must agree. (§3)

- Arguments: none
- Children: yes
- Properties: none besides the common ones

## `identify/usb`

USB ids. (§3)

- Arguments: none
- Children: none

| Property | Meaning |
|---|---|
| `vid` | USB vendor id |
| `pid` | USB product id |
| `bcd` | device release (bcdDevice) |

## `identify/midi-port`

A MIDI port name. (§3)

- Arguments: none
- Children: none

| Property | Meaning |
|---|---|
| `match` | text in the port name |
| `direction` | `in`, `out` or `both` |

## `identify/sysex-identity`

The universal identity reply. (§3)

- Arguments: none
- Children: none

| Property | Meaning |
|---|---|
| `reply` | a reply template, instead of the standard identity reply |
| `manufacturer` | manufacturer id |
| `model` | model id |

## `identify/hid`

A HID interface. (§3)

- Arguments: none
- Children: none

| Property | Meaning |
|---|---|
| `usage-page` | HID usage page |
| `report-size` | report size in bytes |

## `identify/exchange`

Send a message; the device is the one whose reply matches. (§5.1)

- Arguments: 1: a message
- Children: none

| Property | Meaning |
|---|---|
| `ok-when` | an expression over the reply's values |

## `identify/key`

What tells several connected units apart. (§3)

- Arguments: 1: `usb-serial`, …
- Children: none
- Properties: none besides the common ones

## `identify/listen`

Bytes the device sends by itself; a hint, never proof alone. (§5.1)

- Arguments: 1: a reply template
- Children: none
- Properties: none besides the common ones

## `transport`

A path to the device. (§5)

- Arguments: 1: name
- Children: yes

| Property | Meaning |
|---|---|
| `kind` | `midi`, `hid` or `usb-bulk` |
| `port-match` | text in the MIDI port name, both directions |
| `port-match-in` | text in the input port name |
| `port-match-out` | text in the output port name |
| `role` | `config` (editing) or `performance` (playing) |
| `preferred` | the config transport to try first |
| `usage-page` | HID usage page |
| `frame` | fixed report size in bytes; sends are padded with 00 |
| `report-id` | HID report id |
| `seize` | open the HID device exclusively |
| `interface` | USB interface number (bulk) |
| `ep-out` | bulk OUT endpoint |
| `ep-in` | bulk IN endpoint |
| `scan` | a read message sent on each output to find the device |

## `transport/capture`

A value the transport learns once and uses in frames. (§5)

- Arguments: 1: a value name, `device-id`
- Children: none

| Property | Meaning |
|---|---|
| `from` | the message whose reply carries it |

## `types`

Value types. (§4.2)

- Arguments: none
- Children: yes
- Properties: none besides the common ones

## `types/enum`, `errors`

Named values; `errors` are status codes. (§4.2)

- Arguments: 1: name
- Children: yes
- Properties: none besides the common ones

## `types/enum`, `errors/value`

One value. (§4.2)

- Arguments: 2: integer, label
- Children: none
- Properties: none besides the common ones

## `types/int`

An integer type. (§4.2)

- Arguments: 1: name
- Children: none

| Property | Meaning |
|---|---|
| `storage` | `u7`, `u8`, `u14le`, `u14be`, … (the bytes it takes) |
| `range` | allowed values, `lo..=hi` |
| `display` | expression from stored value `v` to the shown one |
| `parse` | expression from the shown value `v` back |
| `special` | named values, `16=Disable` |

## `block`

A unit of device memory: a scene, a sensor. (§4.3)

- Arguments: 1: name
- Children: yes

| Property | Meaning |
|---|---|
| `size` | bytes |
| `count` | elements |
| `names` | element names, space separated |

## `block … field`

One value. (§4.3)

- Arguments: 1: name
- Children: none

| Property | Meaning |
|---|---|
| `offset` | byte offset in the block or group |
| `type` | a storage or `types` name |
| `range` | allowed values, `lo..=hi` |
| `visible-when` | expression; hidden otherwise |
| `fixed` | a value the editor never changes |

## `block … group`

Fields that belong together. (§4.3)

- Arguments: 1: name
- Children: yes

| Property | Meaning |
|---|---|
| `offset` | byte offset |

## `block … array`

Repeated values or fields. (§4.3)

- Arguments: 1: name
- Children: yes

| Property | Meaning |
|---|---|
| `count` | elements |
| `offset` | byte offset |
| `stride` | bytes between elements |
| `type` | element type |
| `driver` | a driver that computes the values |

## `param`

How a field is edited. (§4.5)

- Arguments: 1: a path, `drawbar.*.cc`
- Children: none

| Property | Meaning |
|---|---|
| `label` | UI label |
| `group` | UI group |
| `ui` | `spin`, `combo`, `slider`, … |
| `min-firmware` | the first firmware that has it |

## `panel`

The device drawing. (§13)

- Arguments: none
- Children: yes

| Property | Meaning |
|---|---|
| `picture` | size, `550x329` |
| `image` | a path below the file's directory |
| `source` | where the layout comes from |

## `panel/control`

One control on the drawing. (§13)

- Arguments: 1: name
- Children: none

| Property | Meaning |
|---|---|
| `kind` | `knob`, `fader`, `button`, `pad`, `key`, `touchpad`, `jog`, `meter` |
| `label` | UI label |
| `group` | UI group |
| `box` | `x,y,w,h` in picture pixels; expressions allowed |
| `bind` | the block path it edits |
| `live` | `message.value` it shows |

## `panel/repeat`

Repeats the controls inside. (§13)

- Arguments: 2: variable, `lo..=hi`
- Children: yes
- Properties: none besides the common ones

## `labels`

UI texts. (§13.1)

- Arguments: 1: language, `en`
- Children: yes

| Property | Meaning |
|---|---|
| `source` | where the texts come from |

## `labels/list`

A numbered list of texts. (§13.1)

- Arguments: 1: name
- Children: yes
- Properties: none besides the common ones

## `labels/list/item`

One text. (§13.1)

- Arguments: 2: number, text
- Children: none
- Properties: none besides the common ones

## `midi-map`

What the device sends and receives when played. (§7)

- Arguments: none
- Children: yes

| Property | Meaning |
|---|---|
| `transport` | the performance transport |

## `midi-map/source`, `sink`

One stream of MIDI messages. (§7)

- Arguments: 1: a block or name
- Children: none

| Property | Meaning |
|---|---|
| `count` | elements |
| `kind` | `cc`, `cc14`, `nrpn`, `rpn`, `note`, `pc`, `pb`, `at`, `poly-at`, `clock`, `from-field` |
| `cc` | CC number or expression |
| `note` | note number or expression |
| `channel` | channel or expression |
| `message` | message type expression |
| `range` | value range |

## `sysex-frame`

A vendor's SysEx framing. (§8.1)

- Arguments: 1: name
- Children: yes
- Properties: none besides the common ones

## `sysex-frame/prefix`, `suffix`

Bytes before and after each message body. (§8.1)

- Arguments: 1: template
- Children: none
- Properties: none besides the common ones

## `message`

One request and its replies. (§6)

- Arguments: 1: name
- Children: yes

| Property | Meaning |
|---|---|
| `transport` | a transport name, or `selected` |
| `frame` | a `sysex-frame` |
| `kind` | `read`, `write` or `commit` |

## `message/arg`

The values an argument may take. (§6.5)

- Arguments: 1: an argument name
- Children: none

| Property | Meaning |
|---|---|
| `range` | allowed values, `lo..=hi` |
| `index` | a block: 0..count-1 |

## `message/send`

The bytes sent. (§6)

- Arguments: 0 to 1: template
- Children: none

| Property | Meaning |
|---|---|
| `pattern` | a named pattern instead |

## `message/reply`

An expected reply; the first that matches wins. (§6.1)

- Arguments: 0 to 1: template
- Children: none

| Property | Meaning |
|---|---|
| `timeout` | milliseconds |
| `variant` | name; required with several replies |
| `pattern` | a named pattern instead |
| `ok-when` | expression; overrides the message's |

## `message/ok-when`

When a reply means success. (§6)

- Arguments: 1: expression
- Children: none
- Properties: none besides the common ones

## `procedure`

Steps the editor runs. (§11)

- Arguments: 1: name
- Children: yes

| Property | Meaning |
|---|---|
| `kind` | `read`, `write` or `commit` |
| `transport` | default transport |

## `step send`, `exchange`

Send, or send and wait for the reply. Properties are message arguments, except `into` (block path for the reply), `from` (block path for the template) and `timeout`. (§11)

- Arguments: 1: a message
- Children: none
- Properties: any (see above)

## `step repeat`

Repeats the steps inside. (§11)

- Arguments: 2: variable, `lo..=hi` or a sequence
- Children: yes

| Property | Meaning |
|---|---|
| `size` | chunk size for a sequence |

## `step read-into`

Copies a reply into a block. (§11)

- Arguments: 1: a block path
- Children: none

| Property | Meaning |
|---|---|
| `from` | a message |

## `step delay`

Waits. (§11)

- Arguments: 1: milliseconds
- Children: none
- Properties: none besides the common ones

## `step verify`

Reads back and compares. (§11)

- Arguments: 1: a read procedure
- Children: none

| Property | Meaning |
|---|---|
| `compare` | paths to compare; all written ones if absent |

## `step gate`

Asks the user; `arm` before a commit. (§11)

- Arguments: 1: `confirm` or `arm`
- Children: none

| Property | Meaning |
|---|---|
| `text` | what the user is asked |

## `step restore-point`

Saves what a write will change. (§11)

- Arguments: 1 or more: block paths
- Children: none
- Properties: none besides the common ones

## `step require`, `ok-when`

Stops unless true. (§11)

- Arguments: 1: expression
- Children: none
- Properties: none besides the common ones

## `step call`

Runs another procedure. (§11)

- Arguments: 1: a procedure
- Children: none
- Properties: none besides the common ones

## `step driver`

A step a driver performs. (§14)

- Arguments: 1: a driver
- Children: none

| Property | Meaning |
|---|---|
| `step` | the driver's step name |

## `step expect-identity`

Waits until the device identifies as expected. (§12)

- Arguments: none
- Children: none

| Property | Meaning |
|---|---|
| `product` | the identity's product value |
| `timeout` | milliseconds |

## `firmware`

The firmware update. (§12)

- Arguments: none
- Children: yes
- Properties: none besides the common ones

## `firmware/image`

The update file. (§12)

- Arguments: none
- Children: none

| Property | Meaning |
|---|---|
| `format` | image format |
| `magic` | first bytes |
| `driver` | the driver that reads it |

## `firmware/version-from`

Where the running version comes from. (§12)

- Arguments: 1: a message
- Children: none

| Property | Meaning |
|---|---|
| `field` | the reply values that form the version |

## `firmware/require`

A condition before updating. (§12)

- Arguments: 1: expression
- Children: none
- Properties: none besides the common ones

## `firmware/stage`

One stage of the update, with steps. (§12)

- Arguments: 1: name
- Children: yes

| Property | Meaning |
|---|---|
| `kind` | `read`, `write` or `commit` |

## `firmware/recovery`

What to do when it fails. (§12)

- Arguments: 1: text
- Children: none
- Properties: none besides the common ones

## `lbmt`

A live MIDI rule. (§10.3)

- Arguments: 2: name, LBMT source
- Children: none

| Property | Meaning |
|---|---|
| `from` | input port or transport |
| `to` | output port |
| `enabled` | on at start |

## `driver`

Code the descriptor cannot express. (§14)

- Arguments: 1: name
- Children: yes

| Property | Meaning |
|---|---|
| `provides` | what it supplies: `block:…`, `file:…` |
| `from` | what it reads |

## `driver/owns`

Bytes on record that the driver sends. (§14)

- Arguments: 1: name
- Children: none

| Property | Meaning |
|---|---|
| `bytes` | the bytes the driver builds |
| `frame` | a frame |

## `test-vector`

Bytes the codec must produce and decode. (§15)

- Arguments: 1: name
- Children: yes

| Property | Meaning |
|---|---|
| `message` | the message |
| `block` | or a block, from a file |
| `file` | external data; runs only when present |
| `sha256` | the file's hash |
| `reply` | `#true`: decode with the reply |
| `variant` | the reply variant that must match |

## `test-vector/args`

Argument values, as properties. (§15)

- Arguments: none
- Children: none
- Properties: any (see above)

## `test-vector/bytes`

The whole message. (§15)

- Arguments: 1: hex
- Children: none
- Properties: none besides the common ones

## `test-vector/data`

A byte-array argument. (§15)

- Arguments: 2: name, hex
- Children: none
- Properties: none besides the common ones
