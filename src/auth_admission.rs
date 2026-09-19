//! Login source identity comes from the socket, or a single X-Real-IP supplied
//! by an explicitly configured reverse proxy. Never trust arbitrary forwarding.
use crate::{AppState, auth::ApiError};
use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{StatusCode, request::Parts},
};
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
};

#[derive(Clone, Default)]
pub struct ProxyPolicy {
    trusted: HashSet<IpAddr>,
}
impl ProxyPolicy {
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let mut trusted = HashSet::new();
        if let Some(value) = get("AUTH_TRUSTED_PROXY_IPS") {
            for item in value.split(',').filter(|s| !s.trim().is_empty()) {
                trusted.insert(
                    item.trim()
                        .parse()
                        .map_err(|_| "AUTH_TRUSTED_PROXY_IPS must contain exact IP addresses")?,
                );
            }
        }
        Ok(Self { trusted })
    }
    fn source(&self, parts: &Parts) -> Result<String, ApiError> {
        // Only internal router calls lack socket metadata. The public listener
        // always installs ConnectInfo; its clients cannot select this fallback.
        let Some(peer) = parts.extensions.get::<ConnectInfo<SocketAddr>>() else {
            return Ok("internal".into());
        };
        let peer = peer.0.ip();
        if !self.trusted.contains(&peer) {
            return Ok(peer.to_string());
        }
        let mut headers = parts.headers.get_all("x-real-ip").iter();
        let ip = headers
            .next()
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<IpAddr>().ok());
        if headers.next().is_some() || ip.is_none() {
            return Err(ApiError(StatusCode::BAD_REQUEST, "INVALID_CLIENT_ADDRESS"));
        }
        Ok(ip.unwrap().to_string())
    }
}
pub(crate) struct Source(pub String);
impl FromRequestParts<AppState> for Source {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &AppState) -> Result<Self, ApiError> {
        let policy = parts
            .extensions
            .get::<ProxyPolicy>()
            .cloned()
            .unwrap_or_default();
        policy.source(parts).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    #[test]
    fn forwarding_requires_an_exact_trusted_peer_and_single_valid_address() {
        let mut parts = Request::builder()
            .header("x-real-ip", "203.0.113.7")
            .body(())
            .unwrap()
            .into_parts()
            .0;
        parts.extensions.insert(ConnectInfo(
            "198.51.100.3:1234".parse::<SocketAddr>().unwrap(),
        ));
        assert_eq!(
            ProxyPolicy::default().source(&parts).unwrap(),
            "198.51.100.3"
        );
        let policy = ProxyPolicy::from_lookup(|_| Some("198.51.100.3".into())).unwrap();
        assert_eq!(policy.source(&parts).unwrap(), "203.0.113.7");
        parts
            .headers
            .append("x-real-ip", "203.0.113.8".parse().unwrap());
        assert!(policy.source(&parts).is_err());
        parts.headers.remove("x-real-ip");
        assert!(policy.source(&parts).is_err());
        parts
            .headers
            .insert("x-real-ip", "203.0.113.7, 203.0.113.8".parse().unwrap());
        assert!(policy.source(&parts).is_err());
        assert!(ProxyPolicy::from_lookup(|_| Some("0.0.0.0/0".into())).is_err());
    }
}
