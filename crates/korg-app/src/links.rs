//! A device's MIDI connections, grouped by link.
//!
//! A Studio device connected over USB and Bluetooth at once shows four ports:
//! `nanoKONTROL Studio CTRL` in and out, `nanoKONTROL Studio Bluetooth` in and
//! out (CoreMIDI, measured 2026-10-21).  A job must use the input and output
//! of the same link; a USB request answered on Bluetooth never arrives.

use crate::document::{port_matches_direction, port_name_matches_device};
use korg_midi::{Direction, PortInfo};
use korg_model::DeviceInfo;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LinkKind {
    Usb,
    Bluetooth,
}

impl LinkKind {
    pub fn label(self) -> &'static str {
        match self {
            LinkKind::Usb => "USB",
            LinkKind::Bluetooth => "Bluetooth",
        }
    }

    /// Bluetooth ports carry the word in their name; every other port of the
    /// device is taken as USB.
    pub fn of_port(name: &str) -> Self {
        if name.to_lowercase().contains("bluetooth") {
            LinkKind::Bluetooth
        } else {
            LinkKind::Usb
        }
    }
}

/// One connection to a device: the input and output port names of one link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLink {
    pub kind: LinkKind,
    pub input: String,
    pub output: String,
}

impl DeviceLink {
    /// `USB` or `Bluetooth`.
    pub fn label(&self) -> &'static str {
        self.kind.label()
    }

    /// The port name to show: the shared name, or `in / out` when they differ.
    pub fn ports_text(&self) -> String {
        if self.input == self.output {
            self.input.clone()
        } else {
            format!("{} / {}", self.input, self.output)
        }
    }
}

/// The device's complete links, USB first.  A link needs both directions.
pub fn device_links(ports: &[PortInfo], device: &DeviceInfo) -> Vec<DeviceLink> {
    let mut links = Vec::new();
    for kind in [LinkKind::Usb, LinkKind::Bluetooth] {
        let own: Vec<PortInfo> = ports
            .iter()
            .filter(|port| port_name_matches_device(device, &port.name))
            .filter(|port| LinkKind::of_port(&port.name) == kind)
            .cloned()
            .collect();
        let input = port_matches_direction(&own, device, Some(Direction::Input));
        let output = port_matches_direction(&own, device, Some(Direction::Output));
        if let (Some(input), Some(output)) = (input, output) {
            links.push(DeviceLink {
                kind,
                input: input.to_owned(),
                output: output.to_owned(),
            });
        }
    }
    links
}

/// The link to use: `preferred` when it is connected, else the first (USB,
/// which is about 20 times faster for a scene set).
pub fn choose_link(links: &[DeviceLink], preferred: Option<LinkKind>) -> Option<&DeviceLink> {
    preferred
        .and_then(|kind| links.iter().find(|link| link.kind == kind))
        .or_else(|| links.first())
}

/// The link after `current`, wrapping; `None` when fewer than two exist.
pub fn next_link(links: &[DeviceLink], current: Option<LinkKind>) -> Option<LinkKind> {
    if links.len() < 2 {
        return None;
    }
    let index = current
        .and_then(|kind| links.iter().position(|link| link.kind == kind))
        .map_or(0, |index| (index + 1) % links.len());
    Some(links[index].kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use korg_devices::Catalog;
    use korg_midi::PortId;

    fn port(name: &str, direction: Direction) -> PortInfo {
        PortInfo {
            id: PortId::new(format!("{name}#0")),
            name: name.to_owned(),
            direction,
        }
    }

    fn both(name: &str) -> [PortInfo; 2] {
        [port(name, Direction::Input), port(name, Direction::Output)]
    }

    fn device(class: &str) -> DeviceInfo {
        Catalog::load()
            .expect("catalog")
            .find_by_class(class)
            .expect("class")
            .clone()
    }

    #[test]
    fn splits_usb_and_bluetooth_and_pairs_each_link() {
        let mut ports = Vec::new();
        ports.extend(both("nanoKONTROL Studio Bluetooth"));
        ports.extend(both("nanoKONTROL Studio CTRL"));
        ports.extend(both("nanoKEY Studio CTRL"));
        let links = device_links(&ports, &device("X14802"));
        assert_eq!(
            links,
            vec![
                DeviceLink {
                    kind: LinkKind::Usb,
                    input: "nanoKONTROL Studio CTRL".into(),
                    output: "nanoKONTROL Studio CTRL".into(),
                },
                DeviceLink {
                    kind: LinkKind::Bluetooth,
                    input: "nanoKONTROL Studio Bluetooth".into(),
                    output: "nanoKONTROL Studio Bluetooth".into(),
                },
            ]
        );
    }

    #[test]
    fn keeps_the_nanokey_usb_input_alias() {
        let ports = vec![
            port("nanoKEY Studio KEYBOARD/CTRL", Direction::Input),
            port("nanoKEY Studio CTRL", Direction::Output),
        ];
        let links = device_links(&ports, &device("X14800"));
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].input, "nanoKEY Studio KEYBOARD/CTRL");
        assert_eq!(links[0].output, "nanoKEY Studio CTRL");
        assert_eq!(
            links[0].ports_text(),
            "nanoKEY Studio KEYBOARD/CTRL / nanoKEY Studio CTRL"
        );
    }

    #[test]
    fn a_link_needs_both_directions() {
        let ports = vec![port("nanoKONTROL Studio Bluetooth", Direction::Input)];
        assert!(device_links(&ports, &device("X14802")).is_empty());
    }

    #[test]
    fn chooses_the_preferred_link_or_falls_back_to_usb() {
        let mut ports = Vec::new();
        ports.extend(both("nanoKONTROL Studio Bluetooth"));
        ports.extend(both("nanoKONTROL Studio CTRL"));
        let links = device_links(&ports, &device("X14802"));
        assert_eq!(choose_link(&links, None).unwrap().kind, LinkKind::Usb);
        assert_eq!(
            choose_link(&links, Some(LinkKind::Bluetooth)).unwrap().kind,
            LinkKind::Bluetooth
        );
        let bluetooth_only = &links[1..];
        assert_eq!(
            choose_link(bluetooth_only, Some(LinkKind::Usb))
                .unwrap()
                .kind,
            LinkKind::Bluetooth
        );
        assert_eq!(
            next_link(&links, Some(LinkKind::Usb)),
            Some(LinkKind::Bluetooth)
        );
        assert_eq!(
            next_link(&links, Some(LinkKind::Bluetooth)),
            Some(LinkKind::Usb)
        );
        assert_eq!(next_link(bluetooth_only, None), None);
    }
}
