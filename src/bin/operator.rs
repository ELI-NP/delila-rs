//! DAQ Operator - REST API server for system control
//!
//! Provides HTTP endpoints to control all DAQ components and Swagger UI for documentation.
//!
//! Usage:
//!   cargo run --bin operator                       # Use config.toml
//!   cargo run --bin operator -- -f config.toml     # Explicit config file
//!   cargo run --bin operator -- --port 8080
//!
//! Endpoints:
//!   GET  /api/status    - Get system status
//!   POST /api/configure - Configure all components
//!   POST /api/arm       - Arm all components
//!   POST /api/start     - Start data acquisition
//!   POST /api/stop      - Stop data acquisition
//!   POST /api/reset     - Reset all components
//!
//! Swagger UI: http://localhost:8080/swagger-ui/

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use delila_rs::common::OperatorArgs;
use delila_rs::config::{Config, InfluxDbConfig, MongoConfig};
use delila_rs::operator::{
    ComponentConfig, DigitizerConfigRepository, EmulatorSettings, OperatorConfig, RouterBuilder,
    RunRepository,
};
use tracing::{info, warn, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(
    name = "operator",
    about = "DELILA operator - REST API server for DAQ system control"
)]
struct Args {
    #[command(flatten)]
    operator: OperatorArgs,

    /// MongoDB connection URI (optional)
    #[arg(long, env = "MONGODB_URI")]
    mongodb_uri: Option<String>,

    /// MongoDB database name
    #[arg(long, env = "MONGODB_DATABASE", default_value = "delila")]
    mongodb_database: String,

    /// Only check that the config file loads, then exit (0 = usable). Used by
    /// scripts/restart_daq.sh before it stops anything (TODO 91).
    #[arg(long)]
    check_config: bool,
}

/// Load component configuration, operator config, emulator settings, and config file paths.
///
/// A missing or invalid file is an error, as in every other component: the Operator
/// used to fall back to two emulators on the default port, which on 2026-11-16 (pf-qaq,
/// one stray `[[network.sources]]` line) left a live Operator on the wrong port
/// controlling components that were not running.
#[allow(clippy::type_complexity)]
fn load_config(
    config_file: &str,
) -> anyhow::Result<(
    Vec<ComponentConfig>,
    OperatorConfig,
    EmulatorSettings,
    Vec<(u32, PathBuf)>,
    Option<InfluxDbConfig>,
    Option<MongoConfig>,
    Option<delila_rs::config::BacklogAutostopConfig>,
)> {
    let config = Config::load(config_file)
        .with_context(|| format!("cannot use config file {config_file}"))?;
    info!("Loaded configuration from {}", config_file);
    let components = build_components_from_config(&config);
    // Resolve web_ui_dir: explicit TOML value, or auto-detect
    let web_ui_dir = config.operator.web_ui_dir.map(PathBuf::from).or_else(|| {
        let default_path = PathBuf::from("web/operator-ui/dist/operator-ui/browser");
        if default_path.exists() {
            Some(default_path)
        } else {
            None
        }
    });
    let monitor_http_port = config.network.monitor.as_ref().map(|m| m.http_port);
    let defaults = OperatorConfig::default();
    let operator_config = OperatorConfig {
        port: config.operator.port,
        experiment_name: config.operator.experiment_name,
        web_ui_dir,
        monitor_http_port,
        configure_timeout_ms: config
            .operator
            .configure_timeout_ms
            .unwrap_or(defaults.configure_timeout_ms),
        arm_timeout_ms: config
            .operator
            .arm_timeout_ms
            .unwrap_or(defaults.arm_timeout_ms),
        start_timeout_ms: config
            .operator
            .start_timeout_ms
            .unwrap_or(defaults.start_timeout_ms),
        reset_timeout_ms: config
            .operator
            .reset_timeout_ms
            .unwrap_or(defaults.reset_timeout_ms),
        elog: config.operator.elog,
        drain_stop_timeout_secs: config.operator.drain_stop_timeout_secs,
        no_event_warn_secs: config.operator.no_event_warn_secs,
        root_sink_http_port: config.network.root_sink.as_ref().map(sink_http_port),
    };
    // Load emulator settings from config
    let emulator_settings = if let Ok(settings) = config.settings.get_settings() {
        EmulatorSettings::from(&settings)
    } else {
        EmulatorSettings::default()
    };
    // Extract (source_id, config_file) pairs from digitizer sources
    // TOML source id is the single source of truth for digitizer identification
    let config_files: Vec<(u32, PathBuf)> = config
        .network
        .sources
        .iter()
        .filter(|s| s.is_digitizer())
        .filter_map(|s| s.config_file.as_ref().map(|f| (s.id, PathBuf::from(f))))
        .collect();
    let influxdb_config = config.operator.influxdb;
    let mongodb_config = config.operator.mongodb;
    let backlog_autostop_config = config.operator.backlog_autostop;
    Ok((
        components,
        operator_config,
        emulator_settings,
        config_files,
        influxdb_config,
        mongodb_config,
        backlog_autostop_config,
    ))
}

/// The port the ROOT sink's THttpServer listens on, as start_daq.sh starts it:
/// the sinks' default 8090 when the key is absent, none when `http_port = 0`
/// (TODO 91: the restart guard then cannot read the sink's queue and refuses).
fn sink_http_port(rs: &delila_rs::config::RootSinkNetworkConfig) -> Option<u16> {
    match rs.http_port {
        None => Some(8090),
        Some(0) => None,
        Some(p) => Some(p),
    }
}

/// Build ComponentConfig list from parsed Config
fn build_components_from_config(config: &Config) -> Vec<ComponentConfig> {
    let mut components = Vec::new();

    // Add sources (emulators/readers)
    for source in &config.network.sources {
        let name = if source.name.is_empty() {
            format!("Source {}", source.id)
        } else {
            source.name.clone()
        };
        // Resolve bind address to connect address using source's host
        let address = source.command_connect_address_with_base(config.network.port_base_command);
        components.push(ComponentConfig {
            name,
            address,
            pipeline_order: source.pipeline_order,
            source_id: Some(source.id),
            is_digitizer: source.is_digitizer(),
            config_file: source.config_file.as_ref().map(PathBuf::from),
            source_type: Some(source.source_type.clone()),
        });
    }

    // Add merger
    if let Some(ref merger) = config.network.merger {
        let address = merger
            .command
            .clone()
            .unwrap_or_else(|| "tcp://*:5570".to_string())
            .replace("tcp://*:", "tcp://localhost:");
        components.push(ComponentConfig {
            name: "Merger".to_string(),
            address,
            pipeline_order: merger.pipeline_order,

            source_id: None,
            is_digitizer: false,
            config_file: None,
            source_type: None,
        });
    }

    // Add recorder
    if let Some(ref recorder) = config.network.recorder {
        let address = recorder
            .command
            .clone()
            .unwrap_or_else(|| "tcp://*:5580".to_string())
            .replace("tcp://*:", "tcp://localhost:");
        components.push(ComponentConfig {
            name: "Recorder".to_string(),
            address,
            pipeline_order: recorder.pipeline_order,

            source_id: None,
            is_digitizer: false,
            config_file: None,
            source_type: None,
        });
    }

    // Add monitor
    if let Some(ref monitor) = config.network.monitor {
        let address = monitor
            .command
            .clone()
            .unwrap_or_else(|| "tcp://*:5590".to_string())
            .replace("tcp://*:", "tcp://localhost:");
        components.push(ComponentConfig {
            name: "Monitor".to_string(),
            address,
            pipeline_order: monitor.pipeline_order,

            source_id: None,
            is_digitizer: false,
            config_file: None,
            source_type: None,
        });
    }

    // Add event builder
    if let Some(ref eb) = config.network.event_builder {
        let address = eb
            .command
            .clone()
            .unwrap_or_else(|| "tcp://*:5595".to_string())
            .replace("tcp://*:", "tcp://localhost:");
        components.push(ComponentConfig {
            name: "EventBuilder".to_string(),
            address,
            pipeline_order: eb.pipeline_order,

            source_id: None,
            is_digitizer: false,
            config_file: None,
            source_type: None,
        });
    }

    components
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Load component, operator, and emulator configuration
    let (
        components,
        operator_config,
        emulator_settings,
        config_files,
        influxdb_config,
        mongodb_toml,
        backlog_autostop_config,
    ) = load_config(&args.operator.common.config_file)?;
    if args.check_config {
        println!(
            "config OK: {} ({} component(s), experiment {}, port {})",
            args.operator.common.config_file,
            components.len(),
            operator_config.experiment_name,
            args.operator.port.unwrap_or(operator_config.port)
        );
        return Ok(());
    }
    // Resolve MongoDB settings: CLI flags win, fall back to TOML `[operator.mongodb]`.
    let resolved_mongo_uri: Option<String> = args
        .mongodb_uri
        .clone()
        .or_else(|| mongodb_toml.as_ref().map(|m| m.uri.clone()));
    let resolved_mongo_db: String = if args.mongodb_uri.is_some() {
        // CLI wins for the uri ⇒ also use the CLI database (defaulted to "delila").
        args.mongodb_database.clone()
    } else if let Some(ref m) = mongodb_toml {
        m.database.clone()
    } else {
        args.mongodb_database.clone()
    };
    info!("Loaded {} component(s)", components.len());
    for comp in &components {
        info!("  {} -> {}", comp.name, comp.address);
    }
    info!("Experiment name: {}", operator_config.experiment_name);
    if let Some(ref ui_dir) = operator_config.web_ui_dir {
        info!("Web UI directory: {}", ui_dir.display());
    } else {
        info!("Web UI: not configured (use web_ui_dir in [operator] or build Angular app)");
    }
    info!(
        "Emulator settings: {} events/batch, {}ms interval",
        emulator_settings.events_per_batch, emulator_settings.batch_interval_ms
    );

    // Connect to MongoDB if URI is provided (CLI > TOML).
    let (run_repo, digitizer_repo) = if let Some(ref uri) = resolved_mongo_uri {
        use mongodb::options::ClientOptions;
        use mongodb::Client;

        // Try to connect to MongoDB
        let connect_result: Option<(RunRepository, DigitizerConfigRepository)> = async {
            let options = ClientOptions::parse(uri).await.ok()?;
            let client = Client::with_options(options).ok()?;

            // Test connection
            client
                .database("admin")
                .run_command(mongodb::bson::doc! { "ping": 1 })
                .await
                .ok()?;

            info!(
                "Connected to MongoDB at {} (database: {})",
                uri, resolved_mongo_db
            );

            let run_repo = RunRepository::new(&client, &resolved_mongo_db);
            let digitizer_repo = DigitizerConfigRepository::new(&client, &resolved_mongo_db);
            Some((run_repo, digitizer_repo))
        }
        .await;

        match connect_result {
            Some((run_repo, digitizer_repo)) => (Some(run_repo), Some(digitizer_repo)),
            None => {
                warn!("Failed to connect to MongoDB. Run history will not be available.");
                (None, None)
            }
        }
    } else {
        info!("MongoDB not configured. Run history will not be available.");
        (None, None)
    };

    // Resolve port (CLI --port overrides TOML config)
    let port = args.operator.port.unwrap_or(operator_config.port);

    // How to restart this DAQ from the UI (TODO 91): same config, and
    // start_daq.sh --no-mongo when this Operator runs without MongoDB.
    let restart_settings = delila_rs::operator::daq_restart::RestartSettings {
        config_file: args.operator.common.config_file.clone(),
        no_mongo: resolved_mongo_uri.is_none(),
        sink_http_port: operator_config.root_sink_http_port,
        ..Default::default()
    };

    // Create router with builder
    let (app, app_state) = RouterBuilder::new(components)
        .config(operator_config)
        .config_dir(PathBuf::from("./config/digitizers"))
        .config_files(config_files)
        .run_repo(run_repo)
        .digitizer_repo(digitizer_repo)
        .emulator_settings(emulator_settings)
        .restart_settings(restart_settings)
        .build();

    // Start InfluxDB writer background task if configured
    if let Some(influxdb_config) = influxdb_config {
        let state = app_state.clone();
        tokio::spawn(async move {
            delila_rs::operator::influxdb::run_writer(influxdb_config, state).await;
        });
    }

    // TODO 91: the result of the UI restart that started this Operator is
    // written after it answers; pick it up for a while.
    {
        let state = app_state.clone();
        tokio::spawn(async move {
            delila_rs::operator::daq_restart::watch_result_after_start(state).await;
        });
    }

    // Run guard (TODO 69) — always on: write failure → immediate stop, disk
    // below the Recorder's hard free-space watermark → drain-first stop.
    {
        let state = app_state.clone();
        tokio::spawn(async move {
            delila_rs::operator::run_guard::run_watcher(state).await;
        });
    }

    // Digitizer guard (TODO 85) — always on: a source whose PLL stays not
    // locked / unreadable, or whose Reader stays in Error, during a run →
    // drain-first stop.
    {
        let state = app_state.clone();
        tokio::spawn(async move {
            delila_rs::operator::digitizer_guard::run_watcher(state).await;
        });
    }

    // Run timer (TODO 86, always on): stops a timed run like the Stop button
    // when its time is up, whether or not a browser is open.
    {
        let state = app_state.clone();
        tokio::spawn(async move {
            delila_rs::operator::run_timer::run_watcher(state).await;
        });
    }

    // No-event watcher (TODO 78 a): a Running source without new events
    // (e.g. an S-IN slave that never got its start signal) is shown in the UI.
    if app_state.config.no_event_warn_secs > 0 {
        let state = app_state.clone();
        let threshold = std::time::Duration::from_secs(app_state.config.no_event_warn_secs);
        tokio::spawn(async move {
            delila_rs::operator::event_stall::run_watcher(state, threshold).await;
        });
    }

    // Backlog autostop watcher (TODO 68) — opt-in via [operator.backlog_autostop]
    if let Some(backlog_config) = backlog_autostop_config {
        let state = app_state.clone();
        tokio::spawn(async move {
            delila_rs::operator::backlog_watch::run_watcher(backlog_config, state).await;
        });
    }

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Starting Operator server on http://{}", addr);
    info!("Swagger UI: http://localhost:{}/swagger-ui/", port);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Client addresses for handlers that log who asked (TODO 91 restart).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// One source and a Merger, enough for a real DAQ.
    const VALID: &str = r#"
[operator]
experiment_name = "Test"
port = 9091

[[network.sources]]
id = 0
name = "PSD2-VX2730"
type = "psd2"
digitizer_url = "dig2://172.18.4.57"
config_file = "config/digitizers/x.json"
pipeline_order = 1

[network.merger]
publish = "tcp://*:5557"
command = "tcp://*:5570"
pipeline_order = 2
"#;

    fn toml_file(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().expect("temp file");
        f.write_all(content.as_bytes()).expect("write temp file");
        f
    }

    #[test]
    fn load_config_reads_a_valid_file() {
        let f = toml_file(VALID);
        let path = f.path().to_string_lossy().to_string();
        let (components, operator_config, _, config_files, ..) =
            load_config(&path).expect("valid config");
        let names: Vec<&str> = components.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["PSD2-VX2730", "Merger"]);
        assert_eq!(operator_config.port, 9091);
        assert_eq!(operator_config.experiment_name, "Test");
        assert_eq!(config_files.len(), 1);
    }

    /// pf-qaq 2026-11-16: an uncommented `[[network.sources]]` line without its
    /// body. Every other component refused the file; the Operator used to start
    /// with two emulators on the default port instead.
    #[test]
    fn load_config_refuses_an_invalid_file() {
        let f = toml_file(&format!("{VALID}\n[[network.sources]]\nname = \"stray\"\n"));
        let path = f.path().to_string_lossy().to_string();
        let msg = format!("{:#}", load_config(&path).expect_err("invalid config"));
        assert!(msg.contains(&path), "message names the file: {msg}");
        assert!(msg.contains("id"), "message names the missing field: {msg}");
    }

    /// TODO 91: the restart guard reads the ROOT sink's queue on this port.
    #[test]
    fn load_config_passes_the_sink_http_port() {
        let f = toml_file(VALID);
        let path = f.path().to_string_lossy().to_string();
        let (_, no_sink, ..) = load_config(&path).expect("valid config");
        assert_eq!(no_sink.root_sink_http_port, None);

        let f = toml_file(&format!(
            "{VALID}\n[network.root_sink]\nprogram = \"thgem_sink\"\nhttp_port = 8090\n"
        ));
        let path = f.path().to_string_lossy().to_string();
        let (_, with_sink, ..) = load_config(&path).expect("valid config");
        assert_eq!(with_sink.root_sink_http_port, Some(Some(8090)));

        // As start_daq.sh: no key = the sinks' default 8090, 0 = no HTTP.
        let f = toml_file(&format!(
            "{VALID}\n[network.root_sink]\nprogram = \"root_sink\"\n"
        ));
        let path = f.path().to_string_lossy().to_string();
        let (_, default_port, ..) = load_config(&path).expect("valid config");
        assert_eq!(default_port.root_sink_http_port, Some(Some(8090)));
        let f = toml_file(&format!("{VALID}\n[network.root_sink]\nhttp_port = 0\n"));
        let path = f.path().to_string_lossy().to_string();
        let (_, no_http, ..) = load_config(&path).expect("valid config");
        assert_eq!(no_http.root_sink_http_port, Some(None));
    }

    #[test]
    fn load_config_refuses_a_missing_file() {
        let path = "/nonexistent/delila/config.toml";
        let msg = format!("{:#}", load_config(path).expect_err("missing config"));
        assert!(msg.contains(path), "message names the file: {msg}");
    }
}
