//! Writes of objects the server owns itself (blockers, LIC-75): a whole
//! object is put or deleted as it is, with the same rights, locks and
//! provider error handling as every other write, but without conflict
//! entries. If the provider has a newer state, that state is stored and
//! the caller tries again with it later.

use lichtung_caldav::{Error as DavError, Precondition};
use lichtung_ical::CalendarObject;

use super::{ics_for_put, Origin, Stop, Target, WriteOutcome, Written};
use crate::app::{now_ms, App};
use crate::error::{AppError, AppResult};
use crate::events::normalize_ics;
use crate::events::store as events_store;

/// What a server write did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServerWrite {
    /// Written and stored as this event.
    Written(String),
    /// Deleted (or already gone).
    Deleted,
    /// Changed at the provider meanwhile: its state is stored now, nothing
    /// was written.
    Stale,
    /// Impossible for now or refused; the reason in German.
    Failed(String),
}

fn failed(stop: Stop) -> AppResult<ServerWrite> {
    match stop {
        Stop::Internal(e) => Err(e),
        Stop::Outcome(o) => Ok(ServerWrite::Failed(match *o {
            WriteOutcome::Rejected { message, .. } | WriteOutcome::Retry { message, .. } => message,
            other => other.status().to_owned(),
        })),
    }
}

impl App {
    async fn server_target(&self, calendar_id: &str) -> Result<Target, Stop> {
        let target = self.target(calendar_id).await?;
        Self::check_writable(&Origin::Server, &target)?;
        Ok(target)
    }

    /// Puts `ics` into `calendar_id`: over the stored event `existing`
    /// (`If-Match` its ETag), else (also if it is gone) new at `<calendar>/<uid>.ics`. A resource already there
    /// with the same UID (an earlier attempt whose answer got lost) is
    /// overwritten.
    pub(crate) async fn put_server_object(
        &self,
        calendar_id: &str,
        existing: Option<&str>,
        ics: &str,
    ) -> AppResult<ServerWrite> {
        let object = match CalendarObject::parse(ics) {
            Ok(o) => o,
            Err(e) => return Err(AppError::Internal(anyhow::anyhow!("invalid object: {e}"))),
        };
        let data = ics_for_put(&object);
        let _write = self.write_guard().await;
        let target = match self.server_target(calendar_id).await {
            Ok(t) => t,
            Err(stop) => return failed(stop),
        };
        let client = match self.client_for(&target.account) {
            Ok(c) => c,
            Err(stop) => return failed(stop),
        };
        let _lock = self.lock_calendars(&[calendar_id]).await;
        // Read under the lock: the stored state may be newer.
        let existing = match existing {
            Some(id) => self.event_row(id).await?,
            None => None,
        };
        let (href, precondition) = match existing {
            Some(row) => {
                let etag = match &row.etag {
                    Some(e) => e.clone(),
                    None => match client.get(&row.href).await {
                        Ok(obj) if normalize_ics(&obj.data) == row.ics && obj.etag.is_some() => {
                            obj.etag.expect("checked")
                        }
                        Ok(_) | Err(DavError::NotFound) => {
                            self.refresh_object(&client, calendar_id, &row.href).await?;
                            return Ok(ServerWrite::Stale);
                        }
                        Err(e) => return failed(self.provider_failure(&target.account, e).await),
                    },
                };
                (row.href.clone(), Precondition::IfMatch(etag))
            }
            None => {
                let uid = object.uid().unwrap_or_default();
                match client.new_resource_href(&target.calendar.remote_href, &uid) {
                    Ok(h) => (h.to_string(), Precondition::Create),
                    Err(e) => return failed(self.provider_failure(&target.account, e).await),
                }
            }
        };
        let creating = matches!(precondition, Precondition::Create);
        let put = match client.put(&href, &data, precondition).await {
            Ok(put) => put,
            Err(DavError::PreconditionFailed) if creating => {
                // Something is there already: overwrite it only if it is ours.
                let found = match client.get(&href).await {
                    Ok(f) => f,
                    Err(e) => return failed(self.provider_failure(&target.account, e).await),
                };
                let same = CalendarObject::parse(&found.data)
                    .ok()
                    .and_then(|o| o.uid())
                    .is_some_and(|u| Some(u) == object.uid());
                let Some(etag) = found.etag.filter(|_| same) else {
                    return Ok(ServerWrite::Failed(format!(
                        "Unter {href} liegt bereits ein anderer Termin."
                    )));
                };
                match client.put(&href, &data, Precondition::IfMatch(etag)).await {
                    Ok(put) => put,
                    Err(DavError::PreconditionFailed | DavError::NotFound) => {
                        return Ok(ServerWrite::Stale)
                    }
                    Err(e) => return failed(self.provider_failure(&target.account, e).await),
                }
            }
            Err(DavError::PreconditionFailed | DavError::NotFound) if !creating => {
                self.refresh_object(&client, calendar_id, &href).await?;
                return Ok(ServerWrite::Stale);
            }
            Err(e) => return failed(self.provider_failure(&target.account, e).await),
        };
        let written = Written::from_put(put, data);
        let id = self.store_written(&client, calendar_id, &written).await?;
        Ok(ServerWrite::Written(id))
    }

    /// Deletes a stored object at the provider (`If-Match` its ETag) and
    /// here. Gone already counts as deleted.
    pub(crate) async fn delete_server_object(&self, event_id: &str) -> AppResult<ServerWrite> {
        let _write = self.write_guard().await;
        let Some(row) = self.event_row(event_id).await? else {
            return Ok(ServerWrite::Deleted);
        };
        let target = match self.server_target(&row.calendar_id).await {
            Ok(t) => t,
            Err(stop) => return failed(stop),
        };
        let client = match self.client_for(&target.account) {
            Ok(c) => c,
            Err(stop) => return failed(stop),
        };
        let _lock = self.lock_calendars(&[&row.calendar_id]).await;
        let Some(row) = self.event_row(event_id).await? else {
            return Ok(ServerWrite::Deleted);
        };
        match client.delete(&row.href, row.etag.as_deref()).await {
            Ok(()) | Err(DavError::NotFound) => {}
            Err(DavError::PreconditionFailed) => {
                self.refresh_object(&client, &row.calendar_id, &row.href)
                    .await?;
                return Ok(ServerWrite::Stale);
            }
            Err(e) => return failed(self.provider_failure(&target.account, e).await),
        }
        let (calendar_id, href) = (row.calendar_id.clone(), row.href.clone());
        self.with_change_tx(move |tx| {
            events_store::delete_by_href(tx, &calendar_id, &href, now_ms())?;
            Ok(())
        })
        .await?;
        Ok(ServerWrite::Deleted)
    }
}
