//! The MIDI backend switch, shared by the GUI and the TUI.
//!
//! The platform adapter (`korg-midi-midir`) owns the backends; this module
//! only holds what the front ends show and the advice when a backend cannot
//! run, so both front ends say the same.  Switching is a closure the front
//! end passes in, which keeps this crate free of platform code.

use korg_midi::BackendInfo;

/// What the front ends say when there is nothing to switch to.
pub const ONE_BACKEND: &str = "This build has one MIDI backend.";

pub struct MidiBackendSwitch {
    current: BackendInfo,
    choices: usize,
    next: Box<dyn FnMut() -> BackendInfo>,
}

impl MidiBackendSwitch {
    /// `current` is the backend in use, `choices` how many this build has,
    /// `next` switches to the next one and describes it.
    pub fn new(
        current: BackendInfo,
        choices: usize,
        next: impl FnMut() -> BackendInfo + 'static,
    ) -> Self {
        Self {
            current,
            choices,
            next: Box::new(next),
        }
    }

    /// A switch with one backend: tests and front ends without a platform
    /// transport.
    pub fn fixed(current: BackendInfo) -> Self {
        let again = current.clone();
        Self::new(current, 1, move || again.clone())
    }

    pub fn current(&self) -> &BackendInfo {
        &self.current
    }

    /// True when the build has a second backend to switch to.
    pub fn can_switch(&self) -> bool {
        self.choices > 1
    }

    /// The button text: "MIDI: ALSA sequencer".
    pub fn button_label(&self) -> String {
        format!("MIDI: {}", self.current.label)
    }

    /// Switch to the next backend.  The caller saves `current().id` and
    /// reopens the device; open connections close on their own.
    pub fn switch(&mut self) -> Result<&BackendInfo, &'static str> {
        if !self.can_switch() {
            return Err(ONE_BACKEND);
        }
        self.current = (self.next)();
        Ok(&self.current)
    }

    /// One status line about the backend in use.
    pub fn status(&self) -> String {
        status(&self.current)
    }
}

/// "MIDI backend: JACK." plus why it cannot run and what to do.
pub fn status(info: &BackendInfo) -> String {
    match &info.problem {
        Some(problem) => format!("MIDI backend: {}. {problem}.{}", info.label, advice(info)),
        None => format!("MIDI backend: {}.{}", info.label, advice(info)),
    }
}

/// What to do on this backend, or an empty string.  JACK sees a USB or
/// Bluetooth device only through a bridge, and without a server nothing.
pub fn advice(info: &BackendInfo) -> String {
    if info.id != "jack" {
        return String::new();
    }
    if info.problem.is_some() {
        " Start jackd, run under PipeWire (pw-jack on Debian/Ubuntu), or switch back to the ALSA sequencer.".into()
    } else {
        " The device appears through a bridge: PipeWire's Midi-Bridge, or a2jmidid with jackd. Write long dumps on the ALSA sequencer.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    fn info(id: &str, label: &str, problem: Option<&str>) -> BackendInfo {
        BackendInfo {
            id: id.into(),
            label: label.into(),
            problem: problem.map(Into::into),
        }
    }

    #[test]
    fn a_single_backend_does_not_switch() {
        let mut switch = MidiBackendSwitch::fixed(info("coremidi", "CoreMIDI", None));
        assert_eq!(switch.switch().unwrap_err(), ONE_BACKEND);
        assert_eq!(switch.button_label(), "MIDI: CoreMIDI");
        assert_eq!(switch.status(), "MIDI backend: CoreMIDI.");
    }

    #[test]
    fn switching_cycles_and_names_the_new_backend() {
        let calls = Rc::new(Cell::new(0));
        let seen = calls.clone();
        let mut switch =
            MidiBackendSwitch::new(info("alsa", "ALSA sequencer", None), 2, move || {
                seen.set(seen.get() + 1);
                if seen.get() % 2 == 1 {
                    info("jack", "JACK", None)
                } else {
                    info("alsa", "ALSA sequencer", None)
                }
            });
        assert_eq!(switch.switch().unwrap().id, "jack");
        assert_eq!(switch.button_label(), "MIDI: JACK");
        assert!(switch.status().contains("Midi-Bridge"));
        assert_eq!(switch.switch().unwrap().id, "alsa");
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn jack_without_a_server_says_why_and_how_to_get_back() {
        let jack = info(
            "jack",
            "JACK",
            Some("JACK is not available: no JACK server answers"),
        );
        let text = status(&jack);
        assert!(text.contains("no JACK server answers"), "{text}");
        assert!(text.contains("pw-jack"), "{text}");
        assert!(text.contains("switch back to the ALSA sequencer"), "{text}");
    }
}
