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

pub static SCHEMA: &[NodeSpec] = &[
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
