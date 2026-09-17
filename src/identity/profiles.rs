//! Versioned private profile records. Missing fields and explicit clearing stay distinct.
use super::{
    AuditActorKind, AuditDetails, AuditEntry, AuditOutcome, Role, Status, audit, begin_mutation,
    operations,
};
use crate::auth::{ApiError, digest};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::BTreeMap;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as=IdentityProfileKind)]
pub enum Kind {
    Profile,
    Staff,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Staff => "staff",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as=IdentityProfileField)]
pub enum Field {
    GivenName,
    Surname,
    Gender,
    DateOfBirth,
    Address,
    Nationality,
    PassportNo,
    PassportExpiry,
    Mobile,
    Email,
    AgencyName,
    AgencyLicenseNo,
    AgencyAddress,
    AgencyEmail,
    AgencyMobile,
    Website,
    FacebookPage,
    BankName,
    AccountName,
    AccountNumber,
    RoutingNumber,
    SwiftCode,
    BranchCode,
    Designation,
    Phone,
    AlternativePhone,
    Qualification,
}
impl Field {
    fn shared(self) -> bool {
        matches!(
            self,
            Self::GivenName
                | Self::Surname
                | Self::Gender
                | Self::DateOfBirth
                | Self::Address
                | Self::Nationality
                | Self::PassportNo
                | Self::PassportExpiry
                | Self::Mobile
                | Self::Email
        )
    }
    fn bank(self) -> bool {
        matches!(
            self,
            Self::BankName
                | Self::AccountName
                | Self::AccountNumber
                | Self::RoutingNumber
                | Self::SwiftCode
                | Self::BranchCode
        )
    }
    fn business(self) -> bool {
        matches!(
            self,
            Self::AgencyName
                | Self::AgencyLicenseNo
                | Self::AgencyAddress
                | Self::AgencyEmail
                | Self::AgencyMobile
                | Self::Website
                | Self::FacebookPage
        )
    }
    fn staff(self) -> bool {
        matches!(
            self,
            Self::Designation
                | Self::Email
                | Self::Phone
                | Self::AlternativePhone
                | Self::Address
                | Self::Qualification
        )
    }
    fn allowed(self, kind: Kind, role: Role) -> bool {
        match kind {
            Kind::Staff => self.staff(),
            Kind::Profile => {
                self.shared()
                    || (self.bank() && (role == Role::Customer || role.requires_agency()))
                    || (self.business() && role.requires_agency())
            }
        }
    }
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityProfileQuery)]
pub struct Query {
    pub clerk_user_id: String,
    #[schema(value_type=String,format="uuid")]
    pub target_user_id: Uuid,
    pub kind: Kind,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
#[schema(as=IdentityProfileChange)]
pub enum Change {
    Patch {
        fields: BTreeMap<Field, Option<String>>,
    },
    RemoveStaff {},
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityProfileEdit)]
pub struct Edit {
    pub clerk_user_id: String,
    #[schema(value_type=String,format="uuid")]
    pub target_user_id: Uuid,
    pub kind: Kind,
    #[schema(value_type=String,format="uuid")]
    pub operation_id: Uuid,
    pub expected_version: i64,
    pub expected_identity_version: i64,
    pub change: Change,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityProfileView)]
pub struct View {
    #[schema(value_type=String,format="uuid")]
    pub target_user_id: Uuid,
    pub kind: Kind,
    pub role: Role,
    pub identity_version: i64,
    pub exists: bool,
    pub version: i64,
    pub fields: BTreeMap<Field, String>,
    pub replayed: bool,
    pub committed_version: Option<i64>,
}
#[derive(Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityProfileBrandingQuery)]
pub struct BrandingQuery {
    pub clerk_user_id: String,
    #[schema(value_type=String,format="uuid")]
    pub agency_id: Uuid,
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityProfileBranding)]
pub struct Branding {
    #[schema(value_type=String,format="uuid")]
    pub agency_id: Uuid,
    pub agency_code: String,
    #[schema(value_type=String,format="uuid")]
    pub owner_user_id: Uuid,
    pub profile_version: i64,
    pub fields: BTreeMap<Field, String>,
}
fn invalid() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "IDENTITY_INVALID_PROFILE")
}
fn denied() -> ApiError {
    ApiError(StatusCode::FORBIDDEN, "IDENTITY_PROFILE_FORBIDDEN")
}
fn conflict() -> ApiError {
    ApiError(StatusCode::CONFLICT, "IDENTITY_PROFILE_VERSION_CONFLICT")
}

pub(super) async fn active_agency(
    tx: &mut Transaction<'_, Postgres>,
    user: Uuid,
) -> Result<Uuid, ApiError> {
    let row:Option<(Uuid,String)>=sqlx::query_as("SELECT a.id,u.clerk_user_id FROM portal_agency_memberships m JOIN portal_agencies a ON a.id=m.agency_id JOIN portal_users u ON u.id=a.owner_user_id WHERE m.user_id=$1 AND a.status='active' AND u.status='active'")
        .bind(user).fetch_optional(&mut **tx).await?;
    let (agency, owner) = row.ok_or_else(denied)?;
    super::inbox::guard_subject(tx, &owner).await?;
    Ok(agency)
}
pub(super) async fn scope(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
    target: Uuid,
    kind: Kind,
    write: bool,
) -> Result<(operations::User, operations::User), ApiError> {
    super::api::validate_subject(subject)?;
    super::api::require_bootstrap(tx).await?;
    let actor = operations::actor(tx, subject).await?;
    let target = operations::user(tx, target).await?;
    if matches!(target.actor.status, Status::Deleting | Status::Deleted) {
        return Err(denied());
    }
    super::inbox::guard_subject(tx, &target.subject).await?;
    let own = actor.actor.user_id == target.actor.user_id;
    let agency = if actor.actor.role.requires_agency() {
        Some(active_agency(tx, actor.actor.user_id).await?)
    } else {
        None
    };
    match kind {
        Kind::Profile => {
            if !actor.actor.role.can_manage_profile(target.actor.role)
                && (!own || (write && actor.actor.role.requires_agency()))
            {
                return Err(denied());
            }
        }
        Kind::Staff => {
            if target.actor.role != Role::B2bSub {
                return Err(denied());
            }
            if !own
                && (actor.actor.role != Role::B2b
                    || agency != Some(active_agency(tx, target.actor.user_id).await?))
            {
                return Err(denied());
            }
        }
    }
    Ok((actor, target))
}
async fn load(
    tx: &mut Transaction<'_, Postgres>,
    target: &operations::User,
    kind: Kind,
) -> Result<View, ApiError> {
    let row: Option<(bool, i64, serde_json::Value)> = sqlx::query_as(
        "SELECT present,version,fields FROM portal_identity_profiles WHERE user_id=$1 AND kind=$2",
    )
    .bind(target.actor.user_id)
    .bind(kind.name())
    .fetch_optional(&mut **tx)
    .await?;
    let (exists, version, fields) = row.unwrap_or((false, 0, serde_json::json!({})));
    let mut fields: BTreeMap<Field, String> = serde_json::from_value(fields).map_err(|_| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "IDENTITY_PROFILE_UNAVAILABLE",
        )
    })?;
    // A later role change cannot expose fields outside the current role's contract.
    fields.retain(|field, _| field.allowed(kind, target.actor.role));
    Ok(View {
        target_user_id: target.actor.user_id,
        kind,
        role: target.actor.role,
        identity_version: target.version,
        exists,
        version,
        fields,
        replayed: false,
        committed_version: None,
    })
}
async fn log(
    tx: &mut Transaction<'_, Postgres>,
    actor: &operations::User,
    target: Uuid,
    operation: Uuid,
    action: &str,
    details: AuditDetails,
) -> Result<(), ApiError> {
    audit(
        tx,
        AuditEntry {
            operation_id: operation,
            actor_kind: AuditActorKind::User,
            actor_id: &actor.subject,
            action,
            target_user_id: Some(target),
            target_agency_id: None,
            outcome: AuditOutcome::Succeeded,
            details,
        },
    )
    .await
}
pub(super) async fn limit(
    tx: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    bucket: &str,
    max: i32,
    seconds: i32,
) -> Result<(), ApiError> {
    let count:i32=sqlx::query_scalar("INSERT INTO rate_buckets(bucket_key) VALUES($1) ON CONFLICT(bucket_key) DO UPDATE SET requests=CASE WHEN rate_buckets.window_start<=now()-make_interval(secs=>$2) THEN 1 ELSE rate_buckets.requests+1 END,window_start=CASE WHEN rate_buckets.window_start<=now()-make_interval(secs=>$2) THEN now() ELSE rate_buckets.window_start END RETURNING requests")
        .bind(digest(&format!("identity:profile:{bucket}:{actor}"))).bind(seconds as f64).fetch_one(&mut **tx).await?;
    if count > max {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "IDENTITY_RATE_LIMITED",
        ));
    }
    Ok(())
}
pub async fn query(pool: &PgPool, input: Query) -> Result<View, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    let (actor, target) = scope(
        &mut tx,
        &input.clerk_user_id,
        input.target_user_id,
        input.kind,
        false,
    )
    .await?;
    if actor.actor.role.manages_users() {
        limit(&mut tx, actor.actor.user_id, "read", 60, 3600).await?;
    }
    let view = load(&mut tx, &target, input.kind).await?;
    log(
        &mut tx,
        &actor,
        input.target_user_id,
        Uuid::new_v4(),
        "identity.profile.read",
        AuditDetails::default(),
    )
    .await?;
    tx.commit().await?;
    Ok(view)
}
fn normalized(change: &mut Change, kind: Kind, role: Role) -> Result<(), ApiError> {
    let Change::Patch { fields } = change else {
        return if kind == Kind::Staff {
            Ok(())
        } else {
            Err(invalid())
        };
    };
    if fields.is_empty() {
        return Err(invalid());
    }
    for (field, value) in fields {
        if !field.allowed(kind, role) {
            return Err(invalid());
        }
        let text = value.as_deref().unwrap_or("").trim().to_owned();
        if text.chars().count() > 500 || text.chars().any(|c| c.is_control()) {
            return Err(invalid());
        }
        if !text.is_empty() {
            match field {
                Field::Gender if !["male", "female", "other"].contains(&text.as_str()) => {
                    return Err(invalid());
                }
                Field::DateOfBirth | Field::PassportExpiry => {
                    if text.len() != 10
                        || chrono::NaiveDate::parse_from_str(&text, "%Y-%m-%d").is_err()
                    {
                        return Err(invalid());
                    }
                }
                Field::Email | Field::AgencyEmail => {
                    let parts: Vec<_> = text.split('@').collect();
                    if parts.len() != 2
                        || parts[0].is_empty()
                        || !parts[1].contains('.')
                        || parts[1].starts_with('.')
                        || parts[1].ends_with('.')
                        || text.chars().any(char::is_whitespace)
                    {
                        return Err(invalid());
                    }
                }
                Field::Website | Field::FacebookPage => {
                    let url = url::Url::parse(&text).map_err(|_| invalid())?;
                    if !["https", "http"].contains(&url.scheme())
                        || url.host_str().is_none()
                        || !url.username().is_empty()
                        || url.password().is_some()
                    {
                        return Err(invalid());
                    }
                }
                _ => (),
            }
        }
        // Empty string is a durable explicit removal; absence means never set.
        *value = Some(text);
    }
    Ok(())
}
pub async fn edit(pool: &PgPool, mut input: Edit) -> Result<View, ApiError> {
    if input.expected_version < 0 || input.expected_identity_version < 1 {
        return Err(invalid());
    }
    let mut tx = begin_mutation(pool).await?;
    let (actor, target) = scope(
        &mut tx,
        &input.clerk_user_id,
        input.target_user_id,
        input.kind,
        true,
    )
    .await?;
    normalized(&mut input.change, input.kind, target.actor.role)?;
    let hash = digest(&serde_json::to_string(&input).map_err(|_| invalid())?);
    let previous:Option<(Vec<u8>,i64)>=sqlx::query_as("SELECT request_hash,committed_version FROM portal_identity_profile_mutations WHERE operation_id=$1")
        .bind(input.operation_id).fetch_optional(&mut *tx).await?;
    let mut view = load(&mut tx, &target, input.kind).await?;
    if let Some((stored, committed)) = previous {
        if stored != hash {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "IDENTITY_OPERATION_REPLAY_MISMATCH",
            ));
        }
        view.replayed = true;
        view.committed_version = Some(committed);
        log(
            &mut tx,
            &actor,
            input.target_user_id,
            input.operation_id,
            "identity.profile.replay",
            AuditDetails::default(),
        )
        .await?;
        tx.commit().await?;
        return Ok(view);
    }
    if input.expected_identity_version != target.version || input.expected_version != view.version {
        return Err(conflict());
    }
    if actor.actor.role.manages_users() {
        limit(&mut tx, actor.actor.user_id, "edit", 40, 3600).await?;
    } else {
        limit(&mut tx, actor.actor.user_id, "save", 20, 300).await?;
    }
    // Merge in SQL so retained fields hidden by a role change aren't silently lost.
    let (present, patch) = match input.change {
        Change::Patch { fields } => (true, serde_json::to_value(fields).map_err(|_| invalid())?),
        Change::RemoveStaff {} => (false, serde_json::json!({})),
    };
    sqlx::query("INSERT INTO portal_identity_profiles(user_id,kind,fields,present) VALUES($1,$2,$3,$4) ON CONFLICT(user_id,kind) DO UPDATE SET fields=CASE WHEN EXCLUDED.present THEN portal_identity_profiles.fields || EXCLUDED.fields ELSE '{}'::jsonb END,present=EXCLUDED.present")
        .bind(input.target_user_id).bind(input.kind.name()).bind(patch).bind(present).execute(&mut *tx).await?;
    let committed = view.version + 1;
    sqlx::query("INSERT INTO portal_identity_profile_mutations(operation_id,actor_user_id,target_user_id,kind,request_hash,previous_version,committed_version) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind(input.operation_id).bind(actor.actor.user_id).bind(input.target_user_id).bind(input.kind.name()).bind(hash).bind(view.version).bind(committed).execute(&mut *tx).await?;
    log(
        &mut tx,
        &actor,
        input.target_user_id,
        input.operation_id,
        if input.kind == Kind::Staff {
            "identity.staff.updated"
        } else {
            "identity.profile.updated"
        },
        AuditDetails {
            previous_version: Some(view.version),
            next_version: Some(committed),
            ..Default::default()
        },
    )
    .await?;
    view = load(&mut tx, &target, input.kind).await?;
    view.committed_version = Some(committed);
    tx.commit().await?;
    Ok(view)
}
pub async fn branding(pool: &PgPool, input: BrandingQuery) -> Result<Branding, ApiError> {
    super::api::validate_subject(&input.clerk_user_id)?;
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let actor = operations::actor(&mut tx, &input.clerk_user_id).await?;
    let (owner, code): (Uuid, String) = sqlx::query_as(
        "SELECT owner_user_id,agency_code FROM portal_agencies WHERE id=$1 AND status='active'",
    )
    .bind(input.agency_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(denied)?;
    if !actor.actor.role.can_manage_profile(Role::B2b)
        && (!actor.actor.role.requires_agency()
            || active_agency(&mut tx, actor.actor.user_id).await? != input.agency_id)
    {
        return Err(denied());
    }
    let target = operations::user(&mut tx, owner).await?;
    if target.actor.status != Status::Active {
        return Err(denied());
    }
    super::inbox::guard_subject(&mut tx, &target.subject).await?;
    let mut view = load(&mut tx, &target, Kind::Profile).await?;
    view.fields
        .retain(|field, _| field.business() && *field != Field::AgencyLicenseNo);
    log(
        &mut tx,
        &actor,
        owner,
        Uuid::new_v4(),
        "identity.profile.branding_read",
        AuditDetails::default(),
    )
    .await?;
    tx.commit().await?;
    Ok(Branding {
        agency_id: input.agency_id,
        agency_code: code,
        owner_user_id: owner,
        profile_version: view.version,
        fields: view.fields,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn field_contracts_and_validation() {
        let mut change = Change::Patch {
            fields: BTreeMap::from([
                (Field::GivenName, Some("  Synthetic  ".into())),
                (Field::PassportNo, None),
            ]),
        };
        normalized(&mut change, Kind::Profile, Role::Customer).unwrap();
        let Change::Patch { fields } = change else {
            panic!()
        };
        assert_eq!(fields[&Field::GivenName].as_deref(), Some("Synthetic"));
        assert_eq!(fields[&Field::PassportNo].as_deref(), Some(""));
        for (field, text) in [
            (Field::DateOfBirth, "2025-02-29"),
            (Field::Gender, "unknown"),
            (Field::Email, "a@@b.c"),
            (Field::Website, "javascript:alert(1)"),
            (Field::GivenName, "x\0y"),
        ] {
            assert!(
                normalized(
                    &mut Change::Patch {
                        fields: BTreeMap::from([(field, Some(text.into()))])
                    },
                    Kind::Profile,
                    Role::B2b
                )
                .is_err()
            );
        }
        assert!(!Field::BankName.allowed(Kind::Profile, Role::StaffSupport));
        assert!(!Field::AgencyName.allowed(Kind::Profile, Role::Customer));
        assert!(!Field::PassportNo.allowed(Kind::Staff, Role::B2bSub));
        assert!(
            serde_json::from_str::<Change>(r#"{"action":"patch","fields":{"role":"admin"}}"#)
                .is_err()
        );
    }
}
