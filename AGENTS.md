# Agent instructions: ctrl-editor

Shared platform for three device editors. Read docs/CONCEPT.md first.

## Rules

- On this development computer, build and test only the native platform and
  architecture. Other platforms and universal builds belong on remote LAN CI.
  Cargo's default target directory is outside the checkout; CI can override it.
- Keep docs/TODO.md alive: tick items in the commit that finishes them,
  with the hash; add new items under their phase with an owner.
- Descriptors follow docs/CDL.md. Every value that reaches a device carries
  `status` and `evidence`. Never write a byte you have not seen in a
  capture or code; use `status="unknown"` instead.
- Do not commit vendor binaries, firmware images, manuals or extracted
  vendor content. Test data lives outside the repository.
  Exception, approved by the owner on 2026-10-20: KORG labels converted into
  CDL `labels` and `enum` nodes by `tools/gen-korg-cdl` in the private
  KORG_Kontrol_Editor repository. Device images remain external. Generated
  `*.gen.inc.kdl` files are not edited by hand; rerun the generator.
- The evidence for each device stays in its own repo
  (`KORG_Kontrol_Editor`, `OceanBeach_DB-1`, `TECcontrol`); link to it.
- ctrl-editor is the layer below every device editor. It holds only `ctrl-*`
  crates, tools and the vendor CDL descriptors; no vendor crate (`korg-*`,
  `db1-*`, `tec-*`, ...). Device editors pin the `ctrl-*` crates by rev.
- `lbmt` belongs to zigtact. Use it as a dependency; do not copy or patch
  it here.
- API changes to the core (EditorController, ViewModel) need the KORG lane's
  ack, as in the KORG repo.
- Run `cargo fmt --all` after Rust edits.
- Each editor repo proves its own protocol code against the shared
  descriptor with a conformance test (`ctrl-descriptors` + `ctrl-codec`,
  pinned git dependencies). A descriptor change that alters bytes must
  keep those tests green; bump the pins there in the same breath.

- This repository is public. Evidence names captures, code and documents;
  keep binary analysis (routines, addresses, resource names, hashes) in the
  device repositories, which are private.
- No serial numbers or other identifiers of the owner's units in evidence
  or comments: write `<serial>`. Device ids, model ids and firmware
  versions are fine.
