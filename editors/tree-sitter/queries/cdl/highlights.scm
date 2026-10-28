; inherits: kdl
; CDL (docs/CDL.md) is KDL 2.0 with a fixed set of nodes. These patterns
; come after KDL's, so they win. tools/cdl-queries checks them against the
; tree-sitter-kdl grammar and the node table (crates/ctrl-cdl/src/schema.rs).

; Top-level nodes
((node name: (identifier) @keyword)
  (#any-of? @keyword
    "cdl" "include" "device" "identify" "transport" "types" "block" "param"
    "panel" "midi-map" "sysex-frame" "message" "procedure" "firmware" "lbmt"
    "driver" "test-vector" "const" "labels" "pattern"))

; Procedure steps
((node name: (identifier) @keyword.function)
  (#any-of? @keyword.function
    "send" "exchange" "repeat" "read-into" "delay" "verify" "gate"
    "restore-point" "require" "call" "expect-identity" "ok-when"))

; Nodes inside the others
((node name: (identifier) @tag.attribute)
  (#any-of? @tag.attribute
    "vendor" "model" "class" "file-key" "manual" "replaces" "aka" "image"
    "usb" "midi-port" "sysex-identity" "hid" "key" "listen" "capture" "report-descriptor" "poll" "segment"
    "enum" "errors" "int" "value" "field" "group" "array" "control" "list"
    "item" "source" "sink" "prefix" "suffix" "arg" "reply" "owns"
    "version-from" "stage" "recovery" "args" "bytes" "data"))

; Evidence status: how far a fact can be trusted (CDL §4.1)
((prop
   key: (identifier) @_key
   value: (value value: (string (string_fragment) @constant.builtin)))
  (#eq? @_key "status")
  (#any-of? @constant.builtin "measured" "decoded"))

((prop
   key: (identifier) @_key
   value: (value value: (string (string_fragment) @comment.warning)))
  (#eq? @_key "status")
  (#any-of? @comment.warning "inferred" "unknown"))

((prop
   key: (identifier) @_key
   value: (value value: (string) @string.documentation))
  (#any-of? @_key "evidence" "note"))

; Expressions (CDL §10.1)
((prop
   key: (identifier) @_key
   value: (value value: (string) @string.special))
  (#any-of? @_key "ok-when" "visible-when" "display" "parse" "range" "box"))

; Message templates (CDL §6): byte templates start with hex bytes, `..`
; or a `{…}` placeholder. Matched by content: a pattern that puts
; `node_field` under `node` takes minutes to compile against tree-sitter-kdl.
((value value: (string (string_fragment) @string.special))
  (#match? @string.special "^(([0-9A-Fa-f][0-9A-Fa-f]|[.][.])( |$)|[{])"))
