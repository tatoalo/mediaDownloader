use std::collections::HashMap;

use opentelemetry::KeyValue;

use opentelemetry::sdk::{trace, Resource};
use opentelemetry_otlp::{HttpExporterBuilder, WithExportConfig};
use serde::Deserialize;
use tracing_bunyan_formatter::JsonStorageLayer;
use tracing_subscriber::Registry;
use tracing_subscriber::{prelude::*, EnvFilter};

use crate::{CONFIG_FILE_SYNC, SERVICE_NAME};

const LEVEL_TRACES: &str = "DEBUG";

pub enum TelemetryPurpose {
    Tracing,
    Metrics,
}

#[derive(Debug, Deserialize)]
pub struct TelemetryConfig {
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
}

pub async fn init_telemetry(service_name: Option<String>) {
    let telemetry_config = &CONFIG_FILE_SYNC.telemetry;
    if !is_telemetry_config_valid(telemetry_config) {
        return;
    }

    let service_name = service_name.unwrap_or(SERVICE_NAME.to_string());

    let exporter_tracing = build_purpose_exporter(
        telemetry_config
            .as_ref()
            .unwrap()
            .endpoint
            .as_ref()
            .unwrap()
            .to_string(),
        TelemetryPurpose::Tracing,
        build_headers(
            telemetry_config
                .as_ref()
                .unwrap()
                .api_key
                .as_ref()
                .unwrap()
                .to_string(),
        ),
    );

    // Tracing pipeline
    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter_tracing)
        .with_trace_config(
            trace::config().with_resource(Resource::new(vec![KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                service_name,
            )])),
        )
        .install_batch(opentelemetry::runtime::Tokio)
        .expect("Error: Failed to initialize the tracer.");

    let subscriber = Registry::default();
    let level_filter_layer =
        EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new(LEVEL_TRACES));
    let tracing_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    subscriber
        .with(level_filter_layer)
        .with(tracing_layer)
        .with(JsonStorageLayer)
        .with(tracing_subscriber::fmt::layer())
        .init();
}

fn is_telemetry_config_valid(telemetry_config: &Option<TelemetryConfig>) -> bool {
    match telemetry_config {
        Some(t) => {
            match (t.endpoint.clone(), t.api_key.clone()) {
                (Some(e), Some(a)) => {
                    if e.is_empty() {
                        println!("Telemetry endpoint is empty!");
                        return false;
                    }

                    if a.is_empty() {
                        println!("Telemetry api key is empty!");
                        return false;
                    }
                }
                _ => {
                    println!("Telemetry endpoint or api key must be both set for telemetry initialization!");
                    return false;
                }
            }
        }
        None => {
            println!("Telemetry config is **NOT** populated, skipping initialization.");
            return false;
        }
    }
    true
}

fn build_headers(api_key: String) -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert("authorization".to_string(), api_key);
    map
}

fn build_purpose_exporter(
    endpoint: String,
    purpose: TelemetryPurpose,
    headers: HashMap<String, String>,
) -> HttpExporterBuilder {
    let endpoint_constructed = match purpose {
        TelemetryPurpose::Tracing => endpoint + "traces",
        TelemetryPurpose::Metrics => endpoint + "metrics",
    };

    let http_tracing_exporter = opentelemetry_otlp::new_exporter()
        .http()
        .with_endpoint(endpoint_constructed)
        .with_headers(headers);

    http_tracing_exporter
}
