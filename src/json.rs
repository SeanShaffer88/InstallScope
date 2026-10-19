//! JSON without a library: a strict RFC 8259 reader and a writer whose
//! compact and pretty layouts match what earlier versions of InstallScope
//! produced, so every saved log, setting, backup and package stays
//! readable and diffs cleanly.
//!
//! Objects keep their keys in insertion order (the order fields are
//! written), and reading rejects duplicate keys, invalid escapes, lone
//! surrogates, trailing garbage and nesting deeper than [`MAX_DEPTH`].

use std::fmt::Write as _;

/// Deepest nesting accepted when reading (guards the stack).
pub const MAX_DEPTH: usize = 256;

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    /// Integers that fit keep their exact value.
    Int(i64),
    /// Integers too large for `i64`.
    UInt(u64),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    /// Keys in order.
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Value::Int(i) => Some(i),
            Value::UInt(u) => i64::try_from(u).ok(),
            Value::Float(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => Some(f as i64),
            _ => None,
        }
    }
    pub fn as_u64(&self) -> Option<u64> {
        match *self {
            Value::Int(i) => u64::try_from(i).ok(),
            Value::UInt(u) => Some(u),
            Value::Float(f) if f.fract() == 0.0 && (0.0..9.0e15).contains(&f) => Some(f as u64),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Value::Int(i) => Some(i as f64),
            Value::UInt(u) => Some(u as f64),
            Value::Float(f) => Some(f),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

impl Value {
    /// Set `key` on an object (replacing it, or adding it at the end).
    /// Does nothing to a value that isn't an object.
    pub fn set(&mut self, key: &str, v: Value) {
        if let Value::Object(m) = self {
            match m.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = v,
                None => m.push((key.to_string(), v)),
            }
        }
    }
}

impl std::ops::Index<&str> for Value {
    type Output = Value;
    /// `v["key"]`: [`Value::Null`] when absent, as in JavaScript.
    fn index(&self, key: &str) -> &Value {
        static NULL: Value = Value::Null;
        self.get(key).unwrap_or(&NULL)
    }
}

impl std::ops::Index<usize> for Value {
    type Output = Value;
    fn index(&self, i: usize) -> &Value {
        static NULL: Value = Value::Null;
        match self {
            Value::Array(a) => a.get(i).unwrap_or(&NULL),
            _ => &NULL,
        }
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub msg: String,
    /// Byte offset in the input.
    pub at: usize,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at byte {}", self.msg, self.at)
    }
}

impl std::error::Error for Error {}

struct Parser<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Parser<'a> {
    fn err<T>(&self, msg: &str) -> Result<T, Error> {
        Err(Error {
            msg: msg.to_string(),
            at: self.at,
        })
    }

    fn ws(&mut self) {
        while let Some(&c) = self.b.get(self.at) {
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
                self.at += 1;
            } else {
                break;
            }
        }
    }

    fn eat(&mut self, lit: &[u8]) -> bool {
        if self.b[self.at..].starts_with(lit) {
            self.at += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return self.err("nested too deeply");
        }
        self.ws();
        match self.b.get(self.at) {
            None => self.err("unexpected end"),
            Some(b'n') if self.eat(b"null") => Ok(Value::Null),
            Some(b't') if self.eat(b"true") => Ok(Value::Bool(true)),
            Some(b'f') if self.eat(b"false") => Ok(Value::Bool(false)),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b'[') => {
                self.at += 1;
                let mut a = Vec::new();
                self.ws();
                if self.eat(b"]") {
                    return Ok(Value::Array(a));
                }
                loop {
                    a.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(b",") {
                        continue;
                    }
                    if self.eat(b"]") {
                        return Ok(Value::Array(a));
                    }
                    return self.err("expected , or ]");
                }
            }
            Some(b'{') => {
                self.at += 1;
                let mut m: Vec<(String, Value)> = Vec::new();
                self.ws();
                if self.eat(b"}") {
                    return Ok(Value::Object(m));
                }
                loop {
                    self.ws();
                    if self.b.get(self.at) != Some(&b'"') {
                        return self.err("expected a key");
                    }
                    let key_at = self.at;
                    let k = self.string()?;
                    if m.iter().any(|(x, _)| *x == k) {
                        return Err(Error {
                            msg: format!("duplicate key {k:?}"),
                            at: key_at,
                        });
                    }
                    self.ws();
                    if !self.eat(b":") {
                        return self.err("expected :");
                    }
                    let v = self.value(depth + 1)?;
                    m.push((k, v));
                    self.ws();
                    if self.eat(b",") {
                        continue;
                    }
                    if self.eat(b"}") {
                        return Ok(Value::Object(m));
                    }
                    return self.err("expected , or }");
                }
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            Some(_) => self.err("unexpected character"),
        }
    }

    fn number(&mut self) -> Result<Value, Error> {
        let start = self.at;
        let digits = |p: &mut Self| {
            let s = p.at;
            while p.b.get(p.at).is_some_and(u8::is_ascii_digit) {
                p.at += 1;
            }
            p.at - s
        };
        self.eat(b"-");
        let int_start = self.at;
        let n = digits(self);
        if n == 0 || (n > 1 && self.b[int_start] == b'0') {
            return self.err("bad number");
        }
        let mut float = false;
        if self.eat(b".") {
            float = true;
            if digits(self) == 0 {
                return self.err("bad number");
            }
        }
        if matches!(self.b.get(self.at), Some(b'e' | b'E')) {
            float = true;
            self.at += 1;
            if matches!(self.b.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if digits(self) == 0 {
                return self.err("bad number");
            }
        }
        let text = std::str::from_utf8(&self.b[start..self.at]).unwrap();
        if !float {
            if let Ok(i) = text.parse::<i64>() {
                return Ok(Value::Int(i));
            }
            if let Ok(u) = text.parse::<u64>() {
                return Ok(Value::UInt(u));
            }
        }
        match text.parse::<f64>() {
            Ok(f) if f.is_finite() => Ok(Value::Float(f)),
            _ => self.err("number out of range"),
        }
    }

    fn hex4(&mut self) -> Result<u32, Error> {
        let Some(h) = self.b.get(self.at..self.at + 4) else {
            return self.err("bad \\u escape");
        };
        let s = std::str::from_utf8(h).map_err(|_| Error {
            msg: "bad \\u escape".into(),
            at: self.at,
        })?;
        let v = u32::from_str_radix(s, 16).map_err(|_| Error {
            msg: "bad \\u escape".into(),
            at: self.at,
        })?;
        self.at += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, Error> {
        self.at += 1; // opening quote
        let mut out = String::new();
        loop {
            // Copy the run up to the next quote, backslash or control char.
            let run_start = self.at;
            while let Some(&c) = self.b.get(self.at) {
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.at += 1;
            }
            match std::str::from_utf8(&self.b[run_start..self.at]) {
                Ok(s) => out.push_str(s),
                Err(_) => return self.err("invalid UTF-8"),
            }
            match self.b.get(self.at) {
                None => return self.err("unterminated string"),
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let Some(&e) = self.b.get(self.at) else {
                        return self.err("unterminated string");
                    };
                    self.at += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let c = if (0xD800..0xDC00).contains(&hi) {
                                if !self.eat(b"\\u") {
                                    return self.err("lone surrogate");
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return self.err("lone surrogate");
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return self.err("lone surrogate");
                            } else {
                                hi
                            };
                            out.push(char::from_u32(c).unwrap());
                        }
                        _ => return self.err("bad escape"),
                    }
                }
                Some(_) => return self.err("control character in string"),
            }
        }
    }
}

/// Read a JSON document (one value, surrounded only by whitespace).
pub fn parse(text: &str) -> Result<Value, Error> {
    parse_bytes(text.as_bytes())
}

pub fn parse_bytes(b: &[u8]) -> Result<Value, Error> {
    let b = b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(b);
    let mut p = Parser { b, at: 0 };
    let v = p.value(0)?;
    p.ws();
    if p.at != b.len() {
        return p.err("unexpected text after the value");
    }
    Ok(v)
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

fn write_str(out: &mut String, s: &str) {
    out.push('"');
    let mut start = 0;
    for (i, c) in s.char_indices() {
        let esc = match c {
            '"' => "\\\"",
            '\\' => "\\\\",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            '\u{8}' => "\\b",
            '\u{c}' => "\\f",
            c if (c as u32) < 0x20 => "",
            _ => continue,
        };
        out.push_str(&s[start..i]);
        if esc.is_empty() {
            let _ = write!(out, "\\u{:04x}", c as u32);
        } else {
            out.push_str(esc);
        }
        start = i + c.len_utf8();
    }
    out.push_str(&s[start..]);
    out.push('"');
}

/// Floats in ryū's layout (what serde_json writes): shortest digits that
/// read back exactly, plain notation for decimal exponents from -5 to 16,
/// scientific beyond (`1e-7`, `1e+16`).
fn write_f64(out: &mut String, f: f64) {
    if !f.is_finite() {
        out.push_str("null");
        return;
    }
    if f == 0.0 {
        out.push_str(if f.is_sign_negative() { "-0.0" } else { "0.0" });
        return;
    }
    // `{:e}` gives the shortest round-trip digits: "-1.2345e-7".
    let sci = format!("{f:e}");
    let (mant, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let len = digits.len() as i32;
    let kk = exp + 1; // position of the decimal point
    let k = kk - len; // exponent of the last digit
    if neg {
        out.push('-');
    }
    if (0..=16).contains(&kk) && k >= 0 {
        out.push_str(&digits);
        for _ in 0..k {
            out.push('0');
        }
        out.push_str(".0");
    } else if 0 < kk && kk <= 16 {
        out.push_str(&digits[..kk as usize]);
        out.push('.');
        out.push_str(&digits[kk as usize..]);
    } else if -5 < kk && kk <= 0 {
        out.push_str("0.");
        for _ in 0..-kk {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        let e = kk - 1;
        let sign = if e >= 0 { "+" } else { "" };
        if len == 1 {
            let _ = write!(out, "{digits}e{sign}{e}");
        } else {
            let _ = write!(out, "{}.{}e{sign}{e}", &digits[..1], &digits[1..]);
        }
    }
}

fn write_value(out: &mut String, v: &Value, indent: Option<usize>) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Value::UInt(u) => {
            let _ = write!(out, "{u}");
        }
        Value::Float(f) => write_f64(out, *f),
        Value::String(s) => write_str(out, s),
        Value::Array(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if let Some(n) = indent {
                    newline(out, n + 1);
                }
                write_value(out, x, indent.map(|n| n + 1));
            }
            if let Some(n) = indent {
                newline(out, n);
            }
            out.push(']');
        }
        Value::Object(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if let Some(n) = indent {
                    newline(out, n + 1);
                }
                write_str(out, k);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write_value(out, x, indent.map(|n| n + 1));
            }
            if let Some(n) = indent {
                newline(out, n);
            }
            out.push('}');
        }
    }
}

fn newline(out: &mut String, depth: usize) {
    out.push('\n');
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// One line, no spaces.
pub fn to_string(v: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, v, None);
    out
}

/// Two-space indentation, `"key": value`.
pub fn to_string_pretty(v: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, v, Some(0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_kind_of_value() {
        let v = parse(
            r#" {"a": [1, -2, 3.5, 1e3, 18446744073709551615, true, false, null],
            "s": "q\"\\\/\b\f\n\r\t\u00e9\ud83d\ude00", "o": {}, "e": []} "#,
        )
        .unwrap();
        assert_eq!(v["a"][0], Value::Int(1));
        assert_eq!(v["a"][1], Value::Int(-2));
        assert_eq!(v["a"][2], Value::Float(3.5));
        assert_eq!(v["a"][3], Value::Float(1000.0));
        assert_eq!(v["a"][4], Value::UInt(u64::MAX));
        assert_eq!(v["a"][5], Value::Bool(true));
        assert!(v["a"][7].is_null());
        assert_eq!(v["s"].as_str().unwrap(), "q\"\\/\u{8}\u{c}\n\r\té😀");
        assert_eq!(v["o"], Value::Object(vec![]));
        assert!(v["missing"].is_null() && v["a"][99].is_null());
    }

    #[test]
    fn rejects_what_isnt_json() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "01",
            "1.",
            ".5",
            "-",
            "1e",
            "+1",
            "\"\\x\"",
            "\"\\ud83d\"",
            "\"\\ude00\"",
            "\"a\nb\"",
            "{\"a\":1}{",
            "{a:1}",
            "{\"a\":1,\"a\":2}",
            "nul",
            "[1 2]",
            "\"\u{0}\"",
            "1e999",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(MAX_DEPTH + 2) + &"]".repeat(MAX_DEPTH + 2);
        assert!(parse(&deep).is_err());
        let ok = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(parse(&ok).is_ok());
        assert!(parse_bytes(b"\"\xff\"").is_err(), "invalid UTF-8");
        assert!(parse_bytes(b"\xEF\xBB\xBF{}").is_ok(), "byte-order mark");
    }

    #[test]
    fn writes_like_earlier_versions() {
        let v = Value::Object(vec![
            ("name".into(), Value::String("A \"b\"\n\u{1}".into())),
            ("n".into(), Value::Int(-3)),
            ("f".into(), Value::Float(2.0)),
            ("g".into(), Value::Float(0.25)),
            (
                "list".into(),
                Value::Array(vec![Value::Bool(true), Value::Null]),
            ),
            ("empty".into(), Value::Array(vec![])),
            ("obj".into(), Value::Object(vec![])),
        ]);
        assert_eq!(
            to_string(&v),
            r#"{"name":"A \"b\"\n\u0001","n":-3,"f":2.0,"g":0.25,"list":[true,null],"empty":[],"obj":{}}"#
        );
        assert_eq!(
            to_string_pretty(&v),
            "{\n  \"name\": \"A \\\"b\\\"\\n\\u0001\",\n  \"n\": -3,\n  \"f\": 2.0,\n  \"g\": 0.25,\n  \"list\": [\n    true,\n    null\n  ],\n  \"empty\": [],\n  \"obj\": {}\n}"
        );
    }

    /// A document using every feature, keys sorted (as serde_json wrote them).
    const SAMPLE: &str = r#"{"a":[[],{},[1,[2]],{"k":null}],"big":18446744073709551615,"created":"2026-10-16 05:07:09",
            "f":1.5,"id":"x1","n":42,"neg":-7,"s":"tab\there \u00e9 \ud83d\ude00 \u0007","t":true,"tiny":1e-7,"whole":3.0}"#;

    /// Floats around every layout boundary.
    const FLOATS: &[f64] = &[
        0.0,
        -0.0,
        1.0,
        3.0,
        0.5,
        0.1,
        1e-7,
        1.5e-7,
        0.001,
        0.0001,
        0.00001,
        123456.789,
        1e15,
        1e16,
        1e17,
        1.2345e20,
        9007199254740993.0,
        f64::MIN_POSITIVE,
        f64::MAX,
        -2.5e-300,
        std::f64::consts::PI,
    ];

    #[test]
    fn matches_what_earlier_versions_wrote_byte_for_byte() {
        // The golden files hold serde_json's output for the same input.
        let ours = parse(SAMPLE).unwrap();
        assert_eq!(to_string(&ours), include_str!("testdata/json/sample.json"));
        assert_eq!(
            to_string_pretty(&ours),
            include_str!("testdata/json/sample.pretty.json")
        );
        assert_eq!(
            parse(include_str!("testdata/json/sample.pretty.json")).unwrap(),
            ours
        );
        let golden: Vec<&str> = include_str!("testdata/json/floats.txt").lines().collect();
        assert_eq!(golden.len(), FLOATS.len());
        for (&f, want) in FLOATS.iter().zip(golden) {
            let mut got = String::new();
            write_f64(&mut got, f);
            assert_eq!(got, want, "{f:e}");
            assert_eq!(
                parse(&got).unwrap().as_f64().unwrap().to_bits(),
                f.to_bits(),
                "{got}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Typed conversion
// ---------------------------------------------------------------------------

/// A value that can be written as JSON.
pub trait ToJson {
    fn to_json(&self) -> Value;
    /// Whether a `skip_empty` field leaves this out.
    fn is_empty_json(&self) -> bool {
        false
    }
}

/// A value that can be read from JSON.
pub trait FromJson: Sized {
    fn from_json(v: &Value) -> Result<Self, Error>;
    /// The value of a field that is absent: `None` means it is required.
    fn missing() -> Option<Self> {
        None
    }
}

pub fn type_error<T>(what: &str) -> Result<T, Error> {
    Err(Error {
        msg: format!("expected {what}"),
        at: 0,
    })
}

/// Add the field name to an error's message ("in `changes.keys_added`").
pub fn in_field(e: Error, field: &str) -> Error {
    Error {
        msg: if e.msg.contains(" in `") {
            e.msg.replacen(" in `", &format!(" in `{field}."), 1)
        } else {
            format!("{} in `{field}`", e.msg)
        },
        at: e.at,
    }
}

impl<T: ToJson + ?Sized> ToJson for &T {
    fn to_json(&self) -> Value {
        (**self).to_json()
    }
    fn is_empty_json(&self) -> bool {
        (**self).is_empty_json()
    }
}

impl ToJson for Value {
    fn to_json(&self) -> Value {
        self.clone()
    }
}
impl FromJson for Value {
    fn from_json(v: &Value) -> Result<Self, Error> {
        Ok(v.clone())
    }
}

impl ToJson for String {
    fn to_json(&self) -> Value {
        Value::String(self.clone())
    }
    fn is_empty_json(&self) -> bool {
        self.is_empty()
    }
}
impl ToJson for str {
    fn to_json(&self) -> Value {
        Value::String(self.to_string())
    }
}
impl FromJson for String {
    fn from_json(v: &Value) -> Result<Self, Error> {
        v.as_str()
            .map(str::to_string)
            .map_or_else(|| type_error("a string"), Ok)
    }
}

impl ToJson for bool {
    fn to_json(&self) -> Value {
        Value::Bool(*self)
    }
}
impl FromJson for bool {
    fn from_json(v: &Value) -> Result<Self, Error> {
        v.as_bool().map_or_else(|| type_error("true or false"), Ok)
    }
}

macro_rules! json_int {
    ($($t:ty),*) => {$(
        impl ToJson for $t {
            fn to_json(&self) -> Value {
                match i64::try_from(*self) {
                    Ok(i) => Value::Int(i),
                    Err(_) => Value::UInt(*self as u64),
                }
            }
        }
        impl FromJson for $t {
            fn from_json(v: &Value) -> Result<Self, Error> {
                let n = match *v {
                    Value::Int(i) => <$t>::try_from(i).ok(),
                    Value::UInt(u) => <$t>::try_from(u).ok(),
                    _ => None,
                };
                n.map_or_else(|| type_error(concat!("a whole number (", stringify!($t), ")")), Ok)
            }
        }
    )*};
}
json_int!(u8, u16, u32, u64, usize, i8, i16, i32, i64);

impl ToJson for f64 {
    fn to_json(&self) -> Value {
        Value::Float(*self)
    }
}
impl FromJson for f64 {
    fn from_json(v: &Value) -> Result<Self, Error> {
        v.as_f64().map_or_else(|| type_error("a number"), Ok)
    }
}
impl ToJson for f32 {
    /// Written with the shortest digits that read back as this `f32`
    /// (`0.1`, not its 64-bit widening), as before.
    fn to_json(&self) -> Value {
        if !self.is_finite() {
            return Value::Null;
        }
        let short: f64 = format!("{self:e}").parse().unwrap_or(*self as f64);
        Value::Float(short)
    }
}
impl FromJson for f32 {
    fn from_json(v: &Value) -> Result<Self, Error> {
        v.as_f64()
            .map(|f| f as f32)
            .map_or_else(|| type_error("a number"), Ok)
    }
}

impl<T: ToJson> ToJson for Option<T> {
    fn to_json(&self) -> Value {
        match self {
            Some(x) => x.to_json(),
            None => Value::Null,
        }
    }
    fn is_empty_json(&self) -> bool {
        self.is_none()
    }
}
impl<T: FromJson> FromJson for Option<T> {
    fn from_json(v: &Value) -> Result<Self, Error> {
        if v.is_null() {
            Ok(None)
        } else {
            T::from_json(v).map(Some)
        }
    }
    fn missing() -> Option<Self> {
        Some(None)
    }
}

impl<T: ToJson> ToJson for Box<T> {
    fn to_json(&self) -> Value {
        (**self).to_json()
    }
}
impl<T: FromJson> FromJson for Box<T> {
    fn from_json(v: &Value) -> Result<Self, Error> {
        T::from_json(v).map(Box::new)
    }
}

impl<T: ToJson> ToJson for Vec<T> {
    fn to_json(&self) -> Value {
        Value::Array(self.iter().map(ToJson::to_json).collect())
    }
    fn is_empty_json(&self) -> bool {
        self.is_empty()
    }
}
impl<T: ToJson> ToJson for [T] {
    fn to_json(&self) -> Value {
        Value::Array(self.iter().map(ToJson::to_json).collect())
    }
}
impl<T: FromJson> FromJson for Vec<T> {
    fn from_json(v: &Value) -> Result<Self, Error> {
        let Some(a) = v.as_array() else {
            return type_error("a list");
        };
        a.iter()
            .enumerate()
            .map(|(i, x)| T::from_json(x).map_err(|e| in_field(e, &format!("[{i}]"))))
            .collect()
    }
}

impl<A: ToJson, B: ToJson> ToJson for (A, B) {
    fn to_json(&self) -> Value {
        Value::Array(vec![self.0.to_json(), self.1.to_json()])
    }
}
impl<A: FromJson, B: FromJson> FromJson for (A, B) {
    fn from_json(v: &Value) -> Result<Self, Error> {
        match v.as_array() {
            Some([a, b]) => Ok((A::from_json(a)?, B::from_json(b)?)),
            _ => type_error("a pair"),
        }
    }
}
impl<A: ToJson, B: ToJson, C: ToJson> ToJson for (A, B, C) {
    fn to_json(&self) -> Value {
        Value::Array(vec![self.0.to_json(), self.1.to_json(), self.2.to_json()])
    }
}
impl<A: FromJson, B: FromJson, C: FromJson> FromJson for (A, B, C) {
    fn from_json(v: &Value) -> Result<Self, Error> {
        match v.as_array() {
            Some([a, b, c]) => Ok((A::from_json(a)?, B::from_json(b)?, C::from_json(c)?)),
            _ => type_error("three values"),
        }
    }
}

impl<T: ToJson> ToJson for std::collections::BTreeMap<String, T> {
    fn to_json(&self) -> Value {
        Value::Object(self.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
    }
    fn is_empty_json(&self) -> bool {
        self.is_empty()
    }
}
impl<T: FromJson> FromJson for std::collections::BTreeMap<String, T> {
    fn from_json(v: &Value) -> Result<Self, Error> {
        let Some(m) = v.as_object() else {
            return type_error("an object");
        };
        m.iter()
            .map(|(k, x)| {
                T::from_json(x)
                    .map(|t| (k.clone(), t))
                    .map_err(|e| in_field(e, k))
            })
            .collect()
    }
}
impl<T: ToJson> ToJson for std::collections::HashMap<String, T> {
    /// Keys sorted, so the file is the same every time.
    fn to_json(&self) -> Value {
        let mut keys: Vec<&String> = self.keys().collect();
        keys.sort();
        Value::Object(
            keys.into_iter()
                .map(|k| (k.clone(), self[k].to_json()))
                .collect(),
        )
    }
    fn is_empty_json(&self) -> bool {
        self.is_empty()
    }
}
impl<T: FromJson> FromJson for std::collections::HashMap<String, T> {
    fn from_json(v: &Value) -> Result<Self, Error> {
        let Some(m) = v.as_object() else {
            return type_error("an object");
        };
        m.iter()
            .map(|(k, x)| {
                T::from_json(x)
                    .map(|t| (k.clone(), t))
                    .map_err(|e| in_field(e, k))
            })
            .collect()
    }
}
impl ToJson for std::collections::BTreeSet<String> {
    fn to_json(&self) -> Value {
        Value::Array(self.iter().map(|s| Value::String(s.clone())).collect())
    }
    fn is_empty_json(&self) -> bool {
        self.is_empty()
    }
}
impl FromJson for std::collections::BTreeSet<String> {
    fn from_json(v: &Value) -> Result<Self, Error> {
        Vec::<String>::from_json(v).map(|v| v.into_iter().collect())
    }
}

/// Bytes as lower-case hex text (how registry data is stored).
pub fn hex_to_json(b: &[u8]) -> Value {
    Value::String(crate::model::hexbytes::to_hex(b))
}

pub fn hex_from_json(v: &Value) -> Result<Vec<u8>, Error> {
    let s = v.as_str().map_or_else(|| type_error("hex text"), Ok)?;
    crate::model::hexbytes::from_hex(s).map_err(|e| Error {
        msg: e.to_string(),
        at: 0,
    })
}

/// Look a field up in an object's entries.
pub fn field<'a>(obj: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
    obj.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

/// Write any value as one-line JSON.
pub fn to_json_string<T: ToJson + ?Sized>(v: &T) -> String {
    to_string(&v.to_json())
}

/// Write any value as indented JSON.
pub fn to_json_string_pretty<T: ToJson + ?Sized>(v: &T) -> String {
    to_string_pretty(&v.to_json())
}

/// Read any value from JSON text.
pub fn from_json_str<T: FromJson>(text: &str) -> Result<T, Error> {
    T::from_json(&parse(text)?)
}

pub fn from_json_slice<T: FromJson>(b: &[u8]) -> Result<T, Error> {
    T::from_json(&parse_bytes(b)?)
}

/// `ToJson`/`FromJson` for a struct, field by field, in the order written:
///
/// ```ignore
/// json_struct!(Settings, default, { language, zoom, tags [skip_empty] });
/// json_struct!(RegValue, { name, kind, data [hex] });
/// ```
///
/// `default` takes absent fields from `Type::default()`; without it absent
/// fields are an error (except `Option`s, which become `None`). Per field:
/// `write_only` implements only writing. `[default]` (absent: the type's
/// default), `[or = path]` (absent: the
/// function's result), `[skip_empty]` (not written when empty, absent:
/// default) and `[hex]` (bytes as hex text). Unknown fields are ignored.
#[macro_export]
macro_rules! json_struct {
    ($ty:ident, default, { $($f:ident $([$($m:tt)*])?),* $(,)? }) => {
        $crate::json_struct!(@impl $ty, base, __base, { $($f $([$($m)*])?),* });
    };
    ($ty:ident, { $($f:ident $([$($m:tt)*])?),* $(,)? }) => {
        $crate::json_struct!(@impl $ty, strict, __base, { $($f $([$($m)*])?),* });
    };
    // Only written, never read (fields borrowed from static tables).
    ($ty:ident, write_only, { $($f:ident $([$($m:tt)*])?),* $(,)? }) => {
        impl $crate::json::ToJson for $ty {
            #[allow(clippy::vec_init_then_push)]
            fn to_json(&self) -> $crate::json::Value {
                let mut __o: Vec<(String, $crate::json::Value)> = Vec::new();
                $( $crate::json_struct!(@put __o, self, $f $(, $($m)*)?); )*
                $crate::json::Value::Object(__o)
            }
        }
    };
    (@impl $ty:ident, $mode:ident, $bind:ident, { $($f:ident $([$($m:tt)*])?),* }) => {
        impl $crate::json::ToJson for $ty {
            #[allow(clippy::vec_init_then_push)]
            fn to_json(&self) -> $crate::json::Value {
                let mut __o: Vec<(String, $crate::json::Value)> = Vec::new();
                $( $crate::json_struct!(@put __o, self, $f $(, $($m)*)?); )*
                $crate::json::Value::Object(__o)
            }
        }
        impl $crate::json::FromJson for $ty {
            #[allow(unused_variables, unused_mut)]
            fn from_json(__v: &$crate::json::Value) -> Result<Self, $crate::json::Error> {
                let Some(__obj) = __v.as_object() else {
                    return $crate::json::type_error(concat!("an object (", stringify!($ty), ")"));
                };
                $crate::json_struct!(@bind $mode, $bind, $ty);
                Ok($ty {
                    $( $f: $crate::json_struct!(@get __obj, ($mode $bind), $f $(, $($m)*)?), )*
                })
            }
        }
    };
    // Writing.
    (@put $o:ident, $s:ident, $f:ident) => {
        $o.push((stringify!($f).to_string(), $crate::json::ToJson::to_json(&$s.$f)));
    };
    (@put $o:ident, $s:ident, $f:ident, default) => { $crate::json_struct!(@put $o, $s, $f); };
    (@put $o:ident, $s:ident, $f:ident, or = $d:path) => { $crate::json_struct!(@put $o, $s, $f); };
    (@put $o:ident, $s:ident, $f:ident, skip_empty) => {
        if !$crate::json::ToJson::is_empty_json(&$s.$f) {
            $crate::json_struct!(@put $o, $s, $f);
        }
    };
    (@put $o:ident, $s:ident, $f:ident, hex) => {
        $o.push((stringify!($f).to_string(), $crate::json::hex_to_json(&$s.$f)));
    };
    // Reading.
    (@read $x:expr, $f:ident) => {
        $crate::json::FromJson::from_json($x).map_err(|e| $crate::json::in_field(e, stringify!($f)))?
    };
    (@get $obj:ident, $mode:tt, $f:ident, hex) => {
        match $crate::json::field($obj, stringify!($f)) {
            Some(x) => $crate::json::hex_from_json(x).map_err(|e| $crate::json::in_field(e, stringify!($f)))?,
            None => $crate::json_struct!(@absent $mode, $f),
        }
    };
    (@get $obj:ident, $mode:tt, $f:ident, default) => {
        match $crate::json::field($obj, stringify!($f)) {
            Some(x) => $crate::json_struct!(@read x, $f),
            None => Default::default(),
        }
    };
    (@get $obj:ident, $mode:tt, $f:ident, skip_empty) => {
        $crate::json_struct!(@get $obj, $mode, $f, default)
    };
    (@get $obj:ident, $mode:tt, $f:ident, or = $d:path) => {
        match $crate::json::field($obj, stringify!($f)) {
            Some(x) => $crate::json_struct!(@read x, $f),
            None => $d(),
        }
    };
    (@get $obj:ident, $mode:tt, $f:ident) => {
        match $crate::json::field($obj, stringify!($f)) {
            Some(x) => $crate::json_struct!(@read x, $f),
            None => $crate::json_struct!(@absent $mode, $f),
        }
    };
    (@bind base, $b:ident, $ty:ident) => { let $b = <$ty as Default>::default(); };
    (@bind strict, $b:ident, $ty:ident) => {};
    (@absent (base $b:ident), $f:ident) => { $b.$f };
    (@absent (strict $b:ident), $f:ident) => {
        match $crate::json::FromJson::missing() {
            Some(v) => v,
            None => {
                return Err($crate::json::Error {
                    msg: concat!("missing field `", stringify!($f), "`").to_string(),
                    at: 0,
                })
            }
        }
    };
}

/// `ToJson`/`FromJson` for an enum of unit and struct variants, in the
/// externally tagged layout earlier versions wrote: `"Unit"` and
/// `{"Variant": {"field": ...}}`.
///
/// ```ignore
/// json_enum!(Trust { Signed { signer }, Unsigned, Missing });
/// ```
#[macro_export]
macro_rules! json_enum {
    ($ty:ident { $($var:ident $({ $($f:ident $([$($m:tt)*])?),* $(,)? })?),* $(,)? }) => {
        impl $crate::json::ToJson for $ty {
            #[allow(unused_variables, clippy::vec_init_then_push)]
            fn to_json(&self) -> $crate::json::Value {
                match self {
                    $( $crate::json_enum!(@pat $ty, $var $({ $($f),* })?) =>
                        $crate::json_enum!(@to $var $({ $($f $([$($m)*])?),* })?), )*
                }
            }
        }
        impl $crate::json::FromJson for $ty {
            fn from_json(__v: &$crate::json::Value) -> Result<Self, $crate::json::Error> {
                // A unit variant is a string; a struct variant a one-key object.
                let (__tag, __body) = match __v {
                    $crate::json::Value::String(s) => (s.as_str(), None),
                    $crate::json::Value::Object(m) if m.len() == 1 => (m[0].0.as_str(), Some(&m[0].1)),
                    _ => return $crate::json::type_error(concat!("a ", stringify!($ty))),
                };
                $(
                    if __tag == stringify!($var) {
                        return $crate::json_enum!(@from $ty, $var, __body $({ $($f $([$($m)*])?),* })?);
                    }
                )*
                Err($crate::json::Error {
                    msg: format!(concat!("unknown ", stringify!($ty), " `{}`"), __tag),
                    at: 0,
                })
            }
        }
    };
    (@pat $ty:ident, $var:ident) => { $ty::$var };
    (@pat $ty:ident, $var:ident { $($f:ident),* }) => { $ty::$var { $($f),* } };
    (@to $var:ident) => { $crate::json::Value::String(stringify!($var).to_string()) };
    (@to $var:ident { $($f:ident $([$($m:tt)*])?),* }) => {{
        let mut __o: Vec<(String, $crate::json::Value)> = Vec::new();
        $( $crate::json_enum!(@put __o, $f $(, $($m)*)?); )*
        $crate::json::Value::Object(vec![(stringify!($var).to_string(), $crate::json::Value::Object(__o))])
    }};
    (@put $o:ident, $f:ident) => {
        $o.push((stringify!($f).to_string(), $crate::json::ToJson::to_json($f)));
    };
    (@put $o:ident, $f:ident, hex) => {
        $o.push((stringify!($f).to_string(), $crate::json::hex_to_json($f)));
    };
    (@put $o:ident, $f:ident, default) => { $crate::json_enum!(@put $o, $f); };
    (@from $ty:ident, $var:ident, $body:ident) => { Ok($ty::$var) };
    (@from $ty:ident, $var:ident, $body:ident { $($f:ident $([$($m:tt)*])?),* }) => {{
        let Some(__obj) = $body.and_then(|b| b.as_object()) else {
            return $crate::json::type_error(concat!("the fields of ", stringify!($var)));
        };
        Ok($ty::$var {
            $( $f: $crate::json_struct!(@get __obj, (strict __unused), $f $(, $($m)*)?), )*
        })
    }};
}

#[cfg(test)]
mod typed_tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default, Debug, PartialEq, Clone)]
    struct Loose {
        name: String,
        n: u32,
        big: u64,
        neg: i64,
        zoom: f32,
        ratio: f64,
        on: bool,
        opt: Option<String>,
        list: Vec<u64>,
        map: BTreeMap<String, Vec<String>>,
        tags: Vec<String>,
        inner: Strict,
        kinds: Vec<Kind>,
    }
    crate::json_struct!(Loose, default, {
        name, n, big, neg, zoom, ratio, on, opt, list, map, tags [skip_empty], inner, kinds
    });

    #[derive(Default, Debug, PartialEq, Clone)]
    struct Strict {
        id: String,
        data: Vec<u8>,
        maybe: Option<u32>,
        seven: u32,
    }
    fn seven() -> u32 {
        7
    }
    crate::json_struct!(Strict, { id, data [hex], maybe, seven [or = seven] });

    #[derive(Debug, PartialEq, Clone)]
    enum Kind {
        Plain,
        WithFields { path: String, size: Option<u64> },
        Nested { item: Strict },
    }
    crate::json_enum!(Kind { Plain, WithFields { path, size }, Nested { item } });

    fn sample() -> Loose {
        let mut map = BTreeMap::new();
        map.insert("b".to_string(), vec!["x".to_string()]);
        map.insert("a".to_string(), vec![]);
        Loose {
            name: "Ünï \"code\"\n".into(),
            n: 4_000_000_000,
            big: u64::MAX,
            neg: i64::MIN,
            zoom: 1.1,
            ratio: 1e-7,
            on: true,
            opt: None,
            list: vec![1, 2, 3],
            map,
            tags: vec![],
            inner: Strict {
                id: "s".into(),
                data: vec![0, 15, 255],
                maybe: Some(3),
                seven: 9,
            },
            kinds: vec![
                Kind::Plain,
                Kind::WithFields {
                    path: "C:\\x".into(),
                    size: None,
                },
                Kind::Nested {
                    item: Strict::default(),
                },
            ],
        }
    }

    #[test]
    fn writes_exactly_what_serde_wrote() {
        // Golden files: serde's output for the same types, before the switch.
        let s = sample();
        assert_eq!(to_json_string(&s), include_str!("testdata/json/loose.json"));
        assert_eq!(
            to_json_string_pretty(&s),
            include_str!("testdata/json/loose.pretty.json")
        );
        let mut t = s.clone();
        t.tags = vec!["kept".into()];
        t.opt = Some("o".into());
        assert_eq!(
            to_json_string_pretty(&t),
            include_str!("testdata/json/loose_tags.pretty.json")
        );
    }

    #[test]
    fn reads_what_serde_wrote() {
        let s = sample();
        let ours: Loose = from_json_str(include_str!("testdata/json/loose.pretty.json")).unwrap();
        assert_eq!(ours, s);
        let ours: Loose = from_json_str(include_str!("testdata/json/loose.json")).unwrap();
        assert_eq!(ours, s);
    }

    #[test]
    fn absent_fields_follow_the_same_rules() {
        // Struct-wide default: everything absent is the default.
        let l: Loose = from_json_str("{}").unwrap();
        assert_eq!(l, Loose::default());
        // Strict: a missing required field is an error, Options become
        // None, and `or = ...` supplies its own value.
        let s: Strict = from_json_str(r#"{"id":"a","data":"00ff"}"#).unwrap();
        assert_eq!((s.maybe, s.seven, s.data), (None, 7, vec![0, 255]));
        let e = from_json_str::<Strict>(r#"{"data":""}"#).unwrap_err();
        assert!(e.msg.contains("missing field `id`"), "{e}");
        // Unknown fields are ignored; wrong types say where.
        assert!(from_json_str::<Strict>(r#"{"id":"a","data":"","extra":1}"#).is_ok());
        let e = from_json_str::<Loose>(r#"{"inner":{"id":5,"data":""}}"#).unwrap_err();
        assert!(e.msg.contains("in `inner.id`"), "{e}");
        let e = from_json_str::<Loose>(r#"{"n":-1}"#).unwrap_err();
        assert!(
            e.msg.contains("whole number") && e.msg.contains("`n`"),
            "{e}"
        );
        assert!(from_json_str::<Strict>(r#"{"id":"a","data":"0g"}"#).is_err());
        assert!(from_json_str::<Kind>(r#""Nope""#).is_err());
        assert!(from_json_str::<Kind>(r#"{"Plain":{},"WithFields":{}}"#).is_err());
    }
}

impl std::fmt::Display for Value {
    /// One-line JSON.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&to_string(self))
    }
}

/// The same calls under the names and `Result` shapes used before.
pub mod compat {
    use super::*;

    pub fn to_string<T: ToJson + ?Sized>(v: &T) -> Result<String, Error> {
        Ok(to_json_string(v))
    }
    pub fn to_string_pretty<T: ToJson + ?Sized>(v: &T) -> Result<String, Error> {
        Ok(to_json_string_pretty(v))
    }
    pub fn to_vec<T: ToJson + ?Sized>(v: &T) -> Result<Vec<u8>, Error> {
        Ok(to_json_string(v).into_bytes())
    }
    pub fn to_vec_pretty<T: ToJson + ?Sized>(v: &T) -> Result<Vec<u8>, Error> {
        Ok(to_json_string_pretty(v).into_bytes())
    }
    pub fn to_value<T: ToJson + ?Sized>(v: &T) -> Result<Value, Error> {
        Ok(v.to_json())
    }
    pub fn from_str<T: FromJson>(s: &str) -> Result<T, Error> {
        from_json_str(s)
    }
    pub fn from_slice<T: FromJson>(b: &[u8]) -> Result<T, Error> {
        from_json_slice(b)
    }
}

/// Build a [`Value`] from JSON-like syntax: `json!({"a": x, "b": [1, {"c": y}]})`.
/// Keys are string literals; values are nested literals or any [`ToJson`]
/// expression.
#[macro_export]
macro_rules! json {
    ([ $($tt:tt)* ]) => { $crate::json::Value::Array($crate::json!(@array [] $($tt)*)) };
    ({ $($tt:tt)* }) => { $crate::json::Value::Object($crate::json!(@object [] $($tt)*)) };
    ($e:expr) => { $crate::json::ToJson::to_json(&$e) };

    (@array [$($done:expr,)*]) => { vec![$($done,)*] };
    (@array [$($done:expr,)*] { $($o:tt)* } $(, $($rest:tt)*)?) => {
        $crate::json!(@array [$($done,)* $crate::json!({ $($o)* }),] $($($rest)*)?)
    };
    (@array [$($done:expr,)*] [ $($a:tt)* ] $(, $($rest:tt)*)?) => {
        $crate::json!(@array [$($done,)* $crate::json!([ $($a)* ]),] $($($rest)*)?)
    };
    (@array [$($done:expr,)*] $e:expr $(, $($rest:tt)*)?) => {
        $crate::json!(@array [$($done,)* $crate::json!($e),] $($($rest)*)?)
    };

    (@object [$($done:expr,)*]) => { vec![$($done,)*] };
    (@object [$($done:expr,)*] $k:literal : { $($o:tt)* } $(, $($rest:tt)*)?) => {
        $crate::json!(@object [$($done,)* ($k.to_string(), $crate::json!({ $($o)* })),] $($($rest)*)?)
    };
    (@object [$($done:expr,)*] $k:literal : [ $($a:tt)* ] $(, $($rest:tt)*)?) => {
        $crate::json!(@object [$($done,)* ($k.to_string(), $crate::json!([ $($a)* ])),] $($($rest)*)?)
    };
    (@object [$($done:expr,)*] $k:literal : $e:expr $(, $($rest:tt)*)?) => {
        $crate::json!(@object [$($done,)* ($k.to_string(), $crate::json!($e)),] $($($rest)*)?)
    };
}

#[cfg(test)]
mod macro_tests {
    #[test]
    fn json_macro_builds_the_same_document_as_before() {
        let name = "x";
        let items = vec![1u32, 2];
        let ours = crate::json!({
            "model": name,
            "temperature": 0,
            "stream": false,
            "messages": [
                {"role": "system", "content": "s"},
                {"role": "user", "content": items.len()},
            ],
            "items": items,
            "nested": {"a": [], "b": {}},
        });
        assert_eq!(ours["model"].as_str(), Some("x"));
        assert_eq!(ours["items"][1].as_u64(), Some(2));
        assert_eq!(
            ours.to_string(),
            r#"{"model":"x","temperature":0,"stream":false,"messages":[{"role":"system","content":"s"},{"role":"user","content":2}],"items":[1,2],"nested":{"a":[],"b":{}}}"#
        );
    }
}
