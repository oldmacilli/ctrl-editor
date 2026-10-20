# Agent instructions: ctrl-editor

Shared platform for three device editors. Read docs/CONCEPT.md first.

## Rules

- Keep docs/TODO.md alive: tick items in the commit that finishes them,
  with the hash; add new items under their phase with an owner.
- Descriptors follow docs/CDL.md. Every value that reaches a device carries
  `status` and `evidence`. Never write a byte you have not seen in a
  capture or code; use `status="unknown"` instead.
- Do not commit vendor binaries, firmware images, manuals or extracted
  vendor content. Test data lives under `the local capture archive`.
- The evidence for each device stays in its own repo
  (`KORG_Kontrol_Editor`, `OceanBeach_DB-1`, `TECcontrol`); link to it.
- `lbmt` belongs to zigtact. Use it as a dependency; do not copy or patch
  it here.
- API changes to the core (EditorController, ViewModel) need the KORG lane's
  ack, as in the KORG repo.
- Run `cargo fmt --all` after Rust edits.
