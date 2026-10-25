use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

pub fn object<const N: usize>(entries: [(&str, Json); N]) -> Json {
    Json::Object(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
}

impl From<&str> for Json {
    fn from(s: &str) -> Self {
        Self::String(s.into())
    }
}
impl From<String> for Json {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}
impl From<usize> for Json {
    fn from(n: usize) -> Self {
        Self::Number(n.to_string())
    }
}
impl From<u64> for Json {
    fn from(n: u64) -> Self {
        Self::Number(n.to_string())
    }
}
impl From<bool> for Json {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}
impl From<Vec<Json>> for Json {
    fn from(a: Vec<Json>) -> Self {
        Self::Array(a)
    }
}

impl Json {
    pub fn get(&self, key: &str) -> &Self {
        match self {
            Self::Object(o) => o.get(key).unwrap_or(&Self::Null),
            _ => &Self::Null,
        }
    }
    pub fn has(&self, key: &str) -> bool {
        matches!(self, Self::Object(o) if o.contains_key(key))
    }
    pub fn string(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn s(&self) -> &str {
        self.string().unwrap_or("")
    }
    pub fn array(&self) -> &[Self] {
        match self {
            Self::Array(a) => a,
            _ => &[],
        }
    }
    pub fn number(&self) -> Option<usize> {
        match self {
            Self::Number(s) => s.parse().ok(),
            _ => None,
        }
    }
    pub fn boolean(&self) -> bool {
        *self == Self::Bool(true)
    }
    pub fn insert(&mut self, k: &str, value: Self) {
        if let Self::Object(o) = self {
            o.insert(k.into(), value);
        }
    }
    pub fn remove(&mut self, k: &str) {
        if let Self::Object(o) = self {
            o.remove(k);
        }
    }
    pub fn encode(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(v) => v.to_string(),
            Self::Number(v) => v.clone(),
            Self::String(s) => {
                let mut out = String::from("\"");
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
                        c => out.push(c),
                    }
                }
                out.push('"');
                out
            }
            Self::Array(a) => format!(
                "[{}]",
                a.iter().map(Self::encode).collect::<Vec<_>>().join(",")
            ),
            Self::Object(o) => format!(
                "{{{}}}",
                o.iter()
                    .map(|(k, v)| format!("{}:{}", Self::from(k.clone()).encode(), v.encode()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
}

pub fn parse(s: &str) -> Result<Json, String> {
    let mut p = Parser {
        s: s.as_bytes(),
        i: 0,
    };
    let v = p.value(0)?;
    p.space();
    if p.i != p.s.len() {
        return Err(format!("Trailing JSON at byte {}", p.i));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\r' | b'\n') {
            self.i += 1;
        }
    }
    fn take(&mut self, b: u8) -> bool {
        if self.s.get(self.i) == Some(&b) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: usize) -> Result<Json, String> {
        self.space();
        if depth > 128 {
            return Err("JSON nesting exceeds 128".into());
        }
        match self.s.get(self.i) {
            Some(b'"') => Ok(Json::String(self.text()?)),
            Some(b'{') => {
                self.i += 1;
                self.space();
                let mut o = BTreeMap::new();
                if self.take(b'}') {
                    return Ok(Json::Object(o));
                }
                loop {
                    self.space();
                    let k = self.text()?;
                    self.space();
                    if !self.take(b':') {
                        return Err("Expected JSON colon".into());
                    }
                    let v = self.value(depth + 1)?;
                    if o.insert(k, v).is_some() {
                        return Err("Duplicate JSON key".into());
                    }
                    self.space();
                    if self.take(b'}') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err("Expected JSON comma".into());
                    }
                }
                Ok(Json::Object(o))
            }
            Some(b'[') => {
                self.i += 1;
                self.space();
                let mut a = Vec::new();
                if self.take(b']') {
                    return Ok(Json::Array(a));
                }
                loop {
                    a.push(self.value(depth + 1)?);
                    self.space();
                    if self.take(b']') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err("Expected JSON comma".into());
                    }
                }
                Ok(Json::Array(a))
            }
            Some(b'n') => self.literal(b"null", Json::Null),
            Some(b't') => self.literal(b"true", Json::Bool(true)),
            Some(b'f') => self.literal(b"false", Json::Bool(false)),
            Some(b'-' | b'0'..=b'9') => {
                let start = self.i;
                self.take(b'-');
                if !self.take(b'0') {
                    self.digits()?;
                }
                if self.take(b'.') {
                    self.digits()?;
                }
                if self.take(b'e') || self.take(b'E') {
                    if !self.take(b'+') {
                        self.take(b'-');
                    }
                    self.digits()?;
                }
                Ok(Json::Number(
                    String::from_utf8(self.s[start..self.i].to_vec()).map_err(|e| e.to_string())?,
                ))
            }
            _ => Err(format!("Invalid JSON at byte {}", self.i)),
        }
    }
    fn digits(&mut self) -> Result<(), String> {
        let start = self.i;
        while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
        }
        if self.i == start {
            Err("Expected JSON digit".into())
        } else {
            Ok(())
        }
    }
    fn literal(&mut self, word: &[u8], v: Json) -> Result<Json, String> {
        if self.s.get(self.i..self.i + word.len()) != Some(word) {
            return Err("Invalid JSON literal".into());
        }
        self.i += word.len();
        Ok(v)
    }
    fn hex(&mut self) -> Result<u32, String> {
        let bytes = self
            .s
            .get(self.i..self.i + 4)
            .ok_or("Incomplete JSON escape")?;
        let s = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        let n = u32::from_str_radix(s, 16).map_err(|_| "Invalid JSON escape")?;
        self.i += 4;
        Ok(n)
    }
    fn text(&mut self) -> Result<String, String> {
        if !self.take(b'"') {
            return Err("Expected JSON string".into());
        }
        let mut out = Vec::new();
        loop {
            let b = *self.s.get(self.i).ok_or("Unterminated JSON string")?;
            self.i += 1;
            match b {
                b'"' => return String::from_utf8(out).map_err(|e| e.to_string()),
                0..=31 => return Err("Control character in JSON string".into()),
                b'\\' => {
                    let b = *self.s.get(self.i).ok_or("Incomplete JSON escape")?;
                    self.i += 1;
                    match b {
                        b'"' | b'\\' | b'/' => out.push(b),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let mut n = self.hex()?;
                            if (0xd800..=0xdbff).contains(&n) {
                                if !self.take(b'\\') || !self.take(b'u') {
                                    return Err("Missing low surrogate".into());
                                }
                                let low = self.hex()?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return Err("Invalid low surrogate".into());
                                }
                                n = 0x10000 + ((n - 0xd800) << 10) + (low - 0xdc00);
                            }
                            let c = char::from_u32(n).ok_or("Invalid Unicode scalar")?;
                            out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        _ => return Err("Invalid JSON escape".into()),
                    }
                }
                _ => out.push(b),
            }
        }
    }
}
