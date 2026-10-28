//! Block data the engine reads, edits and writes: addressed memory pieced
//! together from records (CDL §8.3), and fields inside it by path (§4.4).

use ctrl_cdl::{Descriptor, Expr, Index, Item, Path, Storage, TypeRef};
use ctrl_codec::{Bindings, Codec, Memory, Val};

use crate::EngineError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    pub memory: Memory,
}

impl Document {
    /// Decodes `bytes` as the reply of `message`; its records fill memory.
    pub fn load(
        &mut self,
        codec: &Codec,
        d: &Descriptor,
        message: &str,
        bytes: &[u8],
    ) -> Result<Bindings, EngineError> {
        let values = codec.decode_reply(message, bytes, &Bindings::new())?;
        let m = d
            .message(message)
            .ok_or_else(|| EngineError::Codec(format!("unknown message `{message}`")))?;
        for r in &m.records {
            if let Some(Val::Bytes(array)) = values.get(&r.name) {
                for rec in codec.decode_records(message, &r.name, array)? {
                    if let (Some(a), Some(data)) = (rec.address, &rec.data) {
                        self.memory.set(a, data);
                    }
                }
            }
        }
        Ok(values)
    }

    pub fn block_bytes(&self, codec: &Codec, block: &str, index: u32) -> Option<Vec<u8>> {
        codec.block_bytes(&self.memory, block, index)
    }

    /// Writes `value` into the field at `path`. Supports one-byte fields
    /// (with `bits`) and one-byte array elements; the byte must be known.
    pub fn set(
        &mut self,
        codec: &Codec,
        d: &Descriptor,
        path: &Path,
        value: i64,
        vars: &dyn Fn(&str) -> Option<i64>,
    ) -> Result<(), EngineError> {
        let (address, bits) = resolve(codec, d, path, vars)?;
        let old = self
            .memory
            .get(address, 1)
            .ok_or_else(|| EngineError::Check(format!("`{path}`: not in the document")))?[0];
        let (lo, hi) = bits.unwrap_or((0, 7));
        let width = hi - lo + 1;
        let max = (1i64 << width) - 1;
        if !(0..=max).contains(&value) {
            return Err(EngineError::Check(format!(
                "`{path}` = {value} does not fit {width} bits"
            )));
        }
        let mask = (max as u8) << lo;
        let new = (old & !mask) | ((value as u8) << lo);
        self.memory.set(address, &[new]);
        Ok(())
    }

    pub fn get(
        &self,
        codec: &Codec,
        d: &Descriptor,
        path: &Path,
        vars: &dyn Fn(&str) -> Option<i64>,
    ) -> Result<i64, EngineError> {
        let (address, bits) = resolve(codec, d, path, vars)?;
        let b = self
            .memory
            .get(address, 1)
            .ok_or_else(|| EngineError::Check(format!("`{path}`: not in the document")))?[0];
        let (lo, hi) = bits.unwrap_or((0, 7));
        Ok(i64::from((b >> lo) & (((1u16 << (hi - lo + 1)) - 1) as u8)))
    }
}

fn index(
    path: &Path,
    i: &Option<Index>,
    vars: &dyn Fn(&str) -> Option<i64>,
) -> Result<u32, EngineError> {
    match i {
        None => Err(EngineError::Check(format!("`{path}`: an index is missing"))),
        Some(Index::At(e)) => {
            let v = Expr::parse(e)
                .and_then(|x| x.eval(vars))
                .map_err(|e| EngineError::Check(format!("`{path}`: {e}")))?;
            u32::try_from(v).map_err(|_| EngineError::Check(format!("`{path}`: negative index")))
        }
        Some(Index::Range { .. }) => Err(EngineError::Unsupported(format!("`{path}`: a range"))),
    }
}

fn one_byte(d: &Descriptor, ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Storage(s) => matches!(
            s,
            Storage::U7 | Storage::U8 | Storage::Bool | Storage::Bits(_)
        ),
        TypeRef::Named(n) => match d.types.iter().find(|t| t.name() == n) {
            Some(ctrl_cdl::TypeDef::Int(i)) => {
                matches!(
                    i.storage,
                    Storage::U7 | Storage::U8 | Storage::Bool | Storage::Bits(_)
                )
            }
            Some(ctrl_cdl::TypeDef::Enum(_)) => true,
            _ => false,
        },
    }
}

/// The byte address and bit range of a field path.
fn resolve(
    codec: &Codec,
    d: &Descriptor,
    path: &Path,
    vars: &dyn Fn(&str) -> Option<i64>,
) -> Result<(u32, Option<(u8, u8)>), EngineError> {
    let first = &path.segments[0];
    let block = d
        .block(&first.name)
        .ok_or_else(|| EngineError::Check(format!("`{path}`: no block `{}`", first.name)))?;
    let i = if block.count.is_some() {
        index(path, &first.index, vars)?
    } else {
        0
    };
    let (mut at, _) = codec.block_span(&block.name, i)?;
    let mut items = &block.items;
    let rest = &path.segments[1..];
    for (k, seg) in rest.iter().enumerate() {
        let last = k + 1 == rest.len();
        let item = items
            .iter()
            .find(|it| match it {
                Item::Field(f) => f.name == seg.name,
                Item::Group(g) => g.name == seg.name,
                Item::Array(a) => a.name == seg.name,
            })
            .ok_or_else(|| EngineError::Check(format!("`{path}`: no `{}`", seg.name)))?;
        match item {
            Item::Field(f) if last => {
                if !one_byte(d, &f.ty) {
                    return Err(EngineError::Unsupported(format!(
                        "`{path}`: only one-byte fields are set here"
                    )));
                }
                at += f.offset.unwrap_or(0);
                return Ok((at, f.bits.map(|b| (b.lo as u8, b.hi as u8))));
            }
            Item::Group(g) if !last => {
                at += g.offset.unwrap_or(0);
                items = &g.items;
            }
            Item::Array(a) => {
                let n = index(path, &seg.index, vars)?;
                if n >= a.count {
                    return Err(EngineError::Check(format!(
                        "`{path}`: `{}` has {} elements",
                        a.name, a.count
                    )));
                }
                at += a.offset.unwrap_or(0) + n * a.stride.unwrap_or(1);
                if last {
                    match &a.ty {
                        Some(ty) if one_byte(d, ty) => return Ok((at, None)),
                        _ => {
                            return Err(EngineError::Unsupported(format!(
                                "`{path}`: an array of fields; name the field"
                            )))
                        }
                    }
                }
                items = &a.items;
            }
            _ => {
                return Err(EngineError::Check(format!(
                    "`{path}`: `{}` is not a value",
                    seg.name
                )))
            }
        }
    }
    Err(EngineError::Check(format!("`{path}`: names no field")))
}
