# KORG device data

This crate loads KORG CDL descriptions and their includes from an external
descriptor directory. Set `KORG_DESCRIPTOR_DIR` to the directory containing
`korg/` when the descriptions are not installed beside the executable.

From the `ctrl-editor` workspace, run the native tests with:

```sh
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/../ctrl-editor-build/target}"
KORG_DESCRIPTOR_DIR="$PWD/descriptors" cargo test -p korg-devices -p korg-core
```

Four additional tests compare against private extracted analysis. They are
explicitly ignored without those inputs and can be run when the separate KORG
analysis checkout is available:

```sh
KORG_DESCRIPTOR_DIR="$PWD/descriptors" \
KORG_ANALYSIS_ROOT=/path/to/KORG_Kontrol_Editor \
  cargo test -p korg-devices --lib -- --ignored
```

Factory-file checks are separate and use `KORG_PRODUCTS_DIR` when set. No
vendor files or extracted factory data belong in this repository.
