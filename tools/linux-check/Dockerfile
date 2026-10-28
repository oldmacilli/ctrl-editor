# Linux checks for the editors: ALSA/JACK/PipeWire MIDI and Slint on Wayland.
# Built and run by run.sh; nothing here touches hardware.
# libjack-jackd2-dev: the jack crate's build script asks pkg-config for jack on
# Linux; at run time libjack is loaded dynamically (jackd2 or PipeWire's).
FROM rust:1-bookworm
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      pkg-config libasound2-dev libudev-dev libfontconfig1-dev libfreetype-dev \
      libxkbcommon-dev libwayland-dev libxcursor-dev libxi-dev libxrandr-dev libx11-xcb-dev \
      libegl1 libgl1-mesa-dri libgles2 mesa-utils \
      weston wayland-utils \
      jackd2 libjack-jackd2-dev pipewire pipewire-bin pipewire-jack wireplumber dbus dbus-x11 \
      alsa-utils procps \
    && rm -rf /var/lib/apt/lists/*
ENV XDG_RUNTIME_DIR=/run/user/0 CARGO_TERM_COLOR=never
RUN mkdir -p /run/user/0 && chmod 700 /run/user/0
