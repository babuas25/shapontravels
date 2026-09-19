//! The existing portal's request/quotation/confirmation/manual-settlement flow.
//! No supplier calls and no legacy storage. Canonical identity is mandatory.
pub mod client;
#[cfg(test)]
mod client_tests;
pub mod rules;
mod store;
#[cfg(test)]
mod tests;

use super::{Result, conflict, forbidden, invalid, portal::Actor};
use crate::{
    AppState,
    auth::{Admin, digest, rate_limit},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::post,
};
use rules::{Action, NewTicket, Quote, RequestType};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::OpenApi;
use uuid::Uuid;

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "action",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Mutation {
    Review {
        decision: ReviewDecision,
        note: Option<String>,
    },
    PublishQuote {
        quote: Quote,
    },
    CustomerDecision {
        quote_id: Uuid,
        decision: Decision,
        note: Option<String>,
    },
    Requote {
        reason: String,
    },
    Assign {
        assignee_user_id: String,
        reason: Option<String>,
    },
    CompleteRefund {
        note: Option<String>,
    },
    CompleteReissue {
        new_tickets: Vec<NewTicket>,
        note: Option<String>,
    },
    CompleteVoid {
        note: Option<String>,
    },
    ReleaseReissue {
        reason: String,
    },
    ReleaseVoid {
        reason: String,
    },
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReviewDecision {
    Accept,
    Reject,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Approved,
    Rejected,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Command {
    List {
        booking_reference: Option<String>,
        status: Option<rules::Status>,
        kind: Option<Action>,
        request_type: Option<RequestType>,
        limit: Option<i64>,
    },
    Detail {
        request_id: Uuid,
    },
    Availability {
        booking_reference: String,
    },
    Create {
        request_id: Uuid,
        booking_reference: String,
        kind: Action,
        request_type: RequestType,
        passenger_indexes: Vec<usize>,
        route_indexes: Vec<usize>,
        note: Option<String>,
    },
    Mutate {
        request_id: Uuid,
        expected_version: i64,
        request_key: Uuid,
        input: Mutation,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    actor: Actor,
    command: Command,
}

fn label<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}
fn missing() -> crate::auth::ApiError {
    crate::auth::ApiError(
        axum::http::StatusCode::NOT_FOUND,
        "TICKET_REQUEST_NOT_FOUND",
    )
}

#[utoipa::path(post,path="/admin/portal-ticket-management",operation_id="portalTicketManagement",tag="Wallet",security(("identity_bridge"=[])),request_body=Object,responses((status=200,body=Object),(status=403),(status=409),(status=422),(status=503)))]
async fn handle(
    admin: Admin,
    State(state): State<AppState>,
    Json(mut input): Json<Input>,
) -> Result<Json<Value>> {
    admin.portal_bridge()?;
    // A generic admin credential cannot impersonate an owner or reviewer.
    if !crate::identity::business::in_context() {
        return Err(forbidden());
    }
    input.actor.validate()?;
    rate_limit(
        &state.pool,
        &format!("ticket-management:{}", input.actor.external_user_id),
        120,
    )
    .await?;
    if let Command::Create {
        passenger_indexes,
        route_indexes,
        ..
    } = &mut input.command
    {
        passenger_indexes.sort_unstable();
        route_indexes.sort_unstable();
    }
    let actor = &input.actor;
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let hash =
        digest(&json!({"actor":actor.external_user_id,"command":&input.command}).to_string());
    let value = match input.command {
        Command::List {
            booking_reference,
            status,
            kind,
            request_type,
            limit,
        } => {
            store::list(
                &mut tx,
                actor,
                booking_reference.as_deref(),
                status,
                kind,
                request_type,
                limit.unwrap_or(50),
            )
            .await?
        }
        Command::Detail { request_id } => {
            let r = store::request(&mut tx, actor, request_id).await?;
            store::detail(&mut tx, actor, &r).await?
        }
        Command::Availability { booking_reference } => {
            let b = store::booking(&mut tx, actor, &booking_reference).await?;
            store::availability(&mut tx, &b).await?
        }
        Command::Create {
            request_id,
            booking_reference,
            kind,
            request_type,
            passenger_indexes,
            route_indexes,
            note,
        } => {
            rules::owner(&actor.role)?;
            if let Some(result) = store::replay(&mut tx, actor, request_id, &hash).await? {
                store::request(&mut tx, actor, request_id).await?;
                result
            } else {
                let b = store::booking(&mut tx, actor, &booking_reference).await?;
                let result = store::create(
                    &mut tx,
                    actor,
                    &b,
                    request_id,
                    kind,
                    request_type,
                    &passenger_indexes,
                    &route_indexes,
                    note.as_deref(),
                )
                .await?;
                store::save_replay(&mut tx, actor, request_id, request_id, &hash, &result).await?;
                result
            }
        }
        Command::Mutate {
            request_id,
            expected_version,
            request_key,
            input,
        } => {
            if expected_version < 1 {
                return Err(invalid("INVALID_TICKET_VERSION"));
            }
            // The authority lock serializes mutations before a replay lookup.
            // Scope is checked again even for a successful historical response.
            let mut r = store::request(&mut tx, actor, request_id).await?;
            if let Some(result) = store::replay(&mut tx, actor, request_key, &hash).await? {
                result
            } else {
                if r.version != expected_version {
                    return Err(conflict("TICKET_VERSION_CONFLICT"));
                }
                let result = store::mutate(&mut tx, actor, &mut r, input).await?;
                store::save_replay(&mut tx, actor, request_key, request_id, &hash, &result).await?;
                result
            }
        }
    };
    tx.commit().await?;
    Ok(Json(value))
}
#[derive(OpenApi)]
#[openapi(paths(handle))]
pub struct TicketManagementDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/portal-ticket-management", post(handle))
        .merge(client::routes())
        .layer(DefaultBodyLimit::max(32768))
}
