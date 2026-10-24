//! Error type of the crate.
//!
//! Messages are German because they may surface to the user (e.g. through the
//! sync status or an MCP error message).

use thiserror::Error;

/// All errors produced by `lichtung-ical`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    /// The iCalendar text could not be parsed.
    #[error("iCalendar-Fehler in Zeile {line}: {message}")]
    Parse {
        /// 1-based line number (after unfolding, counted on the physical input).
        line: usize,
        /// Human readable description.
        message: String,
    },

    /// A property value has an invalid format.
    #[error("Ungültiger Wert für {property}: „{value}“ ({reason})")]
    InvalidValue {
        /// Property name, e.g. `DTSTART`.
        property: String,
        /// The offending raw value.
        value: String,
        /// Why it is invalid.
        reason: String,
    },

    /// A required property is missing.
    #[error("Pflichtfeld {0} fehlt")]
    MissingProperty(String),

    /// The calendar object does not have the expected shape.
    #[error("Ungültiges Kalenderobjekt: {0}")]
    InvalidObject(String),

    /// The referenced occurrence does not exist in the series.
    #[error("Das Vorkommen {0} gehört nicht zu dieser Serie")]
    UnknownOccurrence(String),

    /// The requested edit is not possible.
    #[error("Bearbeiten nicht möglich: {0}")]
    InvalidEdit(String),
}

/// Convenience alias.
pub type Result<T, E = Error> = std::result::Result<T, E>;
