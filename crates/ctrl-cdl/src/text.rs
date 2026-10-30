//! Texts with `{expressions}` (gate texts, guide steps): `SE{setup + 1}`,
//! and with a zero-padded width, `P{p + 1:02}`.

/// The `{…}` parts of `t`: the expression and its width.
pub fn exprs(t: &str) -> Vec<(&str, Option<usize>)> {
    let mut out = Vec::new();
    let mut rest = t;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        out.push(split_width(&rest[open + 1..open + close]));
        rest = &rest[open + close + 1..];
    }
    out
}

/// `p + 1:02` into `p + 1` and 2.
fn split_width(inner: &str) -> (&str, Option<usize>) {
    match inner.rsplit_once(':') {
        Some((e, w)) if w.starts_with('0') && w.len() > 1 => (e, w[1..].parse().ok()),
        _ => (inner, None),
    }
}

/// `t` with each `{…}` replaced by its value; a part that does not
/// evaluate stays as written.
pub fn interpolate(t: &str, eval: &dyn Fn(&str) -> Result<i64, String>) -> String {
    let mut out = String::new();
    let mut rest = t;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        out.push_str(&rest[..open]);
        let (e, width) = split_width(&rest[open + 1..open + close]);
        match eval(e) {
            Ok(v) => out.push_str(&format!("{v:0w$}", w = width.unwrap_or(0))),
            Err(_) => out.push_str(&rest[open..=open + close]),
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_pad_with_zeros() {
        let eval = |e: &str| crate::Expr::parse(e)?.eval(&|n| (n == "p").then_some(2));
        assert_eq!(interpolate("P{p + 1:02} SE{p + 1}", &eval), "P03 SE3");
        assert_eq!(interpolate("{q}", &eval), "{q}");
        assert_eq!(exprs("a {x:03} b {y}"), vec![("x", Some(3)), ("y", None)]);
    }
}
