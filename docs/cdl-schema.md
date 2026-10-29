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
| `sysex-limit` | longest SysEx the host carries in one write; longer ones are split |
| `segment-delay` | milliseconds between the pieces of a split write |
| `pace` | shortest time between two frames, milliseconds |

## `transport/report-descriptor`

What the device reports; `frame` must match its reports. (§9)

- Arguments: 1: the HID report descriptor, hex bytes
- Children: none
- Properties: none besides the common ones

## `transport/poll`

A message repeated for live values. (§9)

- Arguments: 1: a read message
- Children: none

| Property | Meaning |
|---|---|
| `interval` | milliseconds between polls |

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
| `address` | device memory address of element 0 (§8.3) |
| `chunk` | bytes per addressed record (§8.3) |

## `block … field`

One value. (§4.3)

- Arguments: 1: name
- Children: none

| Property | Meaning |
|---|---|
| `offset` | byte offset in the block or group |
| `type` | a storage or `types` name |
| `bits` | the bits of the byte that hold it, `lo..=hi`, 0 lowest |
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
| `type` | a `types` enum or int, or a storage type: its values |

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

## `message/derive`

A value computed from the others: sent when left out, checked on decode. (§6.7)

- Arguments: 2: name, expression
- Children: none
- Properties: any (see above)

## `message/records`

A run of addressed records inside one message. (§8.3)

- Arguments: 1: the byte array that holds the records
- Children: yes

| Property | Meaning |
|---|---|
| `pattern` | the named pattern of one record |
| `address` | expression: a record's memory address |
| `data` | the record's data bytes, a byte array of the pattern |
| `pace` | ms the host waits after each record it sends |

## `message/records/derive`

A record's value computed from the others. (§8.3)

- Arguments: 2: name, expression
- Children: none
- Properties: any (see above)

## `message/segment`

The device's own multi-message form of a long send. (§8.2)

- Arguments: none
- Children: none

| Property | Meaning |
|---|---|
| `message` | the message that carries one piece |
| `over` | the byte-array argument that is split |
| `size` | bytes of that argument per piece, before encoding |
| `threshold` | split when the whole message is longer; default the transport's `sysex-limit` |

## `procedure`

Steps the editor runs. (§11)

- Arguments: 1: name
- Children: yes

| Property | Meaning |
|---|---|
| `kind` | `read`, `write` or `commit` |
| `transport` | default transport |

## `step send`, `exchange`

Send, or send and wait for the reply. Properties are message arguments, except `into` (block path for the reply), `from` (block path for the template), `records-from` (block paths that fill the message's `records`, §8.3) and `timeout`. (§11)

- Arguments: 1: a message
- Children: none
- Properties: any (see above)

## `step await`

Wait up to `timeout` ms for a message the device sends by itself (a dump the user starts). Properties are values the reply must carry, except `into` and `timeout`. A reply with `records` fills the addressed blocks. (§11)

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

## `hands`

What a person operates on the device: keys, encoders, controls, number entry. (§17.1)

- Arguments: none
- Children: yes
- Properties: none besides the common ones

## `hands/key`

A key, encoder, fader or other control a guide names. (§17.1)

- Arguments: 1: id; with `range`, `{n}` in it stands for each value
- Children: none

| Property | Meaning |
|---|---|
| `label` | as printed on the device; may use `{n}` |
| `range` | `lo..=hi`: one key per value of `n` |

## `hands/entry`

A way of typing numbers digit by digit. (§17.1)

- Arguments: 1: name
- Children: none

| Property | Meaning |
|---|---|
| `text` | how to type a number, shown once per guide |

## `recipe`

Named guide steps; `do` runs them. (§17.3)

- Arguments: 1: name
- Children: yes

| Property | Meaning |
|---|---|
| `args` | parameter names, space-separated |

## `edit`

How a block element's stored fields change by hand. (§17.4)

- Arguments: 1: block
- Children: yes

| Property | Meaning |
|---|---|
| `index` | the element's index variable, `i` when absent |

## `edit/open`, `edit/close`, `edit/check`

Guide steps: make the element current, keep the change in memory, let the editor read it back. (§17.4)

- Arguments: none
- Children: yes
- Properties: none besides the common ones

## `edit/item/select`

Guide steps that select the item. (§17.4)

- Arguments: none
- Children: yes
- Properties: none besides the common ones

## `edit/item`

An array element: how to select it and set its fields. (§17.4)

- Arguments: 1: array path below the block, `group.encoder`
- Children: yes

| Property | Meaning |
|---|---|
| `index` | one index variable per array, space-separated |

## `edit/field`

The steps that set one field to `v`. (§17.4)

- Arguments: 1: field name
- Children: yes

| Property | Meaning |
|---|---|
| `applies` | expression: the field is set by hand only when it holds |
| `index` | for an array of values: the element's index variable |

## `guide press`

Press and release a key. (§17.2)

- Arguments: 1: key
- Children: none

| Property | Meaning |
|---|---|
| `with` | keys held down during the act, space-separated |
| `shows` | what the device shows afterwards |
| `hint` | why, or what to watch for |
| `times` | how often, 1 when absent |

## `guide hold`

Hold a key down. (§17.2)

- Arguments: 1: key
- Children: none

| Property | Meaning |
|---|---|
| `with` | keys held down during the act, space-separated |
| `shows` | what the device shows afterwards |
| `hint` | why, or what to watch for |
| `ms` | how long |
| `until` | what the display shows when to let go |

## `guide chord`

Press keys together. (§17.2)

- Arguments: 2 or more: keys
- Children: none

| Property | Meaning |
|---|---|
| `with` | keys held down during the act, space-separated |
| `shows` | what the device shows afterwards |
| `hint` | why, or what to watch for |

## `guide type`

Type a number digit by digit. (§17.2)

- Arguments: 1: number, `{expression}`
- Children: none

| Property | Meaning |
|---|---|
| `with` | keys held down during the act, space-separated |
| `shows` | what the device shows afterwards |
| `hint` | why, or what to watch for |
| `digits` | zero-padded to this many digits |
| `entry` | the `hands` entry it uses |

## `guide turn`

Turn an encoder until the display shows a value. (§17.2)

- Arguments: 1: key
- Children: none

| Property | Meaning |
|---|---|
| `with` | keys held down during the act, space-separated |
| `shows` | what the device shows afterwards |
| `hint` | why, or what to watch for |
| `to` | what the display shows at the target |
| `as` | a type: `to` is its value and the guide shows the label |

## `guide move`

Move or touch a control. (§17.2)

- Arguments: 1: key
- Children: none

| Property | Meaning |
|---|---|
| `with` | keys held down during the act, space-separated |
| `shows` | what the device shows afterwards |
| `hint` | why, or what to watch for |

## `guide say`

A sentence for the user. (§17.2)

- Arguments: 1: text
- Children: none

| Property | Meaning |
|---|---|
| `with` | keys held down during the act, space-separated |
| `shows` | what the device shows afterwards |
| `hint` | why, or what to watch for |

## `guide ask`

The user reads a value on the device and enters it. (§17.2)

- Arguments: 1: path
- Children: none

| Property | Meaning |
|---|---|
| `text` | what to read on the device |

## `guide one-of`

Alternatives, as `option` children. (§17.2)

- Arguments: none
- Children: yes

| Property | Meaning |
|---|---|
| `shows` | what the device shows afterwards |
| `hint` | why, or what to watch for |

## `guide one-of/option`

Guide steps; any one option will do. (§17.2)

- Arguments: none
- Children: yes
- Properties: none besides the common ones

## `guide when`

Guide steps that apply only when the expression holds. (§17.2)

- Arguments: 1: expression
- Children: yes
- Properties: none besides the common ones

## `guide do`

Runs a recipe; other properties are its arguments. (§17.3)

- Arguments: 1: recipe
- Children: none
- Properties: any (see above)

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
