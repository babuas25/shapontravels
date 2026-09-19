use super::{
    Decision, Mutation, ReviewDecision, label, missing,
    rules::{self, Action, Direction, Quote, RequestType, Status},
};
use crate::wallet::{Result, conflict, core, forbidden, invalid, money, portal::Actor};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

type Tx<'a> = Transaction<'a, Postgres>;
#[derive(sqlx::FromRow)]
pub(super) struct Booking {
    id: Uuid,
    public_ref: String,
    wallet_account_id: Uuid,
    operation_id: Uuid,
    amount: i64,
    currency: String,
    request: Value,
    ticket: Value,
    pricing: Value,
    selling: Value,
    issued_at: DateTime<Utc>,
    owner_type: String,
    owner_key: String,
    display: Value,
    execution_mode: String,
}
#[derive(sqlx::FromRow)]
pub(super) struct Request {
    id: Uuid,
    public_ref: String,
    booking_id: Uuid,
    wallet_account_id: Uuid,
    action: String,
    request_type: String,
    status: String,
    terminal_outcome: Option<String>,
    pub version: i64,
    currency: String,
    request_note: Option<String>,
    snapshot: Value,
    active_quote_id: Option<Uuid>,
    approved_quote_id: Option<Uuid>,
    assignee_user_id: Option<String>,
    assignee_role: Option<String>,
    hold_operation_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    status_changed_at: DateTime<Utc>,
}
#[derive(Clone, sqlx::FromRow)]
struct Entitlement {
    id: Uuid,
    passenger_index: i32,
    passenger_name: String,
    passenger_type: String,
    ticket_number: String,
    amount: i64,
    funding: Value,
    state: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Funding {
    operation_id: Uuid,
    amount: i64,
}

pub(super) async fn booking(tx: &mut Tx<'_>, actor: &Actor, reference: &str) -> Result<Booking> {
    booking_scoped(tx, actor, reference, None).await
}
pub(super) async fn booking_scoped(
    tx: &mut Tx<'_>,
    actor: &Actor,
    reference: &str,
    client: Option<Uuid>,
) -> Result<Booking> {
    if reference.is_empty()
        || reference.len() > 40
        || !reference
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(invalid("INVALID_BOOKING_REFERENCE"));
    }
    // Inner joins intentionally reject unpaid/imported/historical tickets. A
    // missing charge cannot be substituted with the displayed gross fare.
    let mut matches: Vec<Booking> = sqlx::query_as("SELECT b.id,b.public_ref,w.wallet_account_id,w.id operation_id,w.amount,w.currency,b.request,COALESCE(v.public_response,t.public_response) ticket,r.tier_pricing pricing,r.selling,CASE WHEN v.issue_id IS NOT NULL THEN t.created_at ELSE t.updated_at END issued_at,o.owner_type,o.owner_key,jsonb_build_object('name',COALESCE(NULLIF(trim(profile.fields->>'agencyName'),''),NULLIF(trim(concat_ws(' ',owner.first_name,owner.last_name)),''),o.display->>'name')) display,b.execution_mode FROM flight_bookings b JOIN flight_ticket_issues t ON t.booking_id=b.id JOIN flight_reprices r ON r.id=b.price_id AND r.client_id=b.client_id LEFT JOIN flight_ticket_verifications v ON v.issue_id=t.id JOIN wallet_operations w ON w.subject_kind='ticket_issue' AND w.subject_id=t.id AND w.booking_id=b.id JOIN wallet_accounts a ON a.id=w.wallet_account_id JOIN wallet_owners o ON o.id=a.owner_id LEFT JOIN portal_agencies agency ON o.owner_type='agency' AND agency.agency_code=o.owner_key LEFT JOIN portal_users owner ON owner.id=agency.owner_user_id LEFT JOIN portal_identity_profiles profile ON profile.user_id=owner.id AND profile.kind='profile' WHERE b.public_ref=$1 AND ($5::uuid IS NULL OR b.client_id=$5) AND r.accepted_at IS NOT NULL AND w.state='captured' AND (t.state='issued' OR v.issue_id IS NOT NULL) AND ($2 OR (o.owner_type=$3 AND o.owner_key=$4)) ORDER BY b.id LIMIT 2 FOR UPDATE OF b")
        .bind(reference).bind(actor.staff_read()).bind(actor.owner.as_ref().map(|o| &o.owner_type)).bind(actor.owner.as_ref().map(|o| &o.owner_key)).bind(client).fetch_all(&mut **tx).await?;
    if matches.len() > 1 {
        return Err(conflict("AMBIGUOUS_BOOKING_REFERENCE"));
    }
    let b = matches.pop().ok_or_else(missing)?;
    rules::minor(b.amount)?;
    if b.ticket.is_null() || b.pricing.is_null() {
        return Err(conflict("TICKET_ENTITLEMENT_UNAVAILABLE"));
    }
    Ok(b)
}
pub(super) async fn request(tx: &mut Tx<'_>, actor: &Actor, id: Uuid) -> Result<Request> {
    request_scoped(tx, actor, id, None).await
}
pub(super) async fn request_scoped(
    tx: &mut Tx<'_>,
    actor: &Actor,
    id: Uuid,
    client: Option<Uuid>,
) -> Result<Request> {
    let booking: Uuid = sqlx::query_scalar("SELECT r.booking_id FROM ticket_management_requests r JOIN wallet_accounts a ON a.id=r.wallet_account_id JOIN wallet_owners o ON o.id=a.owner_id WHERE r.id=$1 AND ($5::uuid IS NULL OR EXISTS(SELECT 1 FROM flight_bookings b WHERE b.id=r.booking_id AND b.client_id=$5)) AND ($2 OR (o.owner_type=$3 AND o.owner_key=$4))")
        .bind(id).bind(actor.staff_read()).bind(actor.owner.as_ref().map(|o| &o.owner_type)).bind(actor.owner.as_ref().map(|o| &o.owner_key)).bind(client).fetch_optional(&mut **tx).await?.ok_or_else(missing)?;
    sqlx::query("SELECT id FROM flight_bookings WHERE id=$1 FOR UPDATE")
        .bind(booking)
        .execute(&mut **tx)
        .await?;
    let mut r: Request =
        sqlx::query_as("SELECT * FROM ticket_management_requests WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
    expire(tx, &mut r).await?;
    Ok(r)
}
pub(super) async fn replay(
    tx: &mut Tx<'_>,
    actor: &Actor,
    key: Uuid,
    hash: &[u8],
) -> Result<Option<Value>> {
    let old: Option<(Vec<u8>, Value)> = sqlx::query_as("SELECT request_hash,result FROM ticket_management_replays WHERE actor_id=$1 AND request_key=$2").bind(&actor.external_user_id).bind(key).fetch_optional(&mut **tx).await?;
    match old {
        None => Ok(None),
        Some((previous, mut result)) => {
            if previous != hash {
                return Err(conflict("IDEMPOTENCY_KEY_REUSED"));
            }
            result["replay"] = json!(true);
            Ok(Some(result))
        }
    }
}
pub(super) async fn save_replay(
    tx: &mut Tx<'_>,
    actor: &Actor,
    key: Uuid,
    request: Uuid,
    hash: &[u8],
    result: &Value,
) -> Result<()> {
    sqlx::query("INSERT INTO ticket_management_replays(actor_id,request_key,request_hash,request_id,result) VALUES($1,$2,$3,$4,$5)").bind(&actor.external_user_id).bind(key).bind(hash).bind(request).bind(result).execute(&mut **tx).await?;
    Ok(())
}
async fn entitlements(tx: &mut Tx<'_>, booking: Uuid) -> Result<Vec<Entitlement>> {
    Ok(sqlx::query_as("SELECT * FROM ticket_management_entitlements WHERE booking_id=$1 ORDER BY passenger_index,created_at DESC,id").bind(booking).fetch_all(&mut **tx).await?)
}
async fn initialize(tx: &mut Tx<'_>, b: &Booking) -> Result<()> {
    if !entitlements(tx, b.id).await?.is_empty() {
        return Ok(());
    }
    let initial = rules::allocate_initial(
        &b.request["passengerInfoes"],
        &b.ticket["item1"]["ticketInfoes"],
        &b.pricing,
        b.amount,
    )?;
    for e in initial {
        sqlx::query("INSERT INTO ticket_management_entitlements(id,booking_id,wallet_account_id,passenger_index,passenger_name,passenger_type,ticket_number,currency,amount,funding) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(Uuid::new_v4()).bind(b.id).bind(b.wallet_account_id).bind(e.passenger_index as i32).bind(e.passenger_name).bind(e.passenger_type).bind(e.ticket_number).bind(&b.currency).bind(e.amount_minor).bind(json!([Funding { operation_id:b.operation_id, amount:e.amount_minor }])).execute(&mut **tx).await?;
    }
    Ok(())
}
fn routes(b: &Booking) -> Result<Vec<Value>> {
    let dirs = b.selling["item1"]["directions"]
        .as_array()
        .ok_or_else(|| conflict("TICKET_ROUTES_UNAVAILABLE"))?;
    let mut result = Vec::new();
    for (i, alternatives) in dirs.iter().enumerate() {
        let options = alternatives
            .as_array()
            .ok_or_else(|| conflict("TICKET_ROUTES_UNAVAILABLE"))?;
        if options.len() != 1 {
            return Err(conflict("TICKET_ROUTES_UNAVAILABLE"));
        }
        let segments = options[0]["segments"]
            .as_array()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| conflict("TICKET_ROUTES_UNAVAILABLE"))?;
        let first = &segments[0];
        let last = segments.last().unwrap();
        let from = first["from"]
            .as_str()
            .ok_or_else(|| conflict("TICKET_ROUTES_UNAVAILABLE"))?;
        let to = last["to"]
            .as_str()
            .ok_or_else(|| conflict("TICKET_ROUTES_UNAVAILABLE"))?;
        result.push(json!({"routeIndex":i,"label":format!("{from} → {to}"),"origin":from,"destination":to,"departureAt":first["departure"]}));
    }
    Ok(result)
}
pub(super) async fn availability(tx: &mut Tx<'_>, b: &Booking) -> Result<Value> {
    initialize(tx, b).await?;
    expire_booking(tx, b.id).await?;
    let all = entitlements(tx, b.id).await?;
    let claimed: Vec<Uuid> = sqlx::query_scalar("SELECT c.entitlement_id FROM ticket_management_claims c JOIN ticket_management_entitlements e ON e.id=c.entitlement_id WHERE e.booking_id=$1").bind(b.id).fetch_all(&mut **tx).await?;
    let count = b.request["passengerInfoes"].as_array().map_or(0, Vec::len);
    let passengers = (0..count).map(|i| {
        let e = all.iter().filter(|e| e.passenger_index == i as i32).find(|e| e.state == "active").or_else(|| all.iter().find(|e| e.passenger_index == i as i32));
        let reason = match e { Some(e) if claimed.contains(&e.id) => Some("ACTIVE_REQUEST"), Some(e) if e.state == "active" => None,
            Some(e) if e.state == "refunded" => Some("REFUNDED"), Some(e) if e.state == "voided" => Some("VOIDED"), Some(e) if e.state == "reissued" => Some("REISSUED"), _ => Some("UNAVAILABLE") };
        json!({"passengerIndex":i,"available":reason.is_none(),"reasonCode":reason,"message":reason.map(|r| match r { "ACTIVE_REQUEST"=>"This ticket already has an active request.","REFUNDED"=>"This ticket was refunded.","VOIDED"=>"This ticket was voided.",_=>"This ticket is unavailable." })})
    }).collect::<Vec<_>>();
    Ok(
        json!({"passengers":passengers,"routes":routes(b)?,"actions":if b.execution_mode == "hold" { if rules::void_open(b.issued_at, Utc::now()) {vec!["refund","reissue","void"]} else {vec!["refund","reissue"]} } else {vec!["reissue"]}}),
    )
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn create(
    tx: &mut Tx<'_>,
    actor: &Actor,
    b: &Booking,
    id: Uuid,
    action: Action,
    request_type: RequestType,
    passenger_indexes: &[usize],
    route_indexes: &[usize],
    note: Option<&str>,
) -> Result<Value> {
    create_at(
        tx,
        actor,
        b,
        id,
        action,
        request_type,
        passenger_indexes,
        route_indexes,
        note,
        Utc::now(),
    )
    .await
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn create_at(
    tx: &mut Tx<'_>,
    actor: &Actor,
    b: &Booking,
    id: Uuid,
    action: Action,
    request_type: RequestType,
    passenger_indexes: &[usize],
    route_indexes: &[usize],
    note: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Value> {
    create_with_preferences(
        tx,
        actor,
        b,
        id,
        action,
        request_type,
        passenger_indexes,
        route_indexes,
        note,
        &[],
        now,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn create_with_preferences(
    tx: &mut Tx<'_>,
    actor: &Actor,
    b: &Booking,
    id: Uuid,
    action: Action,
    request_type: RequestType,
    passenger_indexes: &[usize],
    route_indexes: &[usize],
    note: Option<&str>,
    preferences: &[super::client::ReissuePreference],
    now: DateTime<Utc>,
) -> Result<Value> {
    rules::text(note, 2000, false)?;
    if b.execution_mode != "hold" && action != Action::Reissue {
        return Err(conflict("TICKET_MANAGEMENT_ACTION_UNAVAILABLE"));
    }
    if action == Action::Void && !rules::void_open(b.issued_at, now) {
        return Err(conflict("VOID_REQUEST_WINDOW_CLOSED"));
    }
    initialize(tx, b).await?;
    expire_booking(tx, b.id).await?;
    let all = entitlements(tx, b.id).await?;
    let routes = routes(b)?;
    rules::indexes(
        passenger_indexes,
        b.request["passengerInfoes"].as_array().map_or(0, Vec::len),
    )?;
    rules::indexes(route_indexes, routes.len())?;
    let selected = passenger_indexes
        .iter()
        .map(|i| {
            all.iter()
                .find(|e| e.passenger_index == *i as i32 && e.state == "active")
                .ok_or_else(|| conflict("TICKET_ENTITLEMENT_UNAVAILABLE"))
        })
        .collect::<Result<Vec<_>>>()?;
    let active:bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ticket_management_requests WHERE booking_id=$1 AND action=$2 AND terminal_outcome IS NULL)").bind(b.id).bind(label(&action)).fetch_one(&mut **tx).await?;
    let claims: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM ticket_management_claims WHERE entitlement_id=ANY($1))",
    )
    .bind(selected.iter().map(|e| e.id).collect::<Vec<_>>())
    .fetch_one(&mut **tx)
    .await?;
    if active || claims {
        return Err(conflict("ACTIVE_REQUEST_EXISTS"));
    }
    let seq: i64 = sqlx::query_scalar("SELECT nextval('ticket_management_reference_seq')")
        .fetch_one(&mut **tx)
        .await?;
    let reference = format!(
        "TMR{}{:012X}",
        match action {
            Action::Refund => "R",
            Action::Reissue => "E",
            Action::Void => "V",
        },
        seq
    );
    let first = &b.selling["item1"]["directions"][0][0]["segments"][0];
    let snapshot = json!({"bookingReference":b.public_ref,"agencyName":b.display["name"],"agencyId":if b.owner_type=="agency"{Some(&b.owner_key)}else{None},"airline":first["airlineCode"],"airlineCode":first["airlineCode"],"flightDate":first["departure"],"issuedAt":b.issued_at,"grossFare":rules::minor(money::major_to_minor(b.pricing["gross"].as_str().ok_or_else(||conflict("TICKET_ENTITLEMENT_UNAVAILABLE"))?)?)?,"userPayable":b.amount,"reissuePreferences":preferences,"routes":route_indexes.iter().map(|i| &routes[*i]).collect::<Vec<_>>()});
    sqlx::query("INSERT INTO ticket_management_requests(id,public_ref,booking_id,wallet_account_id,requested_by_user_id,action,request_type,currency,request_note,snapshot) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)").bind(id).bind(&reference).bind(b.id).bind(b.wallet_account_id).bind(&actor.external_user_id).bind(label(&action)).bind(label(&request_type)).bind(&b.currency).bind(note).bind(snapshot).execute(&mut **tx).await?;
    for e in selected {
        sqlx::query(
            "INSERT INTO ticket_management_selections(request_id,entitlement_id) VALUES($1,$2)",
        )
        .bind(id)
        .bind(e.id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO ticket_management_claims(entitlement_id,request_id) VALUES($1,$2)",
        )
        .bind(e.id)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    }
    let r: Request = sqlx::query_as("SELECT * FROM ticket_management_requests WHERE id=$1")
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
    event(
        tx,
        &r,
        &actor.external_user_id,
        &actor.role,
        "requested",
        None,
        note,
        json!({}),
    )
    .await?;
    Ok(result(&r))
}
fn result(r: &Request) -> Value {
    json!({"ok":true,"requestId":r.id,"publicRef":r.public_ref,"status":r.status,"outcome":r.terminal_outcome,"version":r.version})
}
#[allow(clippy::too_many_arguments)]
async fn event(
    tx: &mut Tx<'_>,
    r: &Request,
    actor: &str,
    role: &str,
    kind: &str,
    from: Option<&str>,
    note: Option<&str>,
    metadata: Value,
) -> Result<()> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO ticket_management_events(id,request_id,request_version,event_type,actor_id,actor_role,from_status,to_status,note,metadata) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)").bind(id).bind(r.id).bind(r.version).bind(kind).bind(actor).bind(role).bind(from).bind(&r.status).bind(note).bind(metadata).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO ticket_management_outbox(event_id,request_id) VALUES($1,$2)")
        .bind(id)
        .bind(r.id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
async fn save(tx: &mut Tx<'_>, r: &mut Request, previous: &str) -> Result<()> {
    let status: Status = serde_json::from_value(json!(r.status))
        .map_err(|_| invalid("INVALID_TICKET_TRANSITION"))?;
    let outcome: Option<rules::Outcome> = serde_json::from_value(json!(r.terminal_outcome))
        .map_err(|_| invalid("INVALID_TICKET_TRANSITION"))?;
    if !rules::outcome_matches(status, outcome) {
        return Err(invalid("INVALID_TICKET_TRANSITION"));
    }
    if r.status != previous {
        let from: Status = serde_json::from_value(json!(previous))
            .map_err(|_| invalid("INVALID_TICKET_TRANSITION"))?;
        rules::transition(from, status, None)?;
    }
    r.version += 1;
    let now = Utc::now();
    r.updated_at = now;
    if r.status != previous {
        r.status_changed_at = now;
    }
    sqlx::query("UPDATE ticket_management_requests SET status=$2,terminal_outcome=$3,version=$4,active_quote_id=$5,approved_quote_id=$6,assignee_user_id=$7,assignee_role=$8,hold_operation_id=$9,updated_at=$10,status_changed_at=$11,settled_at=CASE WHEN $2='approved' AND $3::text IS NOT NULL THEN $10 ELSE NULL END WHERE id=$1")
        .bind(r.id).bind(&r.status).bind(&r.terminal_outcome).bind(r.version).bind(r.active_quote_id).bind(r.approved_quote_id).bind(&r.assignee_user_id).bind(&r.assignee_role).bind(r.hold_operation_id).bind(now).bind(r.status_changed_at).execute(&mut **tx).await?;
    if r.terminal_outcome.is_some() {
        sqlx::query("DELETE FROM ticket_management_claims WHERE request_id=$1")
            .bind(r.id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}
async fn expire(tx: &mut Tx<'_>, r: &mut Request) -> Result<()> {
    if r.status != "awaiting-confirmation" {
        return Ok(());
    }
    let expired:bool=sqlx::query_scalar("SELECT confirmation_deadline_at<=clock_timestamp() FROM ticket_management_quotes WHERE id=$1").bind(r.active_quote_id).fetch_one(&mut **tx).await?;
    if expired {
        r.status = "expired".into();
        r.terminal_outcome = Some("confirmation-expired".into());
        save(tx, r, "awaiting-confirmation").await?;
        event(
            tx,
            r,
            "ticket-management-expiry",
            "system",
            "confirmation-expired",
            Some("awaiting-confirmation"),
            None,
            json!({"quoteId":r.active_quote_id}),
        )
        .await?;
    }
    Ok(())
}
async fn expire_booking(tx: &mut Tx<'_>, booking: Uuid) -> Result<()> {
    let requests:Vec<Request>=sqlx::query_as("SELECT * FROM ticket_management_requests WHERE booking_id=$1 AND status='awaiting-confirmation' ORDER BY id FOR UPDATE").bind(booking).fetch_all(&mut **tx).await?;
    for mut r in requests {
        expire(tx, &mut r).await?;
    }
    Ok(())
}
async fn selections(tx: &mut Tx<'_>, r: &Request) -> Result<Vec<Entitlement>> {
    Ok(sqlx::query_as("SELECT e.* FROM ticket_management_entitlements e JOIN ticket_management_selections s ON s.entitlement_id=e.id WHERE s.request_id=$1 ORDER BY e.passenger_index").bind(r.id).fetch_all(&mut **tx).await?)
}
async fn active_selections(tx: &mut Tx<'_>, r: &Request) -> Result<Vec<Entitlement>> {
    let selected = selections(tx, r).await?;
    let claims: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ticket_management_claims WHERE request_id=$1")
            .bind(r.id)
            .fetch_one(&mut **tx)
            .await?;
    if selected.is_empty()
        || claims != selected.len() as i64
        || selected.iter().any(|e| e.state != "active")
    {
        return Err(conflict("TICKET_ENTITLEMENT_CONFLICT"));
    }
    Ok(selected)
}
async fn quote(tx: &mut Tx<'_>, id: Option<Uuid>) -> Result<Quote> {
    let v: Value = sqlx::query_scalar("SELECT data FROM ticket_management_quotes WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| conflict("TICKET_QUOTE_UNAVAILABLE"))?;
    serde_json::from_value(v).map_err(|_| conflict("TICKET_QUOTE_UNAVAILABLE"))
}

pub(super) async fn mutate(
    tx: &mut Tx<'_>,
    actor: &Actor,
    r: &mut Request,
    input: Mutation,
) -> Result<Value> {
    if matches!(&input, Mutation::ReleaseReissue { .. }) && r.action != "reissue"
        || matches!(&input, Mutation::ReleaseVoid { .. }) && r.action != "void"
    {
        return Err(conflict("TICKET_ACTION_MISMATCH"));
    }
    if r.terminal_outcome.is_some() {
        return Err(conflict("TICKET_REQUEST_TERMINAL"));
    }
    let before = r.status.clone();
    let action: Action =
        serde_json::from_value(json!(r.action)).map_err(|_| invalid("INVALID_TICKET_ACTION"))?;
    let mut metadata = json!({});
    let note: Option<String>;
    let event_type: &str;
    match input {
        Mutation::Review { decision, note: n } => {
            rules::operate(&actor.role)?;
            rules::text(n.as_deref(), 2000, false)?;
            if decision == ReviewDecision::Accept {
                if r.status != "requested" {
                    return Err(conflict("INVALID_TICKET_TRANSITION"));
                }
                r.status = "in-progress".into();
                event_type = "accepted";
            } else {
                if !["requested", "in-progress", "awaiting-confirmation"]
                    .contains(&r.status.as_str())
                {
                    return Err(conflict("INVALID_TICKET_TRANSITION"));
                }
                r.status = "rejected".into();
                r.terminal_outcome = Some("staff-rejected".into());
                event_type = "staff-rejected";
            }
            note = n;
        }
        Mutation::PublishQuote { quote: q } => {
            rules::operate(&actor.role)?;
            if r.status != "in-progress" {
                return Err(conflict("INVALID_TICKET_TRANSITION"));
            }
            let selected = active_selections(tx, r).await?;
            q.validate(
                action,
                &r.currency,
                &selected
                    .iter()
                    .map(|e| (e.id, e.amount))
                    .collect::<Vec<_>>(),
                Utc::now(),
            )?;
            let id = Uuid::new_v4();
            let version:i32=sqlx::query_scalar("SELECT COALESCE(max(quote_version),0)+1 FROM ticket_management_quotes WHERE request_id=$1").bind(r.id).fetch_one(&mut **tx).await?;
            sqlx::query("INSERT INTO ticket_management_quotes(id,request_id,quote_version,data,confirmation_deadline_at,published_by_user_id,published_by_role) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(id).bind(r.id).bind(version).bind(json!(&q)).bind(q.confirmation_deadline_at).bind(&actor.external_user_id).bind(&actor.role).execute(&mut **tx).await?;
            r.active_quote_id = Some(id);
            r.status = "awaiting-confirmation".into();
            metadata = json!({"quoteId":id,"quoteVersion":version,"confirmationDeadlineAt":q.confirmation_deadline_at});
            note = None;
            event_type = "quotation-published";
        }
        Mutation::CustomerDecision {
            quote_id,
            decision,
            note: n,
        } => {
            rules::owner(&actor.role)?;
            rules::text(n.as_deref(), 2000, false)?;
            if r.status != "awaiting-confirmation" || r.active_quote_id != Some(quote_id) {
                return Err(conflict("TICKET_QUOTE_SUPERSEDED"));
            }
            let q = quote(tx, Some(quote_id)).await?;
            if q.confirmation_deadline_at <= Utc::now() {
                return Err(conflict("TICKET_QUOTE_EXPIRED"));
            }
            let selected = active_selections(tx, r).await?;
            q.validate(
                action,
                &r.currency,
                &selected
                    .iter()
                    .map(|e| (e.id, e.amount))
                    .collect::<Vec<_>>(),
                Utc::now(),
            )?;
            if decision == Decision::Approved {
                if q.direction == Direction::Debit {
                    let hold = core::reserve(
                        tx,
                        core::Reservation {
                            id: Uuid::new_v4(),
                            account: r.wallet_account_id,
                            kind: "ticket_management",
                            subject: quote_id,
                            booking: Some(r.booking_id),
                            amount: q.customer_amount_minor,
                            currency: &r.currency,
                            actor: &actor.external_user_id,
                            role: &actor.role,
                        },
                    )
                    .await?;
                    if hold.state != "reserved" {
                        return Err(conflict("TICKET_HOLD_UNAVAILABLE"));
                    }
                    r.hold_operation_id = Some(hold.id);
                }
                r.approved_quote_id = Some(quote_id);
                r.status = "approved".into();
                event_type = "customer-approved";
            } else {
                r.status = "rejected".into();
                r.terminal_outcome = Some("customer-rejected".into());
                event_type = "customer-rejected";
            }
            metadata = json!({"quoteId":quote_id,"decision":label(&decision),"amount":q.customer_amount_minor});
            note = n;
        }
        Mutation::Assign {
            assignee_user_id,
            reason,
        } => {
            rules::operate(&actor.role)?;
            rules::text(reason.as_deref(), 2000, false)?;
            if r.status != "approved" {
                return Err(conflict("INVALID_TICKET_TRANSITION"));
            }
            let role:Option<String>=sqlx::query_scalar("SELECT role FROM portal_users WHERE clerk_user_id=$1 AND status='active' AND role IN ('staff_account','admin','superadmin')").bind(&assignee_user_id).fetch_optional(&mut **tx).await?;
            let role = role.ok_or_else(forbidden)?;
            metadata = json!({"assigneeUserId":assignee_user_id,"assigneeRole":role});
            r.assignee_user_id = Some(assignee_user_id);
            r.assignee_role = Some(role);
            note = reason;
            event_type = "settlement-assigned";
        }
        Mutation::Requote { reason } => {
            rules::operate(&actor.role)?;
            rules::text(Some(&reason), 2000, true)?;
            if r.status != "approved" || r.hold_operation_id.is_some() {
                return Err(conflict("TICKET_HOLD_RELEASE_REQUIRED"));
            }
            reopen(r);
            note = Some(reason);
            event_type = "requote-started";
        }
        Mutation::ReleaseReissue { reason } | Mutation::ReleaseVoid { reason } => {
            rules::finance(
                &actor.role,
                &actor.external_user_id,
                r.assignee_user_id.as_deref(),
            )?;
            rules::text(Some(&reason), 2000, true)?;
            // Caller action must match the request, checked before this union.
            if r.status != "approved" || action == Action::Refund {
                return Err(conflict("INVALID_TICKET_TRANSITION"));
            }
            if let Some(id) = r.hold_operation_id {
                validate_hold(tx, r).await?;
                core::settle(tx, id, false, &actor.external_user_id, &actor.role).await?;
                metadata = json!({"operationId":id,"releasedAmount":quote(tx,r.approved_quote_id).await?.customer_amount_minor});
            }
            reopen(r);
            note = Some(reason);
            event_type = "requote-started";
        }
        Mutation::CompleteRefund { note: n } => {
            if action != Action::Refund {
                return Err(conflict("TICKET_ACTION_MISMATCH"));
            }
            metadata = settle(tx, actor, r, None).await?;
            note = n;
            event_type = "refund-completed";
        }
        Mutation::CompleteVoid { note: n } => {
            if action != Action::Void {
                return Err(conflict("TICKET_ACTION_MISMATCH"));
            }
            metadata = settle(tx, actor, r, None).await?;
            note = n;
            event_type = "void-completed";
        }
        Mutation::CompleteReissue {
            new_tickets,
            note: n,
        } => {
            if action != Action::Reissue {
                return Err(conflict("TICKET_ACTION_MISMATCH"));
            }
            metadata = settle(tx, actor, r, Some(&new_tickets)).await?;
            note = n;
            event_type = "reissue-completed";
        }
    }
    rules::text(note.as_deref(), 2000, false)?;
    save(tx, r, &before).await?;
    event(
        tx,
        r,
        &actor.external_user_id,
        &actor.role,
        event_type,
        Some(&before),
        note.as_deref(),
        metadata,
    )
    .await?;
    Ok(result(r))
}
fn reopen(r: &mut Request) {
    r.status = "in-progress".into();
    r.active_quote_id = None;
    r.approved_quote_id = None;
    r.hold_operation_id = None;
    r.assignee_user_id = None;
    r.assignee_role = None;
}
async fn validate_hold(tx: &mut Tx<'_>, r: &Request) -> Result<()> {
    let q = quote(tx, r.approved_quote_id).await?;
    let id = r
        .hold_operation_id
        .ok_or_else(|| conflict("TICKET_HOLD_UNAVAILABLE"))?;
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM wallet_operations WHERE id=$1 AND wallet_account_id=$2 AND booking_id=$3 AND subject_kind='ticket_management' AND subject_id=$4 AND currency=$5 AND amount=$6 AND state='reserved')")
        .bind(id).bind(r.wallet_account_id).bind(r.booking_id).bind(r.approved_quote_id).bind(&r.currency).bind(q.customer_amount_minor).fetch_one(&mut **tx).await?;
    if !valid || q.direction != Direction::Debit {
        return Err(conflict("TICKET_HOLD_UNAVAILABLE"));
    }
    Ok(())
}
async fn settle(
    tx: &mut Tx<'_>,
    actor: &Actor,
    r: &mut Request,
    new_tickets: Option<&[rules::NewTicket]>,
) -> Result<Value> {
    rules::finance(
        &actor.role,
        &actor.external_user_id,
        r.assignee_user_id.as_deref(),
    )?;
    if r.status != "approved"
        || r.approved_quote_id.is_none()
        || r.approved_quote_id != r.active_quote_id
    {
        return Err(conflict("TICKET_APPROVAL_REQUIRED"));
    }
    let q = quote(tx, r.approved_quote_id).await?;
    let selected = active_selections(tx, r).await?;
    if let Some(tickets) = new_tickets {
        q.validate_tickets(tickets)?;
    }
    core::lock_account(tx, r.wallet_account_id).await?;
    let mut entries = Vec::<Uuid>::new();
    if q.direction == Direction::Debit {
        validate_hold(tx, r).await?;
        core::settle(
            tx,
            r.hold_operation_id.unwrap(),
            true,
            &actor.external_user_id,
            &actor.role,
        )
        .await?;
    } else if r.hold_operation_id.is_some() {
        return Err(conflict("TICKET_HOLD_UNAVAILABLE"));
    }
    if q.direction == Direction::Credit {
        let mut remaining = q.customer_amount_minor;
        // Entitlement funding survives repeated reissues. Refund only the net
        // accepted amount, bounded by each selected slice and the kernel cap.
        let mut credits = std::collections::BTreeMap::<Uuid, i64>::new();
        for e in &selected {
            let funding: Vec<Funding> = serde_json::from_value(e.funding.clone())
                .map_err(|_| conflict("TICKET_ENTITLEMENT_CONFLICT"))?;
            if rules::sum(funding.iter().map(|f| f.amount))? != e.amount {
                return Err(conflict("TICKET_ENTITLEMENT_CONFLICT"));
            }
            for f in funding {
                let amount = f.amount.min(remaining);
                if amount > 0 {
                    *credits.entry(f.operation_id).or_default() += amount;
                    remaining -= amount;
                }
            }
        }
        if remaining != 0 {
            return Err(conflict("TICKET_ENTITLEMENT_CONFLICT"));
        }
        for (operation, amount) in credits {
            let posted=core::post(tx,core::Posting { account:r.wallet_account_id,kind:"refund",amount,operation:Some(operation),booking:Some(r.booking_id),key:&format!("ticket-management:{}:{operation}",r.id),actor:&actor.external_user_id,role:&actor.role,remarks:"Ticket management settlement",metadata:json!({"ticketManagementRequestId":r.id,"quoteId":r.approved_quote_id}) }).await?;
            entries.push(
                serde_json::from_value(posted["id"].clone())
                    .map_err(|_| conflict("TICKET_POSTING_UNAVAILABLE"))?,
            );
        }
    }
    let outcome = match r.action.as_str() {
        "refund" => "refunded",
        "reissue" => "reissued",
        "void" => "voided",
        _ => return Err(invalid("INVALID_TICKET_ACTION")),
    };
    for e in &selected {
        sqlx::query("UPDATE ticket_management_entitlements SET state=$2 WHERE id=$1")
            .bind(e.id)
            .bind(outcome)
            .execute(&mut **tx)
            .await?;
    }
    let mut lineage = Vec::new();
    if let Some(tickets) = new_tickets {
        for e in &selected {
            let t = tickets
                .iter()
                .find(|t| t.predecessor_entitlement_id == e.id)
                .ok_or_else(|| conflict("REISSUE_ALLOCATION_MISMATCH"))?;
            let number = t.new_ticket_number.trim().to_uppercase();
            let duplicate:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ticket_management_entitlements WHERE booking_id=$1 AND ticket_number=$2)").bind(r.booking_id).bind(&number).fetch_one(&mut **tx).await?;
            if duplicate {
                return Err(conflict("REISSUE_TICKET_ALREADY_EXISTS"));
            }
            let mut funding: Vec<Funding> = serde_json::from_value(e.funding.clone())
                .map_err(|_| conflict("TICKET_ENTITLEMENT_CONFLICT"))?;
            if t.fare_difference_amount_minor > 0 {
                funding.push(Funding {
                    operation_id: r
                        .hold_operation_id
                        .ok_or_else(|| conflict("TICKET_HOLD_UNAVAILABLE"))?,
                    amount: t.fare_difference_amount_minor,
                });
            }
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO ticket_management_entitlements(id,booking_id,wallet_account_id,passenger_index,passenger_name,passenger_type,ticket_number,currency,amount,funding,predecessor_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
                .bind(id).bind(r.booking_id).bind(r.wallet_account_id).bind(e.passenger_index).bind(&e.passenger_name).bind(&e.passenger_type).bind(&number).bind(&r.currency).bind(rules::sum([e.amount,t.fare_difference_amount_minor])?).bind(json!(funding)).bind(e.id).execute(&mut **tx).await?;
            lineage.push(json!({"passengerIndex":e.passenger_index,"predecessorEntitlementId":e.id,"successorEntitlementId":id,"previousTicketNumber":e.ticket_number,"newTicketNumber":number,"fareDifferenceAmount":t.fare_difference_amount_minor}));
        }
    }
    r.terminal_outcome = Some(outcome.into());
    Ok(
        json!({"quoteId":r.approved_quote_id,"outcome":outcome,"amount":q.customer_amount_minor,"direction":q.direction,"ledgerEntryIds":entries,"holdOperationId":r.hold_operation_id,"ticketLineage":lineage}),
    )
}

fn summary(r: &Request, staff: bool) -> Value {
    let mut v = json!({"id":r.id,"publicRef":r.public_ref,"bookingReference":r.snapshot["bookingReference"],"action":r.action,"requestType":r.request_type,"status":r.status,"terminalOutcome":r.terminal_outcome,"version":r.version,"currency":r.currency,"activeQuoteId":r.active_quote_id,"approvedQuoteId":r.approved_quote_id,"statusChangedAt":r.status_changed_at,"createdAt":r.created_at,"updatedAt":r.updated_at});
    if staff {
        v["assigneeUserId"] = json!(r.assignee_user_id);
        v["assigneeRole"] = json!(r.assignee_role);
    }
    v
}
fn passenger(e: &Entitlement, actor: &Actor) -> Value {
    let restricted = actor.role == "staff_account";
    let mut v = json!({"passengerIndex":e.passenger_index,"passengerName":if restricted{"Restricted"}else{&e.passenger_name},"passengerType":e.passenger_type,"ticketNumber":if restricted{None}else{Some(&e.ticket_number)}});
    if actor.staff_read() {
        v["entitlementId"] = json!(e.id);
        v["entitlementAmount"] = json!(e.amount);
    }
    v
}
pub(super) async fn detail(tx: &mut Tx<'_>, actor: &Actor, r: &Request) -> Result<Value> {
    let staff = actor.staff_read();
    let selected = selections(tx, r).await?;
    let mut v = summary(r, staff);
    v["requestNote"] = json!(r.request_note);
    v["passengers"] = json!(
        selected
            .iter()
            .map(|e| passenger(e, actor))
            .collect::<Vec<_>>()
    );
    v["routes"] = r.snapshot["routes"].clone();
    v["reissuePreferences"] = r
        .snapshot
        .get("reissuePreferences")
        .cloned()
        .unwrap_or(json!([]));
    let rows:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(q) FROM ticket_management_quotes q WHERE request_id=$1 ORDER BY quote_version").bind(r.id).fetch_all(&mut **tx).await?;
    let mut quotes = Vec::new();
    for row in rows {
        let q: Quote = serde_json::from_value(row["data"].clone())
            .map_err(|_| conflict("TICKET_QUOTE_UNAVAILABLE"))?;
        let mut view = json!({"id":row["id"],"quoteVersion":row["quote_version"],"currency":q.currency,"direction":q.direction,"userPayableEntitlementAmount":q.user_payable_entitlement_amount_minor,"fareDifference":q.fare_difference_minor,"airlineFee":q.airline_fee_minor,"voidFee":q.void_fee_minor,"serviceFee":q.service_fee_minor,"customerAmount":q.customer_amount_minor,"details":q.details,"confirmationDeadlineAt":q.confirmation_deadline_at,"publishedAt":row["published_at"]});
        if staff {
            view["publishedByUserId"] = row["published_by_user_id"].clone();
            view["publishedByRole"] = row["published_by_role"].clone();
            view["fareDifferenceAllocations"]=json!(q.reissue_fare_difference_allocations.iter().map(|a|json!({"entitlementId":a.entitlement_id,"passengerIndex":selected.iter().find(|e|e.id==a.entitlement_id).map(|e|e.passenger_index),"fareDifferenceAmount":a.fare_difference_amount_minor})).collect::<Vec<_>>());
        }
        quotes.push(view);
    }
    v["quotes"] = json!(quotes);
    let events:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('eventType',event_type,'fromStatus',from_status,'toStatus',to_status,'requestVersion',request_version,'actorUserId',actor_id,'actorRole',actor_role,'note',note,'metadata',metadata,'effectiveAt',effective_at) FROM ticket_management_events WHERE request_id=$1 ORDER BY request_version").bind(r.id).fetch_all(&mut **tx).await?;
    v["reissuedTickets"] = json!(events.iter().filter_map(|e| e["metadata"]["ticketLineage"].as_array()).flatten().map(|t| json!({"passengerIndex":t["passengerIndex"],"previousTicketNumber":t["previousTicketNumber"],"newTicketNumber":t["newTicketNumber"]})).collect::<Vec<_>>());
    v["decisions"]=json!(events.iter().filter(|e|["customer-approved","customer-rejected"].contains(&e["eventType"].as_str().unwrap_or(""))).map(|e|{
        let mut d=json!({"decision":e["metadata"]["decision"],"quoteId":e["metadata"]["quoteId"],"decidedAt":e["effectiveAt"]});
        if staff {d["decidedByUserId"]=e["actorUserId"].clone();d["decidedByRole"]=e["actorRole"].clone();} d
    }).collect::<Vec<_>>());
    if staff {
        v["walletResults"] = json!({"holdOperationId":r.hold_operation_id});
        v["assignments"] = json!(
            events
                .iter()
                .filter(|e| e["eventType"] == "settlement-assigned")
                .map(|e| e["metadata"].clone())
                .collect::<Vec<_>>()
        );
        v["ticketLineage"] = json!(
            events
                .iter()
                .filter_map(|e| e["metadata"]["ticketLineage"].as_array())
                .flatten()
                .collect::<Vec<_>>()
        );
        v["events"] = json!(events);
    } else {
        v["events"]=json!(events.into_iter().filter(|e|["requested","accepted","staff-rejected","quotation-published","customer-approved","customer-rejected","confirmation-expired","requote-started"].contains(&e["eventType"].as_str().unwrap_or(""))).map(|e|{
            let mut meta=json!({});for key in ["quoteId","quoteVersion","confirmationDeadlineAt","deadline","outcome","amount"] {if let Some(x)=e["metadata"].get(key){meta[key]=x.clone();}}
            json!({"eventType":e["eventType"],"fromStatus":e["fromStatus"],"toStatus":e["toStatus"],"effectiveAt":e["effectiveAt"],"metadata":meta})
        }).collect::<Vec<_>>());
    }
    Ok(v)
}
pub(super) async fn list(
    tx: &mut Tx<'_>,
    actor: &Actor,
    reference: Option<&str>,
    status: Option<Status>,
    action: Option<Action>,
    request_type: Option<RequestType>,
    limit: i64,
) -> Result<Value> {
    list_scoped(
        tx,
        actor,
        reference,
        status,
        action,
        request_type,
        limit,
        None,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn list_scoped(
    tx: &mut Tx<'_>,
    actor: &Actor,
    reference: Option<&str>,
    status: Option<Status>,
    action: Option<Action>,
    request_type: Option<RequestType>,
    limit: i64,
    client: Option<Uuid>,
) -> Result<Value> {
    if !(1..=100).contains(&limit) {
        return Err(invalid("INVALID_TICKET_QUERY"));
    }
    // Expiry must precede filtering so an elapsed quote never looks actionable.
    // Process the caller's scoped records; never leak a foreign booking ID.
    let ids:Vec<Uuid>=sqlx::query_scalar("SELECT r.id FROM ticket_management_requests r JOIN wallet_accounts a ON a.id=r.wallet_account_id JOIN wallet_owners o ON o.id=a.owner_id WHERE ($9::uuid IS NULL OR EXISTS(SELECT 1 FROM flight_bookings b WHERE b.id=r.booking_id AND b.client_id=$9)) AND ($1 OR (o.owner_type=$2 AND o.owner_key=$3)) AND ($4::text IS NULL OR r.snapshot->>'bookingReference'=$4) AND ($5::text IS NULL OR r.action=$5) AND ($6::text IS NULL OR r.request_type=$6) AND ($7::text IS NULL OR (CASE WHEN r.status='awaiting-confirmation' AND EXISTS(SELECT 1 FROM ticket_management_quotes q WHERE q.id=r.active_quote_id AND q.confirmation_deadline_at<=clock_timestamp()) THEN 'expired' ELSE r.status END)=$7) ORDER BY r.created_at DESC,r.id DESC LIMIT $8")
        .bind(actor.staff_read()).bind(actor.owner.as_ref().map(|o|&o.owner_type)).bind(actor.owner.as_ref().map(|o|&o.owner_key)).bind(reference).bind(action.as_ref().map(label)).bind(request_type.as_ref().map(label)).bind(status.as_ref().map(label)).bind(limit).bind(client).fetch_all(&mut **tx).await?;
    let mut items = Vec::new();
    for id in ids {
        let r = request_scoped(tx, actor, id, client).await?;
        if status.as_ref().is_some_and(|s| label(s) != r.status) {
            continue;
        }
        let mut v = summary(&r, actor.staff_read());
        for k in [
            "agencyName",
            "agencyId",
            "airline",
            "airlineCode",
            "flightDate",
            "issuedAt",
            "grossFare",
            "userPayable",
        ] {
            v[k] = if (["agencyName", "agencyId"].contains(&k) && !actor.staff_read())
                || (k == "grossFare" && actor.role == "customer")
            {
                Value::Null
            } else {
                r.snapshot[k].clone()
            };
        }
        let people = selections(tx, &r).await?;
        v["passengerDetails"] = json!(
            people
                .iter()
                .map(|e| passenger(e, actor))
                .collect::<Vec<_>>()
        );
        let last:Value=sqlx::query_scalar("SELECT jsonb_build_object('role',actor_role,'name',COALESCE(NULLIF(trim(concat_ws(' ',u.first_name,u.last_name)),''),e.actor_role)) FROM ticket_management_events e LEFT JOIN portal_users u ON u.clerk_user_id=e.actor_id WHERE request_id=$1 ORDER BY request_version DESC LIMIT 1").bind(r.id).fetch_one(&mut **tx).await?;
        v["updatedBy"] = if actor.staff_read() {
            last["name"].clone()
        } else {
            last["role"].clone()
        };
        v["updatedByRole"] = last["role"].clone();
        items.push(v);
        if items.len() >= limit as usize {
            break;
        }
    }
    Ok(json!(items))
}
