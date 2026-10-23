//! Lossless iCalendar (RFC 5545) content model: components, properties and
//! parameters, plus parser and serializer.
//!
//! The model keeps everything it reads: unknown properties, unknown (`X-`)
//! components, parameter order, property order and even the original line
//! folding of untouched properties. Parsing and serializing clean input is
//! therefore byte-stable (line endings are normalized to CRLF).

use crate::error::{Error, Result};

/// Maximum line length in octets (without CRLF) as required by RFC 5545 §3.1.
const MAX_LINE_OCTETS: usize = 75;

/// A property parameter such as `TZID=Europe/Berlin` or `CN="Doe, Jane"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    name: String,
    values: Vec<String>,
}

impl Param {
    /// Creates a parameter with a single value.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            values: vec![value.into()],
        }
    }

    /// Creates a parameter with several values (e.g. `MEMBER="a","b"`).
    pub fn with_values(name: impl Into<String>, values: Vec<String>) -> Self {
        Self {
            name: name.into(),
            values,
        }
    }

    /// Parameter name as written in the source (case preserved).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// All values, already unquoted.
    pub fn values(&self) -> &[String] {
        &self.values
    }

    /// The first value (most parameters have exactly one).
    pub fn value(&self) -> &str {
        self.values.first().map(String::as_str).unwrap_or("")
    }

    fn write(&self, out: &mut String) {
        out.push(';');
        out.push_str(&self.name);
        out.push('=');
        for (i, v) in self.values.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let clean: String = v.chars().filter(|c| *c != '"').collect();
            if clean.contains([':', ';', ',']) {
                out.push('"');
                out.push_str(&clean);
                out.push('"');
            } else {
                out.push_str(&clean);
            }
        }
    }
}

/// A content line: `NAME;PARAM=VALUE:value`.
///
/// The value is kept in its raw (escaped) form; use [`Property::text`] /
/// [`Property::set_text`] for TEXT values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    name: String,
    params: Vec<Param>,
    value: String,
    /// Original (folded) source lines joined with CRLF; `None` once modified.
    raw: Option<String>,
}

impl Property {
    /// Creates a property with a raw value (no escaping applied).
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            params: Vec::new(),
            value: value.into(),
            raw: None,
        }
    }

    /// Creates a TEXT property, escaping the value.
    pub fn new_text(name: impl Into<String>, text: &str) -> Self {
        Self::new(name, escape_text(text))
    }

    /// Builder-style: adds a parameter.
    pub fn with_param(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.set_param(name, value);
        self
    }

    /// Property name as written (case preserved).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Case-insensitive name comparison.
    pub fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }

    /// All parameters in source order.
    pub fn params(&self) -> &[Param] {
        &self.params
    }

    /// Looks up a parameter (case-insensitive) and returns its first value.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .map(Param::value)
    }

    /// Sets (replaces or appends) a single-valued parameter.
    pub fn set_param(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();
        self.raw = None;
        if let Some(p) = self
            .params
            .iter_mut()
            .find(|p| p.name.eq_ignore_ascii_case(&name))
        {
            p.values = vec![value];
        } else {
            self.params.push(Param::new(name, value));
        }
    }

    /// Removes a parameter; returns whether it existed.
    pub fn remove_param(&mut self, name: &str) -> bool {
        let before = self.params.len();
        self.params.retain(|p| !p.name.eq_ignore_ascii_case(name));
        let removed = self.params.len() != before;
        if removed {
            self.raw = None;
        }
        removed
    }

    /// The raw value (TEXT values are still escaped).
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Replaces the raw value.
    pub fn set_value(&mut self, value: impl Into<String>) {
        let value = value.into();
        if value != self.value {
            self.value = value;
            self.raw = None;
        }
    }

    /// The value interpreted as TEXT (unescaped).
    pub fn text(&self) -> String {
        unescape_text(&self.value)
    }

    /// Sets the value as TEXT (escaping it).
    pub fn set_text(&mut self, text: &str) {
        self.set_value(escape_text(text));
    }

    /// Whether this property is serialized exactly as it was read.
    pub fn is_pristine(&self) -> bool {
        self.raw.is_some()
    }

    /// The unfolded content line (`NAME;PARAMS:value`).
    pub fn content_line(&self) -> String {
        let mut line = String::with_capacity(self.name.len() + self.value.len() + 16);
        line.push_str(&self.name);
        for p in &self.params {
            p.write(&mut line);
        }
        line.push(':');
        line.push_str(&self.value);
        line
    }

    fn write(&self, out: &mut String) {
        match &self.raw {
            Some(raw) => {
                out.push_str(raw);
                out.push_str("\r\n");
            }
            None => fold_into(&self.content_line(), out),
        }
    }
}

/// A component such as `VCALENDAR`, `VEVENT`, `VALARM`, `VTIMEZONE` or any
/// unknown/`X-` component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    name: String,
    properties: Vec<Property>,
    components: Vec<Component>,
}

impl Component {
    /// Creates an empty component.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            properties: Vec::new(),
            components: Vec::new(),
        }
    }

    /// Component name as written (case preserved).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Case-insensitive name comparison.
    pub fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }

    /// All properties in source order.
    pub fn properties(&self) -> &[Property] {
        &self.properties
    }

    /// Mutable access to the property list.
    pub fn properties_mut(&mut self) -> &mut Vec<Property> {
        &mut self.properties
    }

    /// Sub-components in source order.
    pub fn components(&self) -> &[Component] {
        &self.components
    }

    /// Mutable access to the sub-components.
    pub fn components_mut(&mut self) -> &mut Vec<Component> {
        &mut self.components
    }

    /// First property with the given name (case-insensitive).
    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.is(name))
    }

    /// Mutable variant of [`Component::property`].
    pub fn property_mut(&mut self, name: &str) -> Option<&mut Property> {
        self.properties.iter_mut().find(|p| p.is(name))
    }

    /// All properties with the given name.
    pub fn properties_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Property> {
        self.properties.iter().filter(move |p| p.is(name))
    }

    /// Replaces the first property with the same name in place (keeping its
    /// position) and removes further ones; appends if none exists.
    pub fn set_property(&mut self, property: Property) {
        let name = property.name.clone();
        if let Some(pos) = self.properties.iter().position(|p| p.is(&name)) {
            // Only touch the line when something actually changes, so that the
            // original folding survives no-op updates.
            if self.properties[pos].content_line() != property.content_line() {
                self.properties[pos] = property;
            }
            let mut i = pos + 1;
            while i < self.properties.len() {
                if self.properties[i].is(&name) {
                    self.properties.remove(i);
                } else {
                    i += 1;
                }
            }
        } else {
            self.insert_property(property);
        }
    }

    /// Appends a property (sub-components are always serialized after all
    /// properties).
    pub fn insert_property(&mut self, property: Property) {
        self.properties.push(property);
    }

    /// Removes all properties with the given name; returns how many were removed.
    pub fn remove_properties(&mut self, name: &str) -> usize {
        let before = self.properties.len();
        self.properties.retain(|p| !p.is(name));
        before - self.properties.len()
    }

    /// Sub-components with the given name.
    pub fn components_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Component> {
        self.components.iter().filter(move |c| c.is(name))
    }

    /// Serializes this component (CRLF line endings, folded at 75 octets).
    pub fn to_ics(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        fold_into(&format!("BEGIN:{}", self.name), out);
        for p in &self.properties {
            p.write(out);
        }
        for c in &self.components {
            c.write(out);
        }
        fold_into(&format!("END:{}", self.name), out);
    }
}

/// Parses iCalendar text into its top-level components (normally exactly one
/// `VCALENDAR`).
///
/// Accepts CRLF, LF and CR line endings, folded lines (space or tab), and
/// blank lines. Unbalanced `BEGIN`/`END` pairs are reported as errors.
pub fn parse(input: &str) -> Result<Vec<Component>> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let lines = unfold(input);
    let mut stack: Vec<Component> = Vec::new();
    let mut top: Vec<Component> = Vec::new();
    for line in lines {
        if line.unfolded.trim().is_empty() {
            continue;
        }
        let prop = parse_content_line(&line.unfolded, line.number)?;
        if prop.is("BEGIN") {
            let name = prop.value.trim();
            if name.is_empty() {
                return Err(Error::Parse {
                    line: line.number,
                    message: "BEGIN ohne Komponentenname".into(),
                });
            }
            stack.push(Component::new(name));
        } else if prop.is("END") {
            let name = prop.value.trim();
            let Some(current) = stack.pop() else {
                return Err(Error::Parse {
                    line: line.number,
                    message: format!("END:{name} ohne passendes BEGIN"),
                });
            };
            if !current.name.eq_ignore_ascii_case(name) {
                return Err(Error::Parse {
                    line: line.number,
                    message: format!("END:{name} passt nicht zu BEGIN:{}", current.name),
                });
            }
            match stack.last_mut() {
                Some(parent) => parent.components.push(current),
                None => top.push(current),
            }
        } else {
            let Some(current) = stack.last_mut() else {
                return Err(Error::Parse {
                    line: line.number,
                    message: format!("Eigenschaft {} außerhalb einer Komponente", prop.name),
                });
            };
            let mut prop = prop;
            prop.raw = Some(line.raw);
            current.properties.push(prop);
        }
    }
    if let Some(open) = stack.last() {
        return Err(Error::Parse {
            line: input.lines().count(),
            message: format!("END:{} fehlt am Ende der Daten", open.name),
        });
    }
    if top.is_empty() {
        return Err(Error::Parse {
            line: 1,
            message: "keine Komponente gefunden (BEGIN:VCALENDAR erwartet)".into(),
        });
    }
    Ok(top)
}

/// Serializes several top-level components.
pub fn serialize(components: &[Component]) -> String {
    let mut out = String::new();
    for c in components {
        c.write(&mut out);
    }
    out
}

struct LogicalLine {
    number: usize,
    unfolded: String,
    raw: String,
}

fn unfold(input: &str) -> Vec<LogicalLine> {
    let mut out: Vec<LogicalLine> = Vec::new();
    // Normalize CRLF and lone CR to LF, then split.
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    for (idx, physical) in normalized.split('\n').enumerate() {
        let number = idx + 1;
        if let Some(rest) = physical
            .strip_prefix(' ')
            .or_else(|| physical.strip_prefix('\t'))
        {
            if let Some(last) = out.last_mut() {
                last.unfolded.push_str(rest);
                last.raw.push_str("\r\n");
                last.raw.push_str(physical);
                continue;
            }
        }
        out.push(LogicalLine {
            number,
            unfolded: physical.to_string(),
            raw: physical.to_string(),
        });
    }
    out
}

fn parse_content_line(line: &str, number: usize) -> Result<Property> {
    let err = |message: String| Error::Parse {
        line: number,
        message,
    };
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] != b';' && bytes[i] != b':' {
        i += 1;
    }
    let name = line[..i].trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(err(format!("ungültiger Eigenschaftsname in „{line}“")));
    }
    if i >= bytes.len() {
        return Err(err(format!("Doppelpunkt fehlt in „{line}“")));
    }
    let mut params = Vec::new();
    while i < bytes.len() && bytes[i] == b';' {
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != b'=' && bytes[i] != b':' && bytes[i] != b';' {
            i += 1;
        }
        let pname = line[start..i].to_string();
        if i >= bytes.len() || bytes[i] != b'=' {
            return Err(err(format!("Parameter „{pname}“ ohne Wert")));
        }
        i += 1; // '='
        let mut values = Vec::new();
        loop {
            if i < bytes.len() && bytes[i] == b'"' {
                let start = i + 1;
                let Some(end) = line[start..].find('"') else {
                    return Err(err(format!(
                        "Anführungszeichen in Parameter „{pname}“ nicht geschlossen"
                    )));
                };
                values.push(line[start..start + end].to_string());
                i = start + end + 1;
            } else {
                let start = i;
                while i < bytes.len() && bytes[i] != b',' && bytes[i] != b';' && bytes[i] != b':' {
                    i += 1;
                }
                values.push(line[start..i].to_string());
            }
            if i < bytes.len() && bytes[i] == b',' {
                i += 1;
                continue;
            }
            break;
        }
        params.push(Param {
            name: pname,
            values,
        });
    }
    if i >= bytes.len() || bytes[i] != b':' {
        return Err(err(format!("Doppelpunkt fehlt in „{line}“")));
    }
    Ok(Property {
        name: name.to_string(),
        params,
        value: line[i + 1..].to_string(),
        raw: None,
    })
}

/// Folds a content line at 75 octets (UTF-8 safe) and appends it plus CRLF.
pub(crate) fn fold_into(line: &str, out: &mut String) {
    let mut budget = MAX_LINE_OCTETS;
    let mut used = 0;
    for ch in line.chars() {
        let len = ch.len_utf8();
        if used + len > budget {
            out.push_str("\r\n ");
            budget = MAX_LINE_OCTETS - 1;
            used = 0;
        }
        out.push(ch);
        used += len;
    }
    out.push_str("\r\n");
}

/// Escapes a TEXT value (RFC 5545 §3.3.11).
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\\n");
            }
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// Unescapes a TEXT value (RFC 5545 §3.3.11). Unknown escapes are kept
/// verbatim (without the backslash).
pub fn unescape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_respects_utf8_boundaries() {
        let line = format!("SUMMARY:{}", "ä".repeat(60));
        let mut out = String::new();
        fold_into(&line, &mut out);
        for l in out.split("\r\n").filter(|l| !l.is_empty()) {
            assert!(l.len() <= 75, "line too long: {}", l.len());
        }
        let parsed = parse(&format!("BEGIN:X\r\n{out}END:X\r\n")).unwrap();
        assert_eq!(parsed[0].properties()[0].content_line(), line);
    }

    #[test]
    fn escape_roundtrip() {
        let s = "a; b, c\\d\nzweite Zeile";
        assert_eq!(unescape_text(&escape_text(s)), s);
        assert_eq!(escape_text(s), "a\\; b\\, c\\\\d\\nzweite Zeile");
    }

    #[test]
    fn quoted_params() {
        let p = parse_content_line(
            "ATTENDEE;CN=\"Muster, Erika\";DELEGATED-TO=\"mailto:a@example.org\",\"mailto:b@example.org\":mailto:erika@example.org",
            1,
        )
        .unwrap();
        assert_eq!(p.param("cn"), Some("Muster, Erika"));
        assert_eq!(p.params()[1].values().len(), 2);
        assert_eq!(p.value(), "mailto:erika@example.org");
        assert_eq!(
            p.content_line(),
            "ATTENDEE;CN=\"Muster, Erika\";DELEGATED-TO=\"mailto:a@example.org\",\"mailto:b@example.org\":mailto:erika@example.org"
        );
    }
}
