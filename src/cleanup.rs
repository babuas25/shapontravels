//! Bounded cleanup of expired, unused Search payloads. Business records are retained.
use crate::auth::ApiError;
use axum::http::StatusCode;
use sqlx::{Executor, PgPool, Postgres};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Default, Debug)]
pub struct CleanupStats {
    pub offers: u64,
    pub searches: u64,
    pub markers: u64,
}
impl CleanupStats {
    fn total(&self) -> u64 {
        self.offers + self.searches + self.markers
    }
}

/// One short transaction; skip rows held by RePrice/acceptance/booking.
pub async fn cleanup_batch(pool: &PgPool) -> Result<CleanupStats, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL statement_timeout = '2s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL lock_timeout = '200ms'")
        .execute(&mut *tx)
        .await?;
    let (locked,): (bool,) = sqlx::query_as("SELECT pg_try_advisory_xact_lock(1936220465,12)")
        .fetch_one(&mut *tx)
        .await?;
    if !locked {
        return Ok(CleanupStats::default());
    }
    let (offers,): (i64,) = sqlx::query_as(
        "WITH eligible AS MATERIALIZED (
            SELECT o.id FROM flight_offers o JOIN flight_searches s ON s.id=o.search_id
            WHERE o.created_at <= now()-INTERVAL '15 minutes'
              AND o.expires_at <= now() AND s.expires_at <= now()
              AND NOT EXISTS (SELECT 1 FROM flight_reprices r WHERE r.offer_id=o.id)
              AND NOT EXISTS (SELECT 1 FROM flight_bookings b WHERE b.offer_id=o.id)
            ORDER BY o.created_at,o.id LIMIT 512 FOR UPDATE OF o SKIP LOCKED
        ), removed AS (
            DELETE FROM flight_offers o USING eligible e WHERE o.id=e.id RETURNING o.id,o.client_id
        ), marked AS (
            INSERT INTO expired_flight_offers(id,client_id) SELECT id,client_id FROM removed
            ON CONFLICT(id) DO NOTHING RETURNING id
        ) SELECT count(*) FROM removed",
    )
    .fetch_one(&mut *tx)
    .await?;
    let searches = sqlx::query(
        "WITH eligible AS MATERIALIZED (
            SELECT s.id FROM flight_searches s
            WHERE s.created_at <= now()-INTERVAL '15 minutes' AND s.expires_at <= now()
              AND NOT EXISTS (SELECT 1 FROM flight_offers o WHERE o.search_id=s.id)
            ORDER BY s.created_at,s.id LIMIT 512 FOR UPDATE OF s SKIP LOCKED
        ) DELETE FROM flight_searches s USING eligible e WHERE s.id=e.id",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let markers = sqlx::query(
        "WITH eligible AS MATERIALIZED (
            SELECT id FROM expired_flight_offers WHERE purged_at <= now()-INTERVAL '24 hours'
            ORDER BY purged_at,id LIMIT 512 FOR UPDATE SKIP LOCKED
        ) DELETE FROM expired_flight_offers t USING eligible e WHERE t.id=e.id",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(CleanupStats {
        offers: offers as u64,
        searches,
        markers,
    })
}

pub(crate) async fn missing_offer_error<'e, E: Executor<'e, Database = Postgres>>(
    executor: E,
    id: Uuid,
    client_id: Uuid,
) -> Result<ApiError, sqlx::Error> {
    let (expired,): (bool,) = sqlx::query_as("SELECT EXISTS(SELECT 1 FROM expired_flight_offers WHERE id=$1 AND client_id=$2 AND purged_at>now()-INTERVAL '24 hours')")
        .bind(id).bind(client_id).fetch_one(executor).await?;
    Ok(if expired {
        ApiError(StatusCode::GONE, "OFFER_EXPIRED")
    } else {
        ApiError(StatusCode::NOT_FOUND, "NOT_FOUND")
    })
}

/// Run only in serve mode; never during migration, bootstrap or offline replay.
pub async fn run(pool: PgPool, mut stop: oneshot::Receiver<()>) {
    tracing::info!(
        retention_minutes = 15,
        interval_seconds = 30,
        "Search cleanup worker started"
    );
    let mut interval = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(30),
        Duration::from_secs(30),
    );
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! { _ = &mut stop => return, _ = interval.tick() => {} }
        let started = Instant::now();
        let mut totals = CleanupStats::default();
        for _ in 0..32 {
            if stop.try_recv().is_ok() {
                return;
            }
            match cleanup_batch(&pool).await {
                Ok(stats) => {
                    let empty = stats.total() == 0;
                    totals.offers += stats.offers;
                    totals.searches += stats.searches;
                    totals.markers += stats.markers;
                    if empty {
                        break;
                    }
                }
                Err(_) => {
                    tracing::warn!("Search cleanup batch failed; retrying on next tick");
                    break;
                }
            }
            if started.elapsed() >= Duration::from_secs(5) {
                break;
            }
        }
        if totals.total() > 0 {
            tracing::info!(
                offers_removed = totals.offers,
                searches_removed = totals.searches,
                expired_markers_removed = totals.markers,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "Search cleanup completed"
            );
        }
    }
}
