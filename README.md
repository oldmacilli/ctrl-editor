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
- `descriptors/`: example descriptors (DB-1, TEC, nanoKONTROL Studio,
  firmware template)
- `crates/ctrl-cdl`: CDL loader: KDL 2.0 into a typed IR, includes merged,
  errors as `file:line:col`
- `crates/ctrl-codec`: message templates to bytes and back; runs the
  descriptors' test vectors
- `tools/cdl-parse`: loads, validates and runs the test vectors of
  descriptors

```bash
cargo run -p cdl-parse -- descriptors/*/*.ctrl.kdl
cargo test --workspace
```
