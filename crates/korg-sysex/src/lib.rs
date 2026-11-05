//! KORG SysEx framing and 7-in-8 packing.
//!
//! The framing follows the measured KORG protocols, including Keystage.

use thiserror::Error;

pub const SYSEX_START: u8 = 0xF0;
pub const SYSEX_END: u8 = 0xF7;
pub const KORG_ID: u8 = 0x42;
pub const UNIVERSAL_NON_REALTIME: u8 = 0x7E;
pub const DATA_MARKER: u8 = 0x51;
pub const ACK_COMMAND: u8 = 0x5F;
pub const DATA_COMMAND: u8 = 0x7F;

/// Identity request used as the measured mode check.
pub const IDENTITY_REQUEST: [u8; 6] = [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7];

/// Parsed identity reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityReply {
    pub device_id: u8,
    pub model_id: u32,
    pub family: [u8; 2],
    pub version: [u8; 2],
    pub build: [u8; 2],
}

impl IdentityReply {
    /// Firmware version as the device reports it: bytes `v0 00 v2 00` of the
    /// software revision read as `v2.v0`; `1.05` for the nanoKONTROL Studio
    /// whose v1.05 image was analysed, `1.08` for the nanoKEY Studio.
    pub fn firmware(&self) -> String {
        format!("{}.{:02}", self.build[0], self.version[0])
    }
}

/// KORG's own device search, answered with the identity payload under the
/// KORG header: `F0 42 50 01 <device id> <echo> <family> <member> <version> F7`.
/// Unlike the universal identity request, other manufacturers ignore it.
/// Both Studios answer it on USB (measured 2026-10-28).
pub fn search_request(echo: u8) -> [u8; 6] {
    [SYSEX_START, KORG_ID, 0x50, 0x00, echo & 0x7F, SYSEX_END]
}

/// Parse a search reply; returns the echo byte and the identity.
pub fn parse_search_reply(bytes: &[u8]) -> Result<(u8, IdentityReply), SysexError> {
    match bytes {
        [SYSEX_START, KORG_ID, 0x50, 0x01, device_id, echo, f0, f1, _, _, v0, v1, b0, b1, SYSEX_END] => {
            Ok((
                *echo,
                IdentityReply {
                    device_id: *device_id,
                    model_id: u32::from(*f0) | (u32::from(*f1) << 8),
                    family: [*f0, *f1],
                    version: [*v0, *v1],
                    build: [*b0, *b1],
                },
            ))
        }
        _ => Err(SysexError::BadIdentity),
    }
}

/// One measured receive request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// `0x12`: the official editor's mode check.
    /// The reply `0x42 <mode>` carries the mode, 0 = normal; not a scene.
    SceneSetInfo,
    SelectScene(u8),
    ReadBlock,
    GlobalData,
}

impl Request {
    fn body(self) -> Vec<u8> {
        match self {
            Request::SceneSetInfo => vec![0x01, 0x00, 0x00, 0x12],
            Request::SelectScene(index) => vec![0x02, 0x00, 0x00, 0x14, index],
            Request::ReadBlock => vec![0x01, 0x00, 0x00, 0x10],
            Request::GlobalData => vec![0x01, 0x00, 0x00, 0x0E],
        }
    }
}

/// Parse the universal non-realtime identity reply.
pub fn parse_identity(bytes: &[u8]) -> Result<IdentityReply, SysexError> {
    if bytes.len() < 15 || bytes[0] != SYSEX_START || bytes[bytes.len() - 1] != SYSEX_END {
        return Err(SysexError::Framing);
    }
    if bytes[1] != UNIVERSAL_NON_REALTIME || bytes[5] != KORG_ID {
        return Err(SysexError::BadIdentity);
    }
    if bytes[3] != 0x06 || bytes[4] != 0x02 {
        return Err(SysexError::BadIdentity);
    }
    let family = [bytes[6], bytes[7]];
    Ok(IdentityReply {
        device_id: bytes[2],
        model_id: u32::from(family[0]) | (u32::from(family[1]) << 8),
        family,
        version: [bytes[10], bytes[11]],
        build: [bytes[12], bytes[13]],
    })
}

/// Build one measured request for a device.
pub fn build_request(
    device_id: u8,
    model_id: u32,
    request: Request,
) -> Result<Vec<u8>, SysexError> {
    if device_id > 0x0F {
        return Err(SysexError::InvalidDeviceId(device_id));
    }
    let mut out = vec![SYSEX_START, KORG_ID, 0x40 | device_id];
    push_model(&mut out, model_id)?;
    out.extend_from_slice(&request.body());
    out.push(SYSEX_END);
    Ok(out)
}

/// Parse one measured data reply and unpack its payload.
///
/// The frame is `F0 42 <dev> <model> <4-byte header> <packed payload> F7`.
pub fn parse_data_reply(bytes: &[u8], expected_len: usize) -> Result<Vec<u8>, SysexError> {
    if bytes.len() < 8 || bytes[0] != SYSEX_START || bytes[bytes.len() - 1] != SYSEX_END {
        return Err(SysexError::Framing);
    }
    if bytes[1] != KORG_ID || bytes[2] & 0xF0 != 0x40 {
        return Err(SysexError::Manufacturer);
    }
    let Some((_model, next)) = read_model(bytes, 3) else {
        return Err(SysexError::Truncated);
    };
    let payload_start = next + 4;
    if payload_start >= bytes.len() {
        return Err(SysexError::Truncated);
    }
    // Bytes 6..8 (here `next..next+3`) are the little-endian 7-bit payload
    // length, including the one marker byte.  The original rejects a frame
    // whose declared length does not match the bytes it received
    // and reports a broken message.
    let declared = usize::from(bytes[next])
        | (usize::from(bytes[next + 1]) << 7)
        | (usize::from(bytes[next + 2]) << 14);
    let packed_len = bytes.len() - 1 - payload_start;
    if declared != packed_len + 1 {
        return Err(SysexError::LengthMismatch {
            declared,
            actual: packed_len + 1,
        });
    }
    unpack_7in8(&bytes[payload_start..bytes.len() - 1], expected_len)
}

/// Four-byte data header for one `X14802` scene write.
pub const X14802_SCENE_WRITE_HEADER: [u8; 4] = [0x01, 0x05, 0x00, 0x40];

/// Four-byte data header for a nanoKEY Studio touch-pad calibration step.
pub const X14800_CALIBRATION_HEADER: [u8; 4] = [0x03, 0x00, 0x00, 0x1D];

/// Four-byte data header for one `X14802` global write.
pub const X14802_GLOBAL_WRITE_HEADER: [u8; 4] = [0x14, 0x01, 0x00, 0x51];

/// Build one touch-pad calibration frame (`0x1D`, raw payload, no packing).
pub fn build_calibration_frame(
    device_id: u8,
    model_id: u32,
    command: u8,
) -> Result<Vec<u8>, SysexError> {
    if device_id > 0x0F {
        return Err(SysexError::InvalidDeviceId(device_id));
    }
    if command >= 0x80 {
        return Err(SysexError::InvalidByte(command));
    }
    let mut out = vec![SYSEX_START, KORG_ID, 0x40 | device_id];
    push_model(&mut out, model_id)?;
    out.extend_from_slice(&X14800_CALIBRATION_HEADER);
    out.extend_from_slice(&[0x01, command]);
    out.push(SYSEX_END);
    Ok(out)
}

/// Build the host-to-device scene data frame.
pub fn build_write_frame(
    device_id: u8,
    model_id: u32,
    header: [u8; 4],
    payload: &[u8],
) -> Result<Vec<u8>, SysexError> {
    if device_id > 0x0F {
        return Err(SysexError::InvalidDeviceId(device_id));
    }
    if header.iter().any(|byte| *byte >= 0x80) {
        return Err(SysexError::InvalidByte(
            *header.iter().find(|byte| **byte >= 0x80).unwrap(),
        ));
    }
    let mut out = vec![SYSEX_START, KORG_ID, 0x40 | device_id];
    push_model(&mut out, model_id)?;
    out.extend_from_slice(&header);
    out.extend_from_slice(&pack_7in8(payload));
    out.push(SYSEX_END);
    Ok(out)
}

/// Build the commit message for one scene.
pub fn build_commit(device_id: u8, model_id: u32, scene: u8) -> Result<Vec<u8>, SysexError> {
    if device_id > 0x0F {
        return Err(SysexError::InvalidDeviceId(device_id));
    }
    if scene >= 0x80 {
        return Err(SysexError::InvalidByte(scene));
    }
    let mut out = vec![SYSEX_START, KORG_ID, 0x40 | device_id];
    push_model(&mut out, model_id)?;
    out.extend_from_slice(&[0x02, 0x00, 0x00, 0x11, scene]);
    out.push(SYSEX_END);
    Ok(out)
}

/// Status byte from an acknowledgement frame.
pub fn ack_status(bytes: &[u8]) -> Result<AckStatus, SysexError> {
    if bytes.len() < 3
        || bytes[0] != SYSEX_START
        || bytes[bytes.len() - 1] != SYSEX_END
        || bytes[1] != KORG_ID
    {
        return Err(SysexError::Framing);
    }
    let status = bytes[bytes.len() - 2];
    AckStatus::from_byte(status).ok_or(SysexError::UnknownStatus(status))
}

/// Model id from a measured data reply.
pub fn reply_model(bytes: &[u8]) -> Result<u32, SysexError> {
    if bytes.len() < 6 || bytes[0] != SYSEX_START || bytes[1] != KORG_ID {
        return Err(SysexError::Framing);
    }
    read_model(bytes, 3)
        .map(|(model, _)| model)
        .ok_or(SysexError::Truncated)
}

/// Length encoding used by a generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthStyle {
    /// `7F <size>` for short payloads, `7F 7F 02 <hi> <lo>` for long ones.
    ShortOrLong,
    /// Three 7-bit bytes: `<lo> <mid> <hi>`.  Keystage uses this form.
    Fixed3,
}

/// Descriptor for one outgoing data message.
///
/// `command` and `marker` come from the device descriptor.  Do not guess them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataMessage {
    pub model_id: u32,
    pub channel: u8,
    pub command: u8,
    pub marker: u8,
    pub length_style: LengthStyle,
}

impl DataMessage {
    pub const fn new(
        model_id: u32,
        channel: u8,
        command: u8,
        marker: u8,
        length_style: LengthStyle,
    ) -> Self {
        Self {
            model_id,
            channel,
            command,
            marker,
            length_style,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckStatus {
    WriteComplete,
    WriteError,
    DataReceived,
    Nak,
}

impl AckStatus {
    pub const fn byte(self) -> u8 {
        match self {
            AckStatus::WriteComplete => 0x21,
            AckStatus::WriteError => 0x22,
            AckStatus::DataReceived => 0x23,
            AckStatus::Nak => 0x24,
        }
    }

    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x21 => Some(AckStatus::WriteComplete),
            0x22 => Some(AckStatus::WriteError),
            0x23 => Some(AckStatus::DataReceived),
            0x24 => Some(AckStatus::Nak),
            _ => None,
        }
    }
}

/// Measured transfer phase and the status that completes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    ModeCheck,
    Data,
    Write,
}

impl Phase {
    pub const fn expected(self) -> AckStatus {
        match self {
            Phase::ModeCheck | Phase::Data => AckStatus::DataReceived,
            Phase::Write => AckStatus::WriteComplete,
        }
    }

    pub const fn failure(self) -> PhaseFailure {
        match self {
            Phase::ModeCheck => PhaseFailure::UnsupportedDevice,
            Phase::Data => PhaseFailure::Nak,
            Phase::Write => PhaseFailure::WriteError,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseFailure {
    UnsupportedDevice,
    Nak,
    WriteError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Ack {
        model_id: u32,
        channel: u8,
        status: AckStatus,
    },
    /// One data reply.  `command` is the byte at index 6: the member byte on
    /// a control frame, the descriptor command on a data frame.  `marker` is
    /// the length-form marker byte.  On a fixed3 frame the marker is the
    /// function byte, 0x40 or 0x51.
    Data {
        model_id: u32,
        channel: u8,
        command: u8,
        marker: u8,
        packed: Vec<u8>,
    },
    Universal {
        device_id: u32,
        version: u32,
    },
    /// A reply that is neither an ACK nor data.  `code` is the function byte.
    /// For fixed3 frames `extra` is `Some` only for the status functions 0x42
    /// and 0x4F, the two where the vendor reads a byte after the function;
    /// code 0x26 has no such byte, and its meaning is not measured.  Other
    /// length styles report the byte after any unknown command, if present.
    Status {
        model_id: u32,
        channel: u8,
        code: u8,
        extra: Option<u8>,
    },
}

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum SysexError {
    #[error("message does not start with F0 or does not end with F7")]
    Framing,
    #[error("manufacturer id is not KORG or universal non-realtime")]
    Manufacturer,
    #[error("payload too large")]
    PayloadTooLarge,
    #[error("packed payload is truncated")]
    Truncated,
    #[error("unknown acknowledgement status {0:#04x}")]
    UnknownStatus(u8),
    #[error("function {0:#04x} is not one the vendor reply parser accepts")]
    UnknownFunction(u8),
    #[error("unknown length form")]
    LengthForm,
    #[error("declared payload length {declared} does not match the {actual} bytes received")]
    LengthMismatch { declared: usize, actual: usize },
    #[error("model id {0:#06x} does not fit the measured form")]
    ModelIdRange(u32),
    #[error("channel {0} is outside 0..16")]
    InvalidChannel(u8),
    #[error("byte {0:#04x} is not a valid SysEx data byte")]
    InvalidByte(u8),
    #[error("device id {0} is outside 0..16")]
    InvalidDeviceId(u8),
    #[error("message is not a KORG identity reply")]
    BadIdentity,
}

/// Pack data with the KORG 7-in-8 scheme.
///
/// Each group of seven bytes gains one leading byte that carries the seven
/// high bits.  A partial group still emits the leading byte.
pub fn pack_7in8(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len().div_ceil(7));
    for group in data.chunks(7) {
        let mut high = 0u8;
        for (index, byte) in group.iter().enumerate() {
            if byte & 0x80 != 0 {
                high |= 1 << index;
            }
        }
        out.push(high);
        for byte in group {
            out.push(byte & 0x7F);
        }
    }
    out
}

/// Unpack a 7-in-8 payload.  `expected_len` is the unpacked byte count.
pub fn unpack_7in8(packed: &[u8], expected_len: usize) -> Result<Vec<u8>, SysexError> {
    let groups = expected_len.div_ceil(7);
    let needed = expected_len + groups;
    if packed.len() < needed {
        return Err(SysexError::Truncated);
    }
    let mut out = Vec::with_capacity(expected_len);
    for group in 0..groups {
        let base = group * 8;
        let high = packed[base];
        let remaining = expected_len - out.len();
        let count = remaining.min(7);
        for index in 0..count {
            let low = packed[base + 1 + index];
            out.push(low | (((high >> index) & 1) << 7));
        }
    }
    Ok(out)
}

/// Longest frame the assembler keeps.  The largest measured message is a
/// 560-byte scene payload: 640 packed bytes plus a 10-byte header and `F7`
/// so this leaves ample headroom and still bounds a
/// device that never sends `F7`.
pub const MAX_FRAME_LEN: usize = 4096;

/// Joins SysEx fragments.  Bytes before `F0` are ignored.
#[derive(Debug, Default)]
pub struct Assembler {
    buffer: Vec<u8>,
    /// A frame ran past `MAX_FRAME_LEN`; ignore bytes until the next `F0`.
    discarding: bool,
}

impl Assembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut complete = Vec::new();
        for byte in bytes {
            if self.discarding {
                if *byte == SYSEX_START {
                    self.discarding = false;
                    self.buffer.clear();
                    self.buffer.push(*byte);
                }
                continue;
            }
            if self.buffer.is_empty() {
                if *byte == SYSEX_START {
                    self.buffer.push(*byte);
                }
                continue;
            }
            match *byte {
                SYSEX_END => {
                    self.buffer.push(*byte);
                    if self.buffer.len() <= MAX_FRAME_LEN {
                        complete.push(std::mem::take(&mut self.buffer));
                    } else {
                        self.buffer.clear();
                    }
                }
                // MIDI timing clock may arrive between SysEx bytes.
                0xF8 => {}
                SYSEX_START => {
                    self.buffer.clear();
                    self.buffer.push(*byte);
                }
                // Any other status byte breaks the frame.  Discard it, as the
                // original assembler does.
                byte if byte & 0x80 != 0 => self.buffer.clear(),
                byte => {
                    self.buffer.push(byte);
                    if self.buffer.len() > MAX_FRAME_LEN {
                        self.buffer.clear();
                        self.discarding = true;
                    }
                }
            }
        }
        complete
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.discarding = false;
    }

    pub fn pending(&self) -> usize {
        self.buffer.len()
    }
}

fn push_model(out: &mut Vec<u8>, model_id: u32) -> Result<(), SysexError> {
    if model_id < 0x80 {
        out.push(model_id as u8);
        Ok(())
    } else if model_id <= 0x3FFF && (model_id & 0x80) == 0 {
        out.push(0);
        out.push(((model_id >> 8) & 0x7F) as u8);
        out.push((model_id & 0x7F) as u8);
        Ok(())
    } else {
        Err(SysexError::ModelIdRange(model_id))
    }
}

fn read_model(bytes: &[u8], start: usize) -> Option<(u32, usize)> {
    if start >= bytes.len() {
        return None;
    }
    if bytes[start] == 0 {
        if start + 2 >= bytes.len() {
            return None;
        }
        Some((
            ((bytes[start + 1] as u32) << 8) | bytes[start + 2] as u32,
            start + 3,
        ))
    } else {
        Some((bytes[start] as u32, start + 1))
    }
}

/// Build one data message.
pub fn build_data_message(spec: &DataMessage, payload: &[u8]) -> Result<Vec<u8>, SysexError> {
    if spec.channel > 0x0F {
        return Err(SysexError::InvalidChannel(spec.channel));
    }
    if spec.command >= 0x80 {
        return Err(SysexError::InvalidByte(spec.command));
    }
    if spec.marker >= 0x80 {
        return Err(SysexError::InvalidByte(spec.marker));
    }
    let packed = pack_7in8(payload);
    let length = packed.len() + 1;
    if length > 0x3FFF {
        return Err(SysexError::PayloadTooLarge);
    }
    let mut out = vec![SYSEX_START, KORG_ID, 0x40 | (spec.channel & 0x0F)];
    push_model(&mut out, spec.model_id)?;
    out.push(spec.command);
    match spec.length_style {
        LengthStyle::ShortOrLong => {
            out.push(0x7F);
            // Branch on the encoded length, not the payload length.  The
            // measured generator branches on the payload length and can emit
            // a length byte of 0x7f or above for payloads 110..126.
            if length < 0x7F {
                out.push(length as u8);
            } else {
                out.push(0x7F);
                out.push(0x02);
                out.push(((length >> 7) & 0x7F) as u8);
                out.push((length & 0x7F) as u8);
            }
        }
        LengthStyle::Fixed3 => {
            out.push((length & 0x7F) as u8);
            out.push(((length >> 7) & 0x7F) as u8);
            out.push(((length >> 14) & 0x7F) as u8);
        }
    }
    out.push(spec.marker);
    out.extend(packed);
    out.push(SYSEX_END);
    Ok(out)
}

/// Build an acknowledgement.
pub fn build_ack(model_id: u32, channel: u8, status: AckStatus) -> Result<Vec<u8>, SysexError> {
    if channel > 0x0F {
        return Err(SysexError::InvalidChannel(channel));
    }
    let mut out = vec![SYSEX_START, KORG_ID, 0x40 | (channel & 0x0F)];
    push_model(&mut out, model_id)?;
    out.push(ACK_COMMAND);
    out.push(status.byte());
    out.push(SYSEX_END);
    Ok(out)
}

/// Parse a reply.  The length style is guessed when the caller does not know
/// it.  Prefer `parse_reply_with_style` for a known device.
pub fn parse_reply(bytes: &[u8]) -> Result<Reply, SysexError> {
    parse_reply_with_style(bytes, None)
}

/// Parse a reply with the device length style.
pub fn parse_reply_with_style(
    bytes: &[u8],
    style: Option<LengthStyle>,
) -> Result<Reply, SysexError> {
    if bytes.len() < 4 || bytes[0] != SYSEX_START || *bytes.last().unwrap() != SYSEX_END {
        return Err(SysexError::Framing);
    }
    match bytes[1] {
        KORG_ID => parse_korg(bytes, style),
        UNIVERSAL_NON_REALTIME => {
            // Requires len == 15, 06 02, then manufacturer 0x42,
            // family/model at 6..8, and version at 8..10.
            if bytes.len() >= 15 && bytes[3] == 0x06 && bytes[4] == 0x02 && bytes[5] == KORG_ID {
                let device_id = ((bytes[6] as u32) << 8) | bytes[7] as u32;
                let version = ((bytes[8] as u32) << 8) | bytes[9] as u32;
                Ok(Reply::Universal { device_id, version })
            } else {
                Err(SysexError::Framing)
            }
        }
        _ => Err(SysexError::Manufacturer),
    }
}

fn parse_generic_length(bytes: &[u8], command_index: usize) -> Result<(usize, usize), SysexError> {
    let constant = *bytes.get(command_index + 1).ok_or(SysexError::Truncated)?;
    if constant != 0x7F {
        return Err(SysexError::LengthForm);
    }
    let second = *bytes.get(command_index + 2).ok_or(SysexError::Truncated)?;
    if second == 0x7F {
        let count = *bytes.get(command_index + 3).ok_or(SysexError::Truncated)?;
        if count != 2 {
            return Err(SysexError::LengthForm);
        }
        let high = *bytes.get(command_index + 4).ok_or(SysexError::Truncated)?;
        let low = *bytes.get(command_index + 5).ok_or(SysexError::Truncated)?;
        Ok((((high as usize) << 7) | low as usize, command_index + 6))
    } else {
        Ok((second as usize, command_index + 3))
    }
}

fn parse_fixed3_length(bytes: &[u8], command_index: usize) -> Result<(usize, usize), SysexError> {
    // 7 bits per byte, as required by the measured format.
    let byte = |offset: usize| -> Result<usize, SysexError> {
        Ok((*bytes
            .get(command_index + offset)
            .ok_or(SysexError::Truncated)?
            & 0x7F) as usize)
    };
    let (low, mid, high) = (byte(1)?, byte(2)?, byte(3)?);
    Ok((low | (mid << 7) | (high << 14), command_index + 4))
}

fn parse_korg(bytes: &[u8], style: Option<LengthStyle>) -> Result<Reply, SysexError> {
    let channel = bytes[2] & 0x0F;
    let (model_id, command_index) = read_model(bytes, 3).ok_or(SysexError::Truncated)?;
    if command_index >= bytes.len() - 1 {
        return Err(SysexError::Truncated);
    }
    let command = bytes[command_index];
    // The Keystage family puts the member byte at `command_index`, a 3-byte
    // length next, and the function byte after the length.  Its ACKs are
    // the bare functions 0x21..0x24, not the 0x5F form.
    if matches!(style, Some(LengthStyle::Fixed3)) {
        let (length, function_index) = parse_fixed3_length(bytes, command_index)?;
        let function = *bytes.get(function_index).ok_or(SysexError::Truncated)?;
        // The declared length
        // must equal the bytes from the function to the terminator.
        let received = bytes.len() - 1 - function_index;
        if length != received {
            return Err(SysexError::LengthMismatch {
                declared: length,
                actual: received,
            });
        }
        // The vendor switch accepts only these function bytes.  Every other
        // value in 0x21..=0x51 falls through to "no reply", so a reply the
        // vendor would drop must not become a `Status` here.
        return match function {
            0x21..=0x24 => {
                let status =
                    AckStatus::from_byte(function).ok_or(SysexError::UnknownStatus(function))?;
                Ok(Reply::Ack {
                    model_id,
                    channel,
                    status,
                })
            }
            0x40 | 0x51 => {
                // The length check above proves the payload fits.
                let packed_len = length - 1;
                let packed_start = function_index + 1;
                Ok(Reply::Data {
                    model_id,
                    channel,
                    command,
                    marker: function,
                    packed: bytes[packed_start..packed_start + packed_len].to_vec(),
                })
            }
            // The vendor reads one byte after the function for these two.  A
            // declared length of 1 leaves only the terminator there.
            0x42 | 0x4F => {
                if length < 2 {
                    return Err(SysexError::Truncated);
                }
                let extra = *bytes.get(function_index + 1).ok_or(SysexError::Truncated)?;
                Ok(Reply::Status {
                    model_id,
                    channel,
                    code: function,
                    extra: Some(extra),
                })
            }
            // The vendor accepts 0x26 with a fixed type and reads no byte
            // after it.  Its meaning is not measured, so keep the function.
            0x26 => Ok(Reply::Status {
                model_id,
                channel,
                code: function,
                extra: None,
            }),
            other => Err(SysexError::UnknownFunction(other)),
        };
    }
    if command == ACK_COMMAND {
        let status_byte = *bytes.get(command_index + 1).ok_or(SysexError::Truncated)?;
        let status =
            AckStatus::from_byte(status_byte).ok_or(SysexError::UnknownStatus(status_byte))?;
        return Ok(Reply::Ack {
            model_id,
            channel,
            status,
        });
    }
    if command == DATA_COMMAND {
        let (length, marker_index) = match style {
            Some(LengthStyle::Fixed3) => parse_fixed3_length(bytes, command_index)?,
            Some(LengthStyle::ShortOrLong) => parse_generic_length(bytes, command_index)?,
            None => {
                let constant = *bytes.get(command_index + 1).ok_or(SysexError::Truncated)?;
                if constant == 0x7F {
                    parse_generic_length(bytes, command_index)?
                } else {
                    parse_fixed3_length(bytes, command_index)?
                }
            }
        };
        let marker = *bytes.get(marker_index).ok_or(SysexError::Truncated)?;
        if length == 0 {
            return Err(SysexError::Truncated);
        }
        let packed_len = length - 1;
        let packed_start = marker_index + 1;
        if packed_start + packed_len > bytes.len() - 1 {
            return Err(SysexError::Truncated);
        }
        return Ok(Reply::Data {
            model_id,
            channel,
            command,
            marker,
            packed: bytes[packed_start..packed_start + packed_len].to_vec(),
        });
    }
    let extra = bytes.get(command_index + 1).copied();
    Ok(Reply::Status {
        model_id,
        channel,
        code: command,
        extra,
    })
}

/// Map one acknowledgement to a phase result.
pub fn classify(phase: Phase, status: AckStatus) -> Result<(), PhaseFailure> {
    if status == phase.expected() {
        Ok(())
    } else {
        Err(phase.failure())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(model_id: u32, style: LengthStyle) -> DataMessage {
        DataMessage::new(model_id, 0, DATA_COMMAND, DATA_MARKER, style)
    }

    #[test]
    fn a_status_byte_discards_an_incomplete_frame() {
        let mut assembler = Assembler::new();
        assert!(assembler.push(&[0xF0, 0x42, 0x40, 0xFE, 0xF7]).is_empty());
        assert_eq!(assembler.pending(), 0);
    }

    #[test]
    fn timing_clock_is_ignored_inside_a_frame() {
        let mut assembler = Assembler::new();
        let frame = [0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0xF7];
        let mut bytes = frame.to_vec();
        bytes.insert(3, 0xF8);
        assert_eq!(assembler.push(&bytes), vec![frame.to_vec()]);
    }

    #[test]
    fn a_new_start_restarts_the_frame() {
        let mut assembler = Assembler::new();
        let frame = [0xF0, 0x42, 0x40, 0xF7];
        let bytes = [0xF0, 0x42, 0x01, 0xF0, 0x42, 0x40, 0xF7];
        assert_eq!(assembler.push(&bytes), vec![frame.to_vec()]);
    }

    #[test]
    fn an_overlong_frame_is_dropped_and_resyncs_on_the_next_start() {
        let mut assembler = Assembler::new();
        // `F0` plus one data byte too many, then `F7`: nothing completes.
        let mut overlong = vec![0xF0];
        overlong.extend(std::iter::repeat_n(0x01u8, MAX_FRAME_LEN));
        overlong.push(SYSEX_END);
        assert!(assembler.push(&overlong).is_empty());
        assert_eq!(assembler.pending(), 0);

        // Bytes after the drop are ignored until the next `F0`.
        assert!(assembler.push(&[0x02, 0x03]).is_empty());
        assert_eq!(assembler.pending(), 0);

        // A following well-formed frame is accepted.
        let frame = [0xF0, 0x42, 0x40, 0xF7];
        assert_eq!(assembler.push(&frame), vec![frame.to_vec()]);
    }

    #[test]
    fn a_frame_exactly_at_the_cap_is_kept() {
        let mut assembler = Assembler::new();
        let mut frame = vec![0xF0];
        frame.extend(std::iter::repeat_n(0x01u8, MAX_FRAME_LEN - 2));
        frame.push(SYSEX_END);
        assert_eq!(frame.len(), MAX_FRAME_LEN);
        assert_eq!(assembler.push(&frame), vec![frame]);
    }

    #[test]
    fn a_declared_length_mismatch_is_rejected() {
        // Scene reply for X14802: length 641, but one packed byte is missing.
        let mut frame = vec![0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x01, 0x05, 0x00, 0x40];
        frame.extend(std::iter::repeat_n(0u8, 639));
        frame.push(0xF7);
        assert!(matches!(
            parse_data_reply(&frame, 560),
            Err(SysexError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn packing_round_trips() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|index| (index * 37 % 256) as u8).collect();
            let packed = pack_7in8(&data);
            let unpacked = unpack_7in8(&packed, len).unwrap();
            assert_eq!(unpacked, data, "length {len}");
        }
    }

    #[test]
    fn packing_layout_is_msb_first() {
        assert_eq!(pack_7in8(&[0x80, 0x01]), vec![0x01, 0x00, 0x01]);
        assert_eq!(
            pack_7in8(&[0xFF; 7]),
            vec![0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F]
        );
    }

    #[test]
    fn short_data_message_round_trips() {
        let spec = message(0x112, LengthStyle::ShortOrLong);
        let payload = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 0x80];
        let bytes = build_data_message(&spec, &payload).unwrap();
        assert_eq!(&bytes[..5], &[0xF0, 0x42, 0x40, 0x00, 0x01]);
        let Reply::Data { packed, marker, .. } = parse_reply(&bytes).unwrap() else {
            panic!("expected data reply");
        };
        assert_eq!(marker, DATA_MARKER);
        assert_eq!(unpack_7in8(&packed, payload.len()).unwrap(), payload);
    }

    #[test]
    fn long_data_message_round_trips() {
        let spec = DataMessage::new(
            0x136,
            3,
            DATA_COMMAND,
            DATA_MARKER,
            LengthStyle::ShortOrLong,
        );
        let payload = vec![0x55u8; 200];
        let bytes = build_data_message(&spec, &payload).unwrap();
        let Reply::Data { packed, .. } = parse_reply(&bytes).unwrap() else {
            panic!("expected data reply");
        };
        assert_eq!(unpack_7in8(&packed, payload.len()).unwrap(), payload);
    }

    #[test]
    fn length_boundary_round_trips() {
        for len in 100..=140usize {
            let payload = vec![0xA5u8; len];
            let spec = message(0x136, LengthStyle::ShortOrLong);
            let bytes = build_data_message(&spec, &payload).unwrap();
            for byte in &bytes[1..bytes.len() - 1] {
                assert!(*byte < 0x80, "length {len}: byte {byte:#04x}");
            }
            let Reply::Data { packed, .. } =
                parse_reply_with_style(&bytes, Some(LengthStyle::ShortOrLong)).unwrap()
            else {
                panic!("expected data reply");
            };
            assert_eq!(unpack_7in8(&packed, len).unwrap(), payload);
        }
    }

    /// One Keystage frame: model 0x169, the member byte at index 6, a 3-byte
    /// length from 7, the function at 10, then the payload and `F7`.  The
    /// caller passes the declared length so a mismatch stays testable.
    fn keystage(channel: u8, member: u8, length: usize, function: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![
            0xF0,
            0x42,
            0x40 | channel,
            0x00,
            0x01,
            0x69,
            member,
            (length & 0x7F) as u8,
            ((length >> 7) & 0x7F) as u8,
            ((length >> 14) & 0x7F) as u8,
            function,
        ];
        frame.extend_from_slice(payload);
        frame.push(0xF7);
        frame
    }

    #[test]
    fn fixed3_scene_dump_round_trips() {
        // A scene dump is 500 bytes: 572 packed bytes and a declared length
        // of 573 (`3D 04 00`).  The descriptor command byte at index 6 is 0.
        let spec = DataMessage::new(0x169, 0, 0, 0x51, LengthStyle::Fixed3);
        let payload = vec![0xAAu8; 500];
        let bytes = build_data_message(&spec, &payload).unwrap();
        assert_eq!(&bytes[6..11], &[0x00, 0x3D, 0x04, 0x00, 0x51]);
        let Reply::Data {
            command,
            marker,
            packed,
            ..
        } = parse_reply_with_style(&bytes, Some(LengthStyle::Fixed3)).unwrap()
        else {
            panic!("expected data reply");
        };
        assert_eq!(command, 0, "the member byte is kept");
        assert_eq!(marker, 0x51);
        assert_eq!(unpack_7in8(&packed, payload.len()).unwrap(), payload);
    }

    #[test]
    fn keystage_request_functions_are_rejected() {
        // The editor sends 0x10 (scene dump request) and 0x14 (select scene).
        // The vendor reply parser has no case for them.
        for function in [0x10u8, 0x14, 0x25, 0x30, 0x7F] {
            let frame = keystage(0, 0, 1, function, &[]);
            assert_eq!(
                parse_reply_with_style(&frame, Some(LengthStyle::Fixed3)),
                Err(SysexError::UnknownFunction(function)),
                "function {function:#04x}"
            );
        }
    }

    #[test]
    fn keystage_ack_and_status_frames_parse() {
        // ACK: bare function 0x23, length 1.  Member 1 comes from the firmware
        // template (findings 2026-10-22 §5).  The parser does not use it.
        let ack = keystage(0, 1, 1, 0x23, &[]);
        assert_eq!(
            parse_reply_with_style(&ack, Some(LengthStyle::Fixed3)).unwrap(),
            Reply::Ack {
                model_id: 0x169,
                channel: 0,
                status: AckStatus::DataReceived,
            }
        );

        // 0x42 mode reply: one byte after the function.
        let mode = keystage(0, 1, 2, 0x42, &[0x01]);
        assert_eq!(
            parse_reply_with_style(&mode, Some(LengthStyle::Fixed3)).unwrap(),
            Reply::Status {
                model_id: 0x169,
                channel: 0,
                code: 0x42,
                extra: Some(1),
            }
        );

        // 0x4F is the other function with a byte after it.
        let status = keystage(0, 1, 2, 0x4F, &[0x05]);
        assert_eq!(
            parse_reply_with_style(&status, Some(LengthStyle::Fixed3)).unwrap(),
            Reply::Status {
                model_id: 0x169,
                channel: 0,
                code: 0x4F,
                extra: Some(5),
            }
        );

        // 0x26 is accepted with a fixed type and no byte after the function.
        // Its meaning is not measured, so no extra byte is reported.
        let other = keystage(0, 1, 1, 0x26, &[]);
        assert_eq!(
            parse_reply_with_style(&other, Some(LengthStyle::Fixed3)).unwrap(),
            Reply::Status {
                model_id: 0x169,
                channel: 0,
                code: 0x26,
                extra: None,
            }
        );

        // A status function with a declared length of 1 has no value byte.
        let empty = keystage(0, 1, 1, 0x42, &[]);
        assert_eq!(
            parse_reply_with_style(&empty, Some(LengthStyle::Fixed3)),
            Err(SysexError::Truncated)
        );
    }

    #[test]
    fn fixed3_length_must_match_the_frame() {
        // Five bytes follow the function (four data bytes and `F7`).
        let data = [0x00, 0x01, 0x02, 0x03];
        let short = keystage(0, 0, 4, 0x40, &data);
        assert_eq!(
            parse_reply_with_style(&short, Some(LengthStyle::Fixed3)),
            Err(SysexError::LengthMismatch {
                declared: 4,
                actual: 5,
            })
        );
        let long = keystage(0, 0, 6, 0x40, &data);
        assert_eq!(
            parse_reply_with_style(&long, Some(LengthStyle::Fixed3)),
            Err(SysexError::LengthMismatch {
                declared: 6,
                actual: 5,
            })
        );
    }

    #[test]
    fn keystage_data_frame_matches_the_vendor_generator() {
        let spec = DataMessage::new(0x169, 0, 0, 0x51, LengthStyle::Fixed3);
        let bytes = build_data_message(&spec, &[0x80, 0x01]).unwrap();
        assert_eq!(
            &bytes[..11],
            &[0xF0, 0x42, 0x40, 0x00, 0x01, 0x69, 0x00, 0x04, 0x00, 0x00, 0x51]
        );
        let Reply::Data {
            command,
            marker,
            packed,
            ..
        } = parse_reply_with_style(&bytes, Some(LengthStyle::Fixed3)).unwrap()
        else {
            panic!("expected data");
        };
        assert_eq!(command, 0);
        assert_eq!(marker, 0x51);
        assert_eq!(unpack_7in8(&packed, 2).unwrap(), vec![0x80, 0x01]);
    }

    #[test]
    fn acknowledgement_round_trips() {
        for status in [
            AckStatus::WriteComplete,
            AckStatus::WriteError,
            AckStatus::DataReceived,
            AckStatus::Nak,
        ] {
            let bytes = build_ack(0x112, 5, status).unwrap();
            let reply = parse_reply(&bytes).unwrap();
            assert_eq!(
                reply,
                Reply::Ack {
                    model_id: 0x112,
                    channel: 5,
                    status
                }
            );
        }
    }

    #[test]
    fn identity_reply_round_trips() {
        let bytes = [
            0xF0, 0x7E, 0x00, 0x06, 0x02, 0x42, 0x01, 0x12, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
            0xF7,
        ];
        assert_eq!(
            parse_reply(&bytes).unwrap(),
            Reply::Universal {
                device_id: 0x0112,
                version: 1
            }
        );
    }

    #[test]
    fn assembler_joins_fragments() {
        let message = build_ack(0x112, 0, AckStatus::DataReceived).unwrap();
        let mut assembler = Assembler::default();
        assert!(assembler.push(&message[..2]).is_empty());
        assert!(assembler.push(&message[2..5]).is_empty());
        assert_eq!(assembler.push(&message[5..]), vec![message.clone()]);
        assert_eq!(assembler.pending(), 0);
    }

    #[test]
    fn assembler_ignores_bytes_before_start() {
        let message = build_ack(0x112, 0, AckStatus::DataReceived).unwrap();
        let mut assembler = Assembler::default();
        let mut bytes = vec![0x01, 0x02];
        bytes.extend_from_slice(&message);
        assert_eq!(assembler.push(&bytes), vec![message]);
    }

    #[test]
    fn model_id_range_is_rejected() {
        for model_id in [0x80, 0x1FF, 0x4000] {
            let spec = message(model_id, LengthStyle::ShortOrLong);
            assert_eq!(
                build_data_message(&spec, &[0]),
                Err(SysexError::ModelIdRange(model_id)),
                "model {model_id:#x}"
            );
        }
    }

    #[test]
    fn measured_model_id_encodes_as_three_bytes() {
        let spec = message(0x112, LengthStyle::ShortOrLong);
        let bytes = build_data_message(&spec, &[0]).unwrap();
        assert_eq!(&bytes[3..6], &[0x00, 0x01, 0x12]);
    }

    #[test]
    fn invalid_data_byte_is_rejected() {
        let spec = DataMessage::new(0x112, 0, 0x80, DATA_MARKER, LengthStyle::ShortOrLong);
        assert_eq!(
            build_data_message(&spec, &[0]),
            Err(SysexError::InvalidByte(0x80))
        );
    }

    #[test]
    fn phase_classification() {
        assert_eq!(classify(Phase::ModeCheck, AckStatus::DataReceived), Ok(()));
        assert_eq!(
            classify(Phase::ModeCheck, AckStatus::WriteComplete),
            Err(PhaseFailure::UnsupportedDevice)
        );
        assert_eq!(
            classify(Phase::Data, AckStatus::Nak),
            Err(PhaseFailure::Nak)
        );
        assert_eq!(
            classify(Phase::Write, AckStatus::WriteError),
            Err(PhaseFailure::WriteError)
        );
    }

    #[test]
    fn search_replies_parse_for_both_studios() {
        assert_eq!(search_request(5), [0xF0, 0x42, 0x50, 0x00, 0x05, 0xF7]);
        // Measured 2026-10-28 over USB.
        let control = [
            0xF0, 0x42, 0x50, 0x01, 0x00, 0x05, 0x37, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x00,
            0xF7,
        ];
        let (echo, reply) = parse_search_reply(&control).unwrap();
        assert_eq!(echo, 5);
        assert_eq!(reply.model_id, 0x137);
        assert_eq!(reply.firmware(), "1.05");
        let key = [
            0xF0, 0x42, 0x50, 0x01, 0x0F, 0x05, 0x36, 0x01, 0x00, 0x00, 0x08, 0x00, 0x01, 0x00,
            0xF7,
        ];
        let (_, reply) = parse_search_reply(&key).unwrap();
        assert_eq!((reply.device_id, reply.model_id), (0x0F, 0x136));
        assert_eq!(reply.firmware(), "1.08");
        assert!(parse_search_reply(&search_request(5)).is_err());
    }

    #[test]
    fn identity_request_is_the_mode_check() {
        assert_eq!(IDENTITY_REQUEST, [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]);
    }

    #[test]
    fn identity_replies_parse_for_both_studios() {
        let key = [
            0xF0, 0x7E, 0x0F, 0x06, 0x02, 0x42, 0x36, 0x01, 0x00, 0x00, 0x08, 0x00, 0x01, 0x00,
            0xF7,
        ];
        let reply = parse_identity(&key).unwrap();
        assert_eq!(reply.device_id, 0x0F);
        assert_eq!(reply.model_id, 0x136);
        assert_eq!(reply.family, [0x36, 0x01]);

        let control = [
            0xF0, 0x7E, 0x00, 0x06, 0x02, 0x42, 0x37, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x00,
            0xF7,
        ];
        let reply = parse_identity(&control).unwrap();
        assert_eq!(reply.device_id, 0x00);
        assert_eq!(reply.model_id, 0x137);
    }

    #[test]
    fn measured_requests_match_the_capture() {
        assert_eq!(
            build_request(0x0F, 0x136, Request::SceneSetInfo).unwrap(),
            [0xF0, 0x42, 0x4F, 0x00, 0x01, 0x36, 0x01, 0x00, 0x00, 0x12, 0xF7]
        );
        assert_eq!(
            build_request(0x0F, 0x136, Request::SelectScene(3)).unwrap(),
            [0xF0, 0x42, 0x4F, 0x00, 0x01, 0x36, 0x02, 0x00, 0x00, 0x14, 0x03, 0xF7]
        );
        assert_eq!(
            build_request(0x0F, 0x136, Request::ReadBlock).unwrap(),
            [0xF0, 0x42, 0x4F, 0x00, 0x01, 0x36, 0x01, 0x00, 0x00, 0x10, 0xF7]
        );
        assert_eq!(
            build_request(0x00, 0x137, Request::GlobalData).unwrap(),
            [0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x01, 0x00, 0x00, 0x0E, 0xF7]
        );
    }

    #[test]
    fn measured_data_reply_unpacks_after_the_four_byte_header() {
        let payload: Vec<u8> = (0..10).map(|index| index as u8 | 0x80).collect();
        let packed = pack_7in8(&payload);
        let length = packed.len() + 1;
        let mut frame = vec![
            0xF0,
            0x42,
            0x4F,
            0x00,
            0x01,
            0x36,
            length as u8,
            (length >> 7) as u8,
            (length >> 14) as u8,
            0x40,
        ];
        frame.extend_from_slice(&packed);
        frame.push(0xF7);

        assert_eq!(parse_data_reply(&frame, payload.len()).unwrap(), payload);
        assert_eq!(reply_model(&frame).unwrap(), 0x136);
    }

    #[test]
    fn measured_data_reply_rejects_bad_frames() {
        let payload = [1u8, 2, 3];
        let packed = pack_7in8(&payload);
        let length = packed.len() + 1;
        let mut frame = vec![
            0xF0,
            0x42,
            0x4F,
            0x00,
            0x01,
            0x36,
            length as u8,
            (length >> 7) as u8,
            (length >> 14) as u8,
            0x40,
        ];
        frame.extend_from_slice(&packed);
        frame.push(0xF7);
        let mut short = frame.clone();
        short.drain(10..short.len() - 1);
        assert!(matches!(
            parse_data_reply(&short, payload.len()),
            Err(SysexError::Truncated | SysexError::LengthMismatch { .. })
        ));
        assert_eq!(
            parse_data_reply(&frame[..frame.len() - 3], payload.len()),
            Err(SysexError::Framing)
        );
        let mut wrong = frame.clone();
        wrong[1] = 0x41;
        assert_eq!(
            parse_data_reply(&wrong, payload.len()),
            Err(SysexError::Manufacturer)
        );
    }
    #[test]
    fn calibration_frames_match_the_decompiled_commands() {
        let frame = build_calibration_frame(0x0F, 0x136, 0x03).unwrap();
        assert_eq!(
            frame,
            [0xF0, 0x42, 0x4F, 0x00, 0x01, 0x36, 0x03, 0x00, 0x00, 0x1D, 0x01, 0x03, 0xF7]
        );
    }

    #[test]
    fn measured_write_frames_match_the_capture() {
        let commit = build_commit(0x00, 0x137, 1).unwrap();
        assert_eq!(
            commit,
            [0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x02, 0x00, 0x00, 0x11, 0x01, 0xF7]
        );

        let payload = [0xAAu8; 560];
        let frame = build_write_frame(0x00, 0x137, X14802_SCENE_WRITE_HEADER, &payload).unwrap();
        assert_eq!(
            &frame[..10],
            &[0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x01, 0x05, 0x00, 0x40]
        );
        assert_eq!(parse_data_reply(&frame, payload.len()).unwrap(), payload);

        let ack = [
            0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x01, 0x00, 0x00, 0x21, 0xF7,
        ];
        assert_eq!(ack_status(&ack).unwrap(), AckStatus::WriteComplete);
        assert_eq!(ack_status(&ack[..5]).unwrap_err(), SysexError::Framing);
    }
}
