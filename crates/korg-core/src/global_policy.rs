//! Fail-closed policy checks for global block writes.

use korg_devices::DescriptorStatus;
use korg_model::FileModelKey;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GlobalWritePolicyError {
    #[error("no verified global descriptor for {0}")]
    MissingDescriptor(String),
    #[error("global baseline size does not match the proposed data")]
    SizeMismatch,
    #[error("global field {name} at byte {offset} has an unknown value {value}")]
    Unknown {
        name: String,
        offset: usize,
        value: u8,
    },
    #[error("global field {name} at byte {offset} is decoded but not measured")]
    Unmeasured { name: String, offset: usize },
    #[error("global field {name} at byte {offset} uses inferred value {value}; explicit consent is required")]
    Inferred {
        name: String,
        offset: usize,
        value: u8,
    },
}

#[derive(Clone, Copy)]
enum Trust {
    Measured,
    Decoded,
    Inferred,
    Unknown,
}

struct Binding<'a> {
    name: &'a str,
    offset: usize,
    status: Trust,
    choices: Vec<(u8, Trust)>,
}

fn check_trust(
    status: Trust,
    name: &str,
    offset: usize,
    value: u8,
    consent: bool,
) -> Result<(), GlobalWritePolicyError> {
    match status {
        Trust::Measured => Ok(()),
        Trust::Inferred if consent => Ok(()),
        Trust::Inferred => Err(GlobalWritePolicyError::Inferred {
            name: name.to_owned(),
            offset,
            value,
        }),
        Trust::Decoded => Err(GlobalWritePolicyError::Unmeasured {
            name: name.to_owned(),
            offset,
        }),
        Trust::Unknown => Err(GlobalWritePolicyError::Unknown {
            name: name.to_owned(),
            offset,
            value,
        }),
    }
}

fn check_choices(
    bindings: &[Binding<'_>],
    before: &[u8],
    after: &[u8],
    consent: bool,
) -> Result<(), GlobalWritePolicyError> {
    if before.len() != after.len() {
        return Err(GlobalWritePolicyError::SizeMismatch);
    }
    for binding in bindings {
        let Some((&old, &new)) = before.get(binding.offset).zip(after.get(binding.offset)) else {
            return Err(GlobalWritePolicyError::SizeMismatch);
        };
        if old == new {
            continue;
        }
        check_trust(binding.status, binding.name, binding.offset, new, consent)?;
        if !binding.choices.is_empty() {
            let status = binding
                .choices
                .iter()
                .find(|(value, _)| *value == new)
                .map_or(Trust::Unknown, |(_, status)| *status);
            check_trust(status, binding.name, binding.offset, new, consent)?;
        }
    }
    Ok(())
}

fn trust(status: Option<DescriptorStatus>) -> Trust {
    match status {
        Some(DescriptorStatus::Measured) => Trust::Measured,
        Some(DescriptorStatus::Decoded) => Trust::Decoded,
        Some(DescriptorStatus::Inferred) => Trust::Inferred,
        _ => Trust::Unknown,
    }
}

/// Validate changes against a fresh device baseline. Unmapped changes and
/// unknown values are refused; decoded values are read-only until measured.
pub fn check_global_choices(
    key: &FileModelKey,
    before: &[u8],
    after: &[u8],
    consent: bool,
) -> Result<(), GlobalWritePolicyError> {
    let fields = korg_devices::global_fields_for_file_key(key)
        .ok_or_else(|| GlobalWritePolicyError::MissingDescriptor(key.to_string()))?;
    if before.len() != after.len() {
        return Err(GlobalWritePolicyError::SizeMismatch);
    }
    if let Some(offset) = before.iter().zip(after).position(|(a, b)| a != b) {
        check_trust(
            trust(fields.meta.status),
            "global-fields",
            offset,
            after[offset],
            consent,
        )?;
    }
    let mut covered = vec![false; before.len()];
    for binding in &fields.bindings {
        if let Some(item) = covered.get_mut(binding.offset as usize) {
            *item = true;
        }
    }
    if let Some((start, width)) = fields.name_offset.zip(fields.name_width) {
        for offset in start..start.saturating_add(width) {
            if let Some(item) = covered.get_mut(offset as usize) {
                *item = true;
            }
        }
    }
    if let Some(start) = fields.bpm_offset {
        for offset in start..start.saturating_add(2) {
            if let Some(item) = covered.get_mut(offset as usize) {
                *item = true;
            }
        }
    }
    if let Some(offset) = before
        .iter()
        .zip(after)
        .enumerate()
        .find_map(|(i, (a, b))| (a != b && !covered[i]).then_some(i))
    {
        return Err(GlobalWritePolicyError::Unknown {
            name: "unmapped".to_owned(),
            offset,
            value: after[offset],
        });
    }
    let bindings: Vec<_> = fields
        .bindings
        .iter()
        .map(|binding| Binding {
            name: &binding.name,
            offset: binding.offset as usize,
            status: trust(binding.meta.status),
            choices: binding
                .choices
                .iter()
                .map(|choice| (choice.value, trust(choice.meta.status)))
                .collect(),
        })
        .collect();
    check_choices(&bindings, before, after, consent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_inferred_choices_need_explicit_session_consent() {
        let bindings = [Binding {
            name: "mode",
            offset: 0,
            status: Trust::Measured,
            choices: vec![(1, Trust::Inferred)],
        }];
        assert!(matches!(
            check_choices(&bindings, &[0], &[1], false),
            Err(GlobalWritePolicyError::Inferred { .. })
        ));
        assert!(check_choices(&bindings, &[0], &[1], true).is_ok());
        assert!(check_choices(&bindings, &[1], &[1], false).is_ok());
    }

    #[test]
    fn unknown_and_decoded_choices_are_never_authorized() {
        let bindings = [Binding {
            name: "mode",
            offset: 0,
            status: Trust::Measured,
            choices: vec![(2, Trust::Unknown), (3, Trust::Decoded)],
        }];
        assert!(matches!(
            check_choices(&bindings, &[0], &[2], true),
            Err(GlobalWritePolicyError::Unknown { .. })
        ));
        assert!(matches!(
            check_choices(&bindings, &[0], &[3], true),
            Err(GlobalWritePolicyError::Unmeasured { .. })
        ));
        assert!(matches!(
            check_choices(&bindings, &[0], &[4], true),
            Err(GlobalWritePolicyError::Unknown { .. })
        ));
    }
}
