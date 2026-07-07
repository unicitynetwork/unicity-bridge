//! Tiny JSON object writer (hex / number / string / nested-object / raw values),
//! so the generator needs no serde dependency.

pub struct Obj {
    fields: Vec<(String, String)>,
}

impl Obj {
    pub fn new() -> Self {
        Obj { fields: Vec::new() }
    }

    pub fn str(mut self, k: &str, v: &str) -> Self {
        self.fields.push((k.into(), jstr(v)));
        self
    }

    pub fn num(mut self, k: &str, v: u64) -> Self {
        self.fields.push((k.into(), v.to_string()));
        self
    }

    pub fn hex(mut self, k: &str, v: &[u8]) -> Self {
        self.fields.push((k.into(), format!("\"0x{}\"", hex(v))));
        self
    }

    pub fn obj(mut self, k: &str, child: Obj) -> Self {
        self.fields.push((k.into(), child.render_inline()));
        self
    }

    /// A pre-rendered JSON value (e.g. an array string).
    pub fn raw(mut self, k: &str, v: String) -> Self {
        self.fields.push((k.into(), v));
        self
    }

    /// Single-line object form, used for nesting and array elements.
    pub fn render_inline(&self) -> String {
        let inner: Vec<String> = self
            .fields
            .iter()
            .map(|(k, v)| format!("\"{k}\": {v}"))
            .collect();
        format!("{{ {} }}", inner.join(", "))
    }

    /// Pretty top-level object with a trailing newline.
    pub fn render(&self) -> String {
        let mut s = String::from("{\n");
        for (i, (k, v)) in self.fields.iter().enumerate() {
            let comma = if i + 1 < self.fields.len() { "," } else { "" };
            s.push_str(&format!("  \"{k}\": {v}{comma}\n"));
        }
        s.push_str("}\n");
        s
    }
}

pub fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

fn jstr(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}
