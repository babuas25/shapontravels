//! Super Admin import workflow. Stored quotes bind evidence, owner and price;
//! the existing wallet ledger reserves and captures exactly once per import.
pub(crate) mod api;
mod itinerary;
mod pricing;
mod receipt;
use crate::{
    AppState,
    auth::{ApiError, digest},
    identity::business,
    wallet::core,
};
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
fn invalid() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "INVALID_IMPORT_DATA")
}
fn conflict(code: &'static str) -> ApiError {
    ApiError(StatusCode::CONFLICT, code)
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    History,
    Wallet {
        assigned_user: String,
        currency: String,
    },
    RefreshItinerary {
        reference: String,
        itinerary: Value,
    },
    CorrectLegacyCost {
        reference: String,
        supplier_minor: i64,
        reason: String,
    },
    Price {
        assigned_user: String,
        data: Value,
    },
    Authorize {
        quote_id: Uuid,
        confirmation_id: Uuid,
    },
    SupplierRead {
        supplier: String,
        reference: String,
        filter: Option<String>,
        payload: Option<Value>,
    },
    Read {
        reference: String,
    },
    Prepare {
        assigned_user: String,
        source: String,
        provider: String,
        data: Value,
        gross_minor: i64,
        payable_minor: i64,
        supplier_minor: i64,
        authorize: bool,
    },
    Import {
        quote_id: Uuid,
        request_id: Uuid,
        charge: bool,
        authorization_id: Option<Uuid>,
    },
    Status {
        reference: String,
        expected_version: i64,
        status: String,
        data: Value,
    },
}
#[derive(sqlx::FromRow)]
struct Quote {
    id: Uuid,
    assigned_user_id: Uuid,
    agency_code: String,
    source: String,
    provider: String,
    supplier_reference: String,
    currency: String,
    gross_minor: i64,
    payable_minor: i64,
    supplier_minor: i64,
    data: Value,
    fingerprint: Vec<u8>,
    expires_at: chrono::DateTime<chrono::Utc>,
}
fn str_field<'a>(v: &'a Value, key: &str, max: usize) -> Result<&'a str, ApiError> {
    v[key]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control))
        .ok_or_else(invalid)
}
fn validate(data: &Value) -> Result<(), ApiError> {
    if data.to_string().len() > 180000 {
        return Err(invalid());
    }
    str_field(data, "supplierReference", 120)?;
    str_field(data, "pnr", 120)?;
    let currency = str_field(data, "currency", 3)?;
    crate::wallet::money::currency(currency)?;
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(invalid());
    }
    if !["on-hold", "confirmed", "cancelled", "expired"]
        .contains(&data["lifecycleStatus"].as_str().unwrap_or(""))
    {
        return Err(invalid());
    }
    let pax = data["passengers"]["travellers"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 20)
        .ok_or_else(invalid)?;
    for p in pax {
        str_field(p, "firstName", 100)?;
        str_field(p, "lastName", 100)?;
    }
    let legs = data["itinerary"]["legs"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 20)
        .ok_or_else(invalid)?;
    for l in legs {
        if l["segments"]
            .as_array()
            .is_none_or(|s| s.is_empty() || s.len() > 40)
        {
            return Err(invalid());
        }
    }
    if data["lifecycleStatus"] == "confirmed" {
        evidence(data)?;
    } else if data["ticketNumbers"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
    {
        return Err(invalid());
    }
    Ok(())
}
fn evidence(data: &Value) -> Result<(), ApiError> {
    let pax = data["passengers"]["travellers"]
        .as_array()
        .ok_or_else(invalid)?
        .len();
    let tickets = data["ticketNumbers"]
        .as_array()
        .filter(|a| a.len() == pax && pax > 0)
        .ok_or(conflict("COMPLETE_CONFIRMED_IMPORT_EVIDENCE_REQUIRED"))?;
    let mut seen = std::collections::HashSet::new();
    for t in tickets {
        let t = t
            .as_str()
            .filter(|s| (10..=16).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_digit()))
            .ok_or_else(invalid)?;
        if !seen.insert(t.trim().to_uppercase()) {
            return Err(invalid());
        }
    }
    if let Some(value) = data["issuedAt"].as_str() {
        let date = chrono::DateTime::parse_from_rfc3339(value).map_err(|_| invalid())?;
        if date > chrono::Utc::now() + chrono::Duration::minutes(5) {
            return Err(invalid());
        }
    }
    Ok(())
}
async fn account(
    tx: &mut Transaction<'_, Postgres>,
    agency: &str,
    currency: &str,
) -> Result<Uuid, ApiError> {
    sqlx::query_scalar("SELECT a.id FROM wallet_accounts a JOIN wallet_owners o ON o.id=a.owner_id WHERE o.owner_type='agency' AND o.owner_key=$1 AND a.currency=$2")
 .bind(agency).bind(currency).fetch_optional(&mut **tx).await?.ok_or(conflict("WALLET_ACCOUNT_NOT_FOUND"))
}
async fn charge(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    agency: &str,
    currency: &str,
    amount: i64,
    actor: &business::Principal,
) -> Result<Uuid, ApiError> {
    let account = account(tx, agency, currency).await?;
    let op = core::reserve(
        tx,
        core::Reservation {
            id: Uuid::new_v4(),
            account,
            kind: "manual_issue",
            subject: id,
            booking: None,
            amount,
            currency,
            actor: &actor.subject,
            role: &actor.role,
        },
    )
    .await?;
    core::settle(tx, op.id, true, &actor.subject, &actor.role).await?;
    Ok(op.id)
}
async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    id: Uuid,
    action: &str,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES('admin',$1,$2,'portal_import',$3,'{}')").bind(actor.to_string()).bind(action).bind(id.to_string()).execute(&mut **tx).await?;
    Ok(())
}
async fn result(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    replay: bool,
) -> Result<Value, ApiError> {
    let mut v:Value=sqlx::query_scalar("SELECT jsonb_build_object('ok',true,'success',true,'referenceNo',coalesce(booking_reference,public_ref),'supplierReference',display_supplier_reference,'bookingOrderUrl','/dashboard/bookings/import/'||coalesce(booking_reference,public_ref),'status',status,'paymentState',CASE WHEN operation_id IS NULL THEN 'unpaid' ELSE 'captured' END,'walletCharged',operation_id IS NOT NULL,'chargedAmount',CASE WHEN operation_id IS NULL THEN 0 ELSE payable_minor END,'currency',currency,'version',version) FROM portal_import_bookings WHERE id=$1").bind(id).fetch_one(&mut **tx).await?;
    let document =
        receipt::document(tx, v["referenceNo"].as_str().ok_or_else(invalid)?, None).await?;
    let native = api::for_import(tx, id, document.clone()).await?;
    let response = if native.confirmed() {
        native.ticket()?
    } else {
        native.booking()
    };
    v.as_object_mut()
        .ok_or_else(invalid)?
        .extend(response.as_object().ok_or_else(invalid)?.clone());
    v["booking"] = document["booking"].clone();
    v["travellers"] = document["travellers"].clone();
    v["replay"] = json!(replay);
    Ok(v)
}
async fn execute(
    State(state): State<AppState>,
    Json(cmd): Json<Command>,
) -> Result<Json<Value>, ApiError> {
    let actor = business::current_principal()?;
    if actor.role != "superadmin" {
        return Err(ApiError(StatusCode::FORBIDDEN, "IMPORT_FORBIDDEN"));
    }
    if let Command::SupplierRead {
        supplier,
        reference,
        filter,
        payload,
    } = cmd
    {
        let prefix = match supplier.as_str() {
            "firsttrip" => "FST",
            "takeoff" => "TOT",
            "triplover" => "TLL",
            _ => return Err(invalid()),
        };
        if reference.len() != 21
            || !reference.starts_with(prefix)
            || !reference[3..].bytes().all(|b| b.is_ascii_digit())
        {
            return Err(invalid());
        }
        business::begin(&state.pool).await?.commit().await?;
        let adapter = state
            .suppliers
            .get(&supplier)
            .ok_or(conflict("SUPPLIER_NOT_CONFIGURED"))?;
        let value = if let Some(payload) = payload {
            if payload["UniqueTransID"] != reference {
                return Err(invalid());
            }
            adapter
                .transport
                .read(crate::supplier::ReadOperation::Pnr, &payload)
                .await
        } else {
            adapter
                .transport
                .import_report(&reference, filter.as_deref().unwrap_or("Confirmed"))
                .await
        }
        .map_err(|_| ApiError(StatusCode::BAD_GATEWAY, "SUPPLIER_READ_FAILED"))?;
        business::begin(&state.pool).await?.commit().await?;
        return Ok(Json(value));
    }
    let mut tx = business::begin(&state.pool).await?;
    let output = match cmd {
        Command::SupplierRead { .. } => unreachable!(),
        Command::Wallet { assigned_user, currency } => {
            if !["BDT", "USD"].contains(&currency.as_str()) { return Err(invalid()); }
            let target = business::principal(&mut tx, &assigned_user).await?;
            if !["b2b", "b2b_sub"].contains(&target.role.as_str()) { return Err(conflict("INVALID_IMPORT_ASSIGNEE")); }
            let agency = target.agency_code.ok_or(conflict("ASSIGNEE_AGENCY_REQUIRED"))?;
            let wallet: Option<Value> = sqlx::query_scalar("SELECT jsonb_build_object('currency',a.currency,'availableMinor',a.available_balance::text,'holdMinor',a.hold_balance::text,'status',o.status) FROM wallet_accounts a JOIN wallet_owners o ON o.id=a.owner_id WHERE o.owner_type='agency' AND o.owner_key=$1 AND a.currency=$2").bind(&agency).bind(&currency).fetch_optional(&mut *tx).await?;
            json!({"wallet":wallet,"agencyCode":agency,"currency":currency})
        }

        Command::Price {
            assigned_user,
            data,
        } => pricing::price(&mut tx, &assigned_user, data).await?,
        Command::Authorize {
            quote_id,
            confirmation_id,
        } => {
            let same:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_import_quotes a JOIN portal_import_quotes b ON b.fingerprint=a.fingerprint AND b.actor_id=a.actor_id WHERE a.id=$1 AND b.id=$2 AND a.actor_id=$3 AND a.expires_at>clock_timestamp() AND b.expires_at>clock_timestamp())").bind(quote_id).bind(confirmation_id).bind(actor.id).fetch_one(&mut *tx).await?;
            if !same {
                return Err(conflict("SUPPLIER_REFERENCE_PRICING_CHANGED"));
            }
            // Preview does not reserve funds. Recheck before authorization; reserve()
            // still enforces available balance atomically at the final import.
            let quote: Quote = sqlx::query_as("SELECT * FROM portal_import_quotes WHERE id=$1").bind(quote_id).fetch_one(&mut *tx).await?;
            // A completed exact import may be recovered after a lost response.
            // Its captured payment is not a second proposed debit.
            let already_imported: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_import_bookings b JOIN portal_import_quotes original ON original.id=b.quote_id JOIN portal_import_quotes candidate ON candidate.fingerprint=original.fingerprint AND candidate.actor_id=original.actor_id WHERE candidate.id=$1 AND b.operation_id IS NOT NULL)").bind(quote_id).fetch_one(&mut *tx).await?;
            if quote.data["lifecycleStatus"] == "confirmed" && !already_imported {
                let wallet: Option<(i64, String)> = sqlx::query_as("SELECT a.available_balance,o.status FROM wallet_accounts a JOIN wallet_owners o ON o.id=a.owner_id WHERE o.owner_type='agency' AND o.owner_key=$1 AND a.currency=$2").bind(&quote.agency_code).bind(&quote.currency).fetch_optional(&mut *tx).await?;
                let (available, status) = wallet.ok_or(conflict("WALLET_ACCOUNT_NOT_FOUND"))?;
                if status != "active" { return Err(conflict("WALLET_FROZEN")); }
                if available < quote.payable_minor { return Err(conflict("INSUFFICIENT_FUNDS")); }
            }
            sqlx::query("UPDATE portal_import_quotes SET authorized=true WHERE id=$1")
                .bind(quote_id)
                .execute(&mut *tx)
                .await?;
            json!({"success":true,"authorizationId":quote_id,"walletMutation":false})
        }
        Command::RefreshItinerary { reference, itinerary } => {
            let (id, original):(Uuid,Value)=sqlx::query_as("SELECT b.id,coalesce((SELECT d.itinerary FROM portal_import_itinerary_details d WHERE d.booking_id=b.id ORDER BY d.created_at DESC,d.id DESC LIMIT 1),b.data->'itinerary') FROM portal_import_bookings b WHERE (b.public_ref=$1 OR b.booking_reference=$1) AND b.source<>'MANUAL' FOR UPDATE").bind(reference).fetch_optional(&mut *tx).await?.ok_or(ApiError(StatusCode::NOT_FOUND,"IMPORT_NOT_FOUND"))?;
            let merged = itinerary::merge(&original, &itinerary)?;
            sqlx::query("INSERT INTO portal_import_itinerary_details(id,booking_id,itinerary,actor_id) VALUES($1,$2,$3,$4)").bind(Uuid::new_v4()).bind(id).bind(merged).bind(actor.id).execute(&mut *tx).await?;
            audit(&mut tx, actor.id, id, "import.itinerary_refreshed").await?;
            json!({"ok":true,"walletMutation":false})
        }
        Command::CorrectLegacyCost { reference, supplier_minor, reason } => {
            if !(1..=99_999_999_999_999).contains(&supplier_minor) || !(10..=500).contains(&reason.len()) {
                return Err(invalid());
            }
            let id:Uuid=sqlx::query_scalar("SELECT id FROM portal_import_bookings WHERE (public_ref=$1 OR booking_reference=$1) AND source='IMP_EXP' AND status='confirmed' AND data->'rustPricing' IS NULL FOR UPDATE").bind(reference).fetch_optional(&mut *tx).await?.ok_or(conflict("LEGACY_IMPORT_COST_CORRECTION_UNAVAILABLE"))?;
            sqlx::query("INSERT INTO portal_import_cost_corrections(booking_id,supplier_minor,reason,actor_id) VALUES($1,$2,$3,$4)").bind(id).bind(supplier_minor).bind(reason).bind(actor.id).execute(&mut *tx).await?;
            audit(&mut tx, actor.id, id, "import.supplier_cost_corrected").await?;
            json!({"ok":true,"walletMutation":false})
        }
        Command::History => {
            let rows:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',b.id,'importedOn',b.created_at,'referenceNo',coalesce(b.booking_reference,b.public_ref),'supplierReference',b.display_supplier_reference,'provider',b.provider,'source',b.source,'status',b.status,'paxName',concat_ws(' ',b.data#>>'{passengers,travellers,0,firstName}',b.data#>>'{passengers,travellers,0,lastName}'),'assigned',concat_ws(' ',u.first_name,u.last_name),'supplierGross',b.gross_minor::numeric/100,'userPayable',b.payable_minor::numeric/100,'paymentState',CASE WHEN b.operation_id IS NULL THEN 'unpaid' ELSE 'captured' END,'currency',b.currency) FROM portal_import_bookings b JOIN portal_users u ON u.id=b.assigned_user_id ORDER BY b.created_at DESC,b.id DESC LIMIT 500").fetch_all(&mut *tx).await?;
            json!({"history":rows})
        }
        Command::Read { reference } => sqlx::query_scalar(
            "SELECT (to_jsonb(b)-'quote_id')||jsonb_build_object('public_ref',coalesce(booking_reference,public_ref),'supplier_minor',coalesce((SELECT c.supplier_minor FROM portal_import_cost_corrections c WHERE c.booking_id=b.id),b.supplier_minor)) FROM portal_import_bookings b WHERE public_ref=$1 OR booking_reference=$1",
        )
        .bind(reference)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "IMPORT_NOT_FOUND"))?,
        Command::Prepare {
            assigned_user,
            source,
            provider,
            mut data,
            gross_minor,
            payable_minor,
            supplier_minor,
            authorize,
        } => {
            if !["MANUAL", "IMP_EXP", "SUPPLIER_API"].contains(&source.as_str())
                || !match source.as_str() {
                    "MANUAL" => provider == "MANUAL",
                    "IMP_EXP" => ["US_BANGLA", "AIR_ASTRA", "NOVOAIR"].contains(&provider.as_str()),
                    _ => ["firsttrip", "takeoff", "triplover"].contains(&provider.as_str()),
                }
                || payable_minor <= 0
                || gross_minor < 0
                || supplier_minor < 0
                || [gross_minor, payable_minor, supplier_minor]
                    .iter()
                    .any(|n| *n > 99_999_999_999_999)
            {
                return Err(invalid());
            }
            validate(&data)?;
            if source != "IMP_EXP"
                && data["lifecycleStatus"] == "confirmed"
                && data["issuedAt"].as_str().is_none()
            {
                return Err(conflict("COMPLETE_CONFIRMED_IMPORT_EVIDENCE_REQUIRED"));
            }
            if source == "SUPPLIER_API" || source == "IMP_EXP" {
                let pricing = pricing::price(&mut tx, &assigned_user, data.clone()).await?;
                if crate::wallet::money::major_to_minor(
                    pricing["payable"].as_str().ok_or_else(invalid)?,
                )? != payable_minor
                    || crate::wallet::money::major_to_minor(
                        pricing["gross"].as_str().ok_or_else(invalid)?,
                    )? != gross_minor
                    || data["supplierPayable"].to_string().parse::<bigdecimal::BigDecimal>().map_err(|_| invalid())? != bigdecimal::BigDecimal::from(supplier_minor) / 100
                {
                    return Err(conflict("SUPPLIER_REFERENCE_PRICING_CHANGED"));
                }
                data["rustPricing"] = pricing;
            }

            let target = business::principal(&mut tx, &assigned_user).await?;
            if !["b2b", "b2b_sub"].contains(&target.role.as_str()) {
                return Err(conflict("INVALID_IMPORT_ASSIGNEE"));
            }
            let agency = target
                .agency_code
                .ok_or(conflict("ASSIGNEE_AGENCY_REQUIRED"))?;
            let reference = str_field(&data, "supplierReference", 120)?
                .trim()
                .to_uppercase();
            let currency = str_field(&data, "currency", 3)?.to_owned();
            if authorize && data["lifecycleStatus"] != "confirmed" {
                return Err(conflict("CHARGE_DECISION_NOT_APPLICABLE"));
            }
            // Volatile fetch times never affect replay identity. Every passenger, price and
            // itinerary field remains part of the authorization fingerprint.
            if let Some(o) = data.as_object_mut() {
                for k in [
                    "importedAt",
                    "retrievedAt",
                    "pricingCalculatedAt",
                    "supplierEvidenceTimestamp",
                ] {
                    o.remove(k);
                }
            }
            data["supplierReference"] = json!(reference);
            let hash = digest(
                &json!([
                    target.id,
                    agency,
                    source,
                    provider,
                    currency,
                    gross_minor,
                    payable_minor,
                    supplier_minor,
                    data
                ])
                .to_string(),
            );
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO portal_import_quotes(id,actor_id,assigned_user_id,agency_code,source,provider,supplier_reference,currency,gross_minor,payable_minor,supplier_minor,data,fingerprint,authorized) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)").bind(id).bind(actor.id).bind(target.id).bind(agency).bind(source).bind(provider).bind(reference).bind(currency).bind(gross_minor).bind(payable_minor).bind(supplier_minor).bind(data).bind(hash).bind(authorize).execute(&mut *tx).await?;
            audit(&mut tx, actor.id, id, "import.prepared").await?;
            json!({"quoteId":id,"authorizationId":if authorize{Some(id)}else{None},"expiresAt":chrono::Utc::now()+chrono::Duration::minutes(10),"amount":payable_minor,"walletMutation":false})
        }
        Command::Import {
            quote_id,
            request_id,
            charge: do_charge,
            authorization_id,
        } => {
            let q: Quote =
                sqlx::query_as("SELECT * FROM portal_import_quotes WHERE id=$1 AND actor_id=$2")
                    .bind(quote_id)
                    .bind(actor.id)
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or_else(invalid)?;
            if let Some((id,hash))=sqlx::query_as::<_,(Uuid,Vec<u8>)>("SELECT booking_id,fingerprint FROM portal_import_requests WHERE actor_id=$1 AND request_id=$2").bind(actor.id).bind(request_id).fetch_optional(&mut *tx).await?{
 if hash!=q.fingerprint{return Err(conflict("IMPORT_REQUEST_IDENTITY_MISMATCH"));}result(&mut tx,id,true).await?
 }else{
 if q.expires_at<chrono::Utc::now(){return Err(conflict("IMPORT_QUOTE_EXPIRED"));}
 let subject:String=sqlx::query_scalar("SELECT clerk_user_id FROM portal_users WHERE id=$1").bind(q.assigned_user_id).fetch_one(&mut *tx).await?;
 let current=business::principal(&mut tx,&subject).await?;if current.agency_code.as_deref()!=Some(&q.agency_code){return Err(conflict("IMPORT_ASSIGNEE_MISMATCH"));}
 if do_charge!=(q.data["lifecycleStatus"]=="confirmed"){return Err(conflict("CONFIRMED_IMPORT_REQUIRES_CHARGE"));}
 if do_charge&&q.source!="MANUAL"{
 let auth:Option<Vec<u8>>=sqlx::query_scalar("SELECT fingerprint FROM portal_import_quotes WHERE id=$1 AND actor_id=$2 AND authorized AND expires_at>clock_timestamp()").bind(authorization_id).bind(actor.id).fetch_optional(&mut *tx).await?;
 if auth.as_ref()!=Some(&q.fingerprint){return Err(conflict("FRESH_MATCHING_CHARGE_AUTHORIZATION_REQUIRED"));}
 }
 let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_import_bookings WHERE provider=$1 AND supplier_reference=$2) OR EXISTS(SELECT 1 FROM flight_bookings WHERE supplier_id=$1 AND (supplier_booking_ref=$2 OR request->>'uniqueTransID'=$2))").bind(&q.provider).bind(&q.supplier_reference).fetch_one(&mut *tx).await?;
 if exists{return Err(conflict("BOOKING_ALREADY_IMPORTED"));}
 let id=Uuid::new_v4();
 let reference:Option<String>=sqlx::query_scalar("SELECT booking_reference_from_evidence(jsonb_build_object('item2',jsonb_build_object('isSuccess',true),'item1',jsonb_build_object('pnr',$1::jsonb->'pnr','airlinesPNR',$1::jsonb->'airlinesPnr')),NULL,false)").bind(&q.data).fetch_one(&mut *tx).await?;
 let reference=reference.ok_or(conflict("IMPORT_REFERENCE_EVIDENCE_REQUIRED"))?;
 let duplicate:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM flight_bookings WHERE public_ref=$1) OR EXISTS(SELECT 1 FROM portal_import_bookings WHERE booking_reference=$1)").bind(&reference).fetch_one(&mut *tx).await?;
 if duplicate{return Err(conflict("BOOKING_ALREADY_IMPORTED"));}
 let op=if do_charge{Some(charge(&mut tx,id,&q.agency_code,&q.currency,q.payable_minor,&actor).await?)}else{None};
 let issued=if do_charge{Some(q.data["issuedAt"].as_str().and_then(|s|chrono::DateTime::parse_from_rfc3339(s).ok()).map(|d|d.with_timezone(&chrono::Utc)).unwrap_or_else(chrono::Utc::now))}else{None};
 sqlx::query("INSERT INTO portal_import_bookings(id,public_ref,quote_id,creator_id,assigned_user_id,agency_code,source,provider,supplier_reference,currency,gross_minor,payable_minor,supplier_minor,status,data,operation_id,issued_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)").bind(id).bind(reference).bind(q.id).bind(actor.id).bind(q.assigned_user_id).bind(q.agency_code).bind(q.source).bind(q.provider).bind(q.supplier_reference).bind(q.currency).bind(q.gross_minor).bind(q.payable_minor).bind(q.supplier_minor).bind(q.data["lifecycleStatus"].as_str()).bind(&q.data).bind(op).bind(issued).execute(&mut *tx).await?;
 sqlx::query("INSERT INTO portal_import_requests(actor_id,request_id,fingerprint,booking_id) VALUES($1,$2,$3,$4)").bind(actor.id).bind(request_id).bind(q.fingerprint).bind(id).execute(&mut *tx).await?;
 audit(&mut tx,actor.id,id,"import.created").await?;result(&mut tx,id,false).await?
 }
        }
        Command::Status {
            reference,
            expected_version,
            status,
            data,
        } => {
            let row:(Uuid,String,String,i64,String,Value,i64,String)=sqlx::query_as("SELECT id,agency_code,currency,payable_minor,status,data,version,source FROM portal_import_bookings WHERE (public_ref=$1 OR booking_reference=$1) FOR UPDATE").bind(reference).fetch_optional(&mut *tx).await?.ok_or_else(invalid)?;
            let (id, agency, currency, amount, old, mut saved, version, source) = row;
            if status == "confirmed" && source != "IMP_EXP" && data["issuedAt"].as_str().is_none() {
                return Err(conflict("COMPLETE_CONFIRMED_IMPORT_EVIDENCE_REQUIRED"));
            }
            if version != expected_version {
                return Err(conflict("IMPORT_VERSION_CHANGED"));
            }
            if old != "on-hold" || !["confirmed", "cancelled", "expired"].contains(&status.as_str())
            {
                return Err(conflict("IMPORT_STATUS_NOT_ALLOWED"));
            }
            let op = if status == "confirmed" {
                saved["ticketNumbers"] = data["ticketNumbers"].clone();
                saved["issuedAt"] = data["issuedAt"].clone();
                evidence(&saved)?;
                Some(charge(&mut tx, id, &agency, &currency, amount, &actor).await?)
            } else {
                None
            };
            saved["lifecycleStatus"] = json!(status);
            let issued = if status == "confirmed" {
                Some(
                    saved["issuedAt"]
                        .as_str()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .unwrap_or_else(chrono::Utc::now),
                )
            } else {
                None
            };
            sqlx::query("UPDATE portal_import_bookings SET status=$2,data=$3,operation_id=$4,issued_at=$5,version=version+1,updated_at=clock_timestamp() WHERE id=$1").bind(id).bind(status).bind(saved).bind(op).bind(issued).execute(&mut *tx).await?;
            audit(&mut tx, actor.id, id, "import.status_changed").await?;
            result(&mut tx, id, false).await?
        }
    };
    tx.commit().await?;
    Ok(Json(output))
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/portal-imports", post(execute))
        .route("/admin/portal-imports/receipt", post(receipt::read))
}
