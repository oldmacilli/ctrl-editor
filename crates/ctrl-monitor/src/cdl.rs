//! Names messages by their CDL descriptor (docs/MONITOR.md §4.2, step 1):
//! "nanoKONTROL Studio: scene-write" instead of hex.
//!
//! A port bound to a device is matched against that device's descriptor,
//! every message type. On an unbound port only SysEx is tried, against
//! every descriptor: a CC matches half the descriptors, a SysEx with a
//! manufacturer and model id matches one, and only when its template fixes
//! more than F0 and F7. A port binds itself the first time it is seen when
//! its name contains a descriptor's `port-match`. A reply that several messages
//! share is given to the request sent last on the port; the others are
//! listed as `also`.

use std::collections::HashMap;

use ctrl_cdl::{Descriptor, PortMatch};
use ctrl_codec::{Bindings, Codec, Direction, Match, Val};

use crate::collector::Describer;
use crate::model::{Annotation, Dir, Event, Msg, PortIdx};
use crate::store::Store;

struct Device {
    label: String,
    descriptor: Descriptor,
}

/// Index of a device in a [`CdlDescriber`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceIdx(pub usize);

#[derive(Default)]
pub struct CdlDescriber {
    devices: Vec<Device>,
    bound: HashMap<PortIdx, DeviceIdx>,
    /// The last request described on each port: a reply that several
    /// messages share (an acknowledgement) belongs to it.
    last_request: HashMap<PortIdx, String>,
    /// Ports bound by their name the first time they were seen (the
    /// descriptors' `port-match`), or found to match none.
    auto: HashMap<PortIdx, Option<DeviceIdx>>,
}

/// Literal bytes a SysEx on an unbound port must match: F0, F7 and at least
/// one more, normally the manufacturer. A placeholder such as
/// `F0 {data*} F7` names nothing.
const UNBOUND_LITERALS: usize = 3;

/// The best match for `bytes`, its device, and the other messages that
/// matched as well.
struct Best<'a> {
    device: &'a Device,
    found: Match,
    also: Vec<String>,
}

impl CdlDescriber {
    pub fn new() -> Self {
        CdlDescriber::default()
    }

    /// Every descriptor in this repository (`ctrl-descriptors`).
    #[cfg(feature = "descriptors")]
    pub fn embedded() -> Self {
        let mut c = CdlDescriber::new();
        for path in ctrl_descriptors::devices() {
            if let Ok(d) = ctrl_descriptors::load(path) {
                c.add(d);
            }
        }
        c
    }

    /// Adds a descriptor; its label is the device's vendor and model.
    pub fn add(&mut self, descriptor: Descriptor) -> DeviceIdx {
        let dev = &descriptor.device;
        let label = match (&dev.vendor, &dev.model) {
            (Some(v), Some(m)) => format!("{v} {m}"),
            (None, Some(m)) => m.clone(),
            _ => dev.id.clone(),
        };
        self.devices.push(Device { label, descriptor });
        DeviceIdx(self.devices.len() - 1)
    }

    /// The device on `port`: every message on it is matched against its
    /// descriptor.
    pub fn bind(&mut self, port: PortIdx, device: DeviceIdx) {
        self.bound.insert(port, device);
    }

    /// The device whose `port-match` (either direction) is part of `name`.
    pub fn device_for_port(&self, name: &str) -> Option<DeviceIdx> {
        self.devices
            .iter()
            .position(|d| {
                d.descriptor
                    .transports
                    .iter()
                    .filter_map(|t| t.ports.as_ref())
                    .any(|m| match m {
                        PortMatch::Both(p) => name.contains(p.as_str()),
                        PortMatch::Split { input, output } => {
                            name.contains(input.as_str()) || name.contains(output.as_str())
                        }
                    })
            })
            .map(DeviceIdx)
    }

    fn bound_device(&mut self, store: &Store, port: PortIdx) -> Option<DeviceIdx> {
        if let Some(&d) = self.bound.get(&port) {
            return Some(d);
        }
        if let Some(&d) = self.auto.get(&port) {
            return d;
        }
        let found = store.port_ref(port).and_then(|p| {
            self.device_for_port(&p.name)
                .or_else(|| self.device_for_port(&p.id))
        });
        self.auto.insert(port, found);
        found
    }

    pub fn label(&self, device: DeviceIdx) -> Option<&str> {
        self.devices.get(device.0).map(|d| d.label.as_str())
    }

    /// Ranks matches: the expected direction first, then the reply to the
    /// port's last request, then the most literal bytes; among equals,
    /// descriptor order.
    fn best<'a>(
        bytes: &[u8],
        hint: Option<Direction>,
        candidates: &[&'a Device],
        request: Option<&str>,
    ) -> Option<Best<'a>> {
        let mut top: Vec<(&'a Device, Match)> = Vec::new();
        let mut score = (false, false, 0);
        for &device in candidates {
            let codec = Codec::new(&device.descriptor);
            for found in codec.identify(bytes, None, &Bindings::new()) {
                let s = (
                    hint.is_none_or(|h| h == found.direction),
                    request.is_some_and(|r| r == found.message),
                    found.literals,
                );
                if top.is_empty() || s > score {
                    score = s;
                    top.clear();
                }
                if s == score {
                    top.push((device, found));
                }
            }
        }
        if top.is_empty() {
            return None;
        }
        let (device, found) = top.remove(0);
        let mut also: Vec<String> = top
            .into_iter()
            .map(|(_, m)| m.message)
            .filter(|m| *m != found.message)
            .collect();
        also.dedup();
        Some(Best {
            device,
            found,
            also,
        })
    }
}

/// A field's value as the details show it.
fn show(v: &Val) -> String {
    match v {
        Val::Int(i) => i.to_string(),
        Val::Bytes(b) if b.len() <= 8 => crate::describe::hex(b, 8),
        Val::Bytes(b) => format!("{} bytes", b.len()),
    }
}

impl Describer for CdlDescriber {
    fn describe(&mut self, store: &Store, event: &Event) -> Vec<Annotation> {
        let device = self.bound_device(store, event.port);
        let bound = device.and_then(|d| self.devices.get(d.0));
        let candidates: Vec<&Device> = match (bound, event.msg) {
            (Some(d), _) => vec![d],
            // Universal SysEx (7E, 7F) belongs to no one vendor: every device
            // answers an identity request. The generic names cover it.
            (None, Msg::SysEx { .. })
                if store
                    .wire(&event.msg)
                    .is_some_and(|b| matches!(b.get(1), Some(0x7E | 0x7F))) =>
            {
                return Vec::new()
            }
            (None, Msg::SysEx { .. }) => self.devices.iter().collect(),
            _ => return Vec::new(),
        };
        let Some(bytes) = store.wire(&event.msg) else {
            return Vec::new();
        };
        let hint = match event.dir {
            Dir::In => Some(Direction::In),
            Dir::Out => Some(Direction::Out),
            Dir::Thru => None,
        };
        let request = match event.dir {
            Dir::In => self.last_request.get(&event.port).map(String::as_str),
            _ => None,
        };
        let Some(Best {
            device,
            found,
            also,
        }) = Self::best(&bytes, hint, &candidates, request)
        else {
            return Vec::new();
        };
        if bound.is_none() && found.literals < UNBOUND_LITERALS {
            return Vec::new();
        }
        if found.direction == Direction::Out {
            self.last_request.insert(event.port, found.message.clone());
        }
        let mut name = format!("{}: {}", device.label, found.message);
        if let Some(v) = &found.variant {
            name.push_str(&format!(" ({v})"));
        }
        let mut fields: Vec<(String, String)> = found
            .values
            .iter()
            .map(|(k, v)| (k.clone(), show(v)))
            .collect();
        if found.ok == Some(false) {
            fields.push(("ok".into(), "no".into()));
        }
        if !also.is_empty() {
            fields.push(("also".into(), also.join(", ")));
        }
        vec![Annotation::Described { name, fields }]
    }
}
