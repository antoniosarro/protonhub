//! Valve's KeyValues formats: the text one (config.vdf, appmanifest_*.acf,
//! compatibilitytool.vdf), read and written in Steam's own layout so a
//! rewrite changes only what was edited, and the binary one
//! (shortcuts.vdf, appinfo.vdf), read only.

use std::fmt::Write;

/// A node's children, in file order. Keys repeat in some files and are
/// matched without regard to case, as Steam does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Obj(pub Vec<(String, Value)>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Obj(Obj),
    /// Only in the binary format.
    Int(i64),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&Obj> {
        match self {
            Self::Obj(o) => Some(o),
            _ => None,
        }
    }
}

impl Obj {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    pub fn obj(&self, key: &str) -> Option<&Obj> {
        self.get(key).and_then(Value::as_obj)
    }

    /// The object at `path`, following the first match of each key.
    pub fn path(&self, path: &[&str]) -> Option<&Obj> {
        path.iter().try_fold(self, |o, k| o.obj(k))
    }

    /// The object at `path`, created (appended) where missing.
    pub fn path_mut(&mut self, path: &[&str]) -> &mut Obj {
        let mut o = self;
        for key in path {
            let i = match o
                .0
                .iter()
                .position(|(k, v)| k.eq_ignore_ascii_case(key) && matches!(v, Value::Obj(_)))
            {
                Some(i) => i,
                None => {
                    o.0.push(((*key).to_owned(), Value::Obj(Obj::default())));
                    o.0.len() - 1
                }
            };
            let Value::Obj(child) = &mut o.0[i].1 else {
                unreachable!("position matched an object")
            };
            o = child;
        }
        o
    }

    /// Replaces the first `key` in place, or appends it.
    pub fn set(&mut self, key: &str, value: Value) {
        match self.0.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            Some(slot) => slot.1 = value,
            None => self.0.push((key.to_owned(), value)),
        }
    }

    /// Removes every `key`; whether there was one.
    pub fn remove(&mut self, key: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
        self.0.len() != before
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("line {line}: {message}")]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

/// Parses the text format. `//` comments and `[$PLATFORM]` conditions are
/// skipped; conditions are not evaluated.
pub fn parse(text: &str) -> Result<Obj, ParseError> {
    let mut p = Parser {
        chars: text.chars().peekable(),
        line: 1,
    };
    let obj = p.obj(false)?;
    Ok(obj)
}

enum Token {
    Str(String),
    Open,
    Close,
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: usize,
}

impl Parser<'_> {
    fn err(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            line: self.line,
            message: message.into(),
        }
    }

    fn obj(&mut self, nested: bool) -> Result<Obj, ParseError> {
        let mut obj = Obj::default();
        loop {
            let key = match self.token()? {
                None if nested => return Err(self.err("missing }")),
                None => return Ok(obj),
                Some(Token::Close) if nested => return Ok(obj),
                Some(Token::Close) => return Err(self.err("unmatched }")),
                Some(Token::Open) => return Err(self.err("{ without a key")),
                Some(Token::Str(k)) => k,
            };
            let value = match self.token()? {
                Some(Token::Str(v)) => Value::Str(v),
                Some(Token::Open) => Value::Obj(self.obj(true)?),
                _ => return Err(self.err(format!("{key:?} has no value"))),
            };
            obj.0.push((key, value));
        }
    }

    fn token(&mut self) -> Result<Option<Token>, ParseError> {
        loop {
            let Some(&c) = self.chars.peek() else {
                return Ok(None);
            };
            match c {
                '\n' => {
                    self.line += 1;
                    self.chars.next();
                }
                c if c.is_whitespace() => {
                    self.chars.next();
                }
                '/' => {
                    self.chars.next();
                    if self.chars.next_if_eq(&'/').is_none() {
                        return Err(self.err("stray /"));
                    }
                    while self.chars.next_if(|&c| c != '\n').is_some() {}
                }
                '[' => {
                    while self.chars.next_if(|&c| c != ']' && c != '\n').is_some() {}
                    if self.chars.next_if_eq(&']').is_none() {
                        return Err(self.err("unclosed ["));
                    }
                }
                '{' => {
                    self.chars.next();
                    return Ok(Some(Token::Open));
                }
                '}' => {
                    self.chars.next();
                    return Ok(Some(Token::Close));
                }
                '"' => {
                    self.chars.next();
                    let mut s = String::new();
                    loop {
                        match self.chars.next() {
                            None => return Err(self.err("unclosed string")),
                            Some('"') => break,
                            Some('\\') => match self.chars.next() {
                                Some('n') => s.push('\n'),
                                Some('t') => s.push('\t'),
                                Some(c) => s.push(c),
                                None => return Err(self.err("unclosed string")),
                            },
                            Some(c) => {
                                if c == '\n' {
                                    self.line += 1;
                                }
                                s.push(c);
                            }
                        }
                    }
                    return Ok(Some(Token::Str(s)));
                }
                _ => {
                    let mut s = String::new();
                    while let Some(c) = self
                        .chars
                        .next_if(|&c| !c.is_whitespace() && !matches!(c, '{' | '}' | '"'))
                    {
                        s.push(c);
                    }
                    return Ok(Some(Token::Str(s)));
                }
            }
        }
    }
}

/// Writes the text format as Steam does: tabs, and two between a key and
/// its value.
pub fn to_string(obj: &Obj) -> String {
    fn write(obj: &Obj, depth: usize, out: &mut String) {
        let indent = "\t".repeat(depth);
        for (k, v) in &obj.0 {
            match v {
                Value::Str(s) => {
                    let _ = writeln!(out, "{indent}\"{}\"\t\t\"{}\"", escape(k), escape(s));
                }
                Value::Int(n) => {
                    let _ = writeln!(out, "{indent}\"{}\"\t\t\"{n}\"", escape(k));
                }
                Value::Obj(o) => {
                    let _ = writeln!(out, "{indent}\"{}\"\n{indent}{{", escape(k));
                    write(o, depth + 1, out);
                    let _ = writeln!(out, "{indent}}}");
                }
            }
        }
    }
    let mut out = String::new();
    write(obj, 0, &mut out);
    out
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Parses the binary format (shortcuts.vdf).
pub fn parse_binary(bytes: &[u8]) -> Result<Obj, ParseError> {
    let mut pos = 0;
    let obj = binary_obj(bytes, &mut pos, true, None)?;
    Ok(obj)
}

/// Parses the binary format with each key an index into `keys`, as newer
/// appinfo.vdf files store them.
pub fn parse_binary_keyed(bytes: &[u8], keys: &[String]) -> Result<Obj, ParseError> {
    let mut pos = 0;
    binary_obj(bytes, &mut pos, true, Some(keys))
}

fn binary_obj(
    bytes: &[u8],
    pos: &mut usize,
    root: bool,
    keys: Option<&[String]>,
) -> Result<Obj, ParseError> {
    let err = |pos: usize, message: &str| ParseError {
        line: 0,
        message: format!("byte {pos}: {message}"),
    };
    let cstr = |pos: &mut usize| -> Result<String, ParseError> {
        let len = bytes[*pos..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| err(*pos, "unterminated string"))?;
        let s = String::from_utf8_lossy(&bytes[*pos..*pos + len]).into_owned();
        *pos += len + 1;
        Ok(s)
    };
    let fixed = |pos: &mut usize, n: usize| -> Result<&[u8], ParseError> {
        let raw = bytes
            .get(*pos..*pos + n)
            .ok_or_else(|| err(*pos, "truncated value"))?;
        *pos += n;
        Ok(raw)
    };
    let key = |pos: &mut usize| -> Result<String, ParseError> {
        let Some(keys) = keys else { return cstr(pos) };
        let at = *pos;
        let raw = fixed(pos, 4)?;
        let i = u32::from_le_bytes(raw.try_into().expect("four bytes")) as usize;
        keys.get(i)
            .cloned()
            .ok_or_else(|| err(at, &format!("key {i} is past the key table")))
    };
    let mut obj = Obj::default();
    loop {
        let Some(&kind) = bytes.get(*pos) else {
            return if root {
                Ok(obj)
            } else {
                Err(err(*pos, "missing end of object"))
            };
        };
        *pos += 1;
        match kind {
            0x08 => return Ok(obj),
            0x00 => {
                let key = key(pos)?;
                obj.0
                    .push((key, Value::Obj(binary_obj(bytes, pos, false, keys)?)));
            }
            0x01 => {
                let key = key(pos)?;
                obj.0.push((key, Value::Str(cstr(pos)?)));
            }
            // An int32, a pointer, a color.
            0x02 | 0x04 | 0x06 => {
                let key = key(pos)?;
                let raw = fixed(pos, 4)?;
                let n = i32::from_le_bytes(raw.try_into().expect("four bytes"));
                obj.0.push((key, Value::Int(n.into())));
            }
            // A float, kept as its text: Value is Eq.
            0x03 => {
                let key = key(pos)?;
                let raw = fixed(pos, 4)?;
                let f = f32::from_le_bytes(raw.try_into().expect("four bytes"));
                obj.0.push((key, Value::Str(f.to_string())));
            }
            // A uint64, an int64.
            0x07 | 0x0a => {
                let key = key(pos)?;
                let raw = fixed(pos, 8)?;
                let n = i64::from_le_bytes(raw.try_into().expect("eight bytes"));
                obj.0.push((key, Value::Int(n)));
            }
            other => return Err(err(*pos - 1, &format!("unknown type {other:#04x}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"CompatToolMapping\"\n\t\t\t\t{\n\t\t\t\t\t\"0\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"proton_hotfix\"\n\t\t\t\t\t\t\"config\"\t\t\"\"\n\t\t\t\t\t\t\"priority\"\t\t\"75\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t\t\"path\"\t\t\"C:\\\\Games \\\"x\\\"\"\n\t\t\t}\n\t\t}\n\t}\n}\n";

    #[test]
    fn round_trips_steams_layout() {
        let obj = parse(CONFIG).unwrap();
        assert_eq!(to_string(&obj), CONFIG);
        let steam = obj
            .path(&["installconfigstore", "software", "valve", "steam"])
            .unwrap();
        assert_eq!(steam.str("path"), Some(r#"C:\Games "x""#));
        let default = steam.path(&["CompatToolMapping", "0"]).unwrap();
        assert_eq!(default.str("name"), Some("proton_hotfix"));
    }

    #[test]
    fn edits_in_place() {
        let mut obj = parse(CONFIG).unwrap();
        let map = obj.path_mut(&[
            "InstallConfigStore",
            "Software",
            "Valve",
            "Steam",
            "CompatToolMapping",
        ]);
        let mut entry = Obj::default();
        entry.set("name", Value::Str("GE-Proton11-7".into()));
        map.set("409710", Value::Obj(entry));
        assert!(map.remove("0"));
        assert!(!map.remove("0"));
        let text = to_string(&obj);
        assert!(text.contains(
            "\t\t\t\t\t\"409710\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"GE-Proton11-7\"\n"
        ));
        assert!(!text.contains("proton_hotfix"));
        // Everything else is untouched.
        assert!(
            text.ends_with(
                "\t\t\t\t\"path\"\t\t\"C:\\\\Games \\\"x\\\"\"\n\t\t\t}\n\t\t}\n\t}\n}\n"
            )
        );
    }

    #[test]
    fn reads_compatibilitytool_vdf() {
        let text = r#"
            "compatibilitytools"
            {
              "compat_tools"
              {
                "GE-Proton11-7" // Internal name of this tool
                {
                  "install_path" "."
                  "display_name" "GE-Proton11-7"
                  "from_oslist"  "windows"
                }
              }
            }
            "#;
        let obj = parse(text).unwrap();
        let tools = obj.path(&["compatibilitytools", "compat_tools"]).unwrap();
        assert_eq!(tools.0[0].0, "GE-Proton11-7");
        assert_eq!(
            tools.obj("GE-Proton11-7").unwrap().str("install_path"),
            Some(".")
        );
    }

    #[test]
    fn reports_where_it_breaks() {
        assert_eq!(parse("\"a\"\n{\n\"b\"").unwrap_err().line, 3);
        assert!(parse("\"a\" {").is_err());
        assert!(parse("}").is_err());
        assert!(parse("\"a\" \"b").is_err());
    }

    #[test]
    fn reads_binary_shortcuts() {
        let mut b = vec![0x00];
        b.extend(b"shortcuts\0");
        b.push(0x00);
        b.extend(b"0\0");
        b.push(0x02);
        b.extend(b"appid\0");
        b.extend((-1_234_567_890i32).to_le_bytes());
        b.push(0x01);
        b.extend(b"AppName\0My Game\0");
        b.extend([0x08, 0x08, 0x08]);
        let obj = parse_binary(&b).unwrap();
        let game = obj.path(&["shortcuts", "0"]).unwrap();
        assert_eq!(game.get("AppID"), Some(&Value::Int(-1_234_567_890)));
        assert_eq!(game.str("appname"), Some("My Game"));
        assert!(parse_binary(&b[..b.len() - 6]).is_err());
    }

    #[test]
    fn reads_binary_keys_from_a_table() {
        let keys: Vec<String> = ["appinfo", "common", "name", "gameid"]
            .map(String::from)
            .to_vec();
        let mut b = vec![0x00];
        b.extend(0u32.to_le_bytes());
        b.push(0x00);
        b.extend(1u32.to_le_bytes());
        b.push(0x01);
        b.extend(2u32.to_le_bytes());
        b.extend(b"Hades\0");
        b.push(0x07);
        b.extend(3u32.to_le_bytes());
        b.extend(1_145_360u64.to_le_bytes());
        b.extend([0x08, 0x08, 0x08]);
        let obj = parse_binary_keyed(&b, &keys).unwrap();
        let common = obj.path(&["appinfo", "common"]).unwrap();
        assert_eq!(common.str("name"), Some("Hades"));
        assert_eq!(common.get("gameid"), Some(&Value::Int(1_145_360)));
        assert!(
            parse_binary_keyed(&b, &keys[..2]).is_err(),
            "a key past the table"
        );
    }
}
