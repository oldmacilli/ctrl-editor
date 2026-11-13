# The ctrl-editor philosophy

This is the contract between ctrl-editor and every device editor built on
it: the KORG Kontrol Editor, the TEC breath controller editor, the Ocean
Beach DB-1 editor, the Evolution UC-33 and Faderfox editors, and those to
come. Each rule says what must hold; the files named after it say where it
is enforced. `docs/CDL.md` is the language reference; this file is why
the language and the layers look as they do.

## 1. Layers

```
device editors      KORG_Kontrol_Editor  TECcontrol  OceanBeach_DB-1  …
                    (vendor crates, front ends, vendor file formats)
        │ pin one ctrl-editor rev; load descriptors
ctrl-editor         ctrl-cdl  ctrl-codec  ctrl-engine  ctrl-midi
                    ctrl-descriptors  ctrl-monitor*  ctrl-cli
        │ talk to
devices             MIDI 1.0 ports, USB HID / bulk  (PCIe later)
```

1. **ctrl-editor holds no vendor crate.** It holds `ctrl-*` crates, tools
   and the vendor descriptors (`descriptors/<vendor>/`). Vendor code
   (`korg-*`, `tec-*`, `db1-*`) lives in its editor's repository.
   Enforced: `AGENTS.md`; the workspace member list.
2. **A device editor depends downward only.** It pins every `ctrl-*` crate
   it uses at one ctrl-editor rev and bumps them together, deliberately,
   with its conformance tests. ctrl-editor never depends on an editor.
3. **Shared behaviour goes into the shared layer.** A missing monitor,
   transport or engine feature is added to a `ctrl-*` crate, not copied
   into an editor.

## 2. Descriptors are the device

4. **A device is described, not coded.** Identity, transports, messages,
   blocks, procedures, panels and guides are CDL. Editor code may add a
   vendor file format or a front end; it may not hold a second copy of a
   device's bytes. Where an editor still has its own protocol code, a
   conformance test compares it with the descriptor byte for byte, and a
   mismatch stays a named, ignored test until it is resolved.
5. **Descriptors are source code.** They live in git, are reviewed, and
   are loaded at run time from an installed directory or a pinned rev; no
   binary embeds device data. Generated parts (`*.gen.inc.kdl`) are
   regenerated, never edited; hand-written facts override them.
6. **Every fact carries its status and evidence.** `measured`, `decoded`,
   `inferred` or `unknown`, with the capture, trace or code it comes from
   (CDL §4.1). Unknown values stay unknown: nothing is guessed. An
   `unknown` node is disabled.
7. **Test vectors pin the bytes.** Every measured message has vectors
   from a capture; `ctrl-cli check` runs them.

## 3. Device classes

8. **Every descriptor declares its class** (`interaction`, CDL §5.2):

   | Class | What the device does | What software may do |
   |---|---|---|
   | `active` | answers requests and takes writes over MIDI | read and write on its own |
   | `passive` | sends or takes a SysEx dump only in a transfer mode the user starts | guide the user, then wait for or send the dump |
   | `push-only` | only sends its controls; set up on the device | listen and guide; never send |
   | `usb-mixed` | configured over USB (HID, bulk), plays over MIDI | read and write over USB only |
   | `bus-mixed` | PCIe and USB mixed with MIDI | nothing yet (reserved) |

9. **The class is checked, not trusted.** The validator derives it from
   the descriptor and refuses a declaration that disagrees; the engine
   runs nothing for a descriptor without a supported class or with a
   validation error, and refuses each forbidden step again at run time.
10. **Front ends offer what the class allows.** They read
    `Interaction::capabilities()`: Receive and Write for `active` and
    `usb-mixed`, a guided transfer for `passive`, guides only for
    `push-only`. An editor that supports only some classes refuses the
    others when it loads a descriptor.

## 4. Writing is guarded the same way everywhere

11. **No write without the user's yes.** Every write shows what it will
    change and asks (`gate "confirm"`); a permanent write (flash) also
    needs a runtime arm that expires (`gate "arm"`).
12. **No write without a way back.** The device's current content is read
    and kept as a restore point before the first byte is written.
13. **No write without proof.** After writing, the editor reads back and
    compares; the first differing byte is reported.
14. **A device is unknown until it has been read.** The editor shows
    "unknown", suggests a full receive, and writes only what differs.
15. **Reads are free; writes are opt-in in tools.** `ctrl-cli` sends
    nothing without `--apply`; tests and agents only receive unless the
    owner approves a write.
16. **Firmware updates are a class above writes** (CDL §12): image checks,
    no battery-low, no other job, decoded and captured procedures only.

## 5. Transports and ports

17. **One device, one entry.** A device answering on several ports or
    connections appears once; the editor chooses a preferred config
    transport and asks when the choice is ambiguous, never silently.
18. **Port matches never overlap.** `ctrl-cli check` refuses overlapping
    matches; the engine refuses a port two transports match.
19. **Measured host limits are part of the transport** (SysEx chunk size
    and pause, HID frame size, pacing), not constants in editor code.

## 6. Files and data

20. **Vendor files round-trip byte for byte.** An editor that reads a
    vendor format writes it back identically; no new formats are invented
    for data a vendor format already holds.
21. **Vendor data and assets stay out of the shared layer and out of
    packages.** Tests use synthetic data; tests against real files or
    devices are opt-in.

## 7. Conformance checklist for a device editor

A device editor conforms when:

- [ ] its `ctrl-*` dependencies use one pinned rev;
- [ ] its devices' descriptors pass `ctrl-cli check` at that rev and
      declare a class;
- [ ] it refuses descriptors of classes it does not support;
- [ ] every protocol byte it builds itself is compared with the
      descriptor's messages or vectors in a test;
- [ ] its writes follow §4 (confirm, restore point, verify; arm for
      permanent writes);
- [ ] it holds no copy of shared code (§1.3).

| Editor | Classes | State |
|---|---|---|
| KORG_Kontrol_Editor | `active` | pin 19ebdfa; loader refuses other classes; GUI, TUI and jobs offer transfers from the class |
| TECcontrol | `usb-mixed` | pin 5be3baa; conformance tests check bytes, class and HID-only config |
| OceanBeach_DB-1 | `active` | pin 9a48d0a; conformance tests check bytes and class; writes keep a restore point and read back |
| Faderfox UC4 | `passive` | descriptor, runs on `ctrl-engine` |
| Evolution UC-33 | `push-only` | descriptor, guides |
