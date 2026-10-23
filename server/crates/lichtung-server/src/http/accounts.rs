//! `/api` handlers for accounts, calendars and calendar sets (sync API
//! contract, section 7).
//!
//! | Route | Who |
//! |---|---|
//! | `GET /api/accounts`, `GET /api/calendars`, `GET /api/calendar-sets` | every principal of `/api` |
//! | `POST /api/accounts/{id}/sync` | every principal of `/api` |
//! | `POST /api/accounts`, `PATCH`/`DELETE /api/accounts/{id}` | browser session or device |
//! | `PATCH /api/calendars/{id}` | browser session or device |
//! | `POST /api/calendar-sets`, `PATCH`/`DELETE /api/calendar-sets/{id}` | browser session or device |
//!
//! Accounts hold provider credentials, so they follow the rule for
//! credentials ([`Principal::may_manage_credentials`]). Calendar settings
//! include `mcp_write`, the guard against Claude; API tokens also open
//! `/mcp`, so they may read but not change settings
//! ([`Principal::may_change_settings`]). Reading and triggering a sync
//! reveal or change nothing sensitive (no passwords in any answer). OAuth
//! access tokens never reach `/api`.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};

use super::extract::ApiJson;
use crate::accounts::{
    Account, AccountPatch, Calendar, CalendarPatch, CalendarSet, CalendarSetPatch, NewAccount,
    NewCalendarSet,
};
use crate::app::App;
use crate::auth::Principal;
use crate::error::{AppError, AppResult};

pub fn routes() -> Router<App> {
    Router::new()
        .route("/accounts", get(list_accounts).post(create_account))
        .route(
            "/accounts/{id}",
            patch(update_account).delete(delete_account),
        )
        .route("/accounts/{id}/sync", post(sync_account))
        .route("/calendars", get(list_calendars))
        .route("/calendars/{id}", patch(update_calendar))
        .route(
            "/calendar-sets",
            get(list_calendar_sets).post(create_calendar_set),
        )
        .route(
            "/calendar-sets/{id}",
            patch(update_calendar_set).delete(delete_calendar_set),
        )
}

fn require_account_manager(principal: &Principal) -> AppResult<()> {
    if principal.may_manage_credentials() {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Kalenderkonten lassen sich nur mit einer Browser-Sitzung oder in der \
             Lichtung-App verwalten"
                .into(),
        ))
    }
}

fn require_settings(principal: &Principal) -> AppResult<()> {
    if principal.may_change_settings() {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Kalender-Einstellungen und Kalender-Sets lassen sich nur mit einer \
             Browser-Sitzung oder in der Lichtung-App ändern"
                .into(),
        ))
    }
}

async fn list_accounts(State(app): State<App>) -> AppResult<Json<Vec<Account>>> {
    Ok(Json(app.list_accounts().await?))
}

async fn create_account(
    State(app): State<App>,
    principal: Principal,
    ApiJson(body): ApiJson<NewAccount>,
) -> AppResult<(StatusCode, Json<Account>)> {
    require_account_manager(&principal)?;
    let account = app.create_account(body).await?;
    Ok((StatusCode::CREATED, Json(account)))
}

async fn update_account(
    State(app): State<App>,
    principal: Principal,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<AccountPatch>,
) -> AppResult<Json<Account>> {
    require_account_manager(&principal)?;
    Ok(Json(app.update_account(&id, body).await?))
}

async fn delete_account(
    State(app): State<App>,
    principal: Principal,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    require_account_manager(&principal)?;
    app.delete_account(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn sync_account(
    State(app): State<App>,
    Path(id): Path<String>,
) -> AppResult<(StatusCode, Json<Account>)> {
    let account = app.request_sync(&id).await?;
    Ok((StatusCode::ACCEPTED, Json(account)))
}

async fn list_calendars(State(app): State<App>) -> AppResult<Json<Vec<Calendar>>> {
    Ok(Json(app.list_calendars().await?))
}

async fn update_calendar(
    State(app): State<App>,
    principal: Principal,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<CalendarPatch>,
) -> AppResult<Json<Calendar>> {
    require_settings(&principal)?;
    Ok(Json(app.update_calendar(&id, body).await?))
}

async fn list_calendar_sets(State(app): State<App>) -> AppResult<Json<Vec<CalendarSet>>> {
    Ok(Json(app.list_calendar_sets().await?))
}

async fn create_calendar_set(
    State(app): State<App>,
    principal: Principal,
    ApiJson(body): ApiJson<NewCalendarSet>,
) -> AppResult<(StatusCode, Json<CalendarSet>)> {
    require_settings(&principal)?;
    let set = app.create_calendar_set(body).await?;
    Ok((StatusCode::CREATED, Json(set)))
}

async fn update_calendar_set(
    State(app): State<App>,
    principal: Principal,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<CalendarSetPatch>,
) -> AppResult<Json<CalendarSet>> {
    require_settings(&principal)?;
    Ok(Json(app.update_calendar_set(&id, body).await?))
}

async fn delete_calendar_set(
    State(app): State<App>,
    principal: Principal,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    require_settings(&principal)?;
    app.delete_calendar_set(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
