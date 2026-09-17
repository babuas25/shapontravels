//! Versioned customer applications with one atomic local approval transaction.
use super::{operations::User, phase5::*, *};
use crate::auth::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schema(as=IdentityApplicationFields)]
pub struct Fields {
    pub agency_name: String,
    pub business_mobile: String,
    pub business_email: String,
    pub business_address: String,
    pub full_name: String,
    pub business_type: String,
    pub personal_mobile: String,
    pub personal_address: String,
}
impl Fields {
    fn validate(&mut self) -> Result<(), ApiError> {
        for v in [
            &mut self.agency_name,
            &mut self.business_mobile,
            &mut self.business_email,
            &mut self.business_address,
            &mut self.full_name,
            &mut self.business_type,
            &mut self.personal_mobile,
            &mut self.personal_address,
        ] {
            *v = text(v, 500, true)?;
        }
        let p: Vec<_> = self.business_email.split('@').collect();
        if p.len() != 2
            || p[0].is_empty()
            || !p[1].contains('.')
            || p[1].starts_with('.')
            || p[1].ends_with('.')
            || self.business_email.chars().any(char::is_whitespace)
            || !["proprietor", "partner"].contains(&self.business_type.as_str())
        {
            return Err(bad());
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityApplicationQuery)]
pub struct Query {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
}
#[derive(Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityApplicationSubmit)]
pub struct Submit {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub operation_id: Uuid,
    pub expected_version: i64,
    pub expected_identity_version: i64,
    pub fields: Fields,
    #[schema(value_type=Vec<String>)]
    pub documents: Vec<Uuid>,
}
#[derive(Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as=IdentityApplicationDecision)]
pub enum Decision {
    Accept,
    Reject,
}
#[derive(Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityApplicationReview)]
pub struct Review {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub operation_id: Uuid,
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    pub expected_version: i64,
    pub expected_identity_version: i64,
    pub expected_profile_version: i64,
    pub decision: Decision,
    pub note: String,
}
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityApplicationView)]
pub struct View {
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    pub identity_version: i64,
    pub profile_version: i64,
    pub version: i64,
    pub status: Option<String>,
    pub fields: Option<Fields>,
    #[schema(value_type=Vec<String>)]
    pub documents: Vec<Uuid>,
    #[schema(value_type=Option<String>)]
    pub reviewer_id: Option<Uuid>,
    pub review_note: Option<String>,
    pub replayed: bool,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub reviewed_at: Option<i64>,
}
type ApplicationRow = (
    i64,
    String,
    serde_json::Value,
    Vec<Uuid>,
    Option<Uuid>,
    Option<String>,
    i64,
    i64,
    Option<i64>,
);
async fn load(tx: &mut Transaction<'_, Postgres>, target: &User) -> Result<View, ApiError> {
    let row:Option<ApplicationRow>=sqlx::query_as("SELECT version,status,fields,documents,reviewer_id,review_note,(extract(epoch from created_at)*1000)::bigint,(extract(epoch from updated_at)*1000)::bigint,(extract(epoch from reviewed_at)*1000)::bigint FROM portal_identity_applications WHERE user_id=$1").bind(target.actor.user_id).fetch_optional(&mut **tx).await?;
    let profile_version = sqlx::query_scalar(
        "SELECT version FROM portal_identity_profiles WHERE user_id=$1 AND kind='profile'",
    )
    .bind(target.actor.user_id)
    .fetch_optional(&mut **tx)
    .await?
    .unwrap_or(0);
    let mut v = View {
        target_user_id: target.actor.user_id,
        identity_version: target.version,
        profile_version,
        version: 0,
        status: None,
        fields: None,
        documents: vec![],
        reviewer_id: None,
        review_note: None,
        replayed: false,
        created_at: None,
        updated_at: None,
        reviewed_at: None,
    };
    if let Some((version, status, fields, documents, reviewer, note, created, updated, reviewed)) =
        row
    {
        v.created_at = Some(created);
        v.updated_at = Some(updated);
        v.reviewed_at = reviewed;
        v.version = version;
        v.status = Some(status);
        v.fields = Some(serde_json::from_value(fields).map_err(|_| bad())?);
        v.documents = documents;
        v.reviewer_id = reviewer;
        v.review_note = note;
    }
    Ok(v)
}
pub async fn query(pool: &PgPool, input: Query) -> Result<View, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    let actor = actor(&mut tx, &input.clerk_user_id).await?;
    let target = target(&mut tx, &actor, input.target_user_id).await?;
    if actor.actor.role.manages_users() {
        profiles::limit(&mut tx, actor.actor.user_id, "application_review", 60, 3600).await?;
    }
    let view = load(&mut tx, &target).await?;
    log(
        &mut tx,
        Uuid::new_v4(),
        &actor,
        input.target_user_id,
        "application.read",
    )
    .await?;
    tx.commit().await?;
    Ok(view)
}
pub async fn submit(pool: &PgPool, mut input: Submit) -> Result<View, ApiError> {
    input.fields.validate()?;
    if input.expected_version < 0
        || input.expected_identity_version < 1
        || input.documents.len() > 5
        || input
            .documents
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != input.documents.len()
    {
        return Err(bad());
    }
    let fingerprint = hash(&input)?;
    let mut tx = begin_mutation(pool).await?;
    let actor = actor(&mut tx, &input.clerk_user_id).await?;
    if actor.actor.role != Role::Customer {
        return Err(denied());
    }
    let mut view = load(&mut tx, &actor).await?;
    if replay(&mut tx, input.operation_id, &fingerprint)
        .await?
        .is_some()
    {
        view.replayed = true;
        log(
            &mut tx,
            input.operation_id,
            &actor,
            actor.actor.user_id,
            "application.replay",
        )
        .await?;
        tx.commit().await?;
        return Ok(view);
    }
    if view.version != input.expected_version
        || actor.version != input.expected_identity_version
        || !matches!(view.status.as_deref(), None | Some("rejected"))
    {
        return Err(conflict());
    }
    profiles::limit(&mut tx, actor.actor.user_id, "application_submit", 6, 3600).await?;
    for id in &input.documents {
        let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_assets WHERE id=$1 AND user_id=$2 AND purpose='application' AND state='ready')").bind(id).bind(actor.actor.user_id).fetch_one(&mut *tx).await?;
        if !owned {
            return Err(denied());
        }
    }
    sqlx::query("INSERT INTO portal_identity_applications(user_id,status,fields,documents) VALUES($1,'pending',$2,$3) ON CONFLICT(user_id) DO UPDATE SET status='pending',fields=EXCLUDED.fields,documents=EXCLUDED.documents,reviewer_id=NULL,review_note=NULL,reviewed_at=NULL")
 .bind(actor.actor.user_id).bind(serde_json::to_value(&input.fields).map_err(|_|bad())?).bind(&input.documents).execute(&mut *tx).await?;
    commit(
        &mut tx,
        input.operation_id,
        &actor,
        actor.actor.user_id,
        "application_submit",
        fingerprint,
        view.version + 1,
    )
    .await?;
    view = load(&mut tx, &actor).await?;
    tx.commit().await?;
    Ok(view)
}
pub async fn review(pool: &PgPool, mut input: Review) -> Result<View, ApiError> {
    input.note = text(&input.note, 500, false)?;
    if input.expected_version < 1
        || input.expected_identity_version < 1
        || input.expected_profile_version < 0
    {
        return Err(bad());
    }
    let fingerprint = hash(&input)?;
    let mut tx = begin_mutation(pool).await?;
    let actor = actor(&mut tx, &input.clerk_user_id).await?;
    let target = target(&mut tx, &actor, input.target_user_id).await?;
    if actor.actor.status != Status::Active
        || !actor.actor.role.can_grant(Role::B2b)
        || actor.actor.user_id == input.target_user_id
    {
        return Err(denied());
    }
    let mut view = load(&mut tx, &target).await?;
    if replay(&mut tx, input.operation_id, &fingerprint)
        .await?
        .is_some()
    {
        view.replayed = true;
        log(
            &mut tx,
            input.operation_id,
            &actor,
            input.target_user_id,
            "application.replay",
        )
        .await?;
        tx.commit().await?;
        return Ok(view);
    }
    if target.actor.role != Role::Customer
        || !matches!(target.actor.status, Status::Onboarding | Status::Active)
        || view.status.as_deref() != Some("pending")
        || view.version != input.expected_version
        || target.version != input.expected_identity_version
        || view.profile_version != input.expected_profile_version
    {
        return Err(conflict());
    }
    profiles::limit(&mut tx, actor.actor.user_id, "application_review", 60, 3600).await?;
    let accepted = matches!(input.decision, Decision::Accept);
    if accepted {
        let agency = agencies::provision(&mut tx, &target).await?;
        // Presence, including explicit empty strings, wins over application transfer.
        let f = view.fields.as_ref().ok_or_else(conflict)?;
        let current: serde_json::Value = sqlx::query_scalar(
            "SELECT fields FROM portal_identity_profiles WHERE user_id=$1 AND kind='profile'",
        )
        .bind(input.target_user_id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(serde_json::json!({}));
        let mut patch = serde_json::Map::new();
        for (key, value) in [
            ("agencyName", &f.agency_name),
            ("agencyMobile", &f.business_mobile),
            ("agencyEmail", &f.business_email),
            ("agencyAddress", &f.business_address),
            ("mobile", &f.personal_mobile),
            ("address", &f.personal_address),
        ] {
            if current.get(key).is_none() {
                patch.insert(key.into(), serde_json::json!(value));
            }
        }
        if current.get("givenName").is_none() && current.get("surname").is_none() {
            patch.insert("givenName".into(), serde_json::json!(&f.full_name));
        }
        if current.get("email").is_none() {
            let email: Option<String> =
                sqlx::query_scalar("SELECT email FROM portal_users WHERE id=$1")
                    .bind(input.target_user_id)
                    .fetch_one(&mut *tx)
                    .await?;
            if let Some(email) = email {
                patch.insert("email".into(), serde_json::json!(email));
            }
        }
        sqlx::query("INSERT INTO portal_identity_profiles(user_id,kind,fields) VALUES($1,'profile',$2) ON CONFLICT(user_id,kind) DO UPDATE SET fields=portal_identity_profiles.fields || EXCLUDED.fields").bind(input.target_user_id).bind(serde_json::Value::Object(patch)).execute(&mut *tx).await?;
        operations::revoke_clients(&mut tx, std::slice::from_ref(&target.subject)).await?;
        effects(
            &mut tx,
            input.operation_id,
            &actor,
            &target,
            "approve_application",
            fingerprint.clone(),
            None,
        )
        .await?;
        sqlx::query("INSERT INTO portal_identity_agency_results(operation_id,agency_id,agency_version,agency_status) SELECT $1,id,version,status FROM portal_agencies WHERE id=$2").bind(input.operation_id).bind(agency).execute(&mut *tx).await?;
        mail::enqueue(&mut tx, "b2b_activated", input.target_user_id).await?;
    }
    sqlx::query("UPDATE portal_identity_applications SET status=$2,reviewer_id=$3,review_note=$4,reviewed_at=clock_timestamp() WHERE user_id=$1").bind(input.target_user_id).bind(if accepted{"accepted"}else{"rejected"}).bind(actor.actor.user_id).bind(&input.note).execute(&mut *tx).await?;
    commit(
        &mut tx,
        input.operation_id,
        &actor,
        input.target_user_id,
        if accepted {
            "application_accept"
        } else {
            "application_reject"
        },
        fingerprint,
        view.version + 1,
    )
    .await?;
    let updated = operations::user(&mut tx, input.target_user_id).await?;
    view = load(&mut tx, &updated).await?;
    tx.commit().await?;
    Ok(view)
}
#[derive(Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityApplicationQueue)]
pub struct Queue {
    pub clerk_user_id: String,
    #[schema(value_type=Option<String>)]
    pub after: Option<Uuid>,
    pub limit: i64,
}
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as=IdentityApplicationPage)]
pub struct Page {
    pub items: Vec<Pending>,
    #[schema(value_type=Option<String>)]
    pub next: Option<Uuid>,
}
#[derive(Serialize, sqlx::FromRow, utoipa::ToSchema)]
#[schema(as=IdentityApplicationPending)]
pub struct Pending {
    pub submitted_at: i64,
    #[schema(value_type=String)]
    pub user_id: Uuid,
    pub version: i64,
}
pub async fn queue(pool: &PgPool, input: Queue) -> Result<Page, ApiError> {
    if !(1..=50).contains(&input.limit) {
        return Err(bad());
    }
    let mut tx = begin_mutation(pool).await?;
    let actor = actor(&mut tx, &input.clerk_user_id).await?;
    if actor.actor.status != Status::Active || !actor.actor.role.manages_users() {
        return Err(denied());
    }
    profiles::limit(&mut tx, actor.actor.user_id, "application_review", 60, 3600).await?;
    let mut items:Vec<Pending>=sqlx::query_as("SELECT a.user_id,a.version,(extract(epoch from a.updated_at)*1000)::bigint AS submitted_at FROM portal_identity_applications a JOIN portal_users u ON u.id=a.user_id WHERE a.status='pending' AND u.role='customer' AND u.status IN ('active','onboarding') AND ($1::uuid IS NULL OR a.user_id>$1) ORDER BY a.user_id LIMIT $2").bind(input.after).bind(input.limit+1).fetch_all(&mut *tx).await?;
    let next = if items.len() > input.limit as usize {
        items.pop();
        items.last().map(|i| i.user_id)
    } else {
        None
    };
    tx.commit().await?;
    Ok(Page { items, next })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_fields_are_strict_and_bounded() {
        let value = serde_json::json!({"agencyName":" Agency ","businessMobile":"1","businessEmail":"office@example.invalid","businessAddress":"Office\nSecond line","fullName":"Full Name","businessType":"partner","personalMobile":"2","personalAddress":"Home"});
        let mut fields: Fields = serde_json::from_value(value.clone()).unwrap();
        fields.validate().unwrap();
        assert_eq!(fields.agency_name, "Agency");
        for (key, invalid) in [
            ("agencyName", " "),
            ("businessEmail", "bad@@example.invalid"),
            ("businessType", "company"),
            ("fullName", "bad\0name"),
        ] {
            let mut bad = value.clone();
            bad[key] = serde_json::json!(invalid);
            assert!(
                serde_json::from_value::<Fields>(bad)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        let mut bad = value.clone();
        bad["agencyName"] = serde_json::json!("x".repeat(501));
        assert!(
            serde_json::from_value::<Fields>(bad)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut bad = value;
        bad["role"] = serde_json::json!("admin");
        assert!(serde_json::from_value::<Fields>(bad).is_err());
    }
}
