//! Server-attested private assets: authorize/reserve before upload; publish after revalidation.
use super::{operations::User, phase5::*, *};
use crate::auth::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as=IdentityDocumentPurpose)]
pub enum Purpose {
    Profile,
    Application,
}
impl Purpose {
    fn name(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Application => "application",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as=IdentityDocumentSlot)]
pub enum Slot {
    TradeLicense,
    TinCertificate,
    TravelAgencyLicense,
    NidCard,
    Logo,
    Attachment,
}
impl Slot {
    fn name(self) -> String {
        serde_json::to_value(self).unwrap().as_str().unwrap().into()
    }
}
#[derive(Clone, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityDocumentQuery)]
pub struct Query {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    pub purpose: Purpose,
    pub slot: Slot,
    #[schema(value_type=Option<String>)]
    pub asset_id: Option<Uuid>,
}
#[derive(Clone, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityDocumentPrepare)]
pub struct Prepare {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub operation_id: Uuid,
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    pub purpose: Purpose,
    pub slot: Slot,
    pub expected_version: i64,
    pub expected_identity_version: i64,
    pub format: String,
    pub byte_size: i64,
    pub content_hash: String,
}
#[derive(Clone, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityDocumentAssetRequest)]
pub struct AssetRequest {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub asset_id: Uuid,
}
#[derive(Clone, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as=IdentityDocumentOutcome)]
pub enum Outcome {
    Ready,
    Unknown,
    Abandoned,
}
#[derive(Clone, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityDocumentFinish)]
pub struct Finish {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub asset_id: Uuid,
    pub outcome: Outcome,
    pub public_id: String,
    pub format: String,
    pub byte_size: i64,
    pub content_hash: String,
}
#[derive(Clone, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityDocumentRemove)]
pub struct Remove {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub operation_id: Uuid,
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    pub slot: Slot,
    pub expected_version: i64,
    pub expected_identity_version: i64,
}
#[derive(Clone, Debug, Deserialize, Serialize, sqlx::FromRow, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityDocumentAsset)]
pub struct Asset {
    #[schema(value_type=String)]
    pub id: Uuid,
    #[schema(value_type=String)]
    pub user_id: Uuid,
    pub purpose: String,
    pub slot: String,
    pub public_id: String,
    pub format: String,
    pub byte_size: i64,
    pub content_hash: String,
    pub state: String,
    pub expected_version: i64,
    pub identity_version: i64,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityDocumentView)]
pub struct View {
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    pub identity_version: i64,
    pub version: i64,
    pub slot: Slot,
    pub asset: Option<Asset>,
}
async fn scope(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
    id: Uuid,
    purpose: Purpose,
    slot: Slot,
    write: bool,
) -> Result<(User, User), ApiError> {
    if (purpose == Purpose::Application) != (slot == Slot::Attachment) {
        return Err(bad());
    }
    if purpose == Purpose::Application {
        let actor = actor(tx, subject).await?;
        let target = target(tx, &actor, id).await?;
        if write && (actor.actor.user_id != id || actor.actor.role != Role::Customer) {
            return Err(denied());
        }
        if write {
            let status: Option<String> = sqlx::query_scalar(
                "SELECT status FROM portal_identity_applications WHERE user_id=$1",
            )
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
            if !matches!(status.as_deref(), None | Some("rejected")) {
                return Err(conflict());
            }
        }
        return Ok((actor, target));
    }
    let (actor, mut target) =
        profiles::scope(tx, subject, id, profiles::Kind::Profile, write).await?;
    if !target.actor.role.requires_agency() {
        return Err(denied());
    }
    if slot == Slot::Logo && target.actor.role == Role::B2bSub {
        let owner:Uuid=sqlx::query_scalar("SELECT a.owner_user_id FROM portal_agency_memberships m JOIN portal_agencies a ON a.id=m.agency_id WHERE m.user_id=$1 AND a.status<>'archived'").bind(id).fetch_optional(&mut **tx).await?.ok_or_else(denied)?;
        target = operations::user(tx, owner).await?;
        inbox::guard_subject(tx, &target.subject).await?;
        if matches!(target.actor.status, Status::Deleting | Status::Deleted) {
            return Err(denied());
        }
    }
    Ok((actor, target))
}
async fn version(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    purpose: Purpose,
    slot: Slot,
) -> Result<i64, ApiError> {
    Ok(if purpose == Purpose::Application {
        sqlx::query_scalar("SELECT version FROM portal_identity_applications WHERE user_id=$1")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?
    } else {
        sqlx::query_scalar(
            "SELECT version FROM portal_identity_document_slots WHERE user_id=$1 AND slot=$2",
        )
        .bind(id)
        .bind(slot.name())
        .fetch_optional(&mut **tx)
        .await?
    }
    .unwrap_or(0))
}
async fn asset(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<Asset, ApiError> {
    sqlx::query_as("SELECT id,user_id,purpose,slot,public_id,format,byte_size,content_hash,state,expected_version,identity_version,(extract(epoch from created_at)*1000)::bigint AS created_at,(extract(epoch from completed_at)*1000)::bigint AS completed_at FROM portal_identity_assets WHERE id=$1").bind(id).fetch_optional(&mut **tx).await?.ok_or_else(denied)
}
async fn asset_scope(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
    a: &Asset,
) -> Result<(User, User), ApiError> {
    let purpose = operations::parse(a.purpose.clone())?;
    let slot = serde_json::from_value(serde_json::json!(a.slot)).map_err(|_| bad())?;
    let (actor, target) = scope(tx, subject, a.user_id, purpose, slot, true).await?;
    let original: Uuid =
        sqlx::query_scalar("SELECT actor_id FROM portal_identity_assets WHERE id=$1")
            .bind(a.id)
            .fetch_one(&mut **tx)
            .await?;
    if original != actor.actor.user_id {
        return Err(denied());
    }
    Ok((actor, target))
}
pub async fn query(pool: &PgPool, input: Query) -> Result<View, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    let (actor, target) = scope(
        &mut tx,
        &input.clerk_user_id,
        input.target_user_id,
        input.purpose,
        input.slot,
        false,
    )
    .await?;
    let id = if input.purpose == Purpose::Application {
        let id = input.asset_id.ok_or_else(bad)?;
        let referenced:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_applications WHERE user_id=$1 AND $2=ANY(documents))").bind(target.actor.user_id).bind(id).fetch_one(&mut *tx).await?;
        if !referenced {
            return Err(denied());
        }
        Some(id)
    } else {
        if input.asset_id.is_some() {
            return Err(bad());
        }
        sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT asset_id FROM portal_identity_document_slots WHERE user_id=$1 AND slot=$2",
        )
        .bind(target.actor.user_id)
        .bind(input.slot.name())
        .fetch_optional(&mut *tx)
        .await?
        .flatten()
    };
    let a = if let Some(id) = id {
        let a = asset(&mut tx, id).await?;
        if a.state != "ready"
            || a.user_id != target.actor.user_id
            || a.purpose != input.purpose.name()
            || a.slot != input.slot.name()
        {
            return Err(denied());
        }
        Some(a)
    } else {
        None
    };
    profiles::limit(&mut tx, actor.actor.user_id, "documents", 40, 3600).await?;
    let v = View {
        target_user_id: target.actor.user_id,
        identity_version: target.version,
        version: version(&mut tx, target.actor.user_id, input.purpose, input.slot).await?,
        slot: input.slot,
        asset: a,
    };
    log(
        &mut tx,
        Uuid::new_v4(),
        &actor,
        target.actor.user_id,
        "document.read",
    )
    .await?;
    tx.commit().await?;
    Ok(v)
}
pub async fn prepare(pool: &PgPool, input: Prepare) -> Result<Asset, ApiError> {
    if input.expected_version < 0
        || input.expected_identity_version < 1
        || !(1..=5242880).contains(&input.byte_size)
        || !["pdf", "png", "jpg", "webp", "svg"].contains(&input.format.as_str())
        || input.content_hash.len() != 64
        || !input
            .content_hash
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        || (input.slot == Slot::Logo && (input.format == "pdf" || input.byte_size > 524288))
        || (input.format == "svg" && input.slot != Slot::Logo)
    {
        return Err(bad());
    }
    let fingerprint = hash(&input)?;
    let mut tx = begin_mutation(pool).await?;
    let (actor, target) = scope(
        &mut tx,
        &input.clerk_user_id,
        input.target_user_id,
        input.purpose,
        input.slot,
        true,
    )
    .await?;
    let old: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT request_hash FROM portal_identity_assets WHERE id=$1")
            .bind(input.operation_id)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(old) = old {
        if old != fingerprint {
            return Err(conflict());
        }
        let a = asset(&mut tx, input.operation_id).await?;
        log(
            &mut tx,
            input.operation_id,
            &actor,
            target.actor.user_id,
            "document.replay",
        )
        .await?;
        tx.commit().await?;
        return Ok(a);
    }
    if target.version != input.expected_identity_version
        || version(&mut tx, target.actor.user_id, input.purpose, input.slot).await?
            != input.expected_version
    {
        return Err(conflict());
    }
    profiles::limit(&mut tx, actor.actor.user_id, "documents", 40, 3600).await?;
    sqlx::query("INSERT INTO portal_identity_assets(id,actor_id,user_id,purpose,slot,request_hash,expected_version,identity_version,public_id,format,byte_size,content_hash) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
 .bind(input.operation_id).bind(actor.actor.user_id).bind(target.actor.user_id).bind(input.purpose.name()).bind(input.slot.name()).bind(fingerprint).bind(input.expected_version).bind(input.expected_identity_version).bind(format!("shapon/identity/{}",input.operation_id)).bind(input.format).bind(input.byte_size).bind(input.content_hash).execute(&mut *tx).await?;
    log(
        &mut tx,
        input.operation_id,
        &actor,
        target.actor.user_id,
        "document.prepared",
    )
    .await?;
    let a = asset(&mut tx, input.operation_id).await?;
    tx.commit().await?;
    Ok(a)
}
pub async fn intent(pool: &PgPool, input: AssetRequest) -> Result<Asset, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    let a = asset(&mut tx, input.asset_id).await?;
    let actor = actor(&mut tx, &input.clerk_user_id).await?;
    let original: Uuid =
        sqlx::query_scalar("SELECT actor_id FROM portal_identity_assets WHERE id=$1")
            .bind(a.id)
            .fetch_one(&mut *tx)
            .await?;
    // Intent inspection remains possible after a successful submission/role change.
    if original != actor.actor.user_id {
        return Err(denied());
    }
    target(&mut tx, &actor, a.user_id).await?;
    log(&mut tx, a.id, &actor, a.user_id, "document.intent_read").await?;
    tx.commit().await?;
    Ok(a)
}
pub async fn start(pool: &PgPool, input: AssetRequest) -> Result<Asset, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    let a = asset(&mut tx, input.asset_id).await?;
    let (actor, target) = asset_scope(&mut tx, &input.clerk_user_id, &a).await?;
    let purpose = operations::parse(a.purpose.clone())?;
    let slot = serde_json::from_value(serde_json::json!(a.slot)).map_err(|_| bad())?;
    if a.state != "prepared"
        || target.version != a.identity_version
        || version(&mut tx, target.actor.user_id, purpose, slot).await? != a.expected_version
    {
        return Err(conflict());
    }
    sqlx::query("UPDATE portal_identity_assets SET state='uploading' WHERE id=$1")
        .bind(a.id)
        .execute(&mut *tx)
        .await?;
    log(&mut tx, a.id, &actor, a.user_id, "document.upload_started").await?;
    let a = asset(&mut tx, a.id).await?;
    tx.commit().await?;
    Ok(a)
}
pub async fn finish(pool: &PgPool, input: Finish) -> Result<Asset, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    let a = asset(&mut tx, input.asset_id).await?;
    let (actor, target) = asset_scope(&mut tx, &input.clerk_user_id, &a).await?;
    if input.public_id != a.public_id
        || input.format != a.format
        || input.byte_size != a.byte_size
        || input.content_hash != a.content_hash
    {
        return Err(bad());
    }
    if a.state == "ready" && matches!(input.outcome, Outcome::Ready) {
        tx.commit().await?;
        return Ok(a);
    }
    if !["uploading", "unknown"].contains(&a.state.as_str()) {
        return Err(conflict());
    }
    let next = match input.outcome {
        Outcome::Ready => "ready",
        Outcome::Unknown => "unknown",
        Outcome::Abandoned => "abandoned",
    };
    if next == a.state {
        tx.commit().await?;
        return Ok(a);
    }
    if next == "ready" {
        let purpose = operations::parse(a.purpose.clone())?;
        let slot = serde_json::from_value(serde_json::json!(a.slot)).map_err(|_| bad())?;
        if target.version != a.identity_version
            || version(&mut tx, a.user_id, purpose, slot).await? != a.expected_version
        {
            return Err(conflict());
        }
    }
    sqlx::query("UPDATE portal_identity_assets SET state=$2,completed_at=CASE WHEN $2='ready' THEN clock_timestamp() ELSE NULL END WHERE id=$1").bind(a.id).bind(next).execute(&mut *tx).await?;
    if next == "ready" && a.purpose == "profile" {
        sqlx::query("INSERT INTO portal_identity_document_slots(user_id,slot,asset_id) VALUES($1,$2,$3) ON CONFLICT(user_id,slot) DO UPDATE SET asset_id=EXCLUDED.asset_id").bind(a.user_id).bind(&a.slot).bind(a.id).execute(&mut *tx).await?;
    }
    log(&mut tx, a.id, &actor, a.user_id, "document.upload_result").await?;
    let a = asset(&mut tx, a.id).await?;
    tx.commit().await?;
    Ok(a)
}
pub async fn remove(pool: &PgPool, input: Remove) -> Result<View, ApiError> {
    if input.expected_version < 0 || input.expected_identity_version < 1 {
        return Err(bad());
    }
    let fingerprint = hash(&input)?;
    let mut tx = begin_mutation(pool).await?;
    let (actor, target) = scope(
        &mut tx,
        &input.clerk_user_id,
        input.target_user_id,
        Purpose::Profile,
        input.slot,
        true,
    )
    .await?;
    let old = replay(&mut tx, input.operation_id, &fingerprint).await?;
    let v = version(&mut tx, target.actor.user_id, Purpose::Profile, input.slot).await?;
    if old.is_none() {
        if target.version != input.expected_identity_version || v != input.expected_version {
            return Err(conflict());
        }
        profiles::limit(&mut tx, actor.actor.user_id, "documents", 40, 3600).await?;
        sqlx::query("INSERT INTO portal_identity_document_slots(user_id,slot,asset_id) VALUES($1,$2,NULL) ON CONFLICT(user_id,slot) DO UPDATE SET asset_id=NULL").bind(target.actor.user_id).bind(input.slot.name()).execute(&mut *tx).await?;
        commit(
            &mut tx,
            input.operation_id,
            &actor,
            target.actor.user_id,
            "document_remove",
            fingerprint,
            v + 1,
        )
        .await?;
    } else {
        log(
            &mut tx,
            input.operation_id,
            &actor,
            target.actor.user_id,
            "document.replay",
        )
        .await?;
    }
    // Return the current slot, never an old tombstone after a later replacement.
    let current: Option<Uuid> = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT asset_id FROM portal_identity_document_slots WHERE user_id=$1 AND slot=$2",
    )
    .bind(target.actor.user_id)
    .bind(input.slot.name())
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    let current = if let Some(id) = current {
        Some(asset(&mut tx, id).await?)
    } else {
        None
    };
    let view = View {
        target_user_id: target.actor.user_id,
        identity_version: target.version,
        version: version(&mut tx, target.actor.user_id, Purpose::Profile, input.slot).await?,
        slot: input.slot,
        asset: current,
    };
    tx.commit().await?;
    Ok(view)
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityDocumentUploadsQuery)]
pub struct UploadsQuery {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    #[schema(value_type=Option<String>)]
    pub after: Option<Uuid>,
    pub limit: i64,
}
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as=IdentityDocumentUploads)]
pub struct Uploads {
    pub items: Vec<Asset>,
    #[schema(value_type=Option<String>)]
    pub next: Option<Uuid>,
}
pub async fn uploads(pool: &PgPool, input: UploadsQuery) -> Result<Uploads, ApiError> {
    if !(1..=50).contains(&input.limit) {
        return Err(bad());
    }
    let mut tx = begin_mutation(pool).await?;
    let actor = actor(&mut tx, &input.clerk_user_id).await?;
    target(&mut tx, &actor, input.target_user_id).await?;
    let mut items:Vec<Asset>=sqlx::query_as("SELECT id,user_id,purpose,slot,public_id,format,byte_size,content_hash,state,expected_version,identity_version,(extract(epoch from created_at)*1000)::bigint AS created_at,(extract(epoch from completed_at)*1000)::bigint AS completed_at FROM portal_identity_assets WHERE actor_id=$1 AND user_id=$2 AND state<>'abandoned' AND ($3::uuid IS NULL OR id>$3) ORDER BY id LIMIT $4").bind(actor.actor.user_id).bind(input.target_user_id).bind(input.after).bind(input.limit+1).fetch_all(&mut *tx).await?;
    let next = if items.len() > input.limit as usize {
        items.pop();
        items.last().map(|a| a.id)
    } else {
        None
    };
    log(
        &mut tx,
        Uuid::new_v4(),
        &actor,
        input.target_user_id,
        "document.uploads_read",
    )
    .await?;
    tx.commit().await?;
    Ok(Uploads { items, next })
}
