#![allow(dead_code)]
//! Minimal JSON object writer (output only).

pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

pub fn num(v: f64) -> String {
    if v.is_finite() {
        if v.fract() == 0.0 && v.abs() < 9.0e15 {
            format!("{}", v as i64)
        } else {
            format!("{:.6}", v)
        }
    } else {
        "null".into()
    }
}

#[derive(Default)]
pub struct Obj {
    parts: Vec<String>,
}

impl Obj {
    pub fn new() -> Self {
        Obj::default()
    }
    pub fn str(&mut self, k: &str, v: &str) {
        self.parts.push(format!("{}:{}", esc(k), esc(v)));
    }
    pub fn num(&mut self, k: &str, v: f64) {
        self.parts.push(format!("{}:{}", esc(k), num(v)));
    }
    pub fn bool(&mut self, k: &str, v: bool) {
        self.parts.push(format!("{}:{}", esc(k), v));
    }
    pub fn raw(&mut self, k: &str, v: &str) {
        self.parts.push(format!("{}:{}", esc(k), v));
    }
    pub fn null(&mut self, k: &str) {
        self.parts.push(format!("{}:null", esc(k)));
    }
    pub fn strs(&mut self, k: &str, v: &[String]) {
        let items: Vec<String> = v.iter().map(|s| esc(s)).collect();
        self.parts.push(format!("{}:[{}]", esc(k), items.join(",")));
    }
    pub fn finish(self) -> String {
        format!("{{{}}}", self.parts.join(","))
    }
}
