//! Wireless on/off, shown as information only.
//!
//! No MIDI message sets or reports the wireless function: the device toggles
//! it with a button combination (owner's manuals) and the firmware path is not
//! decoded.  What the editor can see is whether a Bluetooth link to this
//! computer exists.

use crate::links::{DeviceLink, LinkKind};

/// What the editor knows about the device's wireless function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WirelessState {
    /// A Bluetooth link to this computer exists, so wireless is on.
    On,
    /// No Bluetooth link here: wireless is off, or on and connected elsewhere.
    Unknown,
}

impl WirelessState {
    pub fn of_links(links: &[DeviceLink]) -> Self {
        if links.iter().any(|link| link.kind == LinkKind::Bluetooth) {
            WirelessState::On
        } else {
            WirelessState::Unknown
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WirelessState::On => "On (Bluetooth link to this computer)",
            WirelessState::Unknown => "Unknown (no Bluetooth link to this computer)",
        }
    }
}

/// How to switch wireless on the device itself, for the classes whose owner's
/// manual describes it.
pub fn toggle_instruction(class: &str) -> Option<&'static str> {
    match class {
        // nanoKONTROL Studio owner's manual, "Wireless-Funktion aktivieren und
        // deaktivieren".
        "X14802" => Some("Hold Scene and press Cycle"),
        // nanoKEY Studio owner's manual, "Turning the Wireless Feature On and Off".
        "X14800" => Some("Hold Shift/Tap and press Trigger Pad 4 (Wireless)"),
        _ => None,
    }
}

/// The full note: the instruction plus the battery restriction both manuals
/// state, and why the editor cannot do it.
pub fn toggle_note(class: &str) -> Option<String> {
    toggle_instruction(class).map(|how| {
        format!(
            "{how} on the device to switch wireless on or off. Only on USB power; \
             on batteries wireless stays on. The editor cannot switch it: no MIDI \
             message for it is known yet."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(kind: LinkKind) -> DeviceLink {
        DeviceLink {
            kind,
            input: "in".into(),
            output: "out".into(),
        }
    }

    #[test]
    fn a_bluetooth_link_means_on() {
        assert_eq!(
            WirelessState::of_links(&[link(LinkKind::Usb), link(LinkKind::Bluetooth)]),
            WirelessState::On
        );
        assert_eq!(
            WirelessState::of_links(&[link(LinkKind::Usb)]),
            WirelessState::Unknown
        );
    }

    #[test]
    fn both_studio_devices_have_an_instruction() {
        assert_eq!(
            toggle_instruction("X14802"),
            Some("Hold Scene and press Cycle")
        );
        assert!(toggle_note("X14800").unwrap().contains("Trigger Pad 4"));
        assert_eq!(toggle_instruction("X10820"), None);
    }
}
