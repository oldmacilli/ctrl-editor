# ctrl-editor (working name)

One codebase, one GUI (Slint) and one TUI (ratatui) for the KORG Kontrol
Editor, the Ocean Beach DB-1 editor and the TEControl breath controller
editor. Devices are described in CDL (`*.ctrl.kdl`); Rust drivers cover
what data cannot express.

Status: concept; P1 has started. `crates/ctrl-cdl` reads descriptors into a
typed IR.

- [docs/CONCEPT.md](docs/CONCEPT.md): why and how
- [docs/CDL.md](docs/CDL.md): the descriptor language
- [docs/TODO.md](docs/TODO.md): the living plan
- `descriptors/`: DB-1, TEC, all 16 KORG devices (2 measured, 14
  generated skeletons), a firmware template
- `crates/ctrl-cdl`: CDL loader: KDL 2.0 into a typed IR, includes merged,
  errors as `file:line:col`
- `crates/ctrl-codec`: message templates to bytes and back; runs the
  descriptors' test vectors
- `tools/cdl-parse`: loads, validates and runs the test vectors of
  descriptors
- `KORG_Kontrol_Editor/tools/gen-korg-cdl`: generates a CDL base for every device of the KORG
  KONTROL EDITOR from the KORG repo's analysis, with labels and images
  (`python3 KORG_Kontrol_Editor/tools/gen-korg-cdl/gen_korg_cdl.py`, `--check` for staleness)

```bash
cargo run -p cdl-parse -- descriptors/*/*.ctrl.kdl
cargo test --workspace
```
