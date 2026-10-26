//! Tool definitions (names, descriptions, input schemas, annotations).
//!
//! Names and schemas are contract (`docs/mcp.md`) and frozen by the
//! snapshot test `tests/snapshots/mcp_tools.json`.
//!
//! Schemas avoid top-level `oneOf`/`anyOf`, which several clients reject;
//! "date or instant" is described in words and checked at runtime.

use serde_json::{json, Map, Value};

/// One tool.
pub struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
}

impl ToolDef {
    /// The entry in the `tools/list` result.
    pub fn to_json(&self) -> Value {
        let mut annotations = json!({
            "title": self.title,
            "readOnlyHint": self.read_only,
            "openWorldHint": false,
        });
        if !self.read_only {
            annotations["destructiveHint"] = json!(self.destructive);
            annotations["idempotentHint"] = json!(self.idempotent);
        }
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input_schema,
            "annotations": annotations,
        })
    }

    /// Property names allowed by the input schema.
    pub fn properties(&self) -> impl Iterator<Item = &str> {
        self.input_schema["properties"]
            .as_object()
            .into_iter()
            .flat_map(|p| p.keys().map(String::as_str))
    }

    /// Required property names.
    pub fn required(&self) -> impl Iterator<Item = &str> {
        self.input_schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
    }
}

// ----- schema fragments ------------------------------------------------------

fn string(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

fn nullable_string(description: &str) -> Value {
    json!({"type": ["string", "null"], "description": description})
}

fn boolean(description: &str) -> Value {
    json!({"type": "boolean", "description": description})
}

fn integer(description: &str, min: i64, max: i64, default: Option<i64>) -> Value {
    let mut v = json!({
        "type": "integer",
        "description": description,
        "minimum": min,
        "maximum": max,
    });
    if let Some(d) = default {
        v["default"] = json!(d);
    }
    v
}

fn enumeration(codes: &[&str], description: &str) -> Value {
    json!({"type": "string", "enum": codes, "description": description})
}

fn string_list(description: &str) -> Value {
    json!({
        "type": "array",
        "items": {"type": "string", "minLength": 1},
        "description": description,
    })
}

fn object(properties: Vec<(&str, Value)>, required: &[&str]) -> Value {
    let mut props = Map::new();
    for (name, schema) in properties {
        props.insert(name.to_owned(), schema);
    }
    json!({
        "type": "object",
        "properties": props,
        "required": required,
        "additionalProperties": false,
    })
}

fn with(
    mut base: Vec<(&'static str, Value)>,
    more: Vec<(&'static str, Value)>,
) -> Vec<(&'static str, Value)> {
    base.extend(more);
    base
}

pub const WEEKDAYS: &[&str] = &["mo", "tu", "we", "th", "fr", "sa", "su"];
pub const SCOPES: &[&str] = &["this", "this_and_following", "all"];
pub const RESPONSES: &[&str] = &["accepted", "tentative", "declined"];
pub const AVAILABILITY: &[&str] = &["busy", "free"];
pub const ROLES: &[&str] = &["required", "optional", "chair", "non_participant"];

const RANGE_START: &str = "Beginn des Zeitraums (einschließlich): Datum JJJJ-MM-TT (Tagesbeginn \
    in time_zone) oder Zeitpunkt nach RFC 3339 mit Offset, z. B. 2026-11-03T08:00:00+01:00.";
const RANGE_END: &str = "Ende des Zeitraums: Datum JJJJ-MM-TT (dieser Tag zählt noch ganz mit) \
    oder Zeitpunkt nach RFC 3339 mit Offset (ausschließlich). Höchstens 400 Tage nach start.";
const OUTPUT_ZONE: &str = "IANA-Zeitzone der Ausgabe und für Datumsangaben in start/end, z. B. \
    Europe/Berlin. Standard: Zeitzone des Servers.";
const CALENDAR_IDS: &str = "Nur diese Kalender (IDs aus list_calendars).";
const CALENDAR_SET_ID: &str =
    "Nur die Kalender dieses Kalender-Sets (ID aus list_calendar_sets); nicht zusammen mit calendar_ids.";
const LIMIT: &str = "Höchstzahl der Einträge; bei mehr ist truncated true.";

const EVENT_START: &str = "Beginn: Zeitpunkt nach RFC 3339 mit Offset, z. B. \
    2026-11-03T14:00:00+01:00, oder für ganztägige Termine ein Datum JJJJ-MM-TT. Ohne Offset gibt \
    es einen Fehler.";
const EVENT_END: &str = "Ende in derselben Form wie start: Zeitpunkt mit Offset (ausschließlich) \
    oder bei ganztägigen Terminen der letzte Tag als Datum (einschließlich; ein eintägiger Termin \
    hat start = end).";
const EVENT_ZONE: &str = "IANA-Zeitzone, in der der Termin gespeichert wird (Wanduhrzeit, \
    Sommerzeit bei Serien), z. B. Europe/Berlin. Der Zeitpunkt bleibt der angegebene. Standard: \
    die Zeitzone des Servers, wenn ihr Offset zum angegebenen passt, sonst UTC.";
const OCCURRENCE: &str = "Bei Serien das gemeinte Vorkommen: recurrence_id aus query_events \
    (Zeitpunkt mit Offset oder Datum bei ganztägigen Serien).";
const SCOPE: &str = "Geltungsbereich bei Serien (dort Pflicht): this = nur dieses Vorkommen, \
    this_and_following = dieses und alle folgenden, all = die ganze Serie. this und \
    this_and_following brauchen occurrence. Bei Einzelterminen weglassen (gilt als all).";

fn attendees(nullable: bool) -> Value {
    json!({
        "type": if nullable { json!(["array", "null"]) } else { json!("array") },
        "description": format!(
            "Teilnehmer; der Anbieter verschickt echte Einladungen (bei Änderungen \
             Aktualisierungen). Ersetzt die ganze Liste; bekannte Teilnehmer behalten ihre \
             Antwort.{}",
            if nullable { " [] oder null entfernt alle." } else { "" }
        ),
        "items": {
            "type": "object",
            "properties": {
                "email": {"type": "string", "description": "E-Mail-Adresse."},
                "name": {"type": "string", "description": "Anzeigename (optional)."},
                "role": {"type": "string", "enum": ROLES, "description": "Standard: required."},
            },
            "required": ["email"],
            "additionalProperties": false,
        },
    })
}

fn alarms(nullable: bool) -> Value {
    json!({
        "type": if nullable { json!(["array", "null"]) } else { json!("array") },
        "description": format!(
            "Erinnerungen in Minuten vor Beginn, z. B. [10, 60]; ersetzt alle Erinnerungen.{}",
            if nullable { " [] oder null entfernt alle." } else { "" }
        ),
        "items": {"type": "integer", "minimum": -40320, "maximum": 40320},
    })
}

/// Fields of create_event (`nullable` false) and update_event.
fn event_fields(nullable: bool) -> Vec<(&'static str, Value)> {
    let text = |d: &str| {
        if nullable {
            nullable_string(&format!("{d} null entfernt den Wert."))
        } else {
            string(d)
        }
    };
    vec![
        ("title", string("Titel.")),
        ("start", string(EVENT_START)),
        ("end", string(EVENT_END)),
        ("time_zone", string(EVENT_ZONE)),
        ("location", text("Ort.")),
        ("notes", text("Notiz bzw. Beschreibung.")),
        ("url", text("Link (z. B. Videokonferenz).")),
        (
            "recurrence",
            if nullable {
                nullable_string(
                    "Wiederholung als RRULE nach RFC 5545 (ohne „RRULE:“), z. B. \
                     FREQ=WEEKLY;BYDAY=MO;COUNT=10; ersetzt die Regel, Ausnahmen bleiben. null \
                     beendet die Wiederholung. Nicht mit scope this.",
                )
            } else {
                string(
                    "Wiederholung als RRULE nach RFC 5545 (ohne „RRULE:“), z. B. \
                     FREQ=WEEKLY;BYDAY=MO;COUNT=10 oder FREQ=DAILY;UNTIL=20261130T225959Z.",
                )
            },
        ),
        ("alarms", alarms(nullable)),
        (
            "availability",
            enumeration(
                AVAILABILITY,
                "busy (blockiert, Standard) oder free (zählt nicht als belegt, z. B. für \
                 Erinnerungen oder Fokuszeit, die Buchungen nicht verhindern soll).",
            ),
        ),
        ("attendees", attendees(nullable)),
    ]
}

/// All tool definitions in `tools/list` order.
pub fn definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "list_calendars",
            title: "Kalender auflisten",
            description: "Listet alle Kalender mit id, Name, Konto (Art und Name), Farbe und \
                Rechten. writable: der Anbieter erlaubt Schreiben. mcp_writable: Claude darf hier \
                schreiben (writable und in der App freigegeben); nur solche Kalender kommen für \
                create_event, update_event, delete_event und respond_to_invitation in Frage. \
                hidden: in der App ausgeblendet. openings_busy: zählt für freie Zeiten. \
                all_day_blocks: ganztägige Termine blockieren freie Zeiten.",
            input_schema: object(vec![], &[]),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "list_calendar_sets",
            title: "Kalender-Sets auflisten",
            description: "Listet die Kalender-Sets (Gruppen wie „Arbeit“ oder „Privat“) mit id, \
                Name und calendar_ids. Eine Set-ID passt als calendar_set_id in query_events und \
                find_free_times.",
            input_schema: object(vec![], &[]),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "query_events",
            title: "Termine abfragen",
            description: "Liefert die Termine eines Zeitraums, Serien als einzelne Vorkommen, \
                nach Beginn sortiert. Ohne query kommt alles im Zeitraum; mit query nur Termine, \
                deren Titel, Ort oder Notiz alle Wörter enthält (Groß- und Kleinschreibung egal). \
                Ohne calendar_ids und calendar_set_id: alle nicht ausgeblendeten Kalender. \
                Jedes Vorkommen hat event_id und recurrence_id (bei Serien; für occurrence in \
                update_event, delete_event, respond_to_invitation), Titel, notes, Ort, url, \
                start/end in time_zone (ganztägig als Datum, end dann einschließlich), \
                duration_minutes, all_day, event_time_zone (Zeitzone des Termins), status \
                (confirmed, tentative, cancelled), availability (busy, free), organizer, \
                attendees, self_status (eigene Antwort; needs_action = offene Anfrage) und \
                calendar.",
            input_schema: object(
                vec![
                    ("start", string(RANGE_START)),
                    ("end", string(RANGE_END)),
                    ("query", string("Suchbegriff (Wörter, alle müssen vorkommen).")),
                    ("calendar_ids", string_list(CALENDAR_IDS)),
                    ("calendar_set_id", string(CALENDAR_SET_ID)),
                    (
                        "include_cancelled",
                        boolean("Auch abgesagte Termine liefern (Standard: false)."),
                    ),
                    ("time_zone", string(OUTPUT_ZONE)),
                    ("limit", integer(LIMIT, 1, 1000, Some(250))),
                ],
                &["start", "end"],
            ),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "find_free_times",
            title: "Freie Zeiten finden",
            description: "Sucht freie Zeiten von mindestens min_duration_minutes im Zeitraum, nach \
                denselben Regeln wie Buchungslinks: Termine in den einbezogenen Kalendern \
                blockieren, außer abgesagte, als frei markierte (availability free) und eigene \
                abgelehnte Einladungen; ganztägige Termine blockieren nur in Kalendern mit \
                all_day_blocks. Ohne calendar_ids und calendar_set_id zählen alle Kalender mit \
                openings_busy. Optional nur innerhalb eines Tagesfensters (day_start bis day_end \
                in time_zone) und an bestimmten Wochentagen, mit Puffer vor und nach jedem \
                Termin. Liefert Zeitfenster mit start, end (in time_zone) und duration_minutes; \
                ein Fenster kann länger als nötig sein.",
            input_schema: object(
                vec![
                    ("start", string(RANGE_START)),
                    ("end", string(RANGE_END)),
                    (
                        "min_duration_minutes",
                        integer("Mindestdauer eines freien Fensters in Minuten.", 5, 10080, None),
                    ),
                    ("calendar_ids", string_list(CALENDAR_IDS)),
                    ("calendar_set_id", string(CALENDAR_SET_ID)),
                    (
                        "day_start",
                        string("Beginn des Tagesfensters hh:mm in time_zone, z. B. 09:00; nur mit day_end."),
                    ),
                    (
                        "day_end",
                        string("Ende des Tagesfensters hh:mm in time_zone, z. B. 17:00; nach day_start."),
                    ),
                    (
                        "weekdays",
                        json!({
                            "type": "array",
                            "items": {"type": "string", "enum": WEEKDAYS},
                            "description": "Nur diese Wochentage (in time_zone), z. B. \
                                [\"mo\",\"tu\",\"we\",\"th\",\"fr\"] für werktags. Standard: alle.",
                        }),
                    ),
                    (
                        "buffer_minutes",
                        integer("Puffer vor und nach jedem Termin in Minuten.", 0, 1440, Some(0)),
                    ),
                    ("time_zone", string(OUTPUT_ZONE)),
                    ("limit", integer(LIMIT, 1, 1000, Some(100))),
                ],
                &["start", "end", "min_duration_minutes"],
            ),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "list_invitations",
            title: "Offene Terminanfragen",
            description: "Listet offene Terminanfragen (eigene Antwort needs_action), die nicht \
                abgesagt sind, ab jetzt (Standard: die nächsten 180 Tage). Eine Serie erscheint \
                einmal (mit dem nächsten Vorkommen und pending_occurrences); ein einzeln \
                geändertes Vorkommen mit eigener offener Anfrage erscheint mit occurrence. \
                can_respond sagt, ob Claude in diesem Kalender antworten darf (mcp_writable).",
            input_schema: object(
                vec![
                    ("start", string("Beginn wie bei query_events; Standard: jetzt.")),
                    ("end", string("Ende wie bei query_events; Standard: 180 Tage nach start.")),
                    ("calendar_ids", string_list(CALENDAR_IDS)),
                    ("time_zone", string(OUTPUT_ZONE)),
                    ("limit", integer(LIMIT, 1, 1000, Some(100))),
                ],
                &[],
            ),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "create_event",
            title: "Termin anlegen",
            description: "Legt einen Termin an. calendar_id ist Pflicht, es gibt keinen \
                Standardkalender: vorher mit list_calendars einen Kalender mit mcp_writable = true \
                wählen. Zeiten immer mit Offset (2026-11-03T14:00:00+01:00) oder ganztägig als \
                Datum (end = letzter Tag, einschließlich). Mit attendees verschickt der Anbieter \
                echte Einladungen. Liefert den angelegten Termin mit event_id.",
            input_schema: object(
                with(
                    vec![(
                        "calendar_id",
                        string("Zielkalender (id aus list_calendars, mcp_writable muss true sein)."),
                    )],
                    event_fields(false),
                ),
                &["calendar_id", "title", "start", "end"],
            ),
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolDef {
            name: "update_event",
            title: "Termin ändern",
            description: "Ändert einen Termin: nur die angegebenen Felder, der Rest bleibt \
                (auch Felder, die Lichtung nicht kennt). Bei Serien ist scope Pflicht (this, \
                this_and_following, all) und bei this bzw. this_and_following occurrence \
                (recurrence_id aus query_events). Wird nur start geändert, bleibt die Dauer \
                erhalten. move_to_calendar_id verschiebt den ganzen Termin in einen anderen \
                Kalender. Hat sich der Termin inzwischen beim Anbieter geändert, gewinnt der \
                Anbieter: Das Ergebnis ist ein Fehler (conflict) mit dem aktuellen Stand.",
            input_schema: object(
                with(
                    vec![
                        ("event_id", string("event_id aus query_events.")),
                        ("occurrence", string(OCCURRENCE)),
                        ("scope", enumeration(SCOPES, SCOPE)),
                    ],
                    with(
                        event_fields(true),
                        vec![(
                            "move_to_calendar_id",
                            string(
                                "In diesen Kalender verschieben (id aus list_calendars, \
                                 mcp_writable muss true sein; nur ganze Termine, scope all).",
                            ),
                        )],
                    ),
                ),
                &["event_id"],
            ),
            read_only: false,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "delete_event",
            title: "Termin löschen",
            description: "Löscht einen Termin beim Anbieter. Bei Serien ist scope Pflicht: this \
                (nur dieses Vorkommen), this_and_following (dieses und alle folgenden) oder all \
                (die ganze Serie); this und this_and_following brauchen occurrence \
                (recurrence_id aus query_events). Teilnehmer bekommen je nach Anbieter eine \
                Absage.",
            input_schema: object(
                vec![
                    ("event_id", string("event_id aus query_events.")),
                    ("occurrence", string(OCCURRENCE)),
                    ("scope", enumeration(SCOPES, SCOPE)),
                ],
                &["event_id"],
            ),
            read_only: false,
            destructive: true,
            idempotent: false,
        },
        ToolDef {
            name: "respond_to_invitation",
            title: "Auf Einladung antworten",
            description: "Beantwortet eine Terminanfrage: accepted (zusagen), tentative \
                (vielleicht) oder declined (absagen); der Anbieter schickt die Antwort an den \
                Organisator. Ohne occurrence gilt die Antwort für die ganze Serie, mit occurrence \
                nur für dieses Vorkommen. Geht nur in Kalendern mit mcp_writable = true.",
            input_schema: object(
                vec![
                    ("event_id", string("event_id aus list_invitations oder query_events.")),
                    ("occurrence", string(OCCURRENCE)),
                    ("response", enumeration(RESPONSES, "accepted, tentative oder declined.")),
                    ("comment", string("Optionaler Kommentar an den Organisator.")),
                ],
                &["event_id", "response"],
            ),
            read_only: false,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "create_opening_link",
            title: "Einmal-Link anlegen",
            description: "Legt einen Einmal-Buchungslink an und liefert seine url zum \
                Weitergeben. Der Gast bucht darüber genau einen Termin von duration_minutes im \
                Zielkalender; der Anbieter verschickt die Einladung. calendar_id ist Pflicht: \
                vorher mit list_calendars einen Kalender mit mcp_writable = true und can_invite = \
                true wählen. Mit suggestions bietet der Link genau diese Zeiten an (vorher z. B. \
                mit find_free_times suchen); für jeden Vorschlag legt Lichtung sofort einen \
                Platzhalter „Reserviert: <Titel>“ im Zielkalender an, der die Zeit blockiert, \
                bis gebucht oder der Link gelöscht ist. Ohne suggestions bietet der Link freie \
                Zeiten Montag bis Freitag 09:00–17:00 in time_zone an, frühestens 4 Stunden ab \
                jetzt, höchstens 21 Tage voraus. Links laufen nicht ab; löschen lassen sie sich \
                in der App.",
            input_schema: object(
                vec![
                    (
                        "duration_minutes",
                        integer("Dauer des Termins in Minuten.", 5, 720, None),
                    ),
                    (
                        "calendar_id",
                        string(
                            "Zielkalender (id aus list_calendars; mcp_writable und can_invite \
                             müssen true sein).",
                        ),
                    ),
                    (
                        "title",
                        string(
                            "Titel des Termins, den der Gast bucht. Standard: „Termin (<Dauer> \
                             Minuten)“.",
                        ),
                    ),
                    (
                        "suggestions",
                        json!({
                            "type": "array",
                            "maxItems": 20,
                            "description": "Konkrete Vorschläge (höchstens 20), jeder genau \
                                duration_minutes lang, in der Zukunft und höchstens 365 Tage \
                                voraus. Blockieren die Zeit mit einem Platzhalter im Zielkalender.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "start": {"type": "string", "description": "Beginn nach \
                                        RFC 3339 mit Offset, z. B. 2026-11-03T10:00:00+01:00."},
                                    "end": {"type": "string", "description": "Ende mit Offset \
                                        (optional; Standard start + duration_minutes)."},
                                },
                                "required": ["start"],
                                "additionalProperties": false,
                            },
                        }),
                    ),
                    (
                        "time_zone",
                        string(
                            "IANA-Zeitzone des Links (für die Zeitfenster ohne suggestions und \
                             die Ausgabe), z. B. Europe/Berlin. Standard: Zeitzone des Servers.",
                        ),
                    ),
                ],
                &["duration_minutes", "calendar_id"],
            ),
            read_only: false,
            destructive: false,
            idempotent: false,
        },
    ]
}
