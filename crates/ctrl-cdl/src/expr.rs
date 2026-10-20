//! CDL expressions (CDL §10.1): the small integer language of `{…}`
//! placeholders, `ok-when`, `visible-when`, `require` and path indexes.
//!
//! Integers are `i64`; comparisons and `&&`/`||`/`!` give 0 or 1. Operators
//! bind as in C. Arithmetic (`+ - * / %`) and bitwise (`& | ^ << >>`)
//! operators may not be mixed without parentheses: LBMT groups them the other
//! way round, so an unparenthesised mix would read differently there.

use std::fmt;

/// A parsed expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Int(i64),
    Name(String),
    Unary(UnOp, Box<Expr>),
    Binary(Box<Expr>, BinOp, Box<Expr>),
    /// Parentheses, kept so the mixing rule can see them.
    Group(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Shl,
    Shr,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    BitAnd,
    BitXor,
    BitOr,
    And,
    Or,
}

impl BinOp {
    fn info(self) -> (&'static str, u8, Family) {
        use BinOp::*;
        match self {
            Mul => ("*", 10, Family::Arith),
            Div => ("/", 10, Family::Arith),
            Rem => ("%", 10, Family::Arith),
            Add => ("+", 9, Family::Arith),
            Sub => ("-", 9, Family::Arith),
            Shl => ("<<", 8, Family::Bits),
            Shr => (">>", 8, Family::Bits),
            Lt => ("<", 7, Family::Other),
            Le => ("<=", 7, Family::Other),
            Gt => (">", 7, Family::Other),
            Ge => (">=", 7, Family::Other),
            Eq => ("==", 6, Family::Other),
            Ne => ("!=", 6, Family::Other),
            BitAnd => ("&", 5, Family::Bits),
            BitXor => ("^", 4, Family::Bits),
            BitOr => ("|", 3, Family::Bits),
            And => ("&&", 2, Family::Other),
            Or => ("||", 1, Family::Other),
        }
    }

    pub fn symbol(self) -> &'static str {
        self.info().0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Arith,
    Bits,
    Other,
}

const OPERATORS: &[(&str, BinOp)] = &[
    ("<<", BinOp::Shl),
    (">>", BinOp::Shr),
    ("<=", BinOp::Le),
    (">=", BinOp::Ge),
    ("==", BinOp::Eq),
    ("!=", BinOp::Ne),
    ("&&", BinOp::And),
    ("||", BinOp::Or),
    ("*", BinOp::Mul),
    ("/", BinOp::Div),
    ("%", BinOp::Rem),
    ("+", BinOp::Add),
    ("-", BinOp::Sub),
    ("<", BinOp::Lt),
    (">", BinOp::Gt),
    ("&", BinOp::BitAnd),
    ("^", BinOp::BitXor),
    ("|", BinOp::BitOr),
];

impl Expr {
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut p = Parser { s, pos: 0 };
        let e = p.binary(0)?;
        p.skip_ws();
        if p.pos < s.len() {
            return Err(format!("`{s}`: unexpected `{}`", &s[p.pos..]));
        }
        e.check_mixing()
            .map_err(|(a, b)| format!("`{s}`: put parentheses around `{a}` and `{b}`; CDL and LBMT group them differently"))?;
        Ok(e)
    }

    /// Every name the expression reads, in order, without repeats.
    pub fn names(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Self::Int(_) => {}
            Self::Name(n) => {
                if !out.contains(&n.as_str()) {
                    out.push(n);
                }
            }
            Self::Unary(_, e) | Self::Group(e) => e.collect(out),
            Self::Binary(a, _, b) => {
                a.collect(out);
                b.collect(out);
            }
        }
    }

    /// Evaluates with `lookup` for names. Overflow and division by zero are
    /// errors.
    pub fn eval(&self, lookup: &dyn Fn(&str) -> Option<i64>) -> Result<i64, String> {
        match self {
            Self::Int(i) => Ok(*i),
            Self::Name(n) => lookup(n).ok_or_else(|| format!("unknown name `{n}`")),
            Self::Group(e) => e.eval(lookup),
            Self::Unary(op, e) => {
                let v = e.eval(lookup)?;
                Ok(match op {
                    UnOp::Neg => v.checked_neg().ok_or("overflow")?,
                    UnOp::Not => i64::from(v == 0),
                    UnOp::BitNot => !v,
                })
            }
            Self::Binary(a, op, b) => {
                let x = a.eval(lookup)?;
                // && and || short-circuit, so `a != 0 && 10 / a > 1` works.
                match op {
                    BinOp::And if x == 0 => return Ok(0),
                    BinOp::Or if x != 0 => return Ok(1),
                    _ => {}
                }
                let y = b.eval(lookup)?;
                let overflow = || format!("overflow in `{self}`");
                Ok(match op {
                    BinOp::Mul => x.checked_mul(y).ok_or_else(overflow)?,
                    BinOp::Div | BinOp::Rem if y == 0 => {
                        return Err(format!("division by zero in `{self}`"))
                    }
                    BinOp::Div => x.checked_div(y).ok_or_else(overflow)?,
                    BinOp::Rem => x.checked_rem(y).ok_or_else(overflow)?,
                    BinOp::Add => x.checked_add(y).ok_or_else(overflow)?,
                    BinOp::Sub => x.checked_sub(y).ok_or_else(overflow)?,
                    BinOp::Shl | BinOp::Shr if !(0..64).contains(&y) => {
                        return Err(format!("shift by {y} in `{self}`"))
                    }
                    BinOp::Shl => x << y,
                    BinOp::Shr => x >> y,
                    BinOp::Lt => i64::from(x < y),
                    BinOp::Le => i64::from(x <= y),
                    BinOp::Gt => i64::from(x > y),
                    BinOp::Ge => i64::from(x >= y),
                    BinOp::Eq => i64::from(x == y),
                    BinOp::Ne => i64::from(x != y),
                    BinOp::BitAnd => x & y,
                    BinOp::BitXor => x ^ y,
                    BinOp::BitOr => x | y,
                    BinOp::And | BinOp::Or => i64::from(y != 0),
                })
            }
        }
    }

    /// Solves `self == byte` for the one name not known to `lookup`, for the
    /// forms a reply may capture through (CDL §6): `x`, `x + k`, `k + x`,
    /// `x - k` and `k | x` / `x | k`, where `k` is known. For `|`, every bit
    /// of `k` must be set in `byte`, and `x` takes the remaining bits.
    pub fn solve(
        &self,
        byte: i64,
        lookup: &dyn Fn(&str) -> Option<i64>,
    ) -> Result<Option<(String, i64)>, String> {
        match self {
            Self::Name(n) => Ok(Some((n.clone(), byte))),
            Self::Group(e) => e.solve(byte, lookup),
            Self::Binary(a, op, b) => {
                let known = |e: &Expr| e.eval(lookup).ok();
                let (k, x, k_left) = match (known(a), known(b)) {
                    (Some(k), None) => (k, b, true),
                    (None, Some(k)) => (k, a, false),
                    _ => {
                        return Err(format!(
                            "`{self}` cannot capture: it needs one unknown name"
                        ))
                    }
                };
                let target = match op {
                    BinOp::Add => byte - k,
                    BinOp::Sub if !k_left => byte + k,
                    BinOp::BitOr if byte & k == k => byte & !k,
                    BinOp::BitOr => return Ok(None),
                    _ => {
                        return Err(format!(
                            "`{self}` cannot capture: use name, name + k, name - k or k | name"
                        ))
                    }
                };
                x.solve(target, lookup)
            }
            _ => Err(format!("`{self}` cannot capture a value")),
        }
    }

    /// The first pair of directly nested arithmetic and bitwise operators
    /// without parentheses between them.
    fn check_mixing(&self) -> Result<(), (&'static str, &'static str)> {
        match self {
            Self::Int(_) | Self::Name(_) => Ok(()),
            Self::Unary(_, e) | Self::Group(e) => e.check_mixing(),
            Self::Binary(a, op, b) => {
                let (sym, _, family) = op.info();
                for side in [a, b] {
                    if let Self::Binary(_, inner, _) = side.as_ref() {
                        let (isym, _, ifamily) = inner.info();
                        let mixed = matches!(
                            (family, ifamily),
                            (Family::Arith, Family::Bits) | (Family::Bits, Family::Arith)
                        );
                        if mixed {
                            return Err((isym, sym));
                        }
                    }
                }
                a.check_mixing()?;
                b.check_mixing()
            }
        }
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(i) => write!(f, "{i}"),
            Self::Name(n) => f.write_str(n),
            Self::Unary(op, e) => {
                let sym = match op {
                    UnOp::Neg => "-",
                    UnOp::Not => "!",
                    UnOp::BitNot => "~",
                };
                write!(f, "{sym}{e}")
            }
            Self::Binary(a, op, b) => write!(f, "{a} {} {b}", op.symbol()),
            Self::Group(e) => write!(f, "({e})"),
        }
    }
}

struct Parser<'s> {
    s: &'s str,
    pos: usize,
}

impl Parser<'_> {
    fn rest(&self) -> &str {
        &self.s[self.pos..]
    }

    fn skip_ws(&mut self) {
        let trimmed = self.rest().trim_start();
        self.pos = self.s.len() - trimmed.len();
    }

    fn binary(&mut self, min: u8) -> Result<Expr, String> {
        let mut left = self.unary()?;
        loop {
            self.skip_ws();
            let Some((sym, op)) = OPERATORS
                .iter()
                .find(|(sym, _)| self.rest().starts_with(sym))
            else {
                break;
            };
            let (_, prec, _) = op.info();
            if prec < min {
                break;
            }
            self.pos += sym.len();
            let right = self.binary(prec + 1)?;
            left = Expr::Binary(Box::new(left), *op, Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        self.skip_ws();
        let rest = self.rest();
        let op = match rest.chars().next() {
            Some('-') => Some(UnOp::Neg),
            Some('!') if !rest.starts_with("!=") => Some(UnOp::Not),
            Some('~') => Some(UnOp::BitNot),
            _ => None,
        };
        if let Some(op) = op {
            self.pos += 1;
            return Ok(Expr::Unary(op, Box::new(self.unary()?)));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<Expr, String> {
        self.skip_ws();
        let rest = self.rest();
        let Some(c) = rest.chars().next() else {
            return Err(format!("`{}`: expected a value at the end", self.s));
        };
        if c == '(' {
            self.pos += 1;
            let inner = self.binary(0)?;
            self.skip_ws();
            if !self.rest().starts_with(')') {
                return Err(format!("`{}`: missing `)`", self.s));
            }
            self.pos += 1;
            return Ok(Expr::Group(Box::new(inner)));
        }
        if c.is_ascii_digit() {
            let end = rest
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            let token = &rest[..end];
            let digits = token.replace('_', "");
            let value = match digits
                .strip_prefix("0x")
                .or_else(|| digits.strip_prefix("0X"))
            {
                Some(hex) => i64::from_str_radix(hex, 16),
                None => digits.parse(),
            }
            .map_err(|_| format!("`{}`: `{token}` is not a number", self.s))?;
            self.pos += end;
            return Ok(Expr::Int(value));
        }
        if c.is_ascii_alphabetic() || c == '_' || c == '$' {
            let len = name_len(rest);
            let name = rest[..len].to_string();
            self.pos += len;
            return Ok(Expr::Name(name));
        }
        Err(format!("`{}`: unexpected `{c}`", self.s))
    }
}

/// A name: letters, digits, `_`, `$` and `.`, with `-` allowed only between
/// two such characters, so `device-id` is a name and `v - 1` is not.
fn name_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b == b'.';
    let mut i = 0;
    while i < bytes.len() {
        let dash = bytes[i] == b'-'
            && i > 0
            && i + 1 < bytes.len()
            && word(bytes[i + 1])
            && !bytes[i + 1].is_ascii_digit();
        if !(word(bytes[i]) || dash) {
            break;
        }
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(s: &str) -> Result<i64, String> {
        Expr::parse(s)?.eval(&|n| match n {
            "s" => Some(2),
            "device-id" => Some(0x0F),
            "type" => Some(5),
            "image.crc" => Some(7),
            _ => None,
        })
    }

    #[test]
    fn evaluates_like_c() {
        assert_eq!(eval("0x20 + s"), Ok(0x22));
        assert_eq!(eval("0x40 | device-id"), Ok(0x4F));
        assert_eq!(eval("s * 6 + 6"), Ok(18));
        assert_eq!(eval("type == 3 || type >= 5"), Ok(1));
        assert_eq!(eval("type <= 1"), Ok(0));
        assert_eq!(eval("!(type == 5) && 1"), Ok(0));
        assert_eq!(eval("-s + 10"), Ok(8));
        assert_eq!(eval("image.crc == 7"), Ok(1));
        assert_eq!(eval("(s + 1) << 4"), Ok(48));
        assert_eq!(eval("s - 1"), Ok(1));
    }

    #[test]
    fn dashes_belong_to_names_only_between_letters() {
        assert_eq!(
            Expr::parse("device-id").unwrap(),
            Expr::Name("device-id".into())
        );
        assert_eq!(eval("s -1"), Ok(1));
        assert_eq!(Expr::parse("v-1").unwrap().names(), ["v"]);
    }

    #[test]
    fn rejects_mixed_families_without_parentheses() {
        let e = Expr::parse("a + b | c").unwrap_err();
        assert!(e.contains("parentheses"), "{e}");
        assert!(Expr::parse("(a + b) | c").is_ok());
        assert!(Expr::parse("a << 1 + b").is_err());
        assert!(Expr::parse("a & 1 == 1").is_ok());
    }

    #[test]
    fn errors() {
        assert!(eval("1 / (s - 2)")
            .unwrap_err()
            .contains("division by zero"));
        assert!(eval("nope + 1")
            .unwrap_err()
            .contains("unknown name `nope`"));
        assert!(Expr::parse("1 +").is_err());
        assert!(Expr::parse("(1").is_err());
        assert!(Expr::parse("1 2").is_err());
        assert!(Expr::parse("0x").is_err());
        assert!(eval("1 << 64").is_err());
    }

    #[test]
    fn short_circuit() {
        assert_eq!(eval("0 && 1 / 0"), Ok(0));
        assert_eq!(eval("1 || 1 / 0"), Ok(1));
    }

    #[test]
    fn solves_capture_forms() {
        let known = |n: &str| (n == "k").then_some(0x40);
        let solve = |s: &str, b: i64| Expr::parse(s).unwrap().solve(b, &known);
        assert_eq!(
            solve("0x40 | device-id", 0x4F),
            Ok(Some(("device-id".into(), 0x0F)))
        );
        assert_eq!(
            solve("device-id | 0x40", 0x4F),
            Ok(Some(("device-id".into(), 0x0F)))
        );
        assert_eq!(solve("0x40 | device-id", 0x3F), Ok(None));
        assert_eq!(solve("0x10 + s", 0x13), Ok(Some(("s".into(), 3))));
        assert_eq!(solve("s - 1", 3), Ok(Some(("s".into(), 4))));
        assert_eq!(solve("k | x", 0x45), Ok(Some(("x".into(), 5))));
        assert!(solve("s * 2", 4).is_err());
        assert!(solve("a + b", 4).is_err());
    }
}
