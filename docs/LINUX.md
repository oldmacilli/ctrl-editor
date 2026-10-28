# The editors on Linux: MIDI backends, PipeWire, Wayland, USB

Status 2026-10-28. Applies to the KORG, DB-1, TEC and UC-33 editors and to
the shared platform. Claims carry their source; **[unverified]** marks what
no test or capture here has shown yet. The KORG repo's
`docs/packaging-flatpak-hardware.md` has the sourced Flatpak research this
file builds on.

## 1. Summary

| Topic | Answer |
|---|---|
| Default MIDI path | ALSA sequencer (kernel), through midir. Works with or without PipeWire or JACK running |
| Second path | JACK, through `ctrl-midi`'s JACK backend: a real jackd2, or PipeWire's libjack |
| Switching | At run time: GUI button **MIDI: …**, TUI key `M`, CLI `--midi-backend alsa\|jack`, or `CTRL_MIDI_BACKEND`; the choice is saved |
| PipeWire | Nothing PipeWire-specific is needed: the devices are kernel ALSA devices. With PipeWire the ALSA path is unchanged, and the JACK path is PipeWire's JACK |
| Wayland | Every Slint GUI is built with winit's Wayland and X11 support; it uses Wayland when `WAYLAND_DISPLAY` is set. `tools/linux-check` starts each one on a headless Weston |
| TEC breath controller | Configuration is USB HID, not MIDI. Linux is planned on hidraw (§6); today `tec-device` returns "not implemented" there |

## 2. Choosing the MIDI backend

`ctrl-midi` (this repository, `crates/ctrl-midi`) opens MIDI ports on one
of these backends, chosen at run time:

| Backend | id | Through | Default on |
|---|---|---|---|
| ALSA sequencer | `alsa` | `/dev/snd/seq`, midir | Linux |
| JACK | `jack` (also `pipewire`, `pw-jack`) | libjack, loaded at run time | none |
| CoreMIDI | `coremidi` | midir | macOS |

The editor picks, first match wins:

1. `--midi-backend <id>` on the command line (GUI, TUI and CLI);
2. `CTRL_MIDI_BACKEND=<id>` in the environment;
3. the saved choice: `$XDG_DATA_HOME/<Editor>/midi-backend`
   (`~/.local/share/UC33-Editor/midi-backend`), written whenever the
   backend is switched in the GUI or TUI;
4. the platform's own: ALSA on Linux.

Switching closes every port and reopens the device on the new backend
within two seconds; nothing else in the editor changes. If the chosen
backend cannot run, the editor says why ("libjack is not installed", "no
JACK server answers") in its connection line and its advice, and stays on
it until switched back.

```sh
uc33 --midi-backend jack ports            # every backend's ports, the UC-33's marked
CTRL_MIDI_BACKEND=jack uc33-gui
pw-jack uc33-tui --midi-backend jack      # PipeWire's libjack on distributions that keep it out of the library path
```

Building: the `jack` crate's build script asks pkg-config for `jack` on
Linux, so a build host needs `libjack-jackd2-dev` (Debian/Ubuntu),
`jack-audio-connection-kit-devel` or `pipewire-jack-audio-connection-kit-devel`
(Fedora). The binary does not link libjack: a machine without it runs the
ALSA backend and reports JACK as unavailable. `--no-default-features` on
`ctrl-midi` drops JACK entirely.

## 3. ALSA or JACK: what changes

| | ALSA sequencer | JACK |
|---|---|---|
| Needs | the kernel's `snd-seq` (always there with ALSA) | a running JACK server: jackd2, or PipeWire with its JACK layer |
| Sees the device | directly: snd-usb-audio makes every USB-MIDI device a sequencer client; BLE MIDI devices too (bluetoothd) | only through a bridge: PipeWire's Midi-Bridge does it by itself; with jackd2 run `a2jmidid -e` (or start jackd with `-X seq`) |
| Port names | `UC-33 USB MIDI Controller:UC-33 USB MIDI Controller MIDI 1 24:0`; the trailing address is dropped before matching (`ctrl_midi::display_name`) | `Midi-Bridge:UC-33 USB MIDI Controller 1:(capture_0) … MIDI 1` (PipeWire) or `a2j:UC-33 USB MIDI Controller [24] (capture): … MIDI 1` (a2jmidid); aliases carry the ALSA names **[unverified on hardware; formats from the PipeWire and a2jmidid docs]** |
| Matching a descriptor's `port-match` | substring of the name | substring of the name or an alias, so the same descriptors work |
| Timing | events go out as soon as they are written | events go out in the next process cycle: one period of latency (e.g. 256 frames at 48 kHz ≈ 5.3 ms) |
| Long SysEx, editor side | written in pieces of the descriptor's `sysex-limit` with `segment-delay` between them (CDL §8.2: KORG 256 bytes / 5 ms), because the kernel's `snd_seq_midi` drops the tail of an event over its 4096-byte buffer | sent as one JACK event, never split: a JACK MIDI event is a whole message. `ctrl-midi` refuses a message larger than the port's event buffer with an error instead of truncating it |
| Long SysEx, to the device | as above: the device receives one continuous SysEx | the bridge writes it into the ALSA sequencer again, so the same 4096-byte kernel buffer applies there, and the editor cannot pace it. PipeWire's Midi-Bridge has also lost messages under load ([pipewire#2775](https://gitlab.freedesktop.org/pipewire/pipewire/-/issues/2775)) |
| Consequence for dumps | safe at any size | stay on ALSA for transfers above ~4 KiB, or use the device's own multi-part form (CDL `segment`, §8.2), which is short messages on every backend. No device in this repository sends more than 651 bytes today |
| Other programs | any number of programs may subscribe to the same port | the same through the graph; a patchbay (qpwgraph, Carla) shows the editor's `ctrl-editor-in`/`-out` ports and may rewire them |
| Real time | no real-time thread in the editor | the editor's JACK callback runs in the server's real-time thread; it only copies bytes through a lock-free ring buffer |
| When to choose it | the default, and always for writing dumps | when the rest of the studio runs on JACK or PipeWire and the device should appear in that graph, or when ALSA access is not granted (a Flatpak with only the PipeWire socket) |

In short: ALSA is the device's own path and the safer one for writing.
JACK puts the editor into an existing audio graph, at the cost of one
period of latency and a bridge the editor cannot pace.

## 4. PipeWire

PipeWire does not add a path to a USB-MIDI device that ALSA lacks:
snd-usb-audio owns the device in the kernel, and PipeWire's Midi-Bridge is
itself a sequencer client ([PipeWire MIDI](https://docs.pipewire.org/page_midi.html)).
So:

- **ALSA backend under PipeWire:** unchanged. The editor and the
  Midi-Bridge are two sequencer clients of the same port.
- **JACK backend under PipeWire:** the libjack the editor loads is
  PipeWire's (`pipewire-jack`). Fedora and Arch install it as the system
  libjack; Debian and Ubuntu keep it in a private directory, so start the
  editor with `pw-jack` there. The device appears through the Midi-Bridge.
- **No libpipewire dependency** is planned: it would add a third backend
  with the same reach as the two above.

`tools/linux-check` runs the JACK loopback (a CC and a 6002-byte SysEx
through a JACK client and back) under jackd2 and under PipeWire's libjack.

## 5. Wayland and the Slint GUIs

Every GUI uses Slint's winit backend, which is compiled with both Wayland
and X11 support. All four `Cargo.lock` files hold `wayland-client`,
`sctk-adwaita` (window decorations drawn by the client, which GNOME
requires) and `x11rb`. At run time winit takes Wayland when
`WAYLAND_DISPLAY` is set and X11 otherwise.

| Need | Setting |
|---|---|
| Force Wayland or X11 | `SLINT_BACKEND=winit-wayland` / `winit-x11` |
| No GPU (VMs, broken drivers) | `SLINT_BACKEND=winit-software` (software renderer, still Wayland) |
| Flatpak | `--socket=wayland --socket=fallback-x11 --share=ipc --device=dri` (KORG manifest) |
| Renderer per editor | KORG, UC-33: Slint defaults (FemtoVG on EGL, software fallback); DB-1: software renderer; TEC: FemtoVG. FemtoVG needs EGL: Mesa's `libegl1` is enough |

`tools/linux-check/run.sh wayland` builds each GUI in a Debian container,
starts a headless Weston, and runs each GUI for 8 s with
`SLINT_BACKEND=winit-wayland` and with `winit-software`; it passes when the
GUI creates an `xdg_toplevel` (a mapped window) and is still running. That
covers starting and drawing on Wayland, not input or fractional scaling on
a real desktop **[unverified on GNOME/KDE hardware]**.

## 6. The TEC breath controller on Linux

The TEC has two USB functions (CDL `descriptors/tecontrol/breath.ctrl.kdl`
§"The USB side in full"): a class-compliant MIDI function for performance
data, and a HID interface (usage page 0x0C, 8-byte input and output
reports, no report id) that carries the configuration.

| Part | macOS today | Linux plan |
|---|---|---|
| Performance MIDI | CoreMIDI (`tec-midi`) | snd-usb-audio makes it an ALSA device like the others; `tec-midi` moves to `ctrl-midi`, so ALSA and JACK both work |
| Configuration (HID) | IOKit with Seize (`tec-device/src/macos.rs`) | hidraw: the kernel's usbhid binds the interface and exposes `/dev/hidrawN`; the `hidapi` crate's Linux-native backend opens it by VID/PID `2B06:0018` and serial. Platform crate `ctrl-hid-hidapi` (CONCEPT §3) |
| Report framing | 8 bytes, report id stripped | hidapi writes `[0x00] + 8 bytes` (report id 0) and reads 8 bytes: the descriptor's `report-id=0 frame=8` holds unchanged |
| Exclusive access | Seize keeps MIDIServer's HID event service off the interface | hidraw has no exclusive open and needs none: usage 0 on page 0x0C maps to no input event **[unverified: check with `evtest` that no event device appears]**. The hub serialises the editor's own frames; another program writing the same hidraw node would interleave |
| Permissions | none | `/dev/hidraw*` is root-only. Ship a udev rule: `SUBSYSTEM=="hidraw", ATTRS{idVendor}=="2b06", ATTRS{idProduct}=="0018", TAG+="uaccess"` (`70-tecontrol-breath.rules`), which gives the logged-in user access |
| Flatpak | — | hidraw has no device class of its own: needs `--device=all` plus the host's udev rule. The USB portal hands out libusb handles, which would mean detaching usbhid: not used |
| AppImage | — | cannot install udev rules; the README gives the rule and the `udevadm control --reload` step |
| Hot-plug | IOKit notifications | rescan every 2 s (as UC-33 does), or a udev monitor later |
| Write safety | `unsafe-commands`, arm gate, staged first write | the same. The first Linux write repeats HANDOVER "Gate 6": capture with `usbmon` (Wireshark) and compare frame for frame with the macOS HID traces before any commit (`50`) |

Status: `tec-device/src/linux.rs` and `tec-midi/src/linux.rs` are
placeholders that return "not implemented"; the TEC repository builds on
Linux (checked in `tools/linux-check`). The plan is in TODO.md under TEC.

## 7. Editor status

| Editor | MIDI on Linux | Backend switch | Wayland |
|---|---|---|---|
| UC-33 | `ctrl-midi` (ALSA, JACK) | GUI, TUI, CLI | checked (headless Weston, GL and software) |
| KORG | midir ALSA, SysEx chunked 256 B / 5 ms (`korg-midi-midir`) | to do: move to `ctrl-midi` | checked |
| DB-1 | midir ALSA (`db1-midi`) | to do: move to `ctrl-midi` | checked |
| TEC | not implemented (§6) | with `ctrl-midi`, when Linux lands | checked |

## 8. Checking

```sh
tools/linux-check/run.sh            # all: JACK loopback (jackd2, PipeWire), four GUIs on Wayland
tools/linux-check/run.sh midi
tools/linux-check/run.sh wayland
CTRL_MIDI_JACK_TEST=1 cargo test -p ctrl-midi -- --ignored   # on a Linux host with JACK running
```

The container has no `/dev/snd`: Docker's VM kernel has no ALSA
sequencer, so the ALSA backend is only built there, not run. On a Linux
machine with the device, check by hand:

1. `aconnect -l` lists the device; `uc33 --midi-backend alsa ports` marks it.
2. With PipeWire: `uc33 --midi-backend jack ports` (or under `pw-jack`)
   marks it through the Midi-Bridge.
3. The editor follows a control move on both backends.
4. For editors that write: a full dump on ALSA, then read back and compare.
