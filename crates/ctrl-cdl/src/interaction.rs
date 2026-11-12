//! The device class (CDL §5.2): how software reaches a device's
//! configuration. A descriptor declares it with `interaction`; this module
//! derives it from what the descriptor contains, so the validator can refuse
//! a declaration the descriptor does not support, and front ends can offer
//! only what the class allows.

use crate::*;

impl Interaction {
    pub const ALL: [Interaction; 5] = [
        Interaction::Active,
        Interaction::Passive,
        Interaction::PushOnly,
        Interaction::UsbMixed,
        Interaction::BusMixed,
    ];

    /// The keyword in CDL.
    pub fn keyword(self) -> &'static str {
        match self {
            Interaction::Active => "active",
            Interaction::Passive => "passive",
            Interaction::PushOnly => "push-only",
            Interaction::UsbMixed => "usb-mixed",
            Interaction::BusMixed => "bus-mixed",
        }
    }

    /// What a front end may offer for a device of this class.
    pub fn capabilities(self) -> Capabilities {
        match self {
            Interaction::Active => Capabilities {
                software_reads: true,
                software_writes: true,
                user_starts_transfer: false,
                guided_setup: false,
                config_transport: Some(TransportKind::Midi),
                supported: true,
            },
            Interaction::Passive => Capabilities {
                software_reads: false,
                software_writes: false,
                user_starts_transfer: true,
                guided_setup: true,
                config_transport: Some(TransportKind::Midi),
                supported: true,
            },
            Interaction::PushOnly => Capabilities {
                software_reads: false,
                software_writes: false,
                user_starts_transfer: false,
                guided_setup: true,
                config_transport: None,
                supported: true,
            },
            Interaction::UsbMixed => Capabilities {
                software_reads: true,
                software_writes: true,
                user_starts_transfer: false,
                guided_setup: false,
                config_transport: Some(TransportKind::Hid),
                supported: true,
            },
            Interaction::BusMixed => Capabilities {
                software_reads: false,
                software_writes: false,
                user_starts_transfer: false,
                guided_setup: false,
                config_transport: None,
                supported: false,
            },
        }
    }
}

/// What a device class allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Software sends a request and the device answers.
    pub software_reads: bool,
    /// Software sends configuration without the user preparing the device.
    pub software_writes: bool,
    /// A transfer needs the user on the device (a dump mode, a receive mode).
    pub user_starts_transfer: bool,
    /// The editor guides the user through key presses on the device.
    pub guided_setup: bool,
    /// The kind of the config transport; None: there is none.
    pub config_transport: Option<TransportKind>,
    /// False for a reserved class the engine refuses to run.
    pub supported: bool,
}

/// The class the descriptor's contents support, with the facts behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derivation {
    /// None: the descriptor holds too little to tell.
    pub class: Option<Interaction>,
    pub reason: String,
}

/// A step that touches the device, in run order, with the transport kind and
/// role it goes over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Touch {
    Guide,
    Send,
    Exchange,
    Await,
}

/// The procedure's steps in run order, `repeat` and `call` expanded once.
pub(crate) fn touches(d: &Descriptor, p: &Procedure) -> Vec<Touch> {
    let mut out = Vec::new();
    expand(d, &p.steps, 0, &mut out);
    out
}

fn expand(d: &Descriptor, steps: &[Step], depth: u32, out: &mut Vec<Touch>) {
    // A call cycle is reported elsewhere; stop expanding it here.
    if depth > 8 {
        return;
    }
    for s in steps {
        match s {
            Step::Guide(_) => out.push(Touch::Guide),
            Step::Send(_) => out.push(Touch::Send),
            Step::Exchange(_) => out.push(Touch::Exchange),
            Step::Await(_) => out.push(Touch::Await),
            Step::Repeat { steps, .. } => expand(d, steps, depth + 1, out),
            Step::Call { procedure, .. } => {
                if let Some(p) = d.procedure(procedure) {
                    expand(d, &p.steps, depth + 1, out);
                }
            }
            _ => {}
        }
    }
}

/// The transports with `role="config"`.
pub(crate) fn config_transports(d: &Descriptor) -> Vec<&Transport> {
    d.transports
        .iter()
        .filter(|t| t.role == Some(Role::Config))
        .collect()
}

pub(crate) fn writes(d: &Descriptor) -> bool {
    let w = |k: Option<Access>| matches!(k, Some(Access::Write | Access::Commit));
    d.messages.iter().any(|m| w(m.kind)) || d.procedures.iter().any(|p| w(p.kind))
}

/// Derives the class from the transports, messages and procedures.
pub fn derive(d: &Descriptor) -> Derivation {
    let config = config_transports(d);
    let midi = d.transports.iter().any(|t| t.kind == TransportKind::Midi);
    let usb_config: Vec<_> = config
        .iter()
        .filter(|t| t.kind != TransportKind::Midi)
        .collect();
    let found = |class, reason: String| Derivation {
        class: Some(class),
        reason,
    };
    if let Some(t) = usb_config.first() {
        if midi {
            return found(
                Interaction::UsbMixed,
                format!(
                    "config transport `{}` is not MIDI and a MIDI transport exists",
                    t.name
                ),
            );
        }
        return Derivation {
            class: None,
            reason: format!(
                "config transport `{}` is not MIDI and no MIDI transport exists",
                t.name
            ),
        };
    }
    if config.is_empty() {
        if writes(d) {
            return Derivation {
                class: None,
                reason: "it writes but has no `role=\"config\"` transport".into(),
            };
        }
        if d.hands.is_none() {
            // Nothing described is not the same as nothing to configure.
            return Derivation {
                class: None,
                reason: "no config transport and no `hands`: configuration is not described yet"
                    .into(),
            };
        }
        return found(
            Interaction::PushOnly,
            "no `role=\"config\"` transport, nothing that writes, and `hands` to guide the user"
                .into(),
        );
    }
    let all: Vec<Touch> = d.procedures.iter().flat_map(|p| touches(d, p)).collect();
    if all.contains(&Touch::Exchange) {
        return found(
            Interaction::Active,
            "a procedure sends a request and waits for the answer (`exchange`)".into(),
        );
    }
    if all.contains(&Touch::Await) || (all.contains(&Touch::Guide) && all.contains(&Touch::Send)) {
        return found(
            Interaction::Passive,
            "procedures wait for dumps the user starts (`await`) and never `exchange`".into(),
        );
    }
    let config_names: Vec<&str> = config.iter().map(|t| t.name.as_str()).collect();
    let requests = d.messages.iter().any(|m| {
        !m.replies.is_empty()
            && m.send.is_some()
            && m.transport
                .as_deref()
                .is_some_and(|t| t == crate::SELECTED_TRANSPORT || config_names.contains(&t))
    });
    if requests {
        return found(
            Interaction::Active,
            "a message on the config transport has a request and a reply".into(),
        );
    }
    Derivation {
        class: None,
        reason: "a config transport, but no request, reply or dump is described yet".into(),
    }
}
