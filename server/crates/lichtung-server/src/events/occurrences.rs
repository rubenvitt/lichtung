//! `GET /api/occurrences?from&to&calendar_ids` (sync API contract,
//! section 7): occurrences in a time range, expanded by the server. A
//! convenience for tools; the apps expand series themselves.
//!
//! Candidates come from the stored range columns, then
//! [`lichtung_ical::expand`] unfolds each series. Every occurrence carries
//! the fields of the `VEVENT` that provides it (master or override), with
//! `start`/`end` of the occurrence itself.

use std::collections::HashMap;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use lichtung_ical::{expand, CalendarObject, ExpandOptions, Occurrence};
use serde::{Deserialize, Serialize};

use super::json::{event_fields, fmt_date, fmt_utc, start_end, DateValue, EventFields, Zones};
use super::store::{self, EventRow};
use super::EventContext;
use crate::accounts::store as accounts_store;
use crate::app::App;
use crate::error::{AppError, AppResult};

/// Longest range one query may cover.
pub const MAX_RANGE_DAYS: i64 = 400;
/// Most occurrences one answer holds (`truncated: true` beyond).
pub const MAX_OCCURRENCES: usize = 5_000;

/// Query parameters.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccurrenceQuery {
    /// Start (inclusive): ISO date or RFC 3339.
    pub from: Option<String>,
    /// End (exclusive): ISO date or RFC 3339.
    pub to: Option<String>,
    /// Comma-separated calendar IDs; all calendars if missing.
    pub calendar_ids: Option<String>,
    /// IANA zone for ISO dates in `from`/`to`, floating times and all-day
    /// events; the server's zone if missing.
    pub time_zone: Option<String>,
    /// Also return cancelled occurrences (`status: cancelled`).
    pub include_cancelled: Option<bool>,
}

/// One occurrence in the answer.
#[derive(Debug, Clone, Serialize)]
pub struct OccurrenceJson {
    pub event_id: String,
    pub calendar_id: String,
    pub uid: Option<String>,
    pub version: i64,
    /// Original start within the series; `null` for single events.
    pub recurrence_id: Option<DateValue>,
    /// Provided by a changed occurrence (override).
    pub is_override: bool,
    #[serde(flatten)]
    fields: EventFields,
}

/// The answer.
#[derive(Debug, Clone, Serialize)]
pub struct Occurrences {
    pub occurrences: Vec<OccurrenceJson>,
    /// More occurrences exist than [`MAX_OCCURRENCES`] (or a series was too
    /// long to expand completely).
    pub truncated: bool,
}

fn parse_bound(raw: Option<&str>, name: &str, tz: Tz) -> AppResult<DateTime<Utc>> {
    let raw = raw
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .ok_or_else(|| AppError::bad_request(format!("{name} fehlt")))?;
    if let Ok(d) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        let midnight = d.and_hms_opt(0, 0, 0).expect("midnight exists");
        return Ok(lichtung_ical::Zone::Iana(tz).to_utc(midnight));
    }
    DateTime::parse_from_rfc3339(raw)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| {
            AppError::bad_request(format!(
                "{name} muss ein Datum (2026-11-03) oder ein Zeitpunkt nach RFC 3339 \
                 (2026-11-03T13:00:00Z) sein, nicht „{raw}“"
            ))
        })
}

fn occurrence_value(
    o: &Occurrence,
    local: chrono::NaiveDateTime,
    at: DateTime<Utc>,
    label: &(Option<String>, bool),
) -> DateValue {
    let (time_zone, floating) = label;
    DateValue::DateTime {
        date_time: local.format("%Y-%m-%dT%H:%M:%S").to_string(),
        time_zone: if o.time_zone == "UTC" {
            Some("UTC".to_owned())
        } else {
            time_zone.clone()
        },
        utc: (!floating).then(|| fmt_utc(at)),
    }
}

fn expand_row(
    row: &EventRow,
    ctx: &EventContext,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    options: &ExpandOptions,
    out: &mut Vec<(DateTime<Utc>, DateTime<Utc>, OccurrenceJson)>,
) -> bool {
    let Ok(object) = CalendarObject::parse(&row.ics) else {
        return false;
    };
    let Ok(x) = expand(&object, from, to, options) else {
        return false;
    };
    let zones = Zones::new(&object, options.default_tz);
    for o in &x.occurrences {
        let Some(ev) = object.event_at(o.component_index) else {
            continue;
        };
        let rid = o.recurrence_id.as_ref();
        let Ok((cstart, cend)) = start_end(&ev, rid, &zones.resolver) else {
            continue;
        };
        let mut fields = event_fields(&ev, &cstart, &cend, &zones, ctx);
        match &o.all_day {
            Some(ad) => {
                fields.start = DateValue::Date {
                    date: fmt_date(ad.start),
                };
                fields.end = DateValue::Date {
                    date: fmt_date(ad.end),
                };
                fields.all_day = true;
            }
            None => {
                let label = zones.label(&cstart);
                fields.start = occurrence_value(o, o.local_start, o.start, &label);
                fields.end = occurrence_value(o, o.local_end, o.end, &label);
                fields.all_day = false;
            }
        }
        if o.cancelled {
            fields.status = "cancelled";
        }
        out.push((
            o.start,
            o.end,
            OccurrenceJson {
                event_id: row.id.clone(),
                calendar_id: row.calendar_id.clone(),
                uid: row.uid.clone(),
                version: row.version,
                recurrence_id: rid.map(|r| zones.date_value(r)),
                is_override: o.is_override,
                fields,
            },
        ));
    }
    x.truncated
}

impl App {
    /// Occurrences in `[from, to)` (see the module docs).
    pub async fn occurrences(&self, query: OccurrenceQuery) -> AppResult<Occurrences> {
        let tz = match query.time_zone.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => Tz::from_str(name).map_err(|_| {
                AppError::bad_request(format!(
                    "time_zone muss eine IANA-Zeitzone sein (z. B. Europe/Berlin), nicht „{name}“"
                ))
            })?,
            _ => super::default_zone(),
        };
        let from = parse_bound(query.from.as_deref(), "from", tz)?;
        let to = parse_bound(query.to.as_deref(), "to", tz)?;
        if to <= from {
            return Err(AppError::bad_request("to muss nach from liegen"));
        }
        if (to - from).num_days() > MAX_RANGE_DAYS {
            return Err(AppError::bad_request(format!(
                "Der Zeitraum darf höchstens {MAX_RANGE_DAYS} Tage umfassen"
            )));
        }
        let wanted: Option<Vec<String>> = query.calendar_ids.as_deref().map(|ids| {
            ids.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        });
        let include_cancelled = query.include_cancelled.unwrap_or(false);
        self.with_reader(move |conn| {
            let all = accounts_store::calendars(conn, None)?;
            let calendar_ids: Vec<String> = match wanted {
                Some(ids) => {
                    for id in &ids {
                        if !all.iter().any(|c| &c.id == id) {
                            return Err(AppError::bad_request(format!(
                                "Kalender {id} gibt es nicht"
                            )));
                        }
                    }
                    ids
                }
                None => all.iter().map(|c| c.id.clone()).collect(),
            };
            let rows = store::candidates(
                conn,
                &calendar_ids,
                from.timestamp_millis(),
                to.timestamp_millis(),
            )?;
            let mut options = ExpandOptions::new(tz);
            options.include_cancelled = include_cancelled;
            let mut contexts: HashMap<String, EventContext> = HashMap::new();
            let mut found = Vec::new();
            let mut truncated = false;
            for row in &rows {
                if !contexts.contains_key(&row.calendar_id) {
                    let mut ctx = store::context_for_calendar(conn, &row.calendar_id)?;
                    ctx.default_tz = tz;
                    contexts.insert(row.calendar_id.clone(), ctx);
                }
                let ctx = &contexts[&row.calendar_id];
                truncated |= expand_row(row, ctx, from, to, &options, &mut found);
            }
            found.sort_by(|a, b| (a.0, a.1, &a.2.event_id).cmp(&(b.0, b.1, &b.2.event_id)));
            if found.len() > MAX_OCCURRENCES {
                found.truncate(MAX_OCCURRENCES);
                truncated = true;
            }
            Ok(Occurrences {
                occurrences: found.into_iter().map(|(_, _, o)| o).collect(),
                truncated,
            })
        })
        .await
    }
}
