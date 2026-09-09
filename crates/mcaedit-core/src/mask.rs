//! Block masks for brush / fill / replace.
//!
//! Grammar (no spaces required):
//! - `air` / `minecraft:air` — any air-like
//! - `!air` — not air-like
//! - `#solid` — not air-like (alias)
//! - `#existing` — not air-like
//! - exact block: `minecraft:stone` or `stone`
//! - OR: `a|b|c`
//! - AND: `a&b`
//! - NOT: `!expr` (binds tighter than `|`, looser than `&` for simple forms)
//! - include list: `stone,dirt` (= OR)
//!
//! Also: `--mask-exclude` is applied as `base & !(e1|e2|…)`.

use crate::blockstate::BlockState;
use crate::ops::matches_filter;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mask {
    /// Always true.
    Any,
    AirLike,
    Solid,
    Exact(BlockState),
    Not(Box<Mask>),
    And(Box<Mask>, Box<Mask>),
    Or(Box<Mask>, Box<Mask>),
}

impl Mask {
    pub fn any() -> Self {
        Self::Any
    }

    pub fn matches(&self, block: &BlockState) -> bool {
        match self {
            Self::Any => true,
            Self::AirLike => block.is_air_like(),
            Self::Solid => !block.is_air_like(),
            Self::Exact(f) => matches_filter(block, f),
            Self::Not(inner) => !inner.matches(block),
            Self::And(a, b) => a.matches(block) && b.matches(block),
            Self::Or(a, b) => a.matches(block) || b.matches(block),
        }
    }

    pub fn parse(input: &str) -> Result<Self, String> {
        let s = input.trim();
        if s.is_empty() {
            return Ok(Self::Any);
        }
        parse_or(s)
    }

    /// Combine optional include mask with exclude list (`e1|e2|…`).
    pub fn with_exclude(base: Option<Self>, exclude: Option<&str>) -> Result<Self, String> {
        let base = base.unwrap_or(Self::Any);
        let Some(ex) = exclude.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(base);
        };
        let excl = Self::parse(ex)?;
        Ok(Self::And(Box::new(base), Box::new(Self::Not(Box::new(excl)))))
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Any => "*".into(),
            Self::AirLike => "air".into(),
            Self::Solid => "#solid".into(),
            Self::Exact(b) => b.to_compact(),
            Self::Not(inner) => format!("!{}", inner.describe()),
            Self::And(a, b) => format!("({}&{})", a.describe(), b.describe()),
            Self::Or(a, b) => format!("({}|{})", a.describe(), b.describe()),
        }
    }
}

fn parse_or(s: &str) -> Result<Mask, String> {
    let parts = split_top(s, '|');
    if parts.len() == 1 {
        return parse_and(parts[0]);
    }
    let mut acc = parse_and(parts[0])?;
    for p in &parts[1..] {
        acc = Mask::Or(Box::new(acc), Box::new(parse_and(p)?));
    }
    Ok(acc)
}

fn parse_and(s: &str) -> Result<Mask, String> {
    let parts = split_top(s, '&');
    if parts.len() == 1 {
        // comma also means OR at leaf lists without | — handle after unary
        return parse_unary(parts[0]);
    }
    let mut acc = parse_unary(parts[0])?;
    for p in &parts[1..] {
        acc = Mask::And(Box::new(acc), Box::new(parse_unary(p)?));
    }
    Ok(acc)
}

fn parse_unary(s: &str) -> Result<Mask, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty mask atom".into());
    }
    if let Some(rest) = s.strip_prefix('!') {
        return Ok(Mask::Not(Box::new(parse_unary(rest)?)));
    }
    if s.contains(',') && !s.contains('|') && !s.contains('&') {
        let mut parts = s.split(',').map(str::trim).filter(|p| !p.is_empty());
        let first = parts.next().ok_or_else(|| "empty mask list".to_string())?;
        let mut acc = parse_atom(first)?;
        for p in parts {
            acc = Mask::Or(Box::new(acc), Box::new(parse_atom(p)?));
        }
        return Ok(acc);
    }
    parse_atom(s)
}

fn parse_atom(s: &str) -> Result<Mask, String> {
    let s = s.trim();
    if s == "*" || s.eq_ignore_ascii_case("any") {
        return Ok(Mask::Any);
    }
    if s == "#solid" || s == "#existing" {
        return Ok(Mask::Solid);
    }
    if s == "air" || s == "minecraft:air" {
        return Ok(Mask::AirLike);
    }
    let with_ns = if s.contains(':') || s.starts_with('#') {
        s.to_string()
    } else {
        format!("minecraft:{s}")
    };
    let block = BlockState::parse(&with_ns)?;
    if block.is_air_like() {
        Ok(Mask::AirLike)
    } else {
        Ok(Mask::Exact(block))
    }
}

fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn air_and_not() {
        let m = Mask::parse("!air").unwrap();
        assert!(!m.matches(&BlockState::air()));
        assert!(m.matches(&BlockState::parse("minecraft:stone").unwrap()));
    }

    #[test]
    fn or_list() {
        let m = Mask::parse("stone,dirt").unwrap();
        assert!(m.matches(&BlockState::parse("minecraft:stone").unwrap()));
        assert!(m.matches(&BlockState::parse("minecraft:dirt").unwrap()));
        assert!(!m.matches(&BlockState::parse("minecraft:glass").unwrap()));
    }

    #[test]
    fn exclude_compose() {
        let m = Mask::with_exclude(Some(Mask::Solid), Some("bedrock")).unwrap();
        assert!(m.matches(&BlockState::parse("minecraft:stone").unwrap()));
        assert!(!m.matches(&BlockState::parse("minecraft:bedrock").unwrap()));
        assert!(!m.matches(&BlockState::air()));
    }
}
