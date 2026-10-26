//! Tool handlers. Reading goes through the occurrence query
//! ([`App::occurrences`]); every write goes through the write path
//! ([`App::apply_mutation`]) with the caller's origin (`mcp:<token>`), so
//! the calendar guard `mcp_write`, conflict handling (the provider wins)
//! and scopes are exactly those of the apps.

use std::collections::HashMap;
use std::sync::OnceLock;

use chrono::{DateTime, Duration, Utc, Weekday};
use chrono_tz::Tz;
use serde_json::{json, Map, Value};

use super::args::{invalid, ArgResult, Args};
use super::free::{free_slots, FreeQuery, Interval};
use super::schema::{definitions, ToolDef, AVAILABILITY, RESPONSES, ROLES, SCOPES, WEEKDAYS};
use super::time::{
    date_input, date_time_input, fmt_at, fmt_date, midnight, occurrence_input, parse_clock,
    parse_time, parse_zone, pick_zone, range, when_of, TimeArg, When,
};
use crate::accounts::{Account, AccountKind, Calendar, CalendarSet};
use crate::app::App;
use crate::error::AppError;
use crate::events::default_zone;
use crate::events::occurrences::OccurrenceQuery;
use crate::writes::{Origin, WriteOutcome};

/// Default horizon of `list_invitations`.
const INVITATION_DAYS: i64 = 180;

/// A failed tool call: stable code, German message (what is wrong and how
/// to do it right), optional details (e.g. the current state after a
/// conflict).
#[derive(Debug, Clone)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
}

impl ToolError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        ToolError {
            code,
            message: message.into(),
            details: None,
        }
    }

    fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl From<AppError> for ToolError {
    fn from(e: AppError) -> Self {
        if let AppError::Internal(inner) = &e {
            tracing::error!(error = ?inner, "MCP tool failed");
        }
        ToolError::new(e.code(), e.to_string())
    }
}

type ToolResult = Result<Value, ToolError>;

fn tools() -> &'static [ToolDef] {
    static TOOLS: OnceLock<Vec<ToolDef>> = OnceLock::new();
    TOOLS.get_or_init(definitions)
}

/// The `tools/list` result.
pub fn list() -> Value {
    json!({"tools": tools().iter().map(ToolDef::to_json).collect::<Vec<_>>()})
}

/// Runs a tool and returns the `tools/call` result (`None` for an unknown
/// tool). Errors become results with `isError: true`.
pub async fn call(
    app: &App,
    origin: &Origin,
    name: &str,
    args: Map<String, Value>,
) -> Option<Value> {
    let def = tools().iter().find(|t| t.name == name)?;
    let args = Args::new(args);
    let result = match args.check(def.properties(), def.required()) {
        Ok(()) => dispatch(app, origin, name, &args).await,
        Err(e) => Err(e),
    };
    Some(tool_result(result))
}

fn tool_result(result: ToolResult) -> Value {
    match result {
        Ok(value) => json!({
            "content": [{"type": "text", "text": value.to_string()}],
            "structuredContent": value,
            "isError": false,
        }),
        Err(e) => {
            let mut structured = json!({"error": {"code": e.code, "message": e.message}});
            let mut text = e.message.clone();
            if let Some(Value::Object(details)) = e.details {
                text.push_str(&format!(" {}", Value::Object(details.clone())));
                for (k, v) in details {
                    structured[k] = v;
                }
            }
            json!({
                "content": [{"type": "text", "text": text}],
                "structuredContent": structured,
                "isError": true,
            })
        }
    }
}

async fn dispatch(app: &App, origin: &Origin, name: &str, args: &Args) -> ToolResult {
    match name {
        "list_calendars" => list_calendars(app).await,
        "list_calendar_sets" => list_calendar_sets(app).await,
        "query_events" => query_events(app, args).await,
        "find_free_times" => find_free_times(app, args).await,
        "list_invitations" => list_invitations(app, args).await,
        "create_event" => create_event(app, origin, args).await,
        "update_event" => update_event(app, origin, args).await,
        "delete_event" => delete_event(app, origin, args).await,
        "respond_to_invitation" => respond(app, origin, args).await,
        other => Err(AppError::Internal(anyhow::anyhow!("tool {other} has no handler")).into()),
    }
}

// ----- calendars ---------------------------------------------------------------------

/// Calendars, their accounts and the sets.
struct Catalog {
    calendars: Vec<Calendar>,
    accounts: HashMap<String, Account>,
    sets: Vec<CalendarSet>,
}

impl Catalog {
    async fn load(app: &App) -> Result<Self, ToolError> {
        Ok(Catalog {
            calendars: app.list_calendars().await?,
            accounts: app
                .list_accounts()
                .await?
                .into_iter()
                .map(|a| (a.id.clone(), a))
                .collect(),
            sets: app.list_calendar_sets().await?,
        })
    }

    fn get(&self, id: &str) -> Option<&Calendar> {
        self.calendars.iter().find(|c| c.id == id)
    }

    fn require(&self, key: &str, id: &str) -> Result<&Calendar, ToolError> {
        self.get(id).ok_or_else(|| {
            ToolError::new(
                "calendar_not_found",
                format!(
                    "Den Kalender „{id}“ ({key}) gibt es nicht. Gültige IDs liefert list_calendars."
                ),
            )
        })
    }

    fn kind(&self, c: &Calendar) -> Option<AccountKind> {
        self.accounts
            .get(&c.account_id)
            .and_then(|a| AccountKind::parse(a.kind))
    }

    /// The provider lets Lichtung write here (ICS never; Microsoft not
    /// yet).
    fn writable(&self, c: &Calendar) -> bool {
        c.writable
            && !matches!(
                self.kind(c),
                Some(AccountKind::Ics | AccountKind::Microsoft)
            )
    }

    fn mcp_writable(&self, c: &Calendar) -> bool {
        self.writable(c) && c.mcp_write
    }

    fn reference(&self, id: &str) -> Value {
        match self.get(id) {
            Some(c) => json!({"id": c.id, "name": c.name, "mcp_writable": self.mcp_writable(c)}),
            None => json!({"id": id, "name": null, "mcp_writable": false}),
        }
    }

    /// The calendars a read tool looks at: `calendar_ids`, the calendars of
    /// `calendar_set_id`, or those passing `default`.
    fn select(&self, args: &Args, default: impl Fn(&Calendar) -> bool) -> ArgResult<Vec<String>> {
        let ids = args.strings("calendar_ids")?;
        let set = if args.raw("calendar_set_id").is_some() {
            args.non_empty_str("calendar_set_id")?
        } else {
            None
        };
        match (ids, set) {
            (Some(_), Some(_)) => Err(invalid(
                "Entweder calendar_ids oder calendar_set_id angeben, nicht beides.",
            )),
            (Some(ids), None) => {
                for id in &ids {
                    self.require("calendar_ids", id)?;
                }
                Ok(ids)
            }
            (None, Some(set_id)) => self
                .sets
                .iter()
                .find(|s| s.id == set_id)
                .map(|s| s.calendar_ids.clone())
                .ok_or_else(|| {
                    ToolError::new(
                        "calendar_set_not_found",
                        format!(
                            "Das Kalender-Set „{set_id}“ gibt es nicht. Gültige IDs liefert \
                             list_calendar_sets."
                        ),
                    )
                }),
            (None, None) => Ok(self
                .calendars
                .iter()
                .filter(|c| default(c))
                .map(|c| c.id.clone())
                .collect()),
        }
    }
}

async fn list_calendars(app: &App) -> ToolResult {
    let cat = Catalog::load(app).await?;
    let calendars: Vec<Value> = cat
        .calendars
        .iter()
        .map(|c| {
            let account = cat.accounts.get(&c.account_id);
            json!({
                "id": c.id,
                "name": c.name,
                "account": {
                    "id": c.account_id,
                    "kind": account.map(|a| a.kind),
                    "name": account.map(|a| a.name.as_str()),
                },
                "color": c.color,
                "writable": cat.writable(c),
                "mcp_writable": cat.mcp_writable(c),
                "can_invite": c.can_invite,
                "hidden": c.hidden,
                "openings_busy": c.openings_busy,
                "all_day_blocks": c.all_day_blocks,
                "is_work": c.is_work,
            })
        })
        .collect();
    Ok(json!({"calendars": calendars}))
}

async fn list_calendar_sets(app: &App) -> ToolResult {
    let sets: Vec<Value> = app
        .list_calendar_sets()
        .await?
        .into_iter()
        .map(|s| json!({"id": s.id, "name": s.name, "calendar_ids": s.calendar_ids}))
        .collect();
    Ok(json!({"calendar_sets": sets}))
}

// ----- reading -----------------------------------------------------------------------

fn output_zone(args: &Args) -> ArgResult<Tz> {
    match args.non_empty_str("time_zone")? {
        Some(raw) => parse_zone("time_zone", raw),
        None => Ok(default_zone()),
    }
}

fn time_arg(args: &Args, key: &str) -> ArgResult<Option<TimeArg>> {
    args.non_empty_str(key)?
        .map(|raw| parse_time(key, raw))
        .transpose()
}

/// Expanded occurrences (event JSON form) of `calendar_ids` in `[from, to)`.
async fn occurrences(
    app: &App,
    calendar_ids: &[String],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    tz: Tz,
    include_cancelled: bool,
) -> Result<(Vec<Value>, bool), ToolError> {
    if calendar_ids.is_empty() {
        return Ok((Vec::new(), false));
    }
    let found = app
        .occurrences(OccurrenceQuery {
            from: Some(fmt_at(from, Tz::UTC)),
            to: Some(fmt_at(to, Tz::UTC)),
            calendar_ids: Some(calendar_ids.join(",")),
            time_zone: Some(tz.name().to_owned()),
            include_cancelled: Some(include_cancelled),
        })
        .await?;
    let value = serde_json::to_value(&found).map_err(|e| AppError::Internal(e.into()))?;
    let list = value["occurrences"].as_array().cloned().unwrap_or_default();
    Ok((list, found.truncated))
}

/// Start, end (all-day: last day inclusive) and length in minutes.
fn span(start: &Value, end: &Value, tz: Tz) -> (Value, Value, Option<i64>) {
    match (when_of(start, tz), when_of(end, tz)) {
        (Some(When::Date(s)), Some(When::Date(e))) => {
            let last = (e - Duration::days(1)).max(s);
            (
                json!(fmt_date(s)),
                json!(fmt_date(last)),
                Some((e - s).num_days() * 24 * 60),
            )
        }
        (Some(s), Some(e)) => {
            let (s, e) = (s.instant(tz), e.instant(tz));
            (
                json!(fmt_at(s, tz)),
                json!(fmt_at(e, tz)),
                Some((e - s).num_minutes()),
            )
        }
        _ => (Value::Null, Value::Null, None),
    }
}

fn fmt_when(w: When, tz: Tz) -> String {
    match w {
        When::Date(d) => fmt_date(d),
        When::At(t) => fmt_at(t, tz),
    }
}

/// The zone of a stored time (`null` for dates and floating times).
fn zone_label(v: &Value) -> Value {
    v.get("time_zone").cloned().unwrap_or(Value::Null)
}

/// One occurrence as `query_events` shows it.
fn occurrence_view(o: &Value, tz: Tz, cat: &Catalog) -> Value {
    let (start, end, minutes) = span(&o["start"], &o["end"], tz);
    let rid = when_of(&o["recurrence_id"], tz).map(|w| fmt_when(w, tz));
    json!({
        "event_id": o["event_id"],
        "recurrence_id": rid,
        "is_recurring": rid.is_some(),
        "title": o["title"],
        "notes": o["notes"],
        "location": o["location"],
        "url": o["url"],
        "start": start,
        "end": end,
        "all_day": o["all_day"],
        "duration_minutes": minutes,
        "event_time_zone": zone_label(&o["start"]),
        "status": o["status"],
        "availability": o["availability"],
        "organizer": o["organizer"],
        "attendees": o["attendees"],
        "self_status": o["self_status"],
        "calendar": cat.reference(o["calendar_id"].as_str().unwrap_or_default()),
    })
}

fn matches_query(o: &Value, words: &[String]) -> bool {
    let haystack = ["title", "location", "notes"]
        .iter()
        .filter_map(|k| o[*k].as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    words.iter().all(|w| haystack.contains(w.as_str()))
}

async fn query_events(app: &App, args: &Args) -> ToolResult {
    let tz = output_zone(args)?;
    let start = parse_time("start", args.required_str("start")?)?;
    let end = parse_time("end", args.required_str("end")?)?;
    let (from, to) = range(start, end, tz)?;
    let words: Vec<String> = args
        .str("query")?
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let include_cancelled = args.bool("include_cancelled")?.unwrap_or(false);
    let limit = args.int("limit", 1, 1000, 250)? as usize;
    let cat = Catalog::load(app).await?;
    let ids = cat.select(args, |c| !c.hidden)?;
    let (list, mut truncated) = occurrences(app, &ids, from, to, tz, include_cancelled).await?;
    let mut events: Vec<Value> = list
        .iter()
        .filter(|o| matches_query(o, &words))
        .map(|o| occurrence_view(o, tz, &cat))
        .collect();
    if events.len() > limit {
        events.truncate(limit);
        truncated = true;
    }
    Ok(json!({
        "time_zone": tz.name(),
        "start": fmt_at(from, tz),
        "end": fmt_at(to, tz),
        "count": events.len(),
        "truncated": truncated,
        "events": events,
    }))
}

fn weekday(code: &str) -> Weekday {
    match code {
        "mo" => Weekday::Mon,
        "tu" => Weekday::Tue,
        "we" => Weekday::Wed,
        "th" => Weekday::Thu,
        "fr" => Weekday::Fri,
        "sa" => Weekday::Sat,
        _ => Weekday::Sun,
    }
}

async fn find_free_times(app: &App, args: &Args) -> ToolResult {
    let tz = output_zone(args)?;
    let start = parse_time("start", args.required_str("start")?)?;
    let end = parse_time("end", args.required_str("end")?)?;
    let (from, to) = range(start, end, tz)?;
    let min = Duration::minutes(args.int("min_duration_minutes", 5, 10080, 30)?);
    let buffer = Duration::minutes(args.int("buffer_minutes", 0, 1440, 0)?);
    let limit = args.int("limit", 1, 1000, 100)? as usize;
    let window = match (
        args.non_empty_str("day_start")?,
        args.non_empty_str("day_end")?,
    ) {
        (None, None) => None,
        (Some(s), Some(e)) => {
            let (s, e) = (parse_clock("day_start", s)?, parse_clock("day_end", e)?);
            if e <= s {
                return Err(invalid(
                    "day_end muss nach day_start liegen (ein Tagesfenster über Mitternacht geht \
                     nicht), z. B. day_start „09:00“ und day_end „17:00“.",
                ));
            }
            Some((s, e))
        }
        _ => {
            return Err(invalid(
                "day_start und day_end nur zusammen angeben, z. B. „09:00“ und „17:00“.",
            ))
        }
    };
    let weekdays = match args.strings("weekdays")? {
        None => None,
        Some(list) => {
            let mut out = Vec::new();
            for code in &list {
                if !WEEKDAYS.contains(&code.as_str()) {
                    return Err(invalid(format!(
                        "Ungültiger Wochentag „{code}“ in „weekdays“. Erlaubt: {}.",
                        WEEKDAYS.join(", ")
                    )));
                }
                out.push(weekday(code));
            }
            Some(out)
        }
    };
    let cat = Catalog::load(app).await?;
    let ids = cat.select(args, |c| c.openings_busy)?;
    let (list, truncated) = occurrences(app, &ids, from - buffer, to + buffer, tz, false).await?;
    if truncated {
        return Err(ToolError::new(
            "too_many_events",
            "Im Zeitraum liegen zu viele Termine, um freie Zeiten sicher zu bestimmen. Bitte einen \
             kürzeren Zeitraum wählen.",
        ));
    }
    let busy: Vec<Interval> = list
        .iter()
        .filter(|o| o["status"] != "cancelled")
        .filter(|o| o["availability"] != "free")
        .filter(|o| o["self_status"] != "declined")
        .filter(|o| {
            o["all_day"] != true
                || o["calendar_id"]
                    .as_str()
                    .and_then(|id| cat.get(id))
                    .is_some_and(|c| c.all_day_blocks)
        })
        .filter_map(|o| {
            let s = when_of(&o["start"], tz)?.instant(tz);
            let e = when_of(&o["end"], tz)?.instant(tz);
            Some((s, e.max(s)))
        })
        // A zero-length event blocks its moment.
        .map(|(s, e)| {
            if e == s {
                (s, s + Duration::minutes(1))
            } else {
                (s, e)
            }
        })
        .collect();
    let query = FreeQuery {
        from,
        to,
        tz,
        min,
        buffer,
        window,
        weekdays,
    };
    let mut slots = free_slots(&query, busy);
    let truncated = slots.len() > limit;
    slots.truncate(limit);
    let slots: Vec<Value> = slots
        .iter()
        .map(|(s, e)| {
            json!({
                "start": fmt_at(*s, tz),
                "end": fmt_at(*e, tz),
                "duration_minutes": (*e - *s).num_minutes(),
            })
        })
        .collect();
    Ok(json!({
        "time_zone": tz.name(),
        "start": fmt_at(from, tz),
        "end": fmt_at(to, tz),
        "calendar_ids": ids,
        "count": slots.len(),
        "truncated": truncated,
        "free": slots,
    }))
}

async fn list_invitations(app: &App, args: &Args) -> ToolResult {
    let tz = output_zone(args)?;
    let start = time_arg(args, "start")?.unwrap_or(TimeArg::At(Utc::now().fixed_offset()));
    let (from, to) = match time_arg(args, "end")? {
        Some(end) => range(start, end, tz)?,
        None => {
            let from = match start {
                TimeArg::Date(d) => midnight(d, tz),
                TimeArg::At(t) => t.with_timezone(&Utc),
            };
            (from, from + Duration::days(INVITATION_DAYS))
        }
    };
    let limit = args.int("limit", 1, 1000, 100)? as usize;
    let cat = Catalog::load(app).await?;
    let ids = cat.select(args, |_| true)?;
    let (list, mut truncated) = occurrences(app, &ids, from, to, tz, false).await?;
    // One entry per series (answered as a whole) and per changed
    // occurrence with its own open request.
    let mut order: Vec<(String, Option<String>)> = Vec::new();
    let mut groups: HashMap<(String, Option<String>), (Value, usize)> = HashMap::new();
    for o in list
        .iter()
        .filter(|o| o["self_status"] == "needs_action" && o["status"] != "cancelled")
    {
        let single = o["is_override"] == true;
        let key = (
            o["event_id"].as_str().unwrap_or_default().to_owned(),
            single
                .then(|| when_of(&o["recurrence_id"], tz).map(|w| fmt_when(w, tz)))
                .flatten(),
        );
        match groups.get_mut(&key) {
            Some((_, n)) => *n += 1,
            None => {
                order.push(key.clone());
                groups.insert(key, (o.clone(), 1));
            }
        }
    }
    let mut invitations: Vec<Value> = order
        .iter()
        .map(|key| {
            let (o, n) = &groups[key];
            let mut v = occurrence_view(o, tz, &cat);
            v["occurrence"] = json!(key.1);
            v["pending_occurrences"] = json!(n);
            v["can_respond"] = v["calendar"]["mcp_writable"].clone();
            v
        })
        .collect();
    if invitations.len() > limit {
        invitations.truncate(limit);
        truncated = true;
    }
    Ok(json!({
        "time_zone": tz.name(),
        "start": fmt_at(from, tz),
        "end": fmt_at(to, tz),
        "count": invitations.len(),
        "truncated": truncated,
        "invitations": invitations,
    }))
}

// ----- writing -----------------------------------------------------------------------

/// `start`/`end` of a write in the form of the sync API (all-day `end`
/// exclusive there, inclusive here).
fn event_times(start: TimeArg, end: TimeArg, zone: Tz) -> ArgResult<(Value, Value)> {
    match (start, end) {
        (TimeArg::Date(s), TimeArg::Date(e)) => {
            if e < s {
                return Err(invalid(
                    "end liegt vor start. Bei ganztägigen Terminen ist end der letzte Tag \
                     (einschließlich); ein eintägiger Termin hat start = end.",
                ));
            }
            Ok((date_input(s), date_input(e + Duration::days(1))))
        }
        (TimeArg::At(s), TimeArg::At(e)) => {
            if e < s {
                return Err(invalid("end liegt vor start."));
            }
            Ok((date_time_input(s, zone), date_time_input(e, zone)))
        }
        _ => Err(invalid(
            "start und end müssen dieselbe Form haben: beide Zeitpunkte mit Offset oder beide \
             ein Datum (ganztägig).",
        )),
    }
}

fn end_input(end: TimeArg, zone: Tz) -> Value {
    match end {
        TimeArg::Date(e) => date_input(e + Duration::days(1)),
        TimeArg::At(e) => date_time_input(e, zone),
    }
}

fn zone_arg(args: &Args) -> ArgResult<Option<Tz>> {
    args.non_empty_str("time_zone")?
        .map(|raw| parse_zone("time_zone", raw))
        .transpose()
}

fn opt_text(args: &Args, key: &str, nullable: bool, out: &mut Map<String, Value>) -> ArgResult<()> {
    if args.is_null(key) {
        if !nullable {
            return Err(invalid(format!("„{key}“ darf nicht null sein.")));
        }
        out.insert(key.to_owned(), Value::Null);
    } else if let Some(v) = args.str(key)? {
        out.insert(key.to_owned(), json!(v));
    }
    Ok(())
}

/// The fields shared by create and update (without times and recurrence).
fn common_fields(args: &Args, nullable: bool) -> ArgResult<Map<String, Value>> {
    let mut out = Map::new();
    if args.is_null("title") {
        return Err(invalid(
            "Der Titel lässt sich nicht entfernen; title weglassen, wenn er bleiben soll.",
        ));
    }
    if let Some(t) = args.non_empty_str("title")? {
        out.insert("title".into(), json!(t));
    }
    for key in ["location", "notes", "url"] {
        opt_text(args, key, nullable, &mut out)?;
    }
    if let Some(a) = args.code("availability", AVAILABILITY)? {
        out.insert("availability".into(), json!(a));
    } else if args.is_null("availability") {
        return Err(invalid(
            "„availability“ muss busy oder free sein, nicht null.",
        ));
    }
    if args.is_null("alarms") {
        out.insert("alarms".into(), json!([]));
    } else if let Some(v) = args.value("alarms") {
        let items = v.as_array().ok_or_else(|| {
            invalid("„alarms“ muss eine Liste von Minuten vor Beginn sein, z. B. [10, 60].")
        })?;
        let alarms = items
            .iter()
            .map(|m| {
                m.as_i64()
                    .map(|m| json!({"minutes_before": m}))
                    .ok_or_else(|| {
                        invalid("„alarms“ darf nur ganze Zahlen (Minuten vor Beginn) enthalten.")
                    })
            })
            .collect::<ArgResult<Vec<_>>>()?;
        out.insert("alarms".into(), json!(alarms));
    }
    if args.is_null("attendees") {
        out.insert("attendees".into(), json!([]));
    } else if let Some(v) = args.value("attendees") {
        let items = v.as_array().ok_or_else(|| {
            invalid("„attendees“ muss eine Liste sein, z. B. [{\"email\": \"…\"}].")
        })?;
        let mut list = Vec::new();
        for item in items {
            let a = item.as_object().ok_or_else(|| {
                invalid("Jeder Eintrag in „attendees“ ist ein Objekt mit email, z. B. {\"email\": \"…\"}.")
            })?;
            let a = Args::new(a.clone());
            a.check(["email", "name", "role"], ["email"])?;
            let mut entry = json!({"email": a.required_str("email")?});
            if let Some(n) = a.str("name")? {
                entry["name"] = json!(n);
            }
            if let Some(r) = a.code("role", ROLES)? {
                entry["role"] = json!(r);
            }
            list.push(entry);
        }
        out.insert("attendees".into(), json!(list));
    }
    Ok(out)
}

fn rrule_arg(args: &Args) -> ArgResult<Option<String>> {
    Ok(args
        .non_empty_str("recurrence")?
        .map(|r| r.strip_prefix("RRULE:").unwrap_or(r).trim().to_owned()))
}

/// A hint for what to do after a refused write.
fn hint(code: &str) -> &'static str {
    match code {
        "mcp_write_forbidden" => {
            " Wähle mit list_calendars einen Kalender mit mcp_writable = true oder bitte den \
             Nutzer, den Kalender in der App für Claude freizugeben."
        }
        "calendar_read_only" => " Wähle mit list_calendars einen Kalender mit mcp_writable = true.",
        "event_not_found" => " Gültige event_id liefert query_events.",
        "occurrence_not_found" => {
            " occurrence muss eine recurrence_id aus query_events für diesen Termin sein."
        }
        "scope_not_supported" => " Bitte einen anderen scope wählen.",
        _ => "",
    }
}

/// The head of an event (event JSON) after a write.
fn event_view(ev: &Value, tz: Tz, cat: &Catalog) -> Value {
    let (start, end, minutes) = span(&ev["start"], &ev["end"], tz);
    json!({
        "event_id": ev["id"],
        "calendar": cat.reference(ev["calendar_id"].as_str().unwrap_or_default()),
        "title": ev["title"],
        "start": start,
        "end": end,
        "all_day": ev["all_day"],
        "duration_minutes": minutes,
        "event_time_zone": zone_label(&ev["start"]),
        "recurrence": ev["recurrence"].get("rrule").cloned().unwrap_or(Value::Null),
        "location": ev["location"],
        "notes": ev["notes"],
        "url": ev["url"],
        "status": ev["status"],
        "availability": ev["availability"],
        "attendees": ev["attendees"],
        "self_status": ev["self_status"],
        "alarms": ev["alarms"],
    })
}

/// Maps the outcome of the write path onto a tool result.
fn outcome(o: WriteOutcome, tz: Tz, cat: &Catalog) -> ToolResult {
    match o {
        WriteOutcome::Applied {
            events,
            deleted_event_ids,
        } => Ok(json!({
            "status": "applied",
            "events": events.iter().map(|e| event_view(e, tz, cat)).collect::<Vec<_>>(),
            "deleted_event_ids": deleted_event_ids,
        })),
        WriteOutcome::Conflict { conflict, events } => Err(ToolError::new(
            "conflict",
            format!(
                "{} Der Anbieter gewinnt; current zeigt den aktuellen Stand (leer, wenn der Termin \
                 gelöscht wurde). Bitte den Stand prüfen und neu entscheiden, statt die Änderung \
                 blind zu wiederholen.",
                conflict.message
            ),
        )
        .with_details(json!({
            "conflict_id": conflict.id,
            "current": events.iter().map(|e| event_view(e, tz, cat)).collect::<Vec<_>>(),
        }))),
        WriteOutcome::Rejected { code, message } => {
            Err(ToolError::new(code, format!("{message}{}", hint(code))))
        }
        WriteOutcome::Retry {
            code,
            message,
            retry_after_seconds,
        } => {
            let wait = retry_after_seconds
                .map(|s| format!(" Bitte in etwa {s} Sekunden erneut versuchen."))
                .unwrap_or_else(|| " Bitte später erneut versuchen.".to_owned());
            Err(ToolError::new(code, format!("{message}{wait}"))
                .with_details(json!({"retry_after_seconds": retry_after_seconds})))
        }
    }
}

async fn create_event(app: &App, origin: &Origin, args: &Args) -> ToolResult {
    let cat = Catalog::load(app).await?;
    let calendar_id = args.required_str("calendar_id")?;
    cat.require("calendar_id", calendar_id)?;
    let start = parse_time("start", args.required_str("start")?)?;
    let end = parse_time("end", args.required_str("end")?)?;
    let explicit = zone_arg(args)?;
    let zone = match start {
        TimeArg::At(at) => pick_zone(explicit, None, at),
        TimeArg::Date(_) => explicit.unwrap_or_else(default_zone),
    };
    let (start, end) = event_times(start, end, zone)?;
    let mut event = common_fields(args, false)?;
    if !event.contains_key("title") {
        return Err(invalid("Parameter „title“ fehlt."));
    }
    event.insert("start".into(), start);
    event.insert("end".into(), end);
    if let Some(rule) = rrule_arg(args)? {
        event.insert("recurrence".into(), json!({"rrule": rule}));
    }
    let mutation = json!({"op": "create_event", "calendar_id": calendar_id, "event": event});
    let o = app.apply_mutation(origin, &mutation).await?;
    outcome(o, explicit.unwrap_or_else(default_zone), &cat)
}

/// The stored event of an update, delete or answer (`None` if unknown;
/// the write path then answers `event_not_found` or a conflict).
async fn stored_event(app: &App, id: &str) -> Result<Option<Value>, ToolError> {
    match app.event(id).await {
        Ok(ev) => Ok(Some(ev)),
        Err(AppError::NotFound(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn is_series(ev: &Value) -> bool {
    ev["recurrence"].is_object() || ev["overrides"].as_array().is_some_and(|o| !o.is_empty())
}

/// Scope and occurrence of an update or delete (sync API form).
fn scope_of(args: &Args, ev: Option<&Value>) -> ArgResult<(&'static str, Option<Value>)> {
    let scope = args.code("scope", SCOPES)?;
    let occurrence = time_arg(args, "occurrence")?;
    let Some(ev) = ev else {
        return Ok((scope.unwrap_or("all"), occurrence.map(occurrence_input)));
    };
    if !is_series(ev) {
        // A single event: there is only "all".
        return Ok(("all", None));
    }
    let title = ev["title"].as_str().unwrap_or_default();
    let Some(scope) = scope else {
        return Err(ToolError::new(
            "scope_required",
            format!(
                "Der Termin „{title}“ ist eine Serie. Gib scope an: this (nur das Vorkommen \
                 occurrence), this_and_following (dieses und alle folgenden) oder all (die ganze \
                 Serie). Für this und this_and_following occurrence = recurrence_id aus \
                 query_events angeben."
            ),
        ));
    };
    if scope != "all" && occurrence.is_none() {
        return Err(ToolError::new(
            "occurrence_required",
            format!(
                "scope {scope} braucht occurrence: die recurrence_id des gemeinten Vorkommens aus \
                 query_events (z. B. „2026-11-09T09:00:00+01:00“)."
            ),
        ));
    }
    let occurrence = if scope == "all" { None } else { occurrence };
    Ok((scope, occurrence.map(occurrence_input)))
}

fn existing_zone(ev: Option<&Value>) -> Option<Tz> {
    ev.and_then(|e| e["start"]["time_zone"].as_str())
        .and_then(|z| parse_zone("time_zone", z).ok())
}

/// Start and end of the occurrence an update targets (an override's own
/// times if there is one, else the head's).
fn target_span(ev: &Value, occurrence: Option<&Value>, tz: Tz) -> Option<(When, When)> {
    let wanted = occurrence.and_then(|o| when_of(o, tz));
    let source = wanted
        .and_then(|w| {
            ev["overrides"].as_array()?.iter().find(|o| {
                when_of(&o["recurrence_id"], tz).is_some_and(|r| match (r, w) {
                    (When::Date(a), When::Date(b)) => a == b,
                    (a, b) => a.instant(tz) == b.instant(tz),
                })
            })
        })
        .unwrap_or(ev);
    Some((when_of(&source["start"], tz)?, when_of(&source["end"], tz)?))
}

async fn update_event(app: &App, origin: &Origin, args: &Args) -> ToolResult {
    let event_id = args.required_str("event_id")?;
    let cat = Catalog::load(app).await?;
    let ev = stored_event(app, event_id).await?;
    let (scope, occurrence) = scope_of(args, ev.as_ref())?;
    let explicit = zone_arg(args)?;
    let tz = explicit.unwrap_or_else(default_zone);
    let mut changes = common_fields(args, true)?;

    let start = time_arg(args, "start")?;
    let end = time_arg(args, "end")?;
    let zone_for = |at: chrono::DateTime<chrono::FixedOffset>| {
        pick_zone(explicit, existing_zone(ev.as_ref()), at)
    };
    match (start, end) {
        (Some(s), Some(e)) => {
            let zone = match s {
                TimeArg::At(at) => zone_for(at),
                TimeArg::Date(_) => tz,
            };
            let (s, e) = event_times(s, e, zone)?;
            changes.insert("start".into(), s);
            changes.insert("end".into(), e);
        }
        (None, Some(e)) => {
            let zone = match e {
                TimeArg::At(at) => zone_for(at),
                TimeArg::Date(_) => tz,
            };
            changes.insert("end".into(), end_input(e, zone));
        }
        (Some(s), None) => {
            // Keep the length of the targeted occurrence.
            let old = ev
                .as_ref()
                .and_then(|ev| target_span(ev, occurrence.as_ref(), tz));
            let new_end =
                match (s, old) {
                    (TimeArg::Date(d), Some((When::Date(os), When::Date(oe)))) => {
                        TimeArg::Date(d + (oe - os) - Duration::days(1))
                    }
                    (TimeArg::At(at), Some((os @ When::At(_), oe @ When::At(_)))) => {
                        TimeArg::At(at + (oe.instant(tz) - os.instant(tz)))
                    }
                    _ => return Err(invalid(
                        "Bei einem Wechsel zwischen ganztägig und mit Uhrzeit bitte start und end \
                         angeben.",
                    )),
                };
            let zone = match s {
                TimeArg::At(at) => zone_for(at),
                TimeArg::Date(_) => tz,
            };
            let (s, e) = event_times(s, new_end, zone)?;
            changes.insert("start".into(), s);
            changes.insert("end".into(), e);
        }
        (None, None) => {
            if explicit.is_some() {
                return Err(invalid(
                    "time_zone wirkt nur zusammen mit start oder end. Um nur die Zeitzone zu \
                     ändern, start und end mit time_zone angeben.",
                ));
            }
        }
    }
    if args.is_null("recurrence") {
        changes.insert("recurrence".into(), Value::Null);
    } else if let Some(rule) = rrule_arg(args)? {
        let current = ev.as_ref().map(|e| &e["recurrence"]);
        changes.insert(
            "recurrence".into(),
            json!({
                "rrule": rule,
                "rdates": current.and_then(|r| r.get("rdates")).cloned().unwrap_or(json!([])),
                "exdates": current.and_then(|r| r.get("exdates")).cloned().unwrap_or(json!([])),
            }),
        );
    }
    if let Some(target) = args.non_empty_str("move_to_calendar_id")? {
        cat.require("move_to_calendar_id", target)?;
        changes.insert("calendar_id".into(), json!(target));
    }
    if changes.is_empty() {
        return Err(invalid(
            "Keine Änderung angegeben. Mindestens ein Feld wie title, start, end, location, notes, \
             url, recurrence, alarms, availability, attendees oder move_to_calendar_id angeben.",
        ));
    }
    let mut mutation = json!({
        "op": "update_event",
        "event_id": event_id,
        "base_version": ev.as_ref().and_then(|e| e["version"].as_i64()).unwrap_or(0),
        "scope": scope,
        "changes": changes,
    });
    if let Some(o) = occurrence {
        mutation["occurrence"] = o;
    }
    let o = app.apply_mutation(origin, &mutation).await?;
    outcome(o, tz, &cat)
}

async fn delete_event(app: &App, origin: &Origin, args: &Args) -> ToolResult {
    let event_id = args.required_str("event_id")?;
    let cat = Catalog::load(app).await?;
    let ev = stored_event(app, event_id).await?;
    let (scope, occurrence) = scope_of(args, ev.as_ref())?;
    let mut mutation = json!({
        "op": "delete_event",
        "event_id": event_id,
        "base_version": ev.as_ref().and_then(|e| e["version"].as_i64()).unwrap_or(0),
        "scope": scope,
    });
    if let Some(o) = occurrence {
        mutation["occurrence"] = o;
    }
    let o = app.apply_mutation(origin, &mutation).await?;
    outcome(o, default_zone(), &cat)
}

async fn respond(app: &App, origin: &Origin, args: &Args) -> ToolResult {
    let event_id = args.required_str("event_id")?;
    let response = args
        .code("response", RESPONSES)?
        .ok_or_else(|| invalid("Parameter „response“ fehlt."))?;
    let cat = Catalog::load(app).await?;
    let ev = stored_event(app, event_id).await?;
    let occurrence = time_arg(args, "occurrence")?
        .filter(|_| ev.as_ref().is_none_or(is_series))
        .map(occurrence_input);
    let mut mutation = json!({"op": "respond", "event_id": event_id, "status": response});
    if let Some(o) = occurrence {
        mutation["occurrence"] = o;
    }
    if let Some(c) = args.str("comment")? {
        mutation["comment"] = json!(c);
    }
    let o = app.apply_mutation(origin, &mutation).await?;
    outcome(o, default_zone(), &cat)
}
