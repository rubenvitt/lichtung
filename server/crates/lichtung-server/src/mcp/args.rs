//! Typed access to tool arguments with German error messages that say
//! what is wrong and how to do it right.

use serde_json::{Map, Value};

use super::tools::ToolError;

pub type ArgResult<T> = Result<T, ToolError>;

pub fn invalid(message: impl Into<String>) -> ToolError {
    ToolError::new("invalid_arguments", message)
}

/// The arguments of one tool call (or one nested object).
#[derive(Debug, Clone, Default)]
pub struct Args(Map<String, Value>);

impl Args {
    pub fn new(map: Map<String, Value>) -> Self {
        Self(map)
    }

    /// Rejects unknown keys and missing required keys. A missing
    /// `calendar_id` gets its own error (there is no default calendar).
    pub fn check<'a>(
        &self,
        allowed: impl IntoIterator<Item = &'a str>,
        required: impl IntoIterator<Item = &'a str>,
    ) -> ArgResult<()> {
        let allowed: Vec<&str> = allowed.into_iter().collect();
        if let Some(unknown) = self.0.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(invalid(format!(
                "Unbekannter Parameter „{unknown}“. Erlaubt: {}.",
                allowed.join(", ")
            )));
        }
        for key in required {
            if self.value(key).is_none() {
                if key == "calendar_id" {
                    return Err(ToolError::new(
                        "calendar_id_missing",
                        "calendar_id fehlt. Lichtung hat keinen Standardkalender: Wähle mit \
                         list_calendars einen Kalender mit mcp_writable = true und gib dessen id \
                         als calendar_id an (im Zweifel den Nutzer fragen, welcher Kalender \
                         gemeint ist).",
                    ));
                }
                return Err(invalid(format!("Parameter „{key}“ fehlt.")));
            }
        }
        Ok(())
    }

    /// The raw value, `Some(Null)` if explicitly `null`.
    pub fn raw(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// Present and not `null`.
    pub fn value(&self, key: &str) -> Option<&Value> {
        self.0.get(key).filter(|v| !v.is_null())
    }

    /// Explicitly `null`.
    pub fn is_null(&self, key: &str) -> bool {
        self.0.get(key).is_some_and(Value::is_null)
    }

    pub fn str(&self, key: &str) -> ArgResult<Option<&str>> {
        match self.value(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(_) => Err(invalid(format!("„{key}“ muss ein Text sein."))),
        }
    }

    /// A non-empty (after trimming) text, trimmed.
    pub fn non_empty_str(&self, key: &str) -> ArgResult<Option<&str>> {
        match self.str(key)? {
            Some(s) if s.trim().is_empty() => {
                Err(invalid(format!("„{key}“ darf nicht leer sein.")))
            }
            other => Ok(other.map(str::trim)),
        }
    }

    pub fn required_str(&self, key: &str) -> ArgResult<&str> {
        self.non_empty_str(key)?
            .ok_or_else(|| invalid(format!("Parameter „{key}“ fehlt.")))
    }

    pub fn bool(&self, key: &str) -> ArgResult<Option<bool>> {
        match self.value(key) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(invalid(format!("„{key}“ muss true oder false sein."))),
        }
    }

    /// An integer in `min..=max`, `default` when absent.
    pub fn int(&self, key: &str, min: i64, max: i64, default: i64) -> ArgResult<i64> {
        Ok(self.opt_int(key, min, max)?.unwrap_or(default))
    }

    /// An integer in `min..=max`, `None` when absent.
    pub fn opt_int(&self, key: &str, min: i64, max: i64) -> ArgResult<Option<i64>> {
        match self.value(key) {
            None => Ok(None),
            Some(v) => match v.as_i64() {
                Some(n) if (min..=max).contains(&n) => Ok(Some(n)),
                _ => Err(invalid(format!(
                    "„{key}“ muss eine ganze Zahl von {min} bis {max} sein."
                ))),
            },
        }
    }

    /// One of `allowed`.
    pub fn code(&self, key: &str, allowed: &[&'static str]) -> ArgResult<Option<&'static str>> {
        match self.str(key)? {
            None => Ok(None),
            Some(s) => allowed
                .iter()
                .find(|a| **a == s.trim())
                .copied()
                .map(Some)
                .ok_or_else(|| {
                    invalid(format!(
                        "Ungültiger Wert „{s}“ für „{key}“. Erlaubt: {}.",
                        allowed.join(", ")
                    ))
                }),
        }
    }

    /// A list of non-empty texts.
    pub fn strings(&self, key: &str) -> ArgResult<Option<Vec<String>>> {
        let Some(value) = self.value(key) else {
            return Ok(None);
        };
        let items = value
            .as_array()
            .ok_or_else(|| invalid(format!("„{key}“ muss eine Liste von Texten sein.")))?;
        items
            .iter()
            .map(|item| match item.as_str().map(str::trim) {
                Some(s) if !s.is_empty() => Ok(s.to_owned()),
                _ => Err(invalid(format!(
                    "„{key}“ darf nur nicht-leere Texte enthalten."
                ))),
            })
            .collect::<ArgResult<Vec<_>>>()
            .map(Some)
    }
}
