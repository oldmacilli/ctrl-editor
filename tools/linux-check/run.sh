#!/usr/bin/env bash
# Linux checks for the editors, in a container (docs/LINUX.md):
#   1. ctrl-midi's JACK loopback under jackd2 (dummy driver) and under
#      PipeWire's libjack (pw-jack), with a 6002-byte SysEx;
#   2. each Slint GUI starts on Wayland (headless Weston) and maps a window.
# Usage: tools/linux-check/run.sh [midi|wayland|all]
# Source trees are exported read-only from git HEAD (KORG, DB-1, TEC main) or
# mounted (ctrl-editor, UC33-Editor working trees). No hardware is touched.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
dev=$(cd "$here/../../.." && pwd)
what=${1:-all}
img=ctrl-linux-check
docker build -q -t "$img" "$here" >/dev/null

src=$(mktemp -d "${TMPDIR:-/tmp}/ctrl-linux-src.XXXX")
trap 'rm -rf "$src"' EXIT
for r in KORG_Kontrol_Editor OceanBeach_DB-1 TECcontrol; do
  mkdir -p "$src/$r"
  git -C "$dev/$r" archive origin/main | tar -x -C "$src/$r"
done

# the working trees are copied (cargo writes Cargo.lock); targets stay out
for r in ctrl-editor UC33-Editor; do
  mkdir -p "$src/work/$r"
  (cd "$dev/$r" && tar --exclude=./target --exclude=./.git -cf - .) | tar -x -C "$src/work/$r"
done

# --shm-size: jackd keeps its graph in /dev/shm (64 MB by default in Docker)
docker run --rm -i --shm-size=512m \
  -v "$src:/src/main:ro" \
  -v ctrl-linux-cargo:/usr/local/cargo/registry \
  -v ctrl-linux-target:/target \
  -e WHAT="$what" "$img" bash -s <<'IN'
set -uo pipefail
cp -r /src/main/work /work
patch=()
if ! grep -q '^\[patch' /work/UC33-Editor/Cargo.toml; then
  for c in ctrl-cdl ctrl-codec ctrl-descriptors ctrl-midi; do
    patch+=(--config "patch.'https://github.com/oldmacilli/ctrl-editor.git'.$c.path='/work/ctrl-editor/crates/$c'")
  done
fi
fail=0
ok()  { echo "PASS $*"; }
bad() { echo "FAIL $*"; fail=1; }

if [[ $WHAT == midi || $WHAT == all ]]; then
  echo "== ctrl-midi: build, unit tests (ALSA backend compiled in)"
  cd /work/ctrl-editor
  CARGO_TARGET_DIR=/target/ctrl-editor cargo test -p ctrl-midi >/tmp/unit.log 2>&1
  if grep -q "test result: ok. 3 passed" /tmp/unit.log; then ok "ctrl-midi unit tests on Linux (3)"; else bad "ctrl-midi on Linux"; tail -15 /tmp/unit.log; fi
  # the named JACK test must report ok, not merely "0 failed"
  jack_ok() { grep -q "loopback_carries_cc_and_long_sysex ... ok" "$1"; }

  echo "== JACK loopback under jackd2 (dummy driver)"
  jackd --no-realtime --no-mlock -d dummy -r 48000 -p 256 >/tmp/jackd.log 2>&1 &
  jp=$!; sleep 2
  CTRL_MIDI_JACK_TEST=1 CARGO_TARGET_DIR=/target/ctrl-editor cargo test -p ctrl-midi -- --ignored >/tmp/jt.log 2>&1
  if jack_ok /tmp/jt.log; then ok "JACK (jackd2) loopback: CC and 6002-byte SysEx"; else bad "JACK (jackd2) loopback"; tail -15 /tmp/jt.log; tail -5 /tmp/jackd.log; fi
  kill $jp; wait $jp 2>/dev/null; sleep 1

  echo "== JACK loopback under PipeWire (pw-jack)"
  export DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address)
  pipewire >/tmp/pw.log 2>&1 & pw=$!; sleep 1
  wireplumber >/tmp/wp.log 2>&1 & wp=$!; sleep 2
  CTRL_MIDI_JACK_TEST=1 CARGO_TARGET_DIR=/target/ctrl-editor pw-jack cargo test -p ctrl-midi -- --ignored >/tmp/pt.log 2>&1
  if jack_ok /tmp/pt.log; then ok "JACK (PipeWire $(pipewire --version | grep -o 'libpipewire [0-9.]*' | head -1)) loopback: CC and 6002-byte SysEx"; else bad "JACK (PipeWire) loopback"; tail -15 /tmp/pt.log; tail -5 /tmp/pw.log; fi
  kill $wp $pw 2>/dev/null; wait 2>/dev/null
fi

if [[ $WHAT == wayland || $WHAT == all ]]; then
  echo "== Wayland: headless Weston"
  weston --backend=headless-backend.so --use-gl --socket=wayland-9 --idle-time=0 >/tmp/weston.log 2>&1 &
  sleep 2
  export WAYLAND_DISPLAY=wayland-9
  run_gui() { # name dir package bin [cargo args…]
    local name=$1 dir=$2 pkg=$3 bin=$4; shift 4
    echo "-- $name: build"
    ( cd "$dir" && CARGO_TARGET_DIR=/target/$name cargo build -q -p "$pkg" "$@" ) || { bad "$name build"; return; }
    for backend in winit-wayland winit-software; do
      local log=/tmp/$name-$backend.log
      SLINT_BACKEND=$backend WAYLAND_DEBUG=client timeout 8 /target/$name/debug/$bin >$log 2>&1
      local rc=$?
      if [[ $rc == 124 ]] && grep -q "xdg_toplevel" $log && ! grep -q "x11\|DISPLAY" <(grep -i "error" $log); then
        ok "$name on Wayland ($backend): window mapped, still running after 8 s"
      else
        bad "$name on Wayland ($backend), exit $rc"; grep -v "^\[" $log | tail -5
      fi
    done
  }
  run_gui uc33 /work/UC33-Editor uc33-gui uc33-gui "${patch[@]}"
  run_gui korg /src/main/KORG_Kontrol_Editor korg-slint korg-slint
  run_gui db1  /src/main/OceanBeach_DB-1 db1-ui db1-ui
  run_gui tec  /src/main/TECcontrol tec-ui tec-ui
fi
exit $fail
IN
