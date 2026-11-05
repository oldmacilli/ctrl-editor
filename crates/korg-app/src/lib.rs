//! UI-neutral shell logic shared by the KORG editor front ends.
//!
//! `document` opens devices and files and saves documents; `device` runs
//! measured receive and write jobs; `panel` describes each device's controls
//! for a schematic; `pages` holds the parameter pages both front ends show;
//! `links` pairs a device's ports into USB and Bluetooth
//! connections; `detect` asks ports which device answers; `live` decodes
//! what the device sends for the MIDI monitor.  None does terminal or window I/O.

pub mod assign;
pub mod detect;
pub mod device;
pub mod document;
pub mod links;
pub mod live;
pub mod midi_backend;
pub mod monitor;
pub mod pages;
pub mod panel;
pub mod panel_external;
pub mod paths;
pub mod preferences;
pub mod wireless;

pub use device::*;
pub use document::*;
pub use links::*;
pub use midi_backend::MidiBackendSwitch;
