//! Prometheus metrics utilities.
use crate::ctx;
use prometheus::core::{Atomic, GenericGauge};
use std::{collections::HashMap, sync::Weak};

/// Guard which increments the gauge when constructed
/// and decrements it when dropped.
pub struct GaugeGuard<P: Atomic>(GenericGauge<P>);

impl<P: Atomic> From<GenericGauge<P>> for GaugeGuard<P> {
    fn from(g: GenericGauge<P>) -> Self {
        g.inc();
        Self(g)
    }
}

impl<P: Atomic> Drop for GaugeGuard<P> {
    fn drop(&mut self) {
        self.0.dec();
    }
}

type Fetcher<T> = dyn Send + Sync + Fn(&T) -> prometheus::proto::MetricFamily;

/// Collection of metrics. Implements prometheus::core::Collector,
/// it can be used to avoid embedding gauges in objects:
/// instead you can register a collector which will on demand gather
/// the metrics from the specificied object.
pub struct Collector<T> {
    t: Weak<T>,
    descs: Vec<prometheus::core::Desc>,
    builders: Vec<Box<Fetcher<T>>>,
}

impl<T> Collector<T> {
    /// Constructs a new noop Collector.
    pub fn new(t: Weak<T>) -> Self {
        Self {
            t,
            descs: vec![],
            builders: vec![],
        }
    }
    /// Adds a gauge to the Collector.
    /// `f` is expected to fetch the current gauge value.
    pub fn gauge(
        mut self,
        name: &'static str,
        help: &'static str,
        f: impl 'static + Send + Sync + Fn(&T) -> f64,
    ) -> Self {
        self.descs.push(
            prometheus::core::Desc::new(name.to_string(), help.to_string(), vec![], HashMap::new())
                .unwrap(),
        );
        self.builders.push(Box::new(move |t| {
            let mut mf = prometheus::proto::MetricFamily::default();
            mf.set_field_type(prometheus::proto::MetricType::GAUGE);
            mf.set_name(name.to_string());
            mf.set_help(help.to_string());
            mf.mut_metric().push_default().mut_gauge().set_value(f(t));
            mf
        }));
        self
    }
}

impl<T: Send + Sync> prometheus::core::Collector for Collector<T> {
    fn desc(&self) -> Vec<&prometheus::core::Desc> {
        self.descs.iter().collect()
    }
    fn collect(&self) -> Vec<prometheus::proto::MetricFamily> {
        let Some(t) = self.t.upgrade() else {
            return vec![];
        };
        self.builders.iter().map(|b| b(&t)).collect()
    }
}

const TEXT_CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// Runs and HTTP server on the specified address, which exposes the following endpoints:
///
/// - `GET` on any path: serves the metrics from the default prometheus registry
///   in the Open Metrics text format
pub async fn run_server(ctx: &ctx::Ctx, bind_address: std::net::SocketAddr) -> anyhow::Result<()> {
    let serve = |_| async {
        let reg = prometheus::default_registry();
        let enc = prometheus::TextEncoder::new();
        let body = enc.encode_to_string(&reg.gather())?;
        tracing::info!("HTTP request");
        anyhow::Ok(
            hyper::Response::builder()
                .status(hyper::StatusCode::OK)
                .header(hyper::header::CONTENT_TYPE, TEXT_CONTENT_TYPE)
                .body(body)
                .unwrap(),
        )
    };
    // For some unknown reasons `make_service` has to be static.
    let make_service = hyper::service::make_service_fn(|_| async move {
        tracing::info!("HTTP connection");
        anyhow::Ok(hyper::service::service_fn(serve))
    });
    hyper::Server::try_bind(&bind_address)?
        .serve(make_service)
        .with_graceful_shutdown(ctx.canceled())
        .await?;
    Ok(())
}
