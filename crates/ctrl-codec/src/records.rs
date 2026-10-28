//! Records inside one message (CDL §8.3): a byte array of a message is a
//! run of instances of one pattern, each an addressed piece of device
//! memory. A Faderfox dump carries 22 or 430 such chunks in one SysEx.

use std::collections::BTreeMap;

use ctrl_cdl::{Expr, Records};

use crate::{Bindings, Codec, CodecError, Val};

/// One decoded record: its values, and the address and data its `records`
/// node names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub values: Bindings,
    pub address: Option<u32>,
    pub data: Option<Vec<u8>>,
}

/// Device memory pieced together from records; holes stay unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Memory {
    bytes: BTreeMap<u32, u8>,
}

impl Memory {
    /// Writes `data` from `address` on; later records overwrite earlier.
    pub fn set(&mut self, address: u32, data: &[u8]) {
        for (i, b) in data.iter().enumerate() {
            self.bytes.insert(address + i as u32, *b);
        }
    }

    /// `len` bytes from `address`, when every one of them is known.
    pub fn get(&self, address: u32, len: u32) -> Option<Vec<u8>> {
        (address..address + len)
            .map(|a| self.bytes.get(&a).copied())
            .collect()
    }

    pub fn from_records(records: &[Record]) -> Self {
        let mut m = Self::default();
        for r in records {
            if let (Some(a), Some(d)) = (r.address, &r.data) {
                m.set(a, d);
            }
        }
        m
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl<'d> Codec<'d> {
    fn records_node(&self, message: &str, name: &str) -> Result<&'d Records, CodecError> {
        let m = self.message(message)?;
        m.records
            .iter()
            .find(|r| r.name == name)
            .ok_or_else(|| CodecError::Unsupported(format!("`{message}` has no records `{name}`")))
    }

    /// Splits the byte array `name` of `message` (as a reply or send
    /// captured it) into its records; checks each record's derived values.
    pub fn decode_records(
        &self,
        message: &str,
        name: &str,
        bytes: &[u8],
    ) -> Result<Vec<Record>, CodecError> {
        let r = self.records_node(message, name)?;
        let element = &r.element;
        let pattern = element
            .send
            .as_ref()
            .ok_or_else(|| CodecError::NoTemplate(r.pattern.clone(), "pattern"))?;
        let mut out = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let (mut values, used) = self
                .decode_at(element, pattern, &bytes[at..], &Bindings::new(), true)
                .map_err(|e| {
                    CodecError::Unsupported(format!(
                        "record {} (byte {at} of `{name}`): {e}",
                        out.len()
                    ))
                })?;
            if used == 0 {
                return Err(CodecError::Unsupported(format!(
                    "record pattern `{}` matches no bytes",
                    r.pattern
                )));
            }
            let address = match &r.address {
                Some(a) => {
                    let e = Expr::parse(a).map_err(CodecError::Expr)?;
                    let v = e.eval(&self.lookup(&values)).map_err(CodecError::Expr)?;
                    values.insert("address".into(), Val::Int(v));
                    Some(u32::try_from(v).map_err(|_| {
                        CodecError::Range(format!("record address {v} is negative"))
                    })?)
                }
                None => None,
            };
            self.check_derived(element, &values).map_err(|e| {
                CodecError::Unsupported(format!(
                    "record {} (byte {at} of `{name}`): {e}",
                    out.len()
                ))
            })?;
            let data = r.data.as_ref().and_then(|d| match values.get(d) {
                Some(Val::Bytes(b)) => Some(b.clone()),
                _ => None,
            });
            out.push(Record {
                values,
                address,
                data,
            });
            at += used;
        }
        Ok(out)
    }

    /// The bytes of the records, each from its values; derived values
    /// (addresses, sums) are filled in.
    pub fn encode_records(
        &self,
        message: &str,
        name: &str,
        records: &[Bindings],
    ) -> Result<Vec<u8>, CodecError> {
        let r = self.records_node(message, name)?;
        let pattern = r
            .element
            .send
            .as_ref()
            .ok_or_else(|| CodecError::NoTemplate(r.pattern.clone(), "pattern"))?;
        let mut out = Vec::new();
        for values in records {
            out.extend(self.encode_template(&r.element, pattern, values)?.0);
        }
        Ok(out)
    }

    /// Records for memory: one per `(address, data)`, with `address` and
    /// the records' data name bound; `derive` supplies the rest.
    pub fn records_for(
        &self,
        message: &str,
        name: &str,
        pieces: &[(u32, Vec<u8>)],
    ) -> Result<Vec<Bindings>, CodecError> {
        let r = self.records_node(message, name)?;
        let data = r
            .data
            .as_ref()
            .ok_or_else(|| CodecError::Unsupported(format!("records `{name}` names no `data`")))?;
        Ok(pieces
            .iter()
            .map(|(a, d)| {
                Bindings::from([
                    ("address".to_string(), Val::Int(i64::from(*a))),
                    (data.clone(), Val::Bytes(d.clone())),
                ])
            })
            .collect())
    }

    /// Element `index` of an addressed block, when memory holds all of it.
    pub fn block_bytes(&self, memory: &Memory, block: &str, index: u32) -> Option<Vec<u8>> {
        let (address, size) = self.block_span(block, index).ok()?;
        memory.get(address, size)
    }

    /// Where element `index` of `block` lives: its address and size.
    pub fn block_span(&self, block: &str, index: u32) -> Result<(u32, u32), CodecError> {
        let b = self
            .d
            .block(block)
            .ok_or_else(|| CodecError::UnknownName(block.into()))?;
        let (Some(address), Some(size)) = (b.address, b.size) else {
            return Err(CodecError::Unsupported(format!(
                "block `{block}` has no `address` and `size`"
            )));
        };
        if index >= b.count.unwrap_or(1) {
            return Err(CodecError::Range(format!(
                "block `{block}` has no element {index}"
            )));
        }
        Ok((address + index * size, size))
    }

    /// Element `index` of `block` cut into the block's `chunk`-sized pieces,
    /// each with its address.
    pub fn block_pieces(
        &self,
        block: &str,
        index: u32,
        data: &[u8],
    ) -> Result<Vec<(u32, Vec<u8>)>, CodecError> {
        let (address, size) = self.block_span(block, index)?;
        if data.len() != size as usize {
            return Err(CodecError::Range(format!(
                "block `{block}` is {size} bytes, not {}",
                data.len()
            )));
        }
        let chunk = self.d.block(block).and_then(|b| b.chunk).unwrap_or(size) as usize;
        Ok(data
            .chunks(chunk)
            .enumerate()
            .map(|(i, c)| (address + (i * chunk) as u32, c.to_vec()))
            .collect())
    }
}
