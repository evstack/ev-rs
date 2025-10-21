//! Tower middleware for serving observability endpoints.
//!
//! Provides REST endpoints for health checks and Prometheus metrics:
//! - `GET /health` - Returns node health status as JSON
//! - `GET /metrics` - Returns Prometheus-formatted metrics

use crate::health::HealthStatus;
use http::{Method, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use prometheus_client::encoding::text::encode;
use prometheus_client::registry::Registry;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::{Layer, Service};

/// Tower layer that adds observability endpoints.
#[derive(Clone)]
pub struct MetricsLayer {
    registry: Option<Arc<Registry>>,
    health_provider: Option<Arc<dyn Fn() -> HealthStatus + Send + Sync>>,
}

impl MetricsLayer {
    /// Create a new metrics layer with only a Prometheus registry.
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            registry: Some(registry),
            health_provider: None,
        }
    }

    /// Create a metrics layer with both metrics and health.
    pub fn with_health<F>(registry: Arc<Registry>, health_provider: F) -> Self
    where
        F: Fn() -> HealthStatus + Send + Sync + 'static,
    {
        Self {
            registry: Some(registry),
            health_provider: Some(Arc::new(health_provider)),
        }
    }

    /// Create a layer with only health endpoint (no metrics).
    pub fn health_only<F>(health_provider: F) -> Self
    where
        F: Fn() -> HealthStatus + Send + Sync + 'static,
    {
        Self {
            registry: None,
            health_provider: Some(Arc::new(health_provider)),
        }
    }
}

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsService {
            inner,
            registry: self.registry.clone(),
            health_provider: self.health_provider.clone(),
        }
    }
}

/// Tower service that handles observability requests.
#[derive(Clone)]
pub struct MetricsService<S> {
    inner: S,
    registry: Option<Arc<Registry>>,
    health_provider: Option<Arc<dyn Fn() -> HealthStatus + Send + Sync>>,
}

impl<S> Service<Request<Incoming>> for MetricsService<S>
where
    S: Service<Request<Incoming>, Response = Response<Full<Bytes>>> + Clone + Send + 'static,
    S::Future: Send,
{
    type Response = Response<Full<Bytes>>;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Incoming>) -> Self::Future {
        let path = req.uri().path();
        let method = req.method();

        // Handle GET /health
        if method == Method::GET && path == "/health" {
            let health_provider = self.health_provider.clone();
            return Box::pin(async move {
                let status = match health_provider {
                    Some(provider) => provider(),
                    None => HealthStatus::healthy(0, 0, "unknown".to_string(), 0),
                };

                let body = serde_json::to_string(&status).unwrap_or_else(|_| {
                    r#"{"healthy":false,"error":"serialization failed"}"#.to_string()
                });

                let response = Response::builder()
                    .status(if status.healthy {
                        StatusCode::OK
                    } else {
                        StatusCode::SERVICE_UNAVAILABLE
                    })
                    .header("Content-Type", "application/json")
                    .body(Full::new(Bytes::from(body)))
                    .unwrap();

                Ok(response)
            });
        }

        // Handle GET /metrics
        if method == Method::GET && path == "/metrics" {
            let registry = self.registry.clone();
            return Box::pin(async move {
                match registry {
                    Some(reg) => {
                        let mut buffer = String::new();
                        if encode(&mut buffer, &reg).is_err() {
                            let response = Response::builder()
                                .status(StatusCode::INTERNAL_SERVER_ERROR)
                                .body(Full::new(Bytes::from("Failed to encode metrics")))
                                .unwrap();
                            return Ok(response);
                        }

                        let response = Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
                            .body(Full::new(Bytes::from(buffer)))
                            .unwrap();

                        Ok(response)
                    }
                    None => {
                        let response = Response::builder()
                            .status(StatusCode::NOT_FOUND)
                            .body(Full::new(Bytes::from("Metrics not enabled")))
                            .unwrap();
                        Ok(response)
                    }
                }
            });
        }

        // Pass through to the inner service
        let mut inner = self.inner.clone();
        Box::pin(async move { inner.call(req).await })
    }
}

/// Encode metrics from a registry to Prometheus text format.
pub fn encode_metrics(registry: &Registry) -> String {
    let mut buffer = String::new();
    if encode(&mut buffer, registry).is_err() {
        return String::from("# Error encoding metrics\n");
    }
    buffer
}
