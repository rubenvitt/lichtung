//! Calendar listing (PROPFIND Depth 1 on the calendar home) and collection
//! state (ctag / sync token).

use url::Url;

use crate::client::{Client, Ctx};
use crate::error::Result;
use crate::href::same_resource;
use crate::xml::{DavResponse, NS_APPLE, NS_CALDAV, NS_CS, NS_DAV};

const CALENDAR_PROPS: &str = "<d:resourcetype/><d:displayname/><d:owner/>\
<d:current-user-privilege-set/><d:sync-token/><d:supported-report-set/>\
<c:calendar-description/><c:supported-calendar-component-set/>\
<c:schedule-calendar-transp/><cs:getctag/><cs:invite/><a:calendar-color/><a:calendar-order/>";

/// Sharing state of a calendar as seen from this account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sharing {
    /// Not shared (as far as the server tells).
    NotShared,
    /// Owned by this account and shared with others (`cs:shared-owner`
    /// resource type, or a `cs:invite` listing sharees without an
    /// organizer other than this account).
    SharedByMe,
    /// Someone else's calendar shared with this account (`cs:shared`
    /// resource type, a `cs:invite` organizer other than this account, or a
    /// `DAV:owner` different from the principal).
    SharedWithMe,
}

/// `CALDAV:schedule-calendar-transp`: whether events count as busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transparency {
    /// Events block time (default).
    Opaque,
    /// Events do not affect free/busy.
    Transparent,
}

/// One calendar collection in the calendar home.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CalendarInfo {
    /// Normalized absolute URL of the collection.
    pub href: Url,
    /// `DAV:displayname`.
    pub display_name: Option<String>,
    /// `CALDAV:calendar-description`.
    pub description: Option<String>,
    /// `apple:calendar-color` as `#RRGGBB` (alpha dropped).
    pub color: Option<String>,
    /// `apple:calendar-order`.
    pub order: Option<i64>,
    /// `CALDAV:supported-calendar-component-set` (upper-case names). Empty if
    /// the server did not say, which per RFC 4791 means "any component".
    pub components: Vec<String>,
    /// Privilege names from `DAV:current-user-privilege-set`; `None` if the
    /// server did not report the property.
    pub privileges: Option<Vec<String>>,
    /// Whether this account may write events. Assumed `true` when the server
    /// does not report privileges; a later write then fails with
    /// [`crate::Error::Forbidden`].
    pub writable: bool,
    /// Sharing state.
    pub sharing: Sharing,
    /// `DAV:owner`, if reported.
    pub owner: Option<Url>,
    /// `DAV:sync-token` (RFC 6578).
    pub sync_token: Option<String>,
    /// `cs:getctag`.
    pub ctag: Option<String>,
    /// `CALDAV:schedule-calendar-transp`.
    pub transparency: Option<Transparency>,
    /// Report names from `DAV:supported-report-set` (local names).
    pub supported_reports: Vec<String>,
}

impl CalendarInfo {
    /// `true` if the calendar can hold events (VEVENT listed or no list given).
    pub fn supports_events(&self) -> bool {
        self.components.is_empty() || self.components.iter().any(|c| c == "VEVENT")
    }

    /// `true` if `sync-collection` (RFC 6578) looks available: the server
    /// reported a sync token or lists the report.
    pub fn supports_sync_collection(&self) -> bool {
        self.sync_token.is_some()
            || self
                .supported_reports
                .iter()
                .any(|r| r == "sync-collection")
    }
}

/// Lightweight change indicators of one collection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CollectionState {
    /// `cs:getctag`.
    pub ctag: Option<String>,
    /// `DAV:sync-token`.
    pub sync_token: Option<String>,
}

impl Client {
    /// Lists the calendar collections in `home_href` (PROPFIND Depth 1).
    ///
    /// Only collections with resource type `CALDAV:calendar` are returned
    /// (not the home itself, not scheduling inbox/outbox). Calendars that
    /// cannot hold events are included; check [`CalendarInfo::supports_events`].
    /// Pass the principal to detect calendars shared with this account by
    /// owner comparison.
    pub async fn list_calendars(
        &self,
        home_href: &str,
        principal: Option<&Url>,
    ) -> Result<Vec<CalendarInfo>> {
        let home = self.resolve(home_href)?;
        let (_, ms) = self
            .propfind(home.clone(), "1", CALENDAR_PROPS, Ctx::Normal)
            .await?;
        Ok(ms
            .responses
            .iter()
            .filter(|r| !same_resource(&r.href, &home))
            .filter(|r| (200..300).contains(&r.effective_status()))
            .filter(|r| r.has_resource_type(NS_CALDAV, "calendar"))
            .map(|r| calendar_info(r, principal))
            .collect())
    }

    /// ctag and sync token of one collection (PROPFIND Depth 0), for servers
    /// without `sync-collection` or as a cheap "anything changed?" check.
    pub async fn collection_state(&self, calendar_href: &str) -> Result<CollectionState> {
        let url = self.resolve(calendar_href)?;
        let (_, ms) = self
            .propfind(url, "0", "<cs:getctag/><d:sync-token/>", Ctx::Normal)
            .await?;
        let r = ms.responses.first();
        Ok(CollectionState {
            ctag: r.and_then(|r| r.prop_text(NS_CS, "getctag")),
            sync_token: r.and_then(|r| r.prop_text(NS_DAV, "sync-token")),
        })
    }

    /// Just the ctag of a collection (`None` if the server has none).
    pub async fn ctag(&self, calendar_href: &str) -> Result<Option<String>> {
        Ok(self.collection_state(calendar_href).await?.ctag)
    }
}

fn calendar_info(r: &DavResponse, principal: Option<&Url>) -> CalendarInfo {
    let components: Vec<String> = r
        .prop(NS_CALDAV, "supported-calendar-component-set")
        .map(|set| {
            set.children_named(NS_CALDAV, "comp")
                .filter_map(|c| c.attr("name"))
                .map(|n| n.trim().to_ascii_uppercase())
                .collect()
        })
        .unwrap_or_default();

    let privileges: Option<Vec<String>> = r.prop(NS_DAV, "current-user-privilege-set").map(|set| {
        set.children_named(NS_DAV, "privilege")
            .flat_map(|p| p.children.iter())
            .map(|p| p.name.clone())
            .collect()
    });
    let writable = privileges.as_ref().is_none_or(|ps| {
        ps.iter()
            .any(|p| matches!(p.as_str(), "all" | "write" | "write-content"))
    });

    let owner = r
        .prop(NS_DAV, "owner")
        .and_then(|o| o.href_text())
        .and_then(|h| crate::href::normalize_href(&r.href, &h).ok());

    let is_me = |href: &Url| principal.is_none_or(|p| same_principal(href, p));
    // `cs:invite` (CalendarServer sharing, used by iCloud): sharees as
    // `cs:user`, the sharer as `cs:organizer` on the sharee's copy.
    let invite = r.prop(NS_CS, "invite");
    let invite_organizer = invite
        .and_then(|i| i.child(NS_CS, "organizer"))
        .and_then(|o| o.href_text())
        .and_then(|h| crate::href::normalize_href(&r.href, &h).ok())
        // A `mailto:` organizer cannot be compared with the principal.
        .filter(|o| matches!(o.scheme(), "http" | "https"));
    let invite_has_users = invite.is_some_and(|i| i.has_child(NS_CS, "user"));

    let sharing = if r.has_resource_type(NS_CS, "shared") {
        Sharing::SharedWithMe
    } else if r.has_resource_type(NS_CS, "shared-owner") {
        Sharing::SharedByMe
    } else if invite_organizer
        .iter()
        .chain(owner.iter())
        .any(|other| !is_me(other))
    {
        Sharing::SharedWithMe
    } else if invite_has_users {
        Sharing::SharedByMe
    } else {
        Sharing::NotShared
    };

    let transparency = r.prop(NS_CALDAV, "schedule-calendar-transp").and_then(|t| {
        if t.has_child(NS_CALDAV, "transparent") {
            Some(Transparency::Transparent)
        } else if t.has_child(NS_CALDAV, "opaque") {
            Some(Transparency::Opaque)
        } else {
            None
        }
    });

    let supported_reports = r
        .prop(NS_DAV, "supported-report-set")
        .map(|set| {
            set.children_named(NS_DAV, "supported-report")
                .filter_map(|s| s.child(NS_DAV, "report"))
                .flat_map(|rep| rep.children.iter())
                .map(|e| e.name.clone())
                .collect()
        })
        .unwrap_or_default();

    CalendarInfo {
        href: r.href.clone(),
        display_name: r.prop_text(NS_DAV, "displayname"),
        description: r.prop_text(NS_CALDAV, "calendar-description"),
        color: r
            .prop_text(NS_APPLE, "calendar-color")
            .and_then(|c| normalize_color(&c)),
        order: r
            .prop_text(NS_APPLE, "calendar-order")
            .and_then(|o| o.parse().ok()),
        components,
        privileges,
        writable,
        sharing,
        owner,
        sync_token: r.prop_text(NS_DAV, "sync-token"),
        ctag: r.prop_text(NS_CS, "getctag"),
        transparency,
        supported_reports,
    }
}

/// `true` if two principal URLs name the same principal. Besides
/// [`same_resource`], equal paths on different hosts count as equal: iCloud
/// reports the principal on `caldav.icloud.com` but resolves `DAV:owner`
/// hrefs on the calendar listing against the partition host
/// (`pNN-caldav.icloud.com`). Only hrefs of one account's server are
/// compared here, so the path alone identifies the principal.
fn same_principal(a: &Url, b: &Url) -> bool {
    same_resource(a, b)
        || crate::href::decoded_path(a).trim_end_matches('/')
            == crate::href::decoded_path(b).trim_end_matches('/')
}

/// `#RRGGBBAA` / `#RRGGBB` / `#RGB` → `#RRGGBB` (upper-case); anything
/// else → `None`.
pub fn normalize_color(raw: &str) -> Option<String> {
    let hex = raw.trim().strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = match hex.len() {
        6 | 8 => hex[..6].to_string(),
        3 | 4 => hex[..3].chars().flat_map(|c| [c, c]).collect(),
        _ => return None,
    };
    Some(format!("#{}", rgb.to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::normalize_color;

    #[test]
    fn colors() {
        assert_eq!(normalize_color("#3A7BD5FF").as_deref(), Some("#3A7BD5"));
        assert_eq!(normalize_color("#3a7bd5").as_deref(), Some("#3A7BD5"));
        assert_eq!(normalize_color(" #abc ").as_deref(), Some("#AABBCC"));
        assert_eq!(normalize_color("blau"), None);
        assert_eq!(normalize_color("#12345"), None);
    }
}
