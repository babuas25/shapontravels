//! Per-router admission for expensive Search work and its complete response body.
use axum::{
    Json,
    body::{Body, Bytes, HttpBody},
    extract::Request,
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone, Copy, Debug)]
pub struct SearchLimits {
    pub max_active: usize,
    pub max_queued: usize,
    pub wait: Duration,
}
impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_active: 4,
            max_queued: 8,
            wait: Duration::from_secs(2),
        }
    }
}
impl SearchLimits {
    pub fn validate(self) -> Result<Self, &'static str> {
        if !(1..=8).contains(&self.max_active)
            || self.max_queued > 32
            || self.wait < Duration::from_millis(1)
            || self.wait > Duration::from_secs(2)
        {
            return Err("Search limits require active 1..8, queued 0..32, wait 1..2000 ms");
        }
        Ok(self)
    }
}

#[derive(Clone)]
pub(crate) struct Admission {
    active: Arc<Semaphore>,
    total: Arc<Semaphore>,
    wait: Duration,
}
impl Admission {
    pub(crate) fn new(limits: SearchLimits) -> Self {
        let limits = limits.validate().expect("validated Search limits");
        Self {
            active: Arc::new(Semaphore::new(limits.max_active)),
            total: Arc::new(Semaphore::new(limits.max_active + limits.max_queued)),
            wait: limits.wait,
        }
    }
    pub(crate) async fn acquire(&self) -> Result<Permit, Busy> {
        let total = self.total.clone().try_acquire_owned().map_err(|_| Busy)?;
        let active = tokio::time::timeout(self.wait, self.active.clone().acquire_owned())
            .await
            .map_err(|_| Busy)?
            .map_err(|_| Busy)?;
        Ok(Permit {
            _permits: Arc::new(Permits {
                _active: active,
                _total: total,
            }),
        })
    }
}
struct Permits {
    _active: OwnedSemaphorePermit,
    _total: OwnedSemaphorePermit,
}
// Response extensions require Clone. The final body owns the last reference.
#[derive(Clone)]
pub(crate) struct Permit {
    _permits: Arc<Permits>,
}
#[derive(Debug)]
pub(crate) struct Busy;
impl IntoResponse for Busy {
    fn into_response(self) -> Response {
        let mut response = (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":"SEARCH_BUSY"})),
        )
            .into_response();
        response
            .headers_mut()
            .insert("retry-after", HeaderValue::from_static("1"));
        response
    }
}

// Must be OUTSIDE compression so permits also cover gzip encoding/backpressure.
pub(crate) async fn retain_response(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    if let Some(permit) = response.extensions_mut().remove::<Permit>() {
        let (parts, body) = response.into_parts();
        response = Response::from_parts(
            parts,
            Body::new(HeldBody {
                body,
                permit: Some(permit),
            }),
        );
    }
    response
}
struct HeldBody {
    body: Body,
    permit: Option<Permit>,
}
impl HttpBody for HeldBody {
    type Data = Bytes;
    type Error = axum::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, axum::Error>>> {
        let polled = Pin::new(&mut self.body).poll_frame(cx);
        if matches!(polled, Poll::Ready(None) | Poll::Ready(Some(Err(_)))) {
            // Discard any remaining body allocation before releasing capacity.
            self.body = Body::empty();
            self.permit.take();
        }
        polled
    }
    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }
    fn size_hint(&self) -> http_body::SizeHint {
        self.body.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use std::future::{Future, poll_fn};
    use tower::ServiceExt;

    fn gate() -> Admission {
        Admission::new(SearchLimits {
            max_active: 1,
            max_queued: 1,
            wait: Duration::from_millis(25),
        })
    }
    #[tokio::test]
    async fn bounded_queue_cancellation_timeout_and_reuse() {
        let admission = gate();
        let first = admission.acquire().await.unwrap();
        let mut queued = Box::pin(admission.acquire());
        poll_fn(|cx| {
            assert!(queued.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert!(admission.acquire().await.is_err(), "full queue must reject");
        drop(queued);
        assert_eq!(
            admission.total.available_permits(),
            1,
            "cancel frees queue slot"
        );
        assert!(
            admission.acquire().await.is_err(),
            "occupied active slot must time out"
        );
        assert_eq!(
            admission.total.available_permits(),
            1,
            "timeout frees queue slot"
        );
        let mut queued = Box::pin(admission.acquire());
        poll_fn(|cx| {
            assert!(queued.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(first);
        let admitted = queued.await.unwrap();
        assert_eq!(admission.active.available_permits(), 0);
        drop(admitted);
        assert_eq!(admission.active.available_permits(), 1);
        assert_eq!(admission.total.available_permits(), 2);
    }
    #[tokio::test]
    async fn response_permit_covers_compression_completion_and_disconnect() {
        use axum::{Extension, Router, middleware, routing::get};
        let admission = gate();
        let app = Router::new()
            .route(
                "/",
                get(|Extension(a): Extension<Admission>| async move {
                    let permit = a.acquire().await.unwrap();
                    let mut response =
                        Json(serde_json::json!({"data":"compress me".repeat(10000)}))
                            .into_response();
                    response.extensions_mut().insert(permit);
                    response
                }),
            )
            .layer(tower_http::compression::CompressionLayer::new())
            .layer(middleware::from_fn(retain_response))
            .layer(Extension(admission.clone()));
        let request = || {
            Request::builder()
                .uri("/")
                .header("accept-encoding", "gzip")
                .body(Body::empty())
                .unwrap()
        };
        let response = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.headers()["content-encoding"], "gzip");
        assert_eq!(
            admission.active.available_permits(),
            0,
            "headers do not release permit"
        );
        let compressed = response.into_body().collect().await.unwrap().to_bytes();
        let mut decoded = String::new();
        std::io::Read::read_to_string(
            &mut flate2::read::GzDecoder::new(compressed.as_ref()),
            &mut decoded,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&decoded).unwrap()["data"],
            "compress me".repeat(10000)
        );
        assert_eq!(admission.active.available_permits(), 1);
        let response = app.oneshot(request()).await.unwrap();
        assert_eq!(admission.active.available_permits(), 0);
        drop(response);
        assert_eq!(
            admission.active.available_permits(),
            1,
            "disconnect releases permit"
        );
    }
    #[tokio::test]
    async fn erroring_body_and_cancelled_work_release_capacity() {
        struct Failed;
        impl HttpBody for Failed {
            type Data = Bytes;
            type Error = std::io::Error;
            fn poll_frame(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
            ) -> Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
                Poll::Ready(Some(Err(std::io::Error::other("test"))))
            }
        }
        let admission = gate();
        let mut body = HeldBody {
            body: Body::new(Failed),
            permit: Some(admission.acquire().await.unwrap()),
        };
        assert!(body.frame().await.unwrap().is_err());
        assert_eq!(admission.active.available_permits(), 1);
        let a = admission.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _permit = a.acquire().await.unwrap();
            tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(admission.active.available_permits(), 1);
    }
}

#[cfg(test)]
mod burst_tests {
    use super::*;
    use std::future::{Future, poll_fn};

    #[tokio::test]
    async fn twelve_arrivals_fit_default_pool_and_drain_in_order() {
        let admission = Admission::new(SearchLimits::default());
        let mut active = Vec::new();
        for _ in 0..4 {
            active.push(admission.acquire().await.unwrap());
        }
        let mut waiting = Vec::new();
        for _ in 0..8 {
            let mut future = Box::pin(admission.acquire());
            poll_fn(|cx| {
                assert!(future.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            waiting.push(future);
        }
        assert!(
            admission.acquire().await.is_err(),
            "thirteenth arrival must not overfill"
        );
        assert_eq!(admission.active.available_permits(), 0);
        assert_eq!(admission.total.available_permits(), 0);
        drop(active);
        let mut waiting = waiting.into_iter();
        for _ in 0..2 {
            let mut wave = Vec::new();
            for _ in 0..4 {
                wave.push(
                    tokio::time::timeout(Duration::from_secs(1), waiting.next().unwrap())
                        .await
                        .unwrap()
                        .unwrap(),
                );
            }
            assert_eq!(admission.active.available_permits(), 0);
            drop(wave);
        }
        assert_eq!(admission.active.available_permits(), 4);
        assert_eq!(admission.total.available_permits(), 12);
    }
}
