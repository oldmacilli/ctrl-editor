; inherits: kdl
; LBMT source (CDL §10.3), the second argument of an `lbmt` node, usually a
; raw string #"""…"""#. It is recognised by its first directive, `@preset`
; or `@translator`: a pattern that looks at the `lbmt` node itself puts
; `node_field` under `node`, which takes minutes to compile against
; tree-sitter-kdl. No LBMT grammar exists yet; an editor that has one
; named `lbmt` highlights it.
((value value: (string (string_fragment) @injection.content))
  (#match? @injection.content "^[\\s\"]*@(preset|translator)")
  (#set! injection.language "lbmt"))
