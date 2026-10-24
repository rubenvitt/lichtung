//! CalDAV protocol layer for Lichtung.
//!
//! Transports iCalendar data as strings only; parsing happens elsewhere.
//!
//! - Discovery: [`Client::discover`] (`/.well-known/caldav`, principal,
//!   calendar home, scheduling URLs, `DAV` header).
//! - Calendars: [`Client::list_calendars`], [`Client::collection_state`].
//! - Delta sync: [`Client::sync_collection`] (RFC 6578) with
//!   [`Error::SyncTokenInvalid`]; fallback [`Client::list_etags`] + ctag.
//! - Fetching: [`Client::multiget`], [`Client::get`], [`Client::calendar_query`].
//! - Writing: [`Client::put`] with [`Precondition`], [`Client::delete`].
//!
//! ```no_run
//! # async fn demo() -> lichtung_caldav::Result<()> {
//! use lichtung_caldav::{Client, Credentials};
//! let client = Client::new(
//!     "https://caldav.example.org/dav/",
//!     Credentials::basic("anna@example.org", "app-passwort"),
//! )?;
//! let account = client.discover().await?;
//! let home = account.calendar_home().expect("calendar home");
//! for cal in client.list_calendars(home.as_str(), Some(&account.principal)).await? {
//!     let delta = client.sync_collection(cal.href.as_str(), None).await?;
//!     let hrefs: Vec<&str> = delta.changed.iter().map(|c| c.href.as_str()).collect();
//!     let objects = client.multiget(cal.href.as_str(), &hrefs).await?;
//!     println!("{}: {} Termine", cal.href, objects.found.len());
//! }
//! # Ok(()) }
//! ```

mod calendars;
mod client;
mod discovery;
mod error;
pub mod href;
mod objects;
mod sync;
mod xml;

pub use calendars::{normalize_color, CalendarInfo, CollectionState, Sharing, Transparency};
pub use client::{
    Client, ClientBuilder, Credentials, DEFAULT_CONNECT_TIMEOUT, DEFAULT_TIMEOUT, USER_AGENT,
};
pub use discovery::Discovery;
pub use error::{Error, Result};
pub use objects::{
    CalendarObject, MultigetResult, Precondition, PutResult, TimeRange, MULTIGET_CHUNK,
};
pub use sync::{ResourceEtag, SyncResult};
pub use url::Url;
