# Tree-sitter queries for CDL

CDL files are KDL 2.0, so they parse with
[tree-sitter-kdl](https://github.com/amaanq/tree-sitter-kdl) (2.x). The
queries in `queries/cdl/` add CDL's meaning on top of KDL's own queries
(`; inherits: kdl`):

- `highlights.scm`: CDL node names (top level, procedure steps, inner
  nodes); `status` values, `measured`/`decoded` apart from
  `inferred`/`unknown`; `evidence` and `note`; expression properties
  (`ok-when=`, `visible-when=`, …); byte templates.
- `injections.scm`: LBMT source as language `lbmt`. No LBMT grammar
  exists yet; until one does, the source shows as a string.

Templates and LBMT are recognised by their content (a template starts
with hex bytes, `..` or `{`; LBMT with `@preset` or `@translator`), not by
the node they belong to. A pattern that puts `node_field` under `node`
takes minutes to compile against tree-sitter-kdl 2.0 (measured: 4 to 7
minutes per pattern with tree-sitter 0.27), and an editor would stall the
same way. For the same reason the `ok-when "…"` node form is not
highlighted as an expression, only the property form.

Use: register a `cdl` filetype for `*.ctrl.kdl` and `*.inc.kdl` that uses
the `kdl` parser, and copy `queries/cdl/` into the editor's query path
(Neovim: `~/.config/nvim/queries/cdl/`, with
`vim.treesitter.language.register("kdl", "cdl")`).

`tools/cdl-queries` compiles both files against the grammar, runs them on
the repository's descriptors and checks that every node in the CDL node
table (`crates/ctrl-cdl/src/schema.rs`) is highlighted:
`cargo test -p cdl-queries`.
