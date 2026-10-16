//! Just enough JSON for sheets: writing exports and archives, and reading archives back.

use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    /// Keys keep their order.
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Number(n) => {
                let _ = write!(out, "{n}");
            }
            Json::String(s) => quote(s, out),
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Json::Object(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    quote(key, out);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }

    pub fn parse(text: &str) -> Option<Json> {
        let mut reader = Reader { text: text.as_bytes(), at: 0 };
        let value = reader.value()?;
        reader.space();
        (reader.at == reader.text.len()).then_some(value)
    }
}

impl std::fmt::Display for Json {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut out = String::new();
        self.write(&mut out);
        f.write_str(&out)
    }
}

impl From<&str> for Json {
    fn from(s: &str) -> Json {
        Json::String(s.to_string())
    }
}

impl From<String> for Json {
    fn from(s: String) -> Json {
        Json::String(s)
    }
}

impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(value: Option<T>) -> Json {
        value.map_or(Json::Null, Into::into)
    }
}

fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Reader<'a> {
    text: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn space(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn eat(&mut self, word: &str) -> bool {
        let found = self.text[self.at..].starts_with(word.as_bytes());
        if found {
            self.at += word.len();
        }
        found
    }

    fn value(&mut self) -> Option<Json> {
        self.space();
        Some(match *self.text.get(self.at)? {
            b'n' if self.eat("null") => Json::Null,
            b't' if self.eat("true") => Json::Bool(true),
            b'f' if self.eat("false") => Json::Bool(false),
            b'"' => Json::String(self.string()?),
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.space();
                    if self.eat("]") {
                        break;
                    }
                    if !items.is_empty() && !self.eat(",") {
                        return None;
                    }
                    items.push(self.value()?);
                }
                Json::Array(items)
            }
            b'{' => {
                self.at += 1;
                let mut fields = Vec::new();
                loop {
                    self.space();
                    if self.eat("}") {
                        break;
                    }
                    if !fields.is_empty() && !self.eat(",") {
                        return None;
                    }
                    self.space();
                    let key = self.string()?;
                    self.space();
                    if !self.eat(":") {
                        return None;
                    }
                    fields.push((key, self.value()?));
                }
                Json::Object(fields)
            }
            _ => {
                let start = self.at;
                while self.text.get(self.at).is_some_and(|b| b"+-.eE0123456789".contains(b)) {
                    self.at += 1;
                }
                Json::Number(std::str::from_utf8(&self.text[start..self.at]).ok()?.parse().ok()?)
            }
        })
    }

    fn string(&mut self) -> Option<String> {
        if !self.eat("\"") {
            return None;
        }
        let mut bytes = Vec::new();
        loop {
            let b = *self.text.get(self.at)?;
            self.at += 1;
            match b {
                b'"' => return String::from_utf8(bytes).ok(),
                b'\\' => {
                    let escaped = *self.text.get(self.at)?;
                    self.at += 1;
                    let c = match escaped {
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let hex = std::str::from_utf8(self.text.get(self.at..self.at + 4)?).ok()?;
                            self.at += 4;
                            char::from_u32(u32::from_str_radix(hex, 16).ok()?)?
                        }
                        other => other as char,
                    };
                    bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                }
                b => bytes.push(b),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Json;

    #[test]
    fn reads_what_it_writes() {
        let value = Json::Object(vec![
            ("text".into(), "line \"one\"\n€5".into()),
            ("n".into(), Json::Number(2.5)),
            ("items".into(), Json::Array(vec![Json::Null, Json::Bool(true)])),
        ]);
        let text = value.to_string();
        assert_eq!(text, r#"{"text":"line \"one\"\n€5","n":2.5,"items":[null,true]}"#);
        assert_eq!(Json::parse(&text), Some(value));
        assert_eq!(Json::parse(r#" { "a" : [ 1 , -2e3 ] } "#).unwrap().get("a").unwrap().as_array().unwrap().len(), 2);
        assert_eq!(Json::parse("{"), None);
    }
}
