//! The node table: every CDL node with its arguments, properties and
//! children. The converter checks nodes against it and `docs/cdl-schema.md`
//! is printed from it (`ctrl-cli schema`), so the two cannot drift.

/// A property and what it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prop {
    pub name: &'static str,
    pub doc: &'static str,
}

/// One node kind in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeSpec {
    /// Where the node appears: `message/reply`. Names separated by `|`
    /// share one entry.
    pub path: &'static str,
    /// Minimum and maximum argument count; [`MANY`] for no maximum.
    pub args: (usize, usize),
    pub args_doc: &'static str,
    /// None: any property (step arguments, test-vector arguments).
    pub props: Option<&'static [Prop]>,
    pub children: bool,
    /// The CDL.md section that describes the node.
    pub section: &'static str,
    pub doc: &'static str,
}

pub const MANY: usize = usize::MAX;

/// Every node may carry these (CDL §4.1).
pub const COMMON: &[Prop] = &[
    p("status", "`measured`, `decoded`, `inferred` or `unknown`"),
    p("evidence", "where the fact comes from: a capture or code"),
    p("note", "free text"),
];

const fn p(name: &'static str, doc: &'static str) -> Prop {
    Prop { name, doc }
}

#[allow(clippy::too_many_arguments)]
const fn n(
    path: &'static str,
    args: (usize, usize),
    args_doc: &'static str,
    props: Option<&'static [Prop]>,
    children: bool,
    section: &'static str,
    doc: &'static str,
) -> NodeSpec {
    NodeSpec {
        path,
        args,
        args_doc,
        props,
        children,
        section,
        doc,
    }
}

const NONE: Option<&[Prop]> = Some(&[]);

const WITH: Prop = p("with", "keys held down during the act, space-separated");
const SHOWS: Prop = p("shows", "what the device shows afterwards");
const HINT: Prop = p("hint", "why, or what to watch for");
const GUIDE_PROPS: &[Prop] = &[WITH, SHOWS, HINT];
const GUIDE_PROPS_TIMES: &[Prop] = &[WITH, SHOWS, HINT, p("times", "how often, 1 when absent")];
const GUIDE_PROPS_HOLD: &[Prop] = &[
    WITH,
    SHOWS,
    HINT,
    p("ms", "how long"),
    p("until", "what the display shows when to let go"),
];
const GUIDE_PROPS_TYPE: &[Prop] = &[
    WITH,
    SHOWS,
    HINT,
    p("digits", "zero-padded to this many digits"),
    p("entry", "the `hands` entry it uses"),
];
const GUIDE_PROPS_TURN: &[Prop] = &[
    WITH,
    SHOWS,
    HINT,
    p("to", "what the display shows at the target"),
    p(
        "as",
        "a type: `to` is its value and the guide shows the label",
    ),
];

pub static SCHEMA: &[NodeSpec] = &[
    n("control-assignments", (0, 0), "", Some(&[p("global-channel-offset", "global MIDI channel byte")]), true, "4", "Control-to-MIDI assignment decoding."),
    n("global-fields", (0, 0), "", Some(&[p("name-label", "device name display label"), p("name-offset", "device name offset"), p("name-width", "device name width"), p("bpm-label", "BPM display label"), p("bpm-offset", "little-endian BPM word offset"), p("bpm-min", "minimum BPM tenths"), p("bpm-max", "maximum BPM tenths"), p("chord-pad-label", "Chord Pad display label"), p("easy-scale-label", "Easy Scale display label")]), true, "4", "Optional global fields."),
    n("global-fields/binding", (1, 1), "field name", Some(&[p("label", "field display label"), p("offset", "global byte offset"), p("min", "minimum displayed value"), p("max", "maximum displayed value"), p("bias", "stored minus displayed value")]), true, "4", "Named global UI byte binding."),
    n("global-fields/binding/choice", (1, 1), "UI label", Some(&[p("value", "stored byte value")]), false, "4", "Explicit global UI choice."),
    n("control-assignments/assign", (0, 0), "", Some(&[p("group", "control group"), p("label", "control label"), p("codec", "structured assignment codec"), p("codec-offset", "scene byte offset for the structured record"), p("kind", "fixed message kind"), p("kind-offset", "scene byte for kind"), p("fallback", "unmapped kind"), p("number", "fixed MIDI number"), p("number-offset", "scene byte for number"), p("number-required", "require the number byte"), p("channel-offset", "scene byte for channel"), p("channel-required", "require the channel byte"), p("description", "control description"), p("detail-offset", "scene byte selecting a description suffix")]), true, "4", "One control assignment."),
    n("control-assignments/assign/map", (1, 1), "stored byte", Some(&[p("kind", "message kind")]), false, "4", "Map a stored byte to a message kind."),
    n("control-assignments/assign/detail", (1, 1), "stored byte", Some(&[p("text", "description suffix")]), false, "4", "Map a byte to an optional description suffix."),
    n("control-assignments/assign/live-number", (0, 0), "", Some(&[p("offset", "first scene byte"), p("count", "number of bytes"), p("max", "maximum MIDI number (0..127, default 127)"), p("if-offset", "condition byte"), p("if-not-value", "disabled when equal")]), false, "4", "Additional MIDI numbers for live indication."),
    n("editor-pages", (0, 0), "", Some(&[p("default", "default page name")]), true, "4", "Declarative editor pages."),
    n("editor-pages/select", (1, 1), "page name", Some(&[p("group", "control group"), p("label", "control label")]), false, "4", "Select a page for a control."),
    n("editor-pages/page", (1, 1), "page name", NONE, true, "4", "Ordered fields of a page."),
    n("editor-pages/page/field", (1, 1), "field label", Some(&[p("offset", "scene byte offset"), p("kind", "field kind"), p("min", "minimum shown value"), p("max", "maximum shown value"), p("bias", "stored value bias"), p("enabled-if-offset", "scene byte governing editability"), p("enabled-if-value", "required byte value"), p("enabled-if-not-value", "excluded byte value"), p("note-if-offset", "scene byte governing note display"), p("note-if-value", "note display byte value"), p("hidden-if-offset", "scene byte governing visibility"), p("width", "text width in bytes"), p("min-from-offset", "scene byte setting the minimum"), p("max-from-offset", "scene byte setting the maximum")]), true, "4", "Editable field or heading."),
    n("editor-pages/page/field/choice", (1, 1), "choice label", NONE, false, "4", "One ordered choice."),
    n("editor-pages/page/field/hide-if-value", (1, 1), "byte value", NONE, false, "4", "Hide the field for this byte value."),
    n("parameter-layouts", (0, 0), "", NONE, true, "4", "Ordered parameter layouts for the device."),
    n("parameter-layouts/layout", (1, 1), "payload name", Some(&[
        p("order", "zero-based order among editor layouts"),
        p("size", "payload size in bytes"), p("elements", "number of repeated elements"),
        p("stride", "element stride in bytes"), p("version", "layout format version"),
    ]), true, "4", "One payload's editor parameter layout."),
    n("parameter-layouts/layout/parameter", (1, 1), "parameter key", Some(&[
        p("id", "parameter id"), p("offset", "byte offset"), p("width", "byte width"),
        p("shift", "bit shift"), p("mask", "logical bit mask"), p("min", "minimum value"),
        p("max", "maximum value"), p("bias", "stored value bias"), p("kind", "number or enum"),
        p("default", "optional factory default"), p("default-source", "factory or unknown"),
        p("display-label", "optional editor label"),
        p("display-choices-inferred", "fallback choices include inferred values"),
    ]), true, "4", "One ordered parameter, including its packed representation."),
    n("parameter-layouts/layout/parameter/label", (1, 1), "display label", NONE, false, "4", "One ordered enum label."),
    n("parameter-layouts/layout/parameter/display-choice", (1, 1), "fallback choice", NONE, false, "4", "One editor choice when enum labels are absent."),
    n("sysex-commands", (0, 0), "", Some(&[
        p("model-id", "optional 14-bit model id"),
        p("length-style", "optional SysEx length encoding"),
    ]), true, "8", "Recovered device command table."),
    n("sysex-commands/command", (1, 1), "command name", Some(&[
        p("byte", "known 7-bit command byte; omit for unknown"),
    ]), false, "8", "A measured, decoded or unknown command."),
    n("catalog", (0, 0), "", Some(&[
        p("order", "zero-based device-list order"),
        p("name", "device-list display name"),
        p("localized-name", "optional localized display name"),
        p("scenes", "number of scenes"),
        p("scene-size", "bytes in one scene"),
        p("global-size", "bytes in the global block"),
    ]), true, "3", "Editor catalog metadata."),
    n("catalog/extension", (1, 1), "filename extension", Some(&[p("os-type", "optional legacy OS type")]), false, "3", "An accepted file extension."),
    n("catalog/factory-file", (1, 1), "relative factory-file path", NONE, false, "3", "A factory data file, supplied separately."),
    // ---- file -------------------------------------------------------------
    n(
        "cdl",
        (1, 1),
        "the CDL version, `\"0.1\"`",
        NONE,
        false,
        "2",
        "First node of every file.",
    ),
    n(
        "device",
        (1, 1),
        "the device id, `vendor.model`",
        NONE,
        true,
        "3",
        "Who the device is.",
    ),
    n(
        "device/vendor|model|class|file-key|manual|replaces|aka",
        (1, 1),
        "text",
        NONE,
        false,
        "3",
        "One text each; `aka` may repeat.",
    ),
    n(
        "device/image",
        (1, 1),
        "a path below the file's directory",
        Some(&[p("role", "`thumbnail`, `list-icon`, …")]),
        false,
        "13",
        "A picture of the device.",
    ),
    n(
        "const",
        (2, 2),
        "name, integer",
        NONE,
        false,
        "8.1",
        "A named number for templates and expressions.",
    ),
    n(
        "pattern",
        (2, 2),
        "name, reply template",
        NONE,
        false,
        "6.2",
        "A reply shape several messages share (`reply pattern=…`).",
    ),
    // ---- identity ---------------------------------------------------------
    n(
        "identify",
        (0, 0),
        "",
        NONE,
        true,
        "3",
        "How the device is recognised; every matcher must agree.",
    ),
    n(
        "identify/usb",
        (0, 0),
        "",
        Some(&[
            p("vid", "USB vendor id"),
            p("pid", "USB product id"),
            p("bcd", "device release (bcdDevice)"),
        ]),
        false,
        "3",
        "USB ids.",
    ),
    n(
        "identify/midi-port",
        (0, 0),
        "",
        Some(&[
            p("match", "text in the port name"),
            p("direction", "`in`, `out` or `both`"),
        ]),
        false,
        "3",
        "A MIDI port name.",
    ),
    n(
        "identify/sysex-identity",
        (0, 0),
        "",
        Some(&[
            p(
                "reply",
                "a reply template, instead of the standard identity reply",
            ),
            p("manufacturer", "manufacturer id"),
            p("model", "model id"),
        ]),
        false,
        "3",
        "The universal identity reply.",
    ),
    n(
        "identify/hid",
        (0, 0),
        "",
        Some(&[
            p("usage-page", "HID usage page"),
            p("report-size", "report size in bytes"),
        ]),
        false,
        "3",
        "A HID interface.",
    ),
    n(
        "identify/exchange",
        (1, 1),
        "a message",
        Some(&[p("ok-when", "an expression over the reply's values")]),
        false,
        "5.1",
        "Send a message; the device is the one whose reply matches.",
    ),
    n(
        "identify/key",
        (1, 1),
        "`usb-serial`, …",
        NONE,
        false,
        "3",
        "What tells several connected units apart.",
    ),
    n(
        "identify/listen",
        (1, 1),
        "a reply template",
        NONE,
        false,
        "5.1",
        "Bytes the device sends by itself; a hint, never proof alone.",
    ),
    // ---- transports -------------------------------------------------------
    n(
        "transport",
        (1, 1),
        "name",
        Some(&[
            p("kind", "`midi`, `hid` or `usb-bulk`"),
            p("port-match", "text in the MIDI port name, both directions"),
            p("port-match-in", "text in the input port name"),
            p("port-match-out", "text in the output port name"),
            p("role", "`config` (editing) or `performance` (playing)"),
            p("preferred", "the config transport to try first"),
            p("usage-page", "HID usage page"),
            p(
                "frame",
                "fixed report size in bytes; sends are padded with 00",
            ),
            p("report-id", "HID report id"),
            p("seize", "open the HID device exclusively"),
            p("interface", "USB interface number (bulk)"),
            p("ep-out", "bulk OUT endpoint"),
            p("ep-in", "bulk IN endpoint"),
            p(
                "scan",
                "a read message sent on each output to find the device",
            ),
            p(
                "sysex-limit",
                "longest SysEx the host carries in one write; longer ones are split",
            ),
            p("segment-delay", "milliseconds between the pieces of a split write"),
            p("pace", "shortest time between two frames, milliseconds"),
        ]),
        true,
        "5",
        "A path to the device.",
    ),
    n(
        "transport/report-descriptor",
        (1, 1),
        "the HID report descriptor, hex bytes",
        NONE,
        false,
        "9",
        "What the device reports; `frame` must match its reports.",
    ),
    n(
        "transport/poll",
        (1, 1),
        "a read message",
        Some(&[p("interval", "milliseconds between polls")]),
        false,
        "9",
        "A message repeated for live values.",
    ),
    n(
        "transport/capture",
        (1, 1),
        "a value name, `device-id`",
        Some(&[p("from", "the message whose reply carries it")]),
        false,
        "5",
        "A value the transport learns once and uses in frames.",
    ),
    // ---- types and blocks -------------------------------------------------
    n("types", (0, 0), "", NONE, true, "4.2", "Value types."),
    n(
        "types/enum|errors",
        (1, 1),
        "name",
        NONE,
        true,
        "4.2",
        "Named values; `errors` are status codes.",
    ),
    n(
        "types/enum|errors/value",
        (2, 2),
        "integer, label",
        NONE,
        false,
        "4.2",
        "One value.",
    ),
    n(
        "types/int",
        (1, 1),
        "name",
        Some(&[
            p(
                "storage",
                "`u7`, `u8`, `u14le`, `u14be`, … (the bytes it takes)",
            ),
            p("range", "allowed values, `lo..=hi`"),
            p(
                "display",
                "expression from stored value `v` to the shown one",
            ),
            p("parse", "expression from the shown value `v` back"),
            p("special", "named values, `16=Disable`"),
        ]),
        false,
        "4.2",
        "An integer type.",
    ),
    n(
        "block",
        (1, 1),
        "name",
        Some(&[
            p("size", "bytes"),
            p("count", "elements"),
            p("names", "element names, space separated"),
            p("address", "device memory address of element 0 (§8.3)"),
            p("chunk", "bytes per addressed record (§8.3)"),
        ]),
        true,
        "4.3",
        "A unit of device memory: a scene, a sensor.",
    ),
    n(
        "block … field",
        (1, 1),
        "name",
        Some(&[
            p("offset", "byte offset in the block or group"),
            p("type", "a storage or `types` name"),
            p("bits", "the bits of the byte that hold it, `lo..=hi`, 0 lowest"),
            p("range", "allowed values, `lo..=hi`"),
            p("visible-when", "expression; hidden otherwise"),
            p("fixed", "a value the editor never changes"),
        ]),
        false,
        "4.3",
        "One value.",
    ),
    n(
        "block … group",
        (1, 1),
        "name",
        Some(&[p("offset", "byte offset")]),
        true,
        "4.3",
        "Fields that belong together.",
    ),
    n(
        "block … array",
        (1, 1),
        "name",
        Some(&[
            p("count", "elements"),
            p("offset", "byte offset"),
            p("stride", "bytes between elements"),
            p("type", "element type"),
            p("driver", "a driver that computes the values"),
        ]),
        true,
        "4.3",
        "Repeated values or fields.",
    ),
    n(
        "param",
        (1, 1),
        "a path, `drawbar.*.cc`",
        Some(&[
            p("label", "UI label"),
            p("group", "UI group"),
            p("ui", "`spin`, `combo`, `slider`, …"),
            p("min-firmware", "the first firmware that has it"),
        ]),
        false,
        "4.5",
        "How a field is edited.",
    ),
    // ---- panel and labels -------------------------------------------------
    n(
        "panel",
        (0, 0),
        "",
        Some(&[
            p("picture", "size, `550x329`"),
            p("image", "a path below the file's directory"),
            p("source", "where the layout comes from"),
            p("binding-inferred", "parameter binding order is inferred"),
        ]),
        true,
        "13",
        "The device drawing.",
    ),
    n(
        "panel/control",
        (1, 1),
        "name",
        Some(&[
            p(
                "kind",
                "`knob`, `fader`, `button`, `pad`, `key`, `touchpad`, `jog`, `meter`",
            ),
            p("label", "UI label"),
            p("group", "UI group"),
            p("box", "`x,y,w,h` in picture pixels; expressions allowed"),
            p("bind", "the block path it edits"),
            p("live", "`message.value` it shows"),
            p("key-black", "a raised black keyboard key"),
            p("element-index", "parameter schema element index"),
            p("jog-binding", "select the device jog block"),
        ]),
        false,
        "13",
        "One control on the drawing.",
    ),
    n(
        "panel/repeat",
        (2, 2),
        "variable, `lo..=hi`",
        NONE,
        true,
        "13",
        "Repeats the controls inside.",
    ),
    n(
        "labels",
        (1, 1),
        "language, `en`",
        Some(&[p("source", "where the texts come from")]),
        true,
        "13.1",
        "UI texts.",
    ),
    n(
        "labels/list",
        (1, 1),
        "name",
        NONE,
        true,
        "13.1",
        "A numbered list of texts.",
    ),
    n(
        "labels/list/item",
        (2, 2),
        "number, text",
        NONE,
        false,
        "13.1",
        "One text.",
    ),
    // ---- MIDI -------------------------------------------------------------
    n(
        "midi-map",
        (0, 0),
        "",
        Some(&[p("transport", "the performance transport")]),
        true,
        "7",
        "What the device sends and receives when played.",
    ),
    n(
        "midi-map/source|sink",
        (1, 1),
        "a block or name",
        Some(&[
            p("count", "elements"),
            p(
            "kind",
            "`cc`, `cc14`, `nrpn`, `rpn`, `note`, `pc`, `pb`, `at`, `poly-at`, `clock`, `from-field`",
        ),
            p("cc", "CC number or expression"),
            p("note", "note number or expression"),
            p("channel", "channel or expression"),
            p("message", "message type expression"),
            p("range", "value range"),
        ]),
        false,
        "7",
        "One stream of MIDI messages.",
    ),
    // ---- messages ---------------------------------------------------------
    n(
        "sysex-frame",
        (1, 1),
        "name",
        NONE,
        true,
        "8.1",
        "A vendor's SysEx framing.",
    ),
    n(
        "sysex-frame/prefix|suffix",
        (1, 1),
        "template",
        NONE,
        false,
        "8.1",
        "Bytes before and after each message body.",
    ),
    n(
        "message",
        (1, 1),
        "name",
        Some(&[
            p("transport", "a transport name, or `selected`"),
            p("frame", "a `sysex-frame`"),
            p("kind", "`read`, `write` or `commit`"),
        ]),
        true,
        "6",
        "One request and its replies.",
    ),
    n(
        "message/arg",
        (1, 1),
        "an argument name",
        Some(&[
            p("range", "allowed values, `lo..=hi`"),
            p("index", "a block: 0..count-1"),
            p("type", "a `types` enum or int, or a storage type: its values"),
        ]),
        false,
        "6.5",
        "The values an argument may take.",
    ),
    n(
        "message/send",
        (0, 1),
        "template",
        Some(&[p("pattern", "a named pattern instead")]),
        false,
        "6",
        "The bytes sent.",
    ),
    n(
        "message/reply",
        (0, 1),
        "template",
        Some(&[
            p("timeout", "milliseconds"),
            p("variant", "name; required with several replies"),
            p("pattern", "a named pattern instead"),
            p("ok-when", "expression; overrides the message's"),
        ]),
        false,
        "6.1",
        "An expected reply; the first that matches wins.",
    ),
    n(
        "message/ok-when",
        (1, 1),
        "expression",
        NONE,
        false,
        "6",
        "When a reply means success.",
    ),
    n(
        "message/derive",
        (2, 2),
        "name, expression",
        None,
        false,
        "6.7",
        "A value computed from the others: sent when left out, checked on decode.",
    ),
    n(
        "message/records",
        (1, 1),
        "the byte array that holds the records",
        Some(&[
            p("pattern", "the named pattern of one record"),
            p("address", "expression: a record's memory address"),
            p("data", "the record's data bytes, a byte array of the pattern"),
            p("pace", "ms the host waits after each record it sends"),
        ]),
        true,
        "8.3",
        "A run of addressed records inside one message.",
    ),
    n(
        "message/records/derive",
        (2, 2),
        "name, expression",
        None,
        false,
        "8.3",
        "A record's value computed from the others.",
    ),
    n(
        "message/segment",
        (0, 0),
        "",
        Some(&[
            p("message", "the message that carries one piece"),
            p("over", "the byte-array argument that is split"),
            p("size", "bytes of that argument per piece, before encoding"),
            p(
                "threshold",
                "split when the whole message is longer; default the transport's `sysex-limit`",
            ),
        ]),
        false,
        "8.2",
        "The device's own multi-message form of a long send.",
    ),
    // ---- procedures -------------------------------------------------------
    n(
        "procedure",
        (1, 1),
        "name",
        Some(&[
            p("kind", "`read`, `write` or `commit`"),
            p("transport", "default transport"),
        ]),
        true,
        "11",
        "Steps the editor runs.",
    ),
    n(
        "step send|exchange",
        (1, 1),
        "a message",
        None,
        false,
        "11",
        "Send, or send and wait for the reply. Properties are message arguments, \
       except `into` (block path for the reply), `from` (block path for the \
       template), `records-from` (block paths that fill the message's \
       `records`, §8.3) and `timeout`.",
    ),
    n(
        "step await",
        (1, 1),
        "a message",
        None,
        false,
        "11",
        "Wait up to `timeout` ms for a message the device sends by itself (a dump \
       the user starts). Properties are values the reply must carry, except \
       `into` and `timeout`. A reply with `records` fills the addressed blocks.",
    ),
    n(
        "step repeat",
        (2, 2),
        "variable, `lo..=hi` or a sequence",
        Some(&[p("size", "chunk size for a sequence")]),
        true,
        "11",
        "Repeats the steps inside.",
    ),
    n(
        "step read-into",
        (1, 1),
        "a block path",
        Some(&[p("from", "a message")]),
        false,
        "11",
        "Copies a reply into a block.",
    ),
    n(
        "step delay",
        (1, 1),
        "milliseconds",
        NONE,
        false,
        "11",
        "Waits.",
    ),
    n(
        "step verify",
        (1, 1),
        "a read procedure",
        Some(&[p("compare", "paths to compare; all written ones if absent")]),
        false,
        "11",
        "Reads back and compares.",
    ),
    n(
        "step gate",
        (1, 1),
        "`confirm` or `arm`",
        Some(&[p("text", "what the user is asked")]),
        false,
        "11",
        "Asks the user; `arm` before a commit.",
    ),
    n(
        "step restore-point",
        (1, MANY),
        "block paths",
        NONE,
        false,
        "11",
        "Saves what a write will change.",
    ),
    n(
        "step require|ok-when",
        (1, 1),
        "expression",
        NONE,
        false,
        "11",
        "Stops unless true.",
    ),
    n(
        "step call",
        (1, 1),
        "a procedure",
        NONE,
        false,
        "11",
        "Runs another procedure.",
    ),
    n(
        "step driver",
        (1, 1),
        "a driver",
        Some(&[p("step", "the driver's step name")]),
        false,
        "14",
        "A step a driver performs.",
    ),
    n(
        "step expect-identity",
        (0, 0),
        "",
        Some(&[
            p("product", "the identity's product value"),
            p("timeout", "milliseconds"),
        ]),
        false,
        "12",
        "Waits until the device identifies as expected.",
    ),
    // ---- by hand (§17) ----------------------------------------------------
    n(
        "hands",
        (0, 0),
        "",
        NONE,
        true,
        "17.1",
        "What a person operates on the device: keys, encoders, controls, number entry.",
    ),
    n(
        "hands/key",
        (1, 1),
        "id; with `range`, `{n}` in it stands for each value",
        Some(&[
            p("label", "as printed on the device; may use `{n}`"),
            p("range", "`lo..=hi`: one key per value of `n`"),
        ]),
        false,
        "17.1",
        "A key, encoder, fader or other control a guide names.",
    ),
    n(
        "hands/entry",
        (1, 1),
        "name",
        Some(&[p("text", "how to type a number, shown once per guide")]),
        false,
        "17.1",
        "A way of typing numbers digit by digit.",
    ),
    n(
        "recipe",
        (1, 1),
        "name",
        Some(&[p("args", "parameter names, space-separated")]),
        true,
        "17.3",
        "Named guide steps; `do` runs them.",
    ),
    n(
        "edit",
        (1, 1),
        "block",
        Some(&[
            p("index", "the element's index variable, `i` when absent"),
            p("read", "the read procedure that shows what the element holds now"),
            p("stays-open", "#true: after `close` the element is still current"),
        ]),
        true,
        "17.4",
        "How a block element's stored fields change by hand.",
    ),
    n(
        "edit/open|edit/close|edit/check",
        (0, 0),
        "",
        NONE,
        true,
        "17.4",
        "Guide steps: make the element current, keep the change in memory, let the editor read it back.",
    ),
    n(
        "edit/item/select",
        (0, 0),
        "",
        NONE,
        true,
        "17.4",
        "Guide steps that select the item.",
    ),
    n(
        "edit/item",
        (1, 1),
        "array path below the block, `group.encoder`",
        Some(&[p("index", "one index variable per array, space-separated")]),
        true,
        "17.4",
        "An array element: how to select it and set its fields.",
    ),
    n(
        "edit/field",
        (1, 1),
        "field name",
        Some(&[
            p("applies", "expression: the field is set by hand only when it holds"),
            p("index", "for an array of values: the element's index variable"),
        ]),
        true,
        "17.4",
        "The steps that set one field to `v`.",
    ),
    n(
        "kinds",
        (1, 1),
        "array path below the block, `preset.control`",
        Some(&[
            p("by", "the field whose value picks the kind"),
            p("index", "index variables, block first, space-separated"),
        ]),
        true,
        "17.6",
        "What an array element can be set to send, for the editor's type choice.",
    ),
    n(
        "kinds/kind",
        (1, 1),
        "name",
        Some(&[
            p("label", "what the editor shows"),
            p("values", "the `by` values of this kind: a number or `lo..=hi`"),
            p("when", "expression over the index variables: where the kind is offered"),
        ]),
        true,
        "17.6",
        "One kind of message.",
    ),
    n(
        "kinds/kind/param",
        (1, 1),
        "field",
        Some(&[
            p("label", "the field's name for this kind"),
            p("default", "the value it takes when the kind is chosen"),
            p("when", "expression over the index variables: where the field is used"),
        ]),
        false,
        "17.6",
        "A field the kind uses.",
    ),
    n(
        "guide press",
        (1, 1),
        "key",
        Some(GUIDE_PROPS_TIMES),
        false,
        "17.2",
        "Press and release a key.",
    ),
    n(
        "guide hold",
        (1, 1),
        "key",
        Some(GUIDE_PROPS_HOLD),
        false,
        "17.2",
        "Hold a key down.",
    ),
    n(
        "guide chord",
        (2, MANY),
        "keys",
        Some(GUIDE_PROPS),
        false,
        "17.2",
        "Press keys together.",
    ),
    n(
        "guide type",
        (1, 1),
        "number, `{expression}`",
        Some(GUIDE_PROPS_TYPE),
        false,
        "17.2",
        "Type a number digit by digit.",
    ),
    n(
        "guide turn",
        (1, 1),
        "key",
        Some(GUIDE_PROPS_TURN),
        false,
        "17.2",
        "Turn an encoder until the display shows a value.",
    ),
    n(
        "guide move",
        (1, 1),
        "key",
        Some(GUIDE_PROPS),
        false,
        "17.2",
        "Move or touch a control.",
    ),
    n(
        "guide say",
        (1, 1),
        "text",
        Some(GUIDE_PROPS),
        false,
        "17.2",
        "A sentence for the user.",
    ),
    n(
        "guide ask",
        (1, 1),
        "path",
        Some(&[p("text", "what to read on the device")]),
        false,
        "17.2",
        "The user reads a value on the device and enters it.",
    ),
    n(
        "guide one-of",
        (0, 0),
        "",
        Some(&[SHOWS, HINT]),
        true,
        "17.2",
        "Alternatives, as `option` children.",
    ),
    n(
        "guide one-of/option",
        (0, 0),
        "",
        NONE,
        true,
        "17.2",
        "Guide steps; any one option will do.",
    ),
    n(
        "guide when",
        (1, 1),
        "expression",
        NONE,
        true,
        "17.2",
        "Guide steps that apply only when the expression holds.",
    ),
    n(
        "guide do",
        (1, 1),
        "recipe",
        None,
        false,
        "17.3",
        "Runs a recipe; other properties are its arguments.",
    ),
    // ---- firmware, LBMT, drivers ------------------------------------------
    n(
        "firmware",
        (0, 0),
        "",
        NONE,
        true,
        "12",
        "The firmware update.",
    ),
    n(
        "firmware/image",
        (0, 0),
        "",
        Some(&[
            p("format", "image format"),
            p("magic", "first bytes"),
            p("driver", "the driver that reads it"),
        ]),
        false,
        "12",
        "The update file.",
    ),
    n(
        "firmware/version-from",
        (1, 1),
        "a message",
        Some(&[p("field", "the reply values that form the version")]),
        false,
        "12",
        "Where the running version comes from.",
    ),
    n(
        "firmware/require",
        (1, 1),
        "expression",
        NONE,
        false,
        "12",
        "A condition before updating.",
    ),
    n(
        "firmware/stage",
        (1, 1),
        "name",
        Some(&[p("kind", "`read`, `write` or `commit`")]),
        true,
        "12",
        "One stage of the update, with steps.",
    ),
    n(
        "firmware/recovery",
        (1, 1),
        "text",
        NONE,
        false,
        "12",
        "What to do when it fails.",
    ),
    n(
        "lbmt",
        (2, 2),
        "name, LBMT source",
        Some(&[
            p("from", "input port or transport"),
            p("to", "output port"),
            p("enabled", "on at start"),
        ]),
        false,
        "10.3",
        "A live MIDI rule.",
    ),
    n(
        "driver",
        (1, 1),
        "name",
        Some(&[
            p("provides", "what it supplies: `block:…`, `file:…`"),
            p("from", "what it reads"),
        ]),
        true,
        "14",
        "Code the descriptor cannot express.",
    ),
    n(
        "driver/owns",
        (1, 1),
        "name",
        Some(&[
            p("bytes", "the bytes the driver builds"),
            p("frame", "a frame"),
        ]),
        false,
        "14",
        "Bytes on record that the driver sends.",
    ),
    n(
        "test-vector",
        (1, 1),
        "name",
        Some(&[
            p("message", "the message"),
            p("block", "or a block, from a file"),
            p("file", "external data; runs only when present"),
            p("sha256", "the file's hash"),
            p("reply", "`#true`: decode with the reply"),
            p("variant", "the reply variant that must match"),
        ]),
        true,
        "15",
        "Bytes the codec must produce and decode.",
    ),
    n(
        "test-vector/args",
        (0, 0),
        "",
        None,
        false,
        "15",
        "Argument values, as properties.",
    ),
    n(
        "test-vector/bytes",
        (1, 1),
        "hex",
        NONE,
        false,
        "15",
        "The whole message.",
    ),
    n(
        "test-vector/data",
        (2, 2),
        "name, hex",
        NONE,
        false,
        "15",
        "A byte-array argument.",
    ),
];

/// The entry for `path`. Every path the converter uses has one; a test
/// checks it.
pub fn spec(path: &str) -> &'static NodeSpec {
    SCHEMA
        .iter()
        .find(|s| s.path == path)
        .unwrap_or_else(|| panic!("no schema entry for `{path}`"))
}

fn args_text(s: &NodeSpec) -> String {
    match s.args {
        (0, 0) => "none".into(),
        (a, b) if a == b => format!("{a}: {}", s.args_doc),
        (a, MANY) => format!("{a} or more: {}", s.args_doc),
        (a, b) => format!("{a} to {b}: {}", s.args_doc),
    }
}

/// `docs/cdl-schema.md`.
pub fn markdown() -> String {
    let mut out = String::from(
        "# CDL schema\n\n\
         Generated from `crates/ctrl-cdl/src/schema.rs` by `ctrl-cli schema`; do\n\
         not edit. The converter checks every node against the same table. A\n\
         path `a/b` is node `b` inside `a`; `block … field` is a field at any\n\
         depth of a block; `step x` is a step in a procedure, a firmware stage\n\
         or a repeat. Section numbers refer to `docs/CDL.md`.\n\n\
         Every node also takes:\n\n| Property | Meaning |\n|---|---|\n",
    );
    for c in COMMON {
        out += &format!("| `{}` | {} |\n", c.name, c.doc);
    }
    for s in SCHEMA {
        let names = s.path.replace('|', "`, `");
        out += &format!("\n## `{names}`\n\n{} (§{})\n\n", s.doc, s.section);
        out += &format!("- Arguments: {}\n", args_text(s));
        out += &format!("- Children: {}\n", if s.children { "yes" } else { "none" });
        match s.props {
            None => out += "- Properties: any (see above)\n",
            Some([]) => out += "- Properties: none besides the common ones\n",
            Some(props) => {
                out += "\n| Property | Meaning |\n|---|---|\n";
                for p in props {
                    out += &format!("| `{}` | {} |\n", p.name, p.doc);
                }
            }
        }
    }
    out
}
