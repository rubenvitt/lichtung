//! Delta sync with `sync-collection` (RFC 6578) and the ETag listing
//! fallback for servers without it.

use std::collections::BTreeMap;

use url::Url;

use crate::client::{Client, Ctx};
use crate::error::{Error, Result};
use crate::href::same_resource;
use crate::xml::{escape, NS_DAV};

/// Upper bound for follow-up requests of one truncated sync.
const MAX_SYNC_PAGES: usize = 1000;

/// A resource and its ETag, passed through exactly as the server sent it
/// (with or without `W/` and quotes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceEtag {
    /// Normalized absolute URL of the resource.
    pub href: Url,
    /// `DAV:getetag`; `None` if the server did not report one.
    pub etag: Option<String>,
}

/// Result of [`Client::sync_collection`], merged over all pages.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SyncResult {
    /// New or changed resources (fetch them with [`Client::multiget`]).
    pub changed: Vec<ResourceEtag>,
    /// Removed resources (reported with status 404).
    pub deleted: Vec<Url>,
    /// Token to store for the next incremental sync.
    pub sync_token: String,
    /// Number of requests needed (more than one if the server truncated).
    pub pages: usize,
}

enum Change {
    Changed(Option<String>),
    Deleted,
}

impl Client {
    /// Delta sync of one calendar (RFC 6578, `sync-level` 1).
    ///
    /// `token = None` asks for the full initial state. Truncated results
    /// (`507` for the collection inside the multistatus) are followed with the
    /// interim token until complete. An expired or unknown token yields
    /// [`Error::SyncTokenInvalid`]; the caller should then fall back to a full
    /// comparison via [`Client::list_etags`] and start over with `None`.
    pub async fn sync_collection(
        &self,
        calendar_href: &str,
        token: Option<&str>,
    ) -> Result<SyncResult> {
        let calendar = self.resolve(calendar_href)?;
        let mut token = token.map(str::to_string);
        let mut merged: BTreeMap<String, (Url, Change)> = BTreeMap::new();
        for page in 1..=MAX_SYNC_PAGES {
            let token_xml = match &token {
                Some(t) => format!("<d:sync-token>{}</d:sync-token>", escape(t)),
                None => "<d:sync-token/>".to_string(),
            };
            let body = format!(
                r#"<?xml version="1.0" encoding="utf-8"?><d:sync-collection {}>{token_xml}<d:sync-level>1</d:sync-level><d:prop><d:getetag/></d:prop></d:sync-collection>"#,
                crate::xml::NS_DECL
            );
            let (_, ms) = self.report(calendar.clone(), "0", body, Ctx::Sync).await?;
            let mut truncated = false;
            for r in &ms.responses {
                let is_self = same_resource(&r.href, &calendar);
                match r.status {
                    Some(507) => {
                        truncated = true;
                        continue;
                    }
                    Some(404) | Some(410) if !is_self => {
                        merged.insert(r.href.to_string(), (r.href.clone(), Change::Deleted));
                        continue;
                    }
                    _ => {}
                }
                if is_self || r.href.path().ends_with('/') {
                    continue;
                }
                let status = r.effective_status();
                if !(200..300).contains(&status) && r.status.is_some() {
                    continue;
                }
                let etag = r.prop_text(NS_DAV, "getetag");
                merged.insert(r.href.to_string(), (r.href.clone(), Change::Changed(etag)));
            }
            let new_token = ms
                .sync_token
                .ok_or_else(|| Error::Protocol("sync-collection-Antwort ohne sync-token".into()))?;
            if !truncated {
                return Ok(finish(merged, new_token, page));
            }
            if token.as_deref() == Some(new_token.as_str()) {
                return Err(Error::Protocol(
                    "Server kürzt sync-collection, liefert aber keinen neuen Token".into(),
                ));
            }
            token = Some(new_token);
        }
        Err(Error::Protocol(
            "sync-collection nach zu vielen Folgeanfragen abgebrochen".into(),
        ))
    }

    /// ETag listing for servers without `sync-collection` (PROPFIND Depth 1
    /// with `getetag`). Returns every non-collection member of the calendar.
    pub async fn list_etags(&self, calendar_href: &str) -> Result<Vec<ResourceEtag>> {
        let calendar = self.resolve(calendar_href)?;
        let (_, ms) = self
            .propfind(
                calendar.clone(),
                "1",
                "<d:getetag/><d:resourcetype/><d:getcontenttype/>",
                Ctx::Normal,
            )
            .await?;
        Ok(ms
            .responses
            .into_iter()
            .filter(|r| !same_resource(&r.href, &calendar))
            .filter(|r| (200..300).contains(&r.effective_status()))
            .filter(|r| !r.is_collection() && !r.href.path().ends_with('/'))
            .map(|r| ResourceEtag {
                etag: r.prop_text(NS_DAV, "getetag"),
                href: r.href,
            })
            .collect())
    }
}

fn finish(merged: BTreeMap<String, (Url, Change)>, sync_token: String, pages: usize) -> SyncResult {
    let mut changed = Vec::new();
    let mut deleted = Vec::new();
    for (_, (href, change)) in merged {
        match change {
            Change::Changed(etag) => changed.push(ResourceEtag { href, etag }),
            Change::Deleted => deleted.push(href),
        }
    }
    SyncResult {
        changed,
        deleted,
        sync_token,
        pages,
    }
}
