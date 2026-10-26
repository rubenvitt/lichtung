//! The pages of the public booking page: plain server-rendered HTML in
//! German and English, inline CSS, one small inline script (time zone of
//! the browser, progressive enhancement), strict headers. Pages show the
//! link's title and length and free times, never anything from the
//! calendar.

use std::fmt::Write as _;
use std::sync::OnceLock;

use axum::http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, REFERRER_POLICY, RETRY_AFTER};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use base64::Engine;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use chrono_tz::Tz;
use sha2::{Digest, Sha256};

use crate::openings::Slot;

/// Language of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    De,
    En,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::De => "de",
            Lang::En => "en",
        }
    }

    pub fn parse(raw: &str) -> Option<Lang> {
        let primary = raw.trim().split(['-', '_']).next()?.to_ascii_lowercase();
        match primary.as_str() {
            "de" => Some(Lang::De),
            "en" => Some(Lang::En),
            _ => None,
        }
    }

    /// `?lang=` wins, then the first of German and English in
    /// `Accept-Language` (by weight), else English.
    pub fn choose(query: Option<&str>, accept_language: Option<&str>) -> Lang {
        if let Some(lang) = query.and_then(Lang::parse) {
            return lang;
        }
        let mut offered: Vec<(f32, usize, Lang)> = accept_language
            .unwrap_or("")
            .split(',')
            .enumerate()
            .filter_map(|(i, part)| {
                let mut pieces = part.split(';');
                let lang = Lang::parse(pieces.next()?)?;
                let q = pieces
                    .find_map(|p| p.trim().strip_prefix("q=")?.parse::<f32>().ok())
                    .unwrap_or(1.0);
                (q > 0.0).then_some((q, i, lang))
            })
            .collect();
        offered.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        offered.first().map_or(Lang::En, |(_, _, l)| *l)
    }

    pub fn texts(self) -> &'static Texts {
        match self {
            Lang::De => &DE,
            Lang::En => &EN,
        }
    }

    fn other(self) -> Lang {
        match self {
            Lang::De => Lang::En,
            Lang::En => Lang::De,
        }
    }
}

/// The fixed texts of a language.
pub struct Texts {
    pub minutes: &'static str,
    pub pick: &'static str,
    pub times_in: &'static str,
    pub change: &'static str,
    pub zone_label: &'static str,
    pub earlier: &'static str,
    pub later: &'static str,
    pub nothing_here: &'static str,
    pub nothing_at_all: &'static str,
    pub your_details: &'static str,
    pub name: &'static str,
    pub email: &'static str,
    pub note: &'static str,
    pub honeypot: &'static str,
    pub book: &'static str,
    pub other_time: &'static str,
    pub invite_hint: &'static str,
    pub booked_title: &'static str,
    pub booked_text: &'static str,
    pub cancel_hint: &'static str,
    pub taken: &'static str,
    pub not_found_title: &'static str,
    pub not_found_text: &'static str,
    pub used_title: &'static str,
    pub used_text: &'static str,
    pub too_many_title: &'static str,
    pub too_many_text: &'static str,
    pub unavailable_title: &'static str,
    pub unavailable_text: &'static str,
    pub invalid_title: &'static str,
    pub invalid_text: &'static str,
    pub name_invalid: &'static str,
    pub email_invalid: &'static str,
    pub note_invalid: &'static str,
    pub resend: &'static str,
    pub expired: &'static str,
    pub other_language: &'static str,
    pub weekdays: [&'static str; 7],
    pub months: [&'static str; 12],
}

pub static DE: Texts = Texts {
    minutes: "Minuten",
    pick: "Wähle einen Termin.",
    times_in: "Zeiten in",
    change: "Ändern",
    zone_label: "Zeitzone",
    earlier: "← Früher",
    later: "Später →",
    nothing_here: "In diesem Zeitraum ist nichts frei.",
    nothing_at_all: "Gerade ist keine Zeit frei. Versuch es später noch einmal.",
    your_details: "Deine Angaben",
    name: "Name",
    email: "E-Mail",
    note: "Notiz (optional)",
    honeypot: "Bitte leer lassen",
    book: "Termin buchen",
    other_time: "Andere Zeit wählen",
    invite_hint: "Die Einladung kommt per E-Mail.",
    booked_title: "Gebucht",
    booked_text: "Die Einladung kommt per E-Mail an",
    cancel_hint: "Absagen geht über die Einladung.",
    taken: "Diese Zeit ist leider gerade vergeben worden. Bitte wähle eine andere.",
    not_found_title: "Link nicht gefunden",
    not_found_text: "Diesen Buchungslink gibt es nicht (mehr).",
    used_title: "Link bereits genutzt",
    used_text: "Dieser Link wurde bereits für eine Buchung genutzt.",
    too_many_title: "Zu viele Anfragen",
    too_many_text: "Bitte warte ein wenig und versuch es dann noch einmal.",
    unavailable_title: "Gerade nicht möglich",
    unavailable_text: "Die Buchung ist gerade nicht möglich. Bitte versuch es in ein paar Minuten \
                       noch einmal.",
    invalid_title: "Anfrage ungültig",
    invalid_text: "Die Anfrage ließ sich nicht verarbeiten. Bitte lade die Seite neu.",
    name_invalid: "Bitte gib deinen Namen an (höchstens 100 Zeichen, ohne Links).",
    email_invalid: "Bitte gib eine gültige E-Mail-Adresse an.",
    note_invalid: "Die Notiz darf höchstens 1000 Zeichen lang sein.",
    resend: "Bitte prüf deine Angaben und sende das Formular noch einmal.",
    expired: "Das Formular ist abgelaufen. Bitte sende es noch einmal.",
    other_language: "English",
    weekdays: [
        "Montag",
        "Dienstag",
        "Mittwoch",
        "Donnerstag",
        "Freitag",
        "Samstag",
        "Sonntag",
    ],
    months: [
        "Januar",
        "Februar",
        "März",
        "April",
        "Mai",
        "Juni",
        "Juli",
        "August",
        "September",
        "Oktober",
        "November",
        "Dezember",
    ],
};

pub static EN: Texts = Texts {
    minutes: "minutes",
    pick: "Pick a time.",
    times_in: "Times in",
    change: "Change",
    zone_label: "Time zone",
    earlier: "← Earlier",
    later: "Later →",
    nothing_here: "No free times in this period.",
    nothing_at_all: "There are no free times right now. Please try again later.",
    your_details: "Your details",
    name: "Name",
    email: "Email",
    note: "Note (optional)",
    honeypot: "Please leave empty",
    book: "Book",
    other_time: "Pick another time",
    invite_hint: "The invitation comes by email.",
    booked_title: "Booked",
    booked_text: "The invitation is on its way to",
    cancel_hint: "To cancel, decline the invitation.",
    taken: "Sorry, this time was just taken. Please pick another one.",
    not_found_title: "Link not found",
    not_found_text: "This booking link does not exist (anymore).",
    used_title: "Link already used",
    used_text: "This link has already been used for a booking.",
    too_many_title: "Too many requests",
    too_many_text: "Please wait a little and try again.",
    unavailable_title: "Not possible right now",
    unavailable_text: "Booking is not possible right now. Please try again in a few minutes.",
    invalid_title: "Invalid request",
    invalid_text: "The request could not be processed. Please reload the page.",
    name_invalid: "Please enter your name (at most 100 characters, no links).",
    email_invalid: "Please enter a valid email address.",
    note_invalid: "The note can be at most 1000 characters long.",
    resend: "Please check your details and send the form again.",
    expired: "The form has expired. Please send it again.",
    other_language: "Deutsch",
    weekdays: [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ],
    months: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
};

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// `Dienstag, 3. November 2026` / `Tuesday, 3 November 2026`.
pub fn format_day(lang: Lang, day: NaiveDate) -> String {
    let t = lang.texts();
    let weekday = t.weekdays[day.weekday().num_days_from_monday() as usize];
    let month = t.months[day.month0() as usize];
    match lang {
        Lang::De => format!("{weekday}, {}. {month} {}", day.day(), day.year()),
        Lang::En => format!("{weekday}, {} {month} {}", day.day(), day.year()),
    }
}

/// `09:30` in `tz`.
pub fn format_time(t: DateTime<Utc>, tz: Tz) -> String {
    t.with_timezone(&tz).format("%H:%M").to_string()
}

/// Query string from pairs (form-encoded).
pub fn query(pairs: &[(&str, &str)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    format!("?{}", s.finish())
}

/// Sets `?tz=` to the browser's zone when the URL has none (only on the
/// slot list; without JavaScript the server's fallback and the selector
/// remain).
const TZ_SCRIPT: &str = "(function(){try{var u=new URL(location.href);if(u.searchParams.has(\"tz\"))return;var z=Intl.DateTimeFormat().resolvedOptions().timeZone;if(!z)return;u.searchParams.set(\"tz\",z);location.replace(u.href);}catch(e){}})();";

fn csp() -> &'static str {
    static CSP: OnceLock<String> = OnceLock::new();
    CSP.get_or_init(|| {
        let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(TZ_SCRIPT));
        format!(
            "default-src 'none'; style-src 'unsafe-inline'; script-src 'sha256-{hash}'; \
             form-action 'self'; frame-ancestors 'none'; base-uri 'none'"
        )
    })
}

const STYLE: &str = "\
*{box-sizing:border-box}\
body{font-family:system-ui,-apple-system,'Segoe UI',sans-serif;max-width:40rem;margin:0 auto;padding:1.5rem 1rem 3rem;line-height:1.5;color:#1d1d1f;background:#fff}\
@media (prefers-color-scheme: dark){body{color:#ececec;background:#18181b}a.slot,input,textarea,select{background:#27272a;color:#ececec}.notice{background:rgba(217,119,6,.18)}}\
h1{font-size:1.5rem;margin:.5rem 0 .25rem}h2{font-size:1.05rem;margin:1.5rem 0 .5rem}\
.muted{opacity:.7;font-size:.9rem}\
.top{display:flex;justify-content:space-between;gap:1rem;align-items:baseline;font-size:.9rem}\
.slots{display:grid;grid-template-columns:repeat(auto-fill,minmax(5.5rem,1fr));gap:.5rem}\
a.slot{display:block;text-align:center;padding:.55rem 0;border:1px solid #2f7d4f;border-radius:8px;color:#2f7d4f;text-decoration:none;font-variant-numeric:tabular-nums;background:#fff}\
a.slot:hover,a.slot:focus{background:#2f7d4f;color:#fff}\
a{color:#2f7d4f}\
.nav{display:flex;justify-content:space-between;margin-top:1.5rem}\
.notice{border-left:4px solid #d97706;padding:.5rem .75rem;background:rgba(217,119,6,.1);margin:1rem 0}\
form.zone{display:flex;gap:.5rem;align-items:center;flex-wrap:wrap;margin:.75rem 0;font-size:.9rem}\
select{max-width:100%;font:inherit;padding:.25rem}\
label{display:block;margin:.75rem 0 .25rem;font-weight:600}\
input,textarea{width:100%;font:inherit;padding:.5rem;border:1px solid #888;border-radius:6px;background:#fff;color:inherit}\
textarea{min-height:6rem}\
button{font:inherit;padding:.5rem 1.25rem;border-radius:6px;border:1px solid #2f7d4f;cursor:pointer;background:#2f7d4f;color:#fff;margin-top:1rem}\
button.plain{background:transparent;color:inherit;border-color:#888;margin:0;padding:.25rem .75rem}\
.when{font-size:1.1rem;font-weight:600}\
.hp{position:absolute;left:-10000px;width:1px;height:1px;overflow:hidden}\
";

/// A complete page with the headers of the booking page.
pub fn page(lang: Lang, status: StatusCode, title: &str, body: &str, script: bool) -> Response {
    let script = if script {
        format!("<script>{TZ_SCRIPT}</script>")
    } else {
        String::new()
    };
    let html = format!(
        "<!doctype html><html lang=\"{lang}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex, nofollow\">\
         <meta name=\"referrer\" content=\"no-referrer\">\
         <title>{title}</title><style>{STYLE}</style>{script}</head><body>{body}</body></html>",
        lang = lang.code(),
        title = escape(title),
    );
    let mut response = (status, Html(html)).into_response();
    let headers = response.headers_mut();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(
        "x-robots-tag",
        HeaderValue::from_static("noindex, nofollow"),
    );
    if let Ok(v) = HeaderValue::from_str(csp()) {
        headers.insert(CONTENT_SECURITY_POLICY, v);
    }
    response
}

/// A page with a heading and one sentence (errors, used links).
pub fn message(lang: Lang, status: StatusCode, title: &str, text: &str) -> Response {
    page(
        lang,
        status,
        title,
        &format!("<h1>{}</h1><p>{}</p>", escape(title), escape(text)),
        false,
    )
}

/// `429` with `Retry-After`.
pub fn too_many(lang: Lang, retry_after_secs: u64) -> Response {
    let t = lang.texts();
    let mut response = message(
        lang,
        StatusCode::TOO_MANY_REQUESTS,
        t.too_many_title,
        t.too_many_text,
    );
    if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
        response.headers_mut().insert(RETRY_AFTER, v);
    }
    response
}

/// What every page of a link shows and links with.
pub struct Ctx<'a> {
    pub lang: Lang,
    pub tz: Tz,
    pub title: &'a str,
    pub duration_minutes: i64,
}

impl Ctx<'_> {
    fn header(&self) -> String {
        let t = self.lang.texts();
        let other = self.lang.other();
        format!(
            "<div class=\"top\"><span class=\"muted\">{minutes} {minutes_word}</span>\
             <a href=\"{switch}\" hreflang=\"{code}\" lang=\"{code}\">{label}</a></div>\
             <h1>{title}</h1>",
            minutes = self.duration_minutes,
            minutes_word = t.minutes,
            switch = escape(&query(&[("lang", other.code()), ("tz", self.tz.name())])),
            code = other.code(),
            label = t.other_language,
            title = escape(self.title),
        )
    }

    /// The selector of the guest's zone (a GET form, works without
    /// JavaScript).
    fn zone_form(&self, from: Option<NaiveDate>) -> String {
        let t = self.lang.texts();
        let mut out = format!(
            "<form class=\"zone\" method=\"get\"><input type=\"hidden\" name=\"lang\" \
             value=\"{}\">",
            self.lang.code()
        );
        if let Some(from) = from {
            let _ = write!(
                out,
                "<input type=\"hidden\" name=\"from\" value=\"{}\">",
                from.format("%Y-%m-%d")
            );
        }
        let _ = write!(
            out,
            "<span>{} </span><select name=\"tz\" aria-label=\"{}\">",
            t.times_in, t.zone_label
        );
        let current = self.tz.name();
        let mut listed = false;
        for zone in zones() {
            let selected = if *zone == current {
                listed = true;
                " selected"
            } else {
                ""
            };
            let _ = write!(out, "<option{selected}>{}</option>", escape(zone));
        }
        if !listed {
            let _ = write!(out, "<option selected>{}</option>", escape(current));
        }
        let _ = write!(
            out,
            "</select><button class=\"plain\" type=\"submit\">{}</button></form>",
            t.change
        );
        out
    }
}

/// Zones offered in the selector: the regional IANA zones plus UTC.
fn zones() -> &'static [&'static str] {
    static ZONES: OnceLock<Vec<&'static str>> = OnceLock::new();
    ZONES.get_or_init(|| {
        const REGIONS: [&str; 8] = [
            "Africa/",
            "America/",
            "Asia/",
            "Atlantic/",
            "Australia/",
            "Europe/",
            "Indian/",
            "Pacific/",
        ];
        let mut list: Vec<&'static str> = chrono_tz::TZ_VARIANTS
            .iter()
            .map(|tz| tz.name())
            .filter(|name| REGIONS.iter().any(|r| name.starts_with(r)))
            .collect();
        list.sort_unstable();
        list.push("UTC");
        list
    })
}

/// Paging of the slot list.
pub struct Nav {
    pub from: NaiveDate,
    pub earlier: Option<NaiveDate>,
    pub later: Option<NaiveDate>,
}

/// The list of free times, grouped by day in the guest's zone.
pub fn slots_page(
    ctx: &Ctx<'_>,
    slots: &[Slot],
    nav: &Nav,
    notice: Option<&str>,
    status: StatusCode,
) -> Response {
    let t = ctx.lang.texts();
    let mut body = ctx.header();
    if let Some(n) = notice {
        let _ = write!(
            body,
            "<p class=\"notice\" role=\"status\">{}</p>",
            escape(n)
        );
    }
    let _ = write!(body, "<p>{}</p>", t.pick);
    body.push_str(&ctx.zone_form(Some(nav.from)));
    let lang = ctx.lang.code();
    let tz = ctx.tz.name();
    let mut current: Option<NaiveDate> = None;
    for slot in slots {
        let day = slot.start.with_timezone(&ctx.tz).date_naive();
        if current != Some(day) {
            if current.is_some() {
                body.push_str("</div>");
            }
            let _ = write!(
                body,
                "<h2>{}</h2><div class=\"slots\">",
                escape(&format_day(ctx.lang, day))
            );
            current = Some(day);
        }
        let start = slot
            .start
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let href = query(&[("slot", &start), ("tz", tz), ("lang", lang)]);
        let _ = write!(
            body,
            "<a class=\"slot\" href=\"{}\">{}</a>",
            escape(&href),
            format_time(slot.start, ctx.tz)
        );
    }
    if current.is_some() {
        body.push_str("</div>");
    } else {
        let text = if nav.later.is_some() || nav.earlier.is_some() {
            t.nothing_here
        } else {
            t.nothing_at_all
        };
        let _ = write!(body, "<p>{text}</p>");
    }
    if nav.earlier.is_some() || nav.later.is_some() {
        body.push_str("<div class=\"nav\">");
        let link = |day: NaiveDate, label: &str| {
            let from = day.format("%Y-%m-%d").to_string();
            format!(
                "<a href=\"{}\">{label}</a>",
                escape(&query(&[("from", &from), ("tz", tz), ("lang", lang)]))
            )
        };
        body.push_str(&nav.earlier.map(|d| link(d, t.earlier)).unwrap_or_default());
        body.push_str("<span></span>");
        body.push_str(&nav.later.map(|d| link(d, t.later)).unwrap_or_default());
        body.push_str("</div>");
    }
    page(ctx.lang, status, ctx.title, &body, true)
}

/// `Dienstag, 3. November 2026, 09:30–10:00 (Europe/Berlin)`.
pub fn describe_slot(lang: Lang, tz: Tz, start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    format!(
        "{}, {}–{} ({})",
        format_day(lang, start.with_timezone(&tz).date_naive()),
        format_time(start, tz),
        format_time(end, tz),
        tz.name()
    )
}

/// What the guest typed (kept when the form comes back).
#[derive(Debug, Default, Clone)]
pub struct Entered {
    pub name: String,
    pub email: String,
    pub note: String,
}

/// The booking form for one slot.
pub fn form_page(
    ctx: &Ctx<'_>,
    slot: &Slot,
    form_token: &str,
    entered: &Entered,
    errors: &[&str],
    status: StatusCode,
) -> Response {
    let t = ctx.lang.texts();
    let mut body = ctx.header();
    let _ = write!(
        body,
        "<p class=\"when\">{}</p>",
        escape(&describe_slot(ctx.lang, ctx.tz, slot.start, slot.end))
    );
    for e in errors {
        let _ = write!(body, "<p class=\"notice\" role=\"alert\">{}</p>", escape(e));
    }
    let start = slot
        .start
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let _ = write!(
        body,
        "<h2>{details}</h2><form method=\"post\">\
         <input type=\"hidden\" name=\"slot\" value=\"{start}\">\
         <input type=\"hidden\" name=\"tz\" value=\"{tz}\">\
         <input type=\"hidden\" name=\"lang\" value=\"{lang}\">\
         <input type=\"hidden\" name=\"form\" value=\"{form}\">\
         <label for=\"name\">{name_l}</label>\
         <input id=\"name\" name=\"name\" required maxlength=\"{max_name}\" autocomplete=\"name\" value=\"{name}\">\
         <label for=\"email\">{email_l}</label>\
         <input id=\"email\" name=\"email\" type=\"email\" required maxlength=\"{max_email}\" autocomplete=\"email\" value=\"{email}\">\
         <label for=\"note\">{note_l}</label>\
         <textarea id=\"note\" name=\"note\" maxlength=\"{max_note}\">{note}</textarea>\
         <div class=\"hp\" aria-hidden=\"true\"><label for=\"website\">{hp}</label>\
         <input id=\"website\" name=\"website\" tabindex=\"-1\" autocomplete=\"off\"></div>\
         <button type=\"submit\">{book}</button></form>\
         <p class=\"muted\">{hint}</p>\
         <p><a href=\"{back}\">{other}</a></p>",
        details = t.your_details,
        tz = escape(ctx.tz.name()),
        lang = ctx.lang.code(),
        form = escape(form_token),
        name_l = t.name,
        email_l = t.email,
        note_l = t.note,
        max_name = super::guard::MAX_NAME_CHARS,
        max_email = super::guard::MAX_EMAIL_CHARS,
        max_note = super::guard::MAX_NOTE_CHARS,
        name = escape(&entered.name),
        email = escape(&entered.email),
        note = escape(&entered.note),
        hp = t.honeypot,
        book = t.book,
        hint = t.invite_hint,
        back = escape(&query(&[
            ("from", &slot.start.with_timezone(&ctx.tz).date_naive().format("%Y-%m-%d").to_string()),
            ("tz", ctx.tz.name()),
            ("lang", ctx.lang.code()),
        ])),
        other = t.other_time,
    );
    page(ctx.lang, status, ctx.title, &body, false)
}

/// After a booking.
pub fn success_page(
    ctx: &Ctx<'_>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    email: &str,
) -> Response {
    let t = ctx.lang.texts();
    let body = format!(
        "<h1>{booked}</h1><p class=\"muted\">{title}</p><p class=\"when\">{when}</p>\
         <p>{text} <strong>{email}</strong>.</p><p class=\"muted\">{cancel}</p>",
        booked = t.booked_title,
        title = escape(ctx.title),
        when = escape(&describe_slot(ctx.lang, ctx.tz, start, end)),
        text = t.booked_text,
        email = escape(email),
        cancel = t.cancel_hint,
    );
    page(ctx.lang, StatusCode::OK, t.booked_title, &body, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_choice() {
        assert_eq!(Lang::choose(Some("en"), Some("de-DE")), Lang::En);
        assert_eq!(Lang::choose(Some("xx"), Some("de-DE,de;q=0.9")), Lang::De);
        assert_eq!(
            Lang::choose(None, Some("fr-FR,en;q=0.5,de;q=0.8")),
            Lang::De
        );
        assert_eq!(Lang::choose(None, Some("en-US,de;q=0.5")), Lang::En);
        assert_eq!(Lang::choose(None, Some("fr")), Lang::En);
        assert_eq!(Lang::choose(None, Some("de;q=0")), Lang::En);
        assert_eq!(Lang::choose(None, None), Lang::En);
    }

    #[test]
    fn dates() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        assert_eq!(format_day(Lang::De, d), "Dienstag, 3. November 2026");
        assert_eq!(format_day(Lang::En, d), "Tuesday, 3 November 2026");
    }

    #[test]
    fn script_hash_matches() {
        assert!(csp().contains("script-src 'sha256-"));
        assert!(!csp().contains("http"), "no external sources");
        assert!(zones().contains(&"Europe/Berlin") && zones().contains(&"UTC"));
    }
}
