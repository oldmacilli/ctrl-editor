# ctrl-editor (working name)

One codebase, one GUI (Slint) and one TUI (ratatui) for the KORG Kontrol
Editor, the Ocean Beach DB-1 editor and the TEControl breath controller
editor. Devices are described in CDL (`*.ctrl.kdl`); Rust drivers cover
what data cannot express.

Status: concept. No platform crates yet.

- [docs/CONCEPT.md](docs/CONCEPT.md): why and how
- [docs/CDL.md](docs/CDL.md): the descriptor language
- [docs/TODO.md](docs/TODO.md): the living plan
- `descriptors/`: example descriptors (DB-1, TEC, nanoKONTROL Studio,
  firmware template)
- `tools/cdl-parse`: KDL 2.0 syntax check for descriptors

```bash
cargo run --manifest-path tools/cdl-parse/Cargo.toml -- descriptors/*/*.kdl
```
