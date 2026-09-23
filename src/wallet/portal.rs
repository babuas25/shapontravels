use super::{Result, core, forbidden, invalid, missing, reads};
use crate::{
    AppState,
    auth::{Admin, rate_limit},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use utoipa::OpenApi;
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub external_user_id: String,
    pub role: String,
    pub owner: Option<core::Owner>,
    #[serde(default = "empty_object")]
    pub display: Value,
}
fn empty_object() -> Value {
    json!({})
}
impl Actor {
    pub fn validate(&self) -> Result<()> {
        if !self.external_user_id.starts_with("user_")
            || self.external_user_id.len() > 128
            || !self
                .external_user_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || ![
                "superadmin",
                "admin",
                "staff_account",
                "staff_support",
                "b2b",
                "b2b_sub",
                "customer",
            ]
            .contains(&self.role.as_str())
            || !self.display.is_object()
            || self.display.to_string().len() > 4096
        {
            return Err(forbidden());
        }
        if let Some(owner) = &self.owner {
            owner.validate()?;
            if self.staff_read()
                || (self.role == "customer"
                    && (owner.owner_type != "user" || owner.owner_key != self.external_user_id))
                || (["b2b", "b2b_sub"].contains(&self.role.as_str())
                    && owner.owner_type != "agency")
            {
                return Err(forbidden());
            }
        } else if !self.staff_read() {
            return Err(forbidden());
        }
        Ok(())
    }
    pub fn staff_read(&self) -> bool {
        ["superadmin", "admin", "staff_account", "staff_support"].contains(&self.role.as_str())
    }
    pub fn finance(&self) -> Result<()> {
        if ["superadmin", "admin", "staff_account"].contains(&self.role.as_str()) {
            Ok(())
        } else {
            Err(forbidden())
        }
    }
    pub fn superadmin(&self) -> Result<()> {
        if self.role == "superadmin" {
            Ok(())
        } else {
            Err(forbidden())
        }
    }
    pub async fn account(&self, pool: &PgPool, currency: &str) -> Result<Uuid> {
        reads::owner_account(pool, self.owner.as_ref().ok_or_else(forbidden)?, currency).await
    }
    pub async fn check_account(&self, pool: &PgPool, id: Uuid) -> Result<()> {
        if self.staff_read() {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM wallet_accounts WHERE id=$1)")
                    .bind(id)
                    .fetch_one(pool)
                    .await?;
            return if exists { Ok(()) } else { Err(missing()) };
        }
        let owner = self.owner.as_ref().ok_or_else(forbidden)?;
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM wallet_accounts a JOIN wallet_owners o ON o.id=a.owner_id WHERE a.id=$1 AND o.owner_type=$2 AND o.owner_key=$3)")
            .bind(id).bind(&owner.owner_type).bind(&owner.owner_key).fetch_one(pool).await?;
        if exists { Ok(()) } else { Err(missing()) }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    actor: Actor,
    command: Command,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Provision {
        owner: Option<core::Owner>,
        currency: String,
        display: Option<Value>,
        client_id: Option<Uuid>,
    },
    Summary {
        account_id: Option<Uuid>,
        currency: String,
    },
    Statement {
        account_id: Option<Uuid>,
        query: reads::ReadQuery,
    },
    List {
        kind: String,
        limit: Option<i64>,
        cursor: Option<String>,
    },
    ReportSummary,
    LedgerDashboard,
    Request {
        id: Uuid,
        kind: String,
    },
    Setting {
        kind: String,
        operation: String,
        id: Option<Uuid>,
        data: Option<Value>,
    },
    Deposit {
        id: Uuid,
        data: Value,
    },
    Adjustment {
        id: Uuid,
        account_id: Uuid,
        amount: String,
        adjustment_type: String,
        reason: String,
    },
    ReverseDeposit {
        id: Uuid,
        deposit_id: Uuid,
        amount: String,
        reason: String,
    },
    Review {
        id: Uuid,
        decision: String,
        remarks: String,
    },
    Freeze {
        wallet_id: Uuid,
        status: String,
        reason: String,
    },
    NotificationStatus {
        request_id: Uuid,
    },
    NotificationRetry {
        input: super::notifications::Retry,
    },
}

#[utoipa::path(post,path="/admin/portal-wallet",tag="Wallet",security(("admin_session"=[])),request_body=Object,responses((status=200,body=Object),(status=403),(status=409),(status=422)))]
async fn portal(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<Input>,
) -> Result<Json<Value>> {
    admin.portal_bridge()?;
    input.actor.validate()?;
    rate_limit(
        &state.pool,
        &format!("wallet:{}", input.actor.external_user_id),
        120,
    )
    .await?;
    let actor = input.actor;
    let value = match input.command {
        Command::Provision {
            owner,
            currency,
            display,
            client_id,
        } => {
            let selected = if actor.staff_read() {
                actor.finance()?;
                owner.ok_or_else(|| invalid("WALLET_OWNER_REQUIRED"))?
            } else {
                if owner.is_some() || display.is_some() || client_id.is_some() {
                    return Err(forbidden());
                }
                actor.owner.clone().ok_or_else(forbidden)?
            };
            let mut tx = crate::identity::business::begin(&state.pool).await?;
            let account = core::provision(
                &mut tx,
                &selected,
                &currency,
                &display.unwrap_or(actor.display.clone()),
            )
            .await?;
            let client = if actor.role == "b2b" {
                sqlx::query_scalar("SELECT id FROM api_clients WHERE external_user_id=$1 AND active AND audience='b2b'")
                    .bind(&actor.external_user_id).fetch_optional(&mut *tx).await?
            } else {
                client_id
            };
            if let Some(id) = client {
                core::link_client(&mut tx, id, account).await?;
            }
            tx.commit().await?;
            reads::summary(&state.pool, account).await?
        }
        Command::Summary {
            account_id,
            currency,
        } => {
            let account = match account_id {
                Some(id) => {
                    actor.check_account(&state.pool, id).await?;
                    id
                }
                None => actor.account(&state.pool, &currency).await?,
            };
            reads::summary(&state.pool, account).await?
        }
        Command::Statement { account_id, query } => {
            let account = match account_id {
                Some(id) => {
                    actor.check_account(&state.pool, id).await?;
                    id
                }
                None => actor.account(&state.pool, &query.currency).await?,
            };
            reads::statement(&state.pool, account, &query).await?
        }
        Command::List {
            kind,
            limit,
            cursor,
        } => super::reports::list(&state.pool, &actor, &kind, limit, cursor).await?,
        Command::Request { id, kind } => {
            super::workflows::lookup(&state.pool, &actor, id, &kind).await?
        }
        Command::ReportSummary => super::reports::summary(&state.pool, &actor).await?,
        Command::LedgerDashboard => super::reports::ledger_dashboard(&state.pool, &actor).await?,
        Command::Setting {
            kind,
            operation,
            id,
            data,
        } => super::settings::execute(&state.pool, &actor, &kind, &operation, id, data).await?,
        Command::Deposit { id, data } => {
            super::workflows::deposit(&state.pool, &actor, id, data).await?
        }
        Command::Adjustment {
            id,
            account_id,
            amount,
            adjustment_type,
            reason,
        } => {
            super::workflows::adjustment(
                &state.pool,
                &actor,
                id,
                account_id,
                &amount,
                &adjustment_type,
                &reason,
            )
            .await?
        }
        Command::ReverseDeposit {
            id,
            deposit_id,
            amount,
            reason,
        } => {
            super::workflows::reverse_deposit(&state.pool, &actor, id, deposit_id, &amount, &reason)
                .await?
        }
        Command::Review {
            id,
            decision,
            remarks,
        } => super::workflows::review(&state.pool, &actor, id, &decision, &remarks).await?,
        Command::Freeze {
            wallet_id,
            status,
            reason,
        } => {
            actor.finance()?;
            if !["active", "frozen"].contains(&status.as_str())
                || reason.len() > 1000
                || (status == "frozen" && reason.trim().is_empty())
            {
                return Err(invalid("INVALID_WALLET_STATUS"));
            }
            let mut tx = crate::identity::business::begin(&state.pool).await?;
            let result = sqlx::query("UPDATE wallet_owners SET status=$2 WHERE id=$1")
                .bind(wallet_id)
                .bind(&status)
                .execute(&mut *tx)
                .await?;
            if result.rows_affected() != 1 {
                return Err(missing());
            }
            audit(
                &mut tx,
                &actor,
                "status",
                wallet_id,
                json!({"status":status,"reason":reason}),
            )
            .await?;
            tx.commit().await?;
            json!({"ok":true})
        }
        Command::NotificationStatus { request_id } => {
            super::notifications::status(&state.pool, &actor, request_id).await?
        }
        Command::NotificationRetry { input } => {
            super::notifications::retry(&state.pool, &actor, input).await?
        }
    };
    Ok(Json(value))
}
pub async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: &Actor,
    action: &str,
    id: Uuid,
    metadata: Value,
) -> Result<()> {
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES('system',$1,$2,'wallet',$3,$4)")
        .bind(&actor.external_user_id).bind(format!("wallet.{action}")).bind(id.to_string()).bind(json!({"actorRole":actor.role,"details":metadata})).execute(&mut **tx).await?;
    Ok(())
}
#[derive(OpenApi)]
#[openapi(paths(portal))]
pub struct PortalWalletDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/portal-wallet", post(portal))
        .layer(DefaultBodyLimit::max(128 * 1024))
}
