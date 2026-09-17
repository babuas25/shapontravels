//! Read-only activation review. A clean snapshot is not permission to cut over.
use crate::{AppState, auth::ApiError};
use axum::{Extension, Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Work {
    pub pending: i64,
    pub uncertain: i64,
}

/// All unresolved queues, including blocked/dead-letter work. Counts across
/// queues can refer to the same operation; they are not a unique event count.
pub async fn backlog(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<BTreeMap<String, Work>, ApiError> {
    let mut result = BTreeMap::new();
    for (name, table, column, terminal, uncertain) in [
        (
            "creates",
            "portal_identity_creates",
            "state",
            "'completed','cancelled'",
            "'dispatching','needs_reconciliation','reconciling'",
        ),
        (
            "invitations",
            "portal_identity_invitations",
            "state",
            "'accepted','revoked','cancelled','expired'",
            "'issuing','issue_unknown','observing_issue','revoking','revoke_unknown','observing_revoke'",
        ),
        (
            "deletions",
            "portal_identity_deletions",
            "state",
            "'completed'",
            "'dispatching','needs_reconciliation','reconciling'",
        ),
        (
            "effects",
            "portal_identity_effects",
            "state",
            "'confirmed','superseded'",
            "'dispatching','needs_reconciliation','reconciling'",
        ),
        (
            "events",
            "portal_identity_inbox",
            "state",
            "'completed'",
            "'processing','dead_letter'",
        ),
        (
            "mail",
            "portal_identity_mail",
            "state",
            "'sent'",
            "'sending','unknown','blocked'",
        ),
        (
            "assets",
            "portal_identity_assets",
            "state",
            "'ready','abandoned'",
            "'uploading','unknown'",
        ),
        (
            "wallet_notifications",
            "wallet_notifications",
            "state",
            "'sent','suppressed'",
            "'preparing','sending','unknown','failed'",
        ),
        (
            "wallet_deliveries",
            "wallet_notification_deliveries",
            "state",
            "'sent','suppressed'",
            "'sending','unknown','failed'",
        ),
        (
            "wallet_requests",
            "wallet_requests",
            "status",
            "'approved','rejected'",
            "NULL",
        ),
        (
            "wallet_reservations",
            "wallet_operations",
            "state",
            "'captured','released'",
            "'reconciliation'",
        ),
        (
            "bookings",
            "flight_bookings",
            "state",
            "'held','issued','manually_resolved'",
            "'pending','outcome_unknown'",
        ),
        (
            "ticket_issues",
            "flight_ticket_outcomes",
            "state",
            "'issued','not_issued'",
            "'pending','outcome_unknown'",
        ),
        (
            "cancellations",
            "flight_cancellations",
            "state",
            "'cancelled'",
            "'pending','outcome_unknown'",
        ),
    ] {
        // Identifiers/expressions are fixed source constants, never request input.
        let sql = format!(
            "SELECT count(*) FILTER(WHERE {column} NOT IN ({terminal})) pending, count(*) FILTER(WHERE {column} IN ({uncertain})) uncertain FROM {table}"
        );
        result.insert(
            name.into(),
            sqlx::query_as(&sql).fetch_one(&mut **tx).await?,
        );
    }
    Ok(result)
}

pub async fn read_transaction(pool: &PgPool) -> Result<Transaction<'_, Postgres>, ApiError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='5s'")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightRequest {
    pub clerk_user_id: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Fingerprint {
    pub rows: i64,
    pub sha256: String,
}

#[derive(Serialize)]
pub struct Report {
    pub mapping_digest: String,
    pub format_version: u8,
    pub observed_at: String,
    pub authority_mode: &'static str,
    pub schema_ready: bool,
    pub bootstrap_ready: bool,
    pub selected_operator_active_superadmin: bool,
    pub operator_provider_verified: bool,
    pub maintenance_enabled: bool,
    pub worker_paused: bool,
    pub backlog: BTreeMap<String, Work>,
    pub mapping_issues: BTreeMap<String, i64>,
    pub mapping_fingerprints: BTreeMap<String, Fingerprint>,
    pub database_review_clear: bool,
    pub activation_ready: bool,
    pub blockers: Vec<String>,
}

pub async fn inspect(pool: &PgPool, subject: &str) -> Result<Report, ApiError> {
    super::api::validate_subject(subject)?;
    let schema_ready = crate::schema_ready(pool).await;
    if !schema_ready {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "IDENTITY_SCHEMA_NOT_READY",
        ));
    }
    let mut tx = read_transaction(pool).await?;
    let observed_at: String = sqlx::query_scalar("SELECT to_char(transaction_timestamp() AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')").fetch_one(&mut *tx).await?;
    let bootstrap_ready: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_control WHERE singleton AND authority_mode='staged') AND EXISTS(SELECT 1 FROM portal_users WHERE role='superadmin' AND status='active')").fetch_one(&mut *tx).await?;
    let selected_operator_active_superadmin: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_users WHERE clerk_user_id=$1 AND role='superadmin' AND status='active')").bind(subject).fetch_one(&mut *tx).await?;
    let backlog = backlog(&mut tx).await?;
    let mut mapping_issues = BTreeMap::new();
    for (name, sql) in [
        (
            "agency_wallet_missing",
            "SELECT count(*) FROM portal_agencies a LEFT JOIN portal_agency_wallets w ON w.agency_id=a.id WHERE a.status<>'archived' AND w.agency_id IS NULL",
        ),
        (
            "unmapped_wallet_owners",
            "SELECT count(*) FROM wallet_owners w LEFT JOIN portal_agency_wallets a ON a.wallet_owner_id=w.id LEFT JOIN portal_users u ON w.owner_type='user' AND u.clerk_user_id=w.owner_key WHERE a.agency_id IS NULL AND u.id IS NULL",
        ),
        (
            "unmapped_client_subjects",
            "SELECT count(*) FROM api_clients c LEFT JOIN portal_users u ON u.clerk_user_id=c.external_user_id WHERE c.external_user_id IS NOT NULL AND u.id IS NULL",
        ),
        (
            "unmapped_staff_subjects",
            "SELECT count(*) FROM portal_staff_clients c LEFT JOIN portal_users u ON u.clerk_user_id=c.external_user_id WHERE u.id IS NULL",
        ),
        (
            "unmapped_draft_subjects",
            "SELECT count(*) FROM portal_hold_drafts d LEFT JOIN portal_users o ON o.clerk_user_id=d.owner_external_user_id LEFT JOIN portal_users c ON c.clerk_user_id=d.creator_external_user_id WHERE o.id IS NULL OR c.id IS NULL",
        ),
        (
            "unmapped_booking_creators",
            "SELECT count(*) FROM flight_bookings b LEFT JOIN portal_users u ON u.clerk_user_id=b.created_by_external_user_id WHERE b.created_by_external_user_id IS NOT NULL AND u.id IS NULL",
        ),
        (
            "agency_client_wallet_mismatch",
            "SELECT count(*) FROM api_clients c JOIN portal_users u ON u.clerk_user_id=c.external_user_id JOIN portal_agency_memberships m ON m.user_id=u.id LEFT JOIN portal_agency_wallets a ON a.agency_id=m.agency_id LEFT JOIN wallet_client_links w ON w.client_id=c.id WHERE w.owner_id IS DISTINCT FROM a.wallet_owner_id OR a.wallet_owner_id IS NULL",
        ),
        (
            "active_agency_users_without_membership",
            "SELECT count(*) FROM portal_users u LEFT JOIN portal_agency_memberships m ON m.user_id=u.id WHERE u.status='active' AND u.role IN ('b2b','b2b_sub') AND m.user_id IS NULL",
        ),
    ] {
        mapping_issues.insert(
            name.into(),
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(&mut *tx)
                .await?,
        );
    }
    let mut mapping_fingerprints = BTreeMap::new();
    // No names, email, passwords, token hashes, document bodies or booking PII.
    // A bounded digest binds an operator's subsequent review to these mappings.
    for (table, fields) in [
        (
            "portal_identity_control",
            "bootstrap_user_id,authority_mode,bootstrapped_at",
        ),
        (
            "portal_users",
            "id,clerk_user_id,role,status,version,authorization_version",
        ),
        (
            "portal_agencies",
            "id,agency_code,owner_user_id,status,version",
        ),
        (
            "portal_agency_memberships",
            "user_id,agency_id,kind,version",
        ),
        ("portal_agency_wallets", "agency_id,wallet_owner_id"),
        ("wallet_owners", "id,owner_type,owner_key,status"),
        ("wallet_client_links", "client_id,owner_id"),
        ("api_clients", "id,external_user_id"),
        ("portal_staff_clients", "external_user_id,client_id"),
        (
            "portal_hold_drafts",
            "id,owner_external_user_id,creator_external_user_id,client_id",
        ),
        (
            "flight_bookings",
            "id,client_id,portal_hold_draft_id,created_by_external_user_id,state,updated_at",
        ),
    ] {
        let count: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM (SELECT 1 FROM {table} LIMIT 100001) bounded"
        ))
        .fetch_one(&mut *tx)
        .await?;
        if count > 100_000 {
            return Err(ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "IDENTITY_PREFLIGHT_REVIEW_LIMIT",
            ));
        }
        let sql = format!(
            "SELECT count(*) rows, encode(sha256(convert_to(coalesce(string_agg(to_jsonb(t)::text,E'\\n' ORDER BY to_jsonb(t)::text),''),'UTF8')),'hex') sha256 FROM (SELECT {fields} FROM {table}) t"
        );
        mapping_fingerprints.insert(
            table.into(),
            sqlx::query_as(&sql).fetch_one(&mut *tx).await?,
        );
    }
    tx.rollback().await?;
    let mut blockers = Vec::new();
    if !bootstrap_ready {
        blockers.push("bootstrap_required".into());
    }
    if !selected_operator_active_superadmin {
        blockers.push("selected_operator_not_active_superadmin".into());
    }
    for (name, work) in &backlog {
        if work.pending > 0 {
            blockers.push(format!("unresolved_{name}"));
        }
    }
    for (name, count) in &mapping_issues {
        if *count > 0 {
            blockers.push(format!("mapping_review_{name}"));
        }
    }
    let database_review_clear = blockers.is_empty();
    let mapping_digest =
        crate::auth::digest(&serde_json::to_string(&mapping_fingerprints).map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "IDENTITY_PREFLIGHT_UNAVAILABLE",
            )
        })?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    // These cannot be established by a read of the application's database.
    blockers.extend(
        [
            "explicit_operator_activation_required",
            "selected_provider_operator_not_verified",
            "target_backup_restore_evidence_required",
            "old_writers_shutdown_evidence_required",
            "deployment_configuration_review_required",
            "live_cutover_not_authorized",
        ]
        .map(String::from),
    );
    Ok(Report {
        mapping_digest,
        format_version: 1,
        observed_at,
        authority_mode: "staged",
        schema_ready,
        bootstrap_ready,
        selected_operator_active_superadmin,
        operator_provider_verified: false,
        maintenance_enabled: false,
        worker_paused: false,
        backlog,
        mapping_issues,
        mapping_fingerprints,
        database_review_clear,
        activation_ready: false,
        blockers,
    })
}

#[utoipa::path(post,path="/admin/portal-identity/preflight",request_body=PreflightRequest,security(("identity_operator"=[])),responses((status=200,body=Object),(status=401),(status=503)))]
pub async fn preflight(
    State(state): State<AppState>,
    Extension(runtime): Extension<super::api::Runtime>,
    Json(input): Json<PreflightRequest>,
) -> Result<Json<Report>, ApiError> {
    let mut report = inspect(&state.pool, &input.clerk_user_id).await?;
    report.maintenance_enabled = runtime.maintenance_enabled();
    report.worker_paused = runtime.maintenance_enabled();
    report.authority_mode = if runtime.rollout_pin().is_some() {
        "canonical"
    } else {
        "staged"
    };
    Ok(Json(report))
}
