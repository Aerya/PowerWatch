use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use powerwatch_core::energy;
use clap::Parser;
use reqwest::Client;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::task::JoinSet;

const INDEX_HTML: &str = include_str!("../static/index.html");
const BACKFILL_DAYS: u32 = 400;
const BACKFILL_MAX_POINTS: u32 = 5_000;

#[derive(Parser, Debug)]
#[command(name = "powerwatch-hub")]
struct Args {
    #[arg(long, default_value = "127.0.0.1")]
    host: IpAddr,

    #[arg(long, default_value_t = 3000)]
    port: u16,

    #[arg(long, default_value = "/data/powerwatch-hub.json")]
    config: PathBuf,

    #[arg(long, default_value = "/data/powerwatch-hub.db")]
    database: PathBuf,

    #[arg(long, default_value_t = 2)]
    refresh_interval: u64,

    #[arg(long, default_value_t = 60)]
    history_interval: u64,

    #[arg(long, default_value_t = 15)]
    stale_after: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct NodeConfig {
    id: String,
    name: String,
    url: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_true")]
    include_in_total: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_token: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HubConfig {
    #[serde(default)]
    nodes: Vec<NodeConfig>,
}

#[derive(Debug, Clone, Serialize)]
struct NodeConfigView {
    id: String,
    name: String,
    url: String,
    enabled: bool,
    include_in_total: bool,
    has_api_token: bool,
}

impl From<&NodeConfig> for NodeConfigView {
    fn from(node: &NodeConfig) -> Self {
        Self {
            id: node.id.clone(),
            name: node.name.clone(),
            url: node.url.clone(),
            enabled: node.enabled,
            include_in_total: node.include_in_total,
            has_api_token: node
                .api_token
                .as_deref()
                .is_some_and(|token| !token.is_empty()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct HubConfigView {
    nodes: Vec<NodeConfigView>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RemoteReading {
    watts: f64,
    confidence: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RemoteSensor {
    name: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    reading: Option<RemoteReading>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RemoteMemoryModuleInfo {
    #[serde(default)]
    locator: Option<String>,
    #[serde(default)]
    bank_locator: Option<String>,
    #[serde(default)]
    size_bytes: u64,
    #[serde(default)]
    memory_type: Option<String>,
    #[serde(default)]
    form_factor: Option<String>,
    #[serde(default)]
    speed: Option<String>,
    #[serde(default)]
    manufacturer: Option<String>,
    #[serde(default)]
    part_number: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RemotePowerSupplyInfo {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    manufacturer: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    supply_type: Option<String>,
    #[serde(default)]
    max_power_watts: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct RemoteSystemInfo {
    #[serde(default)]
    name: String,
    #[serde(default)]
    cpu_model: Option<String>,
    #[serde(default)]
    memory_total_bytes: Option<u64>,
    #[serde(default)]
    memory_installed_bytes: Option<u64>,
    #[serde(default)]
    memory_modules: Vec<RemoteMemoryModuleInfo>,
    #[serde(default)]
    power_supplies: Vec<RemotePowerSupplyInfo>,
    #[serde(default)]
    smbios_available: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RemoteSnapshot {
    timestamp: DateTime<Utc>,
    #[serde(default)]
    system: Option<RemoteSystemInfo>,
    #[serde(default)]
    sensors: Vec<RemoteSensor>,
    #[serde(default)]
    total: Option<RemoteReading>,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteRangeHistoryResponse {
    #[serde(default)]
    points: Vec<RemoteAggregatedReading>,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteAggregatedReading {
    component: serde_json::Value,
    timestamp: DateTime<Utc>,
    avg_watts: f64,
}

#[derive(Debug, Clone, Default)]
struct NodeRuntime {
    snapshot: Option<RemoteSnapshot>,
    last_seen: Option<DateTime<Utc>>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
enum NodeStatus {
    Online,
    Stale,
    Offline,
    Disabled,
}

#[derive(Debug, Clone, Serialize)]
struct HubNodeView {
    id: String,
    name: String,
    url: String,
    enabled: bool,
    include_in_total: bool,
    has_api_token: bool,
    status: NodeStatus,
    last_seen: Option<DateTime<Utc>>,
    last_error: Option<String>,
    total_watts: Option<f64>,
    cpu_model: Option<String>,
    memory_total_bytes: Option<u64>,
    memory_installed_bytes: Option<u64>,
    memory_modules: Vec<RemoteMemoryModuleInfo>,
    power_supplies: Vec<RemotePowerSupplyInfo>,
    smbios_available: bool,
    sensors: Vec<RemoteSensor>,
}

#[derive(Debug, Clone, Serialize)]
struct HubSnapshot {
    timestamp: DateTime<Utc>,
    total_watts: f64,
    online_nodes: usize,
    configured_nodes: usize,
    nodes: Vec<HubNodeView>,
}

#[derive(Debug, Clone, Serialize)]
struct HistoryPoint {
    timestamp: DateTime<Utc>,
    global_watts: f64,
    nodes: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct HistoryParams {
    #[serde(default = "default_since")]
    since: String,
}

fn default_since() -> String {
    "24h".to_string()
}

#[derive(Debug, Clone, Deserialize)]
struct ProbeRequest {
    url: String,
    #[serde(default)]
    api_token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ProbeResponse {
    ok: bool,
    message: String,
    total_watts: Option<f64>,
    instance_name: Option<String>,
    cpu_model: Option<String>,
}

#[derive(Clone)]
struct AppState {
    config: Arc<RwLock<HubConfig>>,
    runtime: Arc<RwLock<HashMap<String, NodeRuntime>>>,
    config_path: PathBuf,
    storage: Arc<Mutex<HubStorage>>,
    stale_after: Duration,
    client: Client,
    admin_auth: HubAdminAuth,
}

#[derive(Clone)]
struct HubAdminAuth {
    token_hash: Option<[u8; 32]>,
}

#[derive(Serialize)]
struct HubAdminStatus {
    enabled: bool,
}

impl HubAdminAuth {
    fn load() -> Result<Self, String> {
        let token = if let Ok(path) = std::env::var("POWERWATCH_HUB_ADMIN_TOKEN_FILE") {
            Some(
                std::fs::read_to_string(path.trim())
                    .map_err(|error| {
                        format!("cannot read POWERWATCH_HUB_ADMIN_TOKEN_FILE: {error}")
                    })?
                    .trim()
                    .to_string(),
            )
        } else {
            std::env::var("POWERWATCH_HUB_ADMIN_TOKEN")
                .ok()
                .map(|token| token.trim().to_string())
                .filter(|token| !token.is_empty())
        };

        if token.as_deref().is_some_and(|token| token.len() < 24) {
            return Err(
                "POWERWATCH_HUB_ADMIN_TOKEN must contain at least 24 characters".to_string(),
            );
        }

        Ok(Self {
            token_hash: token.as_deref().map(hash_secret),
        })
    }

    #[cfg(test)]
    fn with_token(token: Option<&str>) -> Self {
        Self {
            token_hash: token.map(hash_secret),
        }
    }

    fn enabled(&self) -> bool {
        self.token_hash.is_some()
    }

    fn authorizes(&self, headers: &HeaderMap) -> bool {
        let Some(expected) = self.token_hash else {
            return true;
        };
        let Some(token) = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return false;
        };
        constant_time_bytes_eq(&expected, &hash_secret(token))
    }
}

fn hash_secret(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

fn constant_time_bytes_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

struct HubStorage {
    conn: Connection,
}

impl HubStorage {
    fn open(path: &FsPath) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS node_totals (
                timestamp TEXT NOT NULL,
                node_id TEXT NOT NULL,
                watts REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_node_totals_time
                ON node_totals(timestamp);
            CREATE INDEX IF NOT EXISTS idx_node_totals_node_time
                ON node_totals(node_id, timestamp);
            DELETE FROM node_totals
              WHERE rowid NOT IN (
                SELECT MIN(rowid)
                FROM node_totals
                GROUP BY timestamp, node_id
              );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_node_totals_unique
                ON node_totals(timestamp, node_id);
            ",
        )?;
        let mut storage=Self { conn };
        energy::init(&storage.conn)?;
        // Upgrade pre-existing Hub history, with no duplicate kWh on restarts.
        let migration_needed:bool=storage.conn.query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM energy_last LIMIT 1)",[],|r|r.get(0))?;
        if migration_needed {
            let mut by_node:BTreeMap<String,Vec<(i64,f64,bool)>>=BTreeMap::new();
            {
                let mut stmt=storage.conn.prepare(
                    "SELECT node_id,CAST(strftime('%s',timestamp) AS INTEGER),watts
                     FROM node_totals ORDER BY node_id,timestamp")?;
                let rows=stmt.query_map([],|r|Ok((r.get::<_,String>(0)?,
                    r.get::<_,i64>(1)?,r.get::<_,f64>(2)?)))?;
                for row in rows {
                    let (node,epoch,watts)=row?;
                    by_node.entry(node).or_default().push((epoch,watts,true));
                }
            }
            for (node,points) in by_node {
                energy::import_samples(&mut storage.conn,&node,&points,600)?;
            }
            energy::compact(&mut storage.conn,Utc::now().timestamp())?;
        }
        Ok(storage)
    }

    fn write_totals(
        &mut self,
        timestamp: DateTime<Utc>,
        totals: &[(String, f64)],
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO node_totals(timestamp, node_id, watts) VALUES (?1, ?2, ?3)",
            )?;
            for (node_id, watts) in totals {
                stmt.execute(params![timestamp.to_rfc3339(), node_id, watts])?;
            }
        }

        let cutoff = timestamp - chrono::Duration::days(BACKFILL_DAYS as i64);
        tx.execute(
            "DELETE FROM node_totals WHERE timestamp < ?1",
            params![cutoff.to_rfc3339()],
        )?;
        tx.commit()?;
        for (node,watts) in totals {
            energy::record(&mut self.conn,node,timestamp.timestamp(),*watts,true,180)?;
        }
        Ok(())
    }

    fn write_history_points(
        &mut self,
        node_id: &str,
        points: &[(DateTime<Utc>, f64)],
    ) -> rusqlite::Result<usize> {
        let tx = self.conn.transaction()?;
        let mut changed = 0usize;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO node_totals(timestamp, node_id, watts) VALUES (?1, ?2, ?3)",
            )?;
            for (timestamp, watts) in points {
                changed += stmt.execute(params![timestamp.to_rfc3339(), node_id, watts])?;
            }
        }
        tx.commit()?;
        let samples=points.iter().map(|(at,watts)|(at.timestamp(),*watts,true))
            .collect::<Vec<_>>();
        energy::import_samples(&mut self.conn,node_id,&samples,180)?;
        Ok(changed)
    }

    fn history_since(
        &self,
        cutoff: DateTime<Utc>,
        included_nodes: &HashSet<String>,
    ) -> rusqlite::Result<Vec<HistoryPoint>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, node_id, watts
             FROM node_totals
             WHERE timestamp >= ?1
             ORDER BY timestamp ASC, node_id ASC",
        )?;
        let rows = stmt.query_map(params![cutoff.to_rfc3339()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?;

        let mut grouped: BTreeMap<DateTime<Utc>, BTreeMap<String, f64>> = BTreeMap::new();
        for row in rows {
            let (timestamp, node_id, watts) = row?;
            let Ok(timestamp) = DateTime::parse_from_rfc3339(&timestamp) else {
                continue;
            };
            grouped
                .entry(timestamp.with_timezone(&Utc))
                .or_default()
                .insert(node_id, watts);
        }

        Ok(grouped
            .into_iter()
            .map(|(timestamp, nodes)| HistoryPoint {
                timestamp,
                global_watts: nodes
                    .iter()
                    .filter(|(node_id, _)| included_nodes.contains(*node_id))
                    .map(|(_, watts)| *watts)
                    .sum(),
                nodes,
            })
            .collect())
    }
}

fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn validate_node(node: &NodeConfig) -> Result<(), String> {
    if node.id.is_empty()
        || !node
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("id must contain only letters, numbers, '-' or '_'".to_string());
    }
    if node.name.trim().is_empty() {
        return Err("name cannot be empty".to_string());
    }
    let url = normalize_url(&node.url);
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("url must start with http:// or https://".to_string());
    }
    Ok(())
}

fn preserve_or_clear_token_for_update(existing: &NodeConfig, requested: &mut NodeConfig) {
    match requested.api_token.as_deref() {
        None if normalize_url(&existing.url) == normalize_url(&requested.url) => {
            requested.api_token = existing.api_token.clone();
        }
        None => {
            // A stored credential is bound to its existing destination. Moving
            // it to another URL requires the caller to submit it explicitly.
            requested.api_token = None;
        }
        Some(token) if token.trim().is_empty() => {
            requested.api_token = None;
        }
        Some(_) => {}
    }
}

fn load_config(path: &FsPath) -> HubConfig {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|error| {
            eprintln!("warning: invalid hub config {}: {error}", path.display());
            HubConfig::default()
        }),
        Err(_) => HubConfig::default(),
    }
}

fn save_config(path: &FsPath, config: &HubConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
    std::fs::write(&tmp, data).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|error| error.to_string())
}

fn parse_since(value: &str) -> Result<chrono::Duration, String> {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.len() < 2 {
        return Err("invalid duration".to_string());
    }
    let (number, unit) = trimmed.split_at(trimmed.len() - 1);
    let amount: i64 = number.parse().map_err(|_| "invalid duration".to_string())?;
    if amount <= 0 {
        return Err("duration must be positive".to_string());
    }
    match unit {
        "m" => Ok(chrono::Duration::minutes(amount)),
        "h" => Ok(chrono::Duration::hours(amount)),
        "d" => Ok(chrono::Duration::days(amount)),
        "w" => Ok(chrono::Duration::weeks(amount)),
        _ => Err("duration unit must be m, h, d or w".to_string()),
    }
}

fn is_total_component(component: &serde_json::Value) -> bool {
    component.as_str().is_some_and(|value| value == "Total")
}

async fn fetch_snapshot(
    client: &Client,
    base_url: &str,
    api_token: Option<&str>,
) -> Result<RemoteSnapshot, String> {
    let url = format!("{}/api/snapshot", normalize_url(base_url));
    let mut request = client.get(url);
    if let Some(token) = api_token.filter(|token| !token.trim().is_empty()) {
        request = request.bearer_auth(token.trim());
    }
    let response = request
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    response
        .json::<RemoteSnapshot>()
        .await
        .map_err(|error| error.to_string())
}

async fn fetch_historical_totals(
    client: &Client,
    base_url: &str,
    api_token: Option<&str>,
) -> Result<Vec<(DateTime<Utc>, f64)>, String> {
    let url = format!("{}/api/history/range", normalize_url(base_url));
    let mut request = client.get(url);
    if let Some(token) = api_token.filter(|token| !token.trim().is_empty()) {
        request = request.bearer_auth(token.trim());
    }
    let response = request
        .query(&[
            ("amount", BACKFILL_DAYS.to_string()),
            ("unit", "days".to_string()),
            ("max_points", BACKFILL_MAX_POINTS.to_string()),
        ])
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    let history = response
        .json::<RemoteRangeHistoryResponse>()
        .await
        .map_err(|error| error.to_string())?;

    Ok(history
        .points
        .into_iter()
        .filter(|point| is_total_component(&point.component))
        .map(|point| (point.timestamp, point.avg_watts))
        .collect())
}

async fn backfill_node(state: AppState, node: NodeConfig) {
    if !node.enabled {
        return;
    }

    match fetch_historical_totals(&state.client, &node.url, node.api_token.as_deref()).await {
        Ok(points) => {
            if points.is_empty() {
                eprintln!(
                    "hub history backfill {}: no historical total points available",
                    node.id
                );
                return;
            }

            match state.storage.lock() {
                Ok(mut storage) => match storage.write_history_points(&node.id, &points) {
                    Ok(_) => println!(
                        "hub history backfill {}: imported {} total points",
                        node.id,
                        points.len()
                    ),
                    Err(error) => eprintln!(
                        "warning: failed to persist history backfill for {}: {error}",
                        node.id
                    ),
                },
                Err(_) => eprintln!("warning: hub history storage lock poisoned"),
            }
        }
        Err(error) => eprintln!(
            "warning: history backfill failed for {} ({}): {error}",
            node.id, node.url
        ),
    }
}

async fn backfill_all(state: AppState) {
    let nodes = state.config.read().await.nodes.clone();
    let mut join_set = JoinSet::new();
    for node in nodes.into_iter().filter(|node| node.enabled) {
        join_set.spawn(backfill_node(state.clone(), node));
    }
    while join_set.join_next().await.is_some() {}
}

async fn poll_node(client: Client, node: NodeConfig) -> (String, Result<RemoteSnapshot, String>) {
    let id = node.id.clone();
    let result = fetch_snapshot(&client, &node.url, node.api_token.as_deref()).await;
    (id, result)
}

async fn poll_loop(state: AppState, refresh_interval: Duration) {
    let mut ticker = tokio::time::interval(refresh_interval);
    loop {
        ticker.tick().await;

        let nodes = state.config.read().await.nodes.clone();
        {
            let mut runtime = state.runtime.write().await;
            runtime.retain(|id, _| nodes.iter().any(|node| &node.id == id));
        }

        let mut join_set = JoinSet::new();
        for node in nodes.into_iter().filter(|node| node.enabled) {
            join_set.spawn(poll_node(state.client.clone(), node));
        }

        while let Some(result) = join_set.join_next().await {
            let Ok((id, result)) = result else {
                continue;
            };
            let mut runtime = state.runtime.write().await;
            let entry = runtime.entry(id).or_default();
            match result {
                Ok(snapshot) => {
                    entry.last_seen = Some(Utc::now());
                    entry.snapshot = Some(snapshot);
                    entry.last_error = None;
                }
                Err(error) => {
                    entry.last_error = Some(error);
                }
            }
        }
    }
}

async fn history_loop(state: AppState, history_interval: Duration) {
    let mut last_energy_compaction = Instant::now() - Duration::from_secs(3600);
    let mut ticker = tokio::time::interval(history_interval);
    ticker.tick().await;

    loop {
        ticker.tick().await;

        let config = state.config.read().await.clone();
        let runtime = state.runtime.read().await.clone();
        let now = Utc::now();
        let totals = config
            .nodes
            .iter()
            .filter(|node| node.enabled)
            .filter_map(|node| {
                let item = runtime.get(&node.id)?;
                if item.last_error.is_some() {
                    return None;
                }
                let last_seen = item.last_seen?;
                if now.signed_duration_since(last_seen).to_std().ok()? > state.stale_after {
                    return None;
                }
                let watts = item.snapshot.as_ref()?.total.as_ref()?.watts;
                Some((node.id.clone(), watts))
            })
            .collect::<Vec<_>>();

        if totals.is_empty() {
            continue;
        }

        if let Ok(mut storage) = state.storage.lock() {
            if let Err(error) = storage.write_totals(now, &totals) {
                eprintln!("warning: failed to persist hub history: {error}");
            }
            if last_energy_compaction.elapsed() >= Duration::from_secs(3600) {
                if let Err(error) = energy::compact(&mut storage.conn,now.timestamp()) {
                    eprintln!("warning: energy archive maintenance failed: {error}");
                } else {
                    last_energy_compaction=Instant::now();
                }
            }
        }
    }
}

async fn index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Html(INDEX_HTML),
    )
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn snapshot(State(state): State<AppState>) -> Json<HubSnapshot> {
    let config = state.config.read().await.clone();
    let runtime = state.runtime.read().await.clone();
    let now = Utc::now();

    let mut nodes = Vec::with_capacity(config.nodes.len());
    let mut total_watts = 0.0;
    let mut online_nodes = 0usize;

    for node in config.nodes {
        let runtime_node = runtime.get(&node.id).cloned().unwrap_or_default();
        let age = runtime_node
            .last_seen
            .and_then(|last_seen| now.signed_duration_since(last_seen).to_std().ok());

        let status = if !node.enabled {
            NodeStatus::Disabled
        } else if runtime_node.last_error.is_none() && runtime_node.snapshot.is_some() {
            NodeStatus::Online
        } else if runtime_node.snapshot.is_some()
            && age.is_some_and(|value| value <= state.stale_after)
        {
            NodeStatus::Stale
        } else {
            NodeStatus::Offline
        };

        let node_total = runtime_node
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.total.as_ref())
            .map(|reading| reading.watts);

        if matches!(status, NodeStatus::Online) {
            online_nodes += 1;
            if node.include_in_total {
                total_watts += node_total.unwrap_or(0.0);
            }
        }

        let system = runtime_node
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.system.clone())
            .unwrap_or_default();

        nodes.push(HubNodeView {
            has_api_token: node
                .api_token
                .as_deref()
                .is_some_and(|token| !token.is_empty()),
            id: node.id,
            name: node.name,
            url: node.url,
            enabled: node.enabled,
            include_in_total: node.include_in_total,
            status,
            last_seen: runtime_node.last_seen,
            last_error: runtime_node.last_error,
            total_watts: node_total,
            cpu_model: system.cpu_model,
            memory_total_bytes: system.memory_total_bytes,
            memory_installed_bytes: system.memory_installed_bytes,
            memory_modules: system.memory_modules,
            power_supplies: system.power_supplies,
            smbios_available: system.smbios_available,
            sensors: runtime_node
                .snapshot
                .map(|snapshot| snapshot.sensors)
                .unwrap_or_default(),
        });
    }

    Json(HubSnapshot {
        timestamp: now,
        total_watts,
        online_nodes,
        configured_nodes: nodes.len(),
        nodes,
    })
}

async fn list_nodes(State(state): State<AppState>) -> Json<HubConfigView> {
    let config = state.config.read().await;
    Json(HubConfigView {
        nodes: config.nodes.iter().map(NodeConfigView::from).collect(),
    })
}

async fn add_node(
    State(state): State<AppState>,
    Json(mut node): Json<NodeConfig>,
) -> Result<(StatusCode, Json<NodeConfigView>), (StatusCode, String)> {
    node.id = node.id.trim().to_string();
    node.name = node.name.trim().to_string();
    node.url = normalize_url(&node.url);
    validate_node(&node).map_err(|error| (StatusCode::BAD_REQUEST, error))?;

    let mut config = state.config.write().await;
    if config.nodes.iter().any(|existing| existing.id == node.id) {
        return Err((StatusCode::CONFLICT, "node id already exists".to_string()));
    }

    config.nodes.push(node.clone());
    config.nodes.sort_by(|a, b| a.name.cmp(&b.name));
    save_config(&state.config_path, &config)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    drop(config);

    tokio::spawn(backfill_node(state.clone(), node.clone()));

    Ok((StatusCode::CREATED, Json(NodeConfigView::from(&node))))
}

async fn update_node(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(mut node): Json<NodeConfig>,
) -> Result<Json<NodeConfigView>, (StatusCode, String)> {
    node.id = id.clone();
    node.name = node.name.trim().to_string();
    node.url = normalize_url(&node.url);
    validate_node(&node).map_err(|error| (StatusCode::BAD_REQUEST, error))?;

    let mut config = state.config.write().await;
    let Some(existing) = config.nodes.iter_mut().find(|existing| existing.id == id) else {
        return Err((StatusCode::NOT_FOUND, "node not found".to_string()));
    };
    preserve_or_clear_token_for_update(existing, &mut node);
    *existing = node.clone();

    save_config(&state.config_path, &config)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    drop(config);

    if node.enabled {
        tokio::spawn(backfill_node(state.clone(), node.clone()));
    }

    Ok(Json(NodeConfigView::from(&node)))
}

async fn delete_node(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut config = state.config.write().await;
    let before = config.nodes.len();
    config.nodes.retain(|node| node.id != id);
    if config.nodes.len() == before {
        return Err((StatusCode::NOT_FOUND, "node not found".to_string()));
    }

    save_config(&state.config_path, &config)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    state.runtime.write().await.remove(&id);

    Ok(StatusCode::NO_CONTENT)
}

async fn probe_node(
    State(state): State<AppState>,
    Json(request): Json<ProbeRequest>,
) -> Json<ProbeResponse> {
    match fetch_snapshot(&state.client, &request.url, request.api_token.as_deref()).await {
        Ok(snapshot) => {
            let instance_name = snapshot
                .system
                .as_ref()
                .map(|system| system.name.trim().to_string())
                .filter(|name| !name.is_empty());
            let cpu_model = snapshot
                .system
                .as_ref()
                .and_then(|system| system.cpu_model.clone());
            Json(ProbeResponse {
                ok: true,
                message: "PowerWatch API reachable".to_string(),
                total_watts: snapshot.total.map(|reading| reading.watts),
                instance_name,
                cpu_model,
            })
        }
        Err(error) => Json(ProbeResponse {
            ok: false,
            message: error,
            total_watts: None,
            instance_name: None,
            cpu_model: None,
        }),
    }
}

async fn history(
    State(state): State<AppState>,
    Query(params): Query<HistoryParams>,
) -> Result<Json<Vec<HistoryPoint>>, (StatusCode, String)> {
    let duration = parse_since(&params.since).map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    let cutoff = Utc::now() - duration;

    let included_nodes = state
        .config
        .read()
        .await
        .nodes
        .iter()
        .filter(|node| node.enabled && node.include_in_total)
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();

    let storage = state.storage.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage lock poisoned".to_string(),
        )
    })?;
    let rows = storage
        .history_since(cutoff, &included_nodes)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    Ok(Json(rows))
}


#[derive(Deserialize)]
struct EnergyQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

#[derive(Serialize, Default)]
struct HubEnergyGroup {
    windows: BTreeMap<&'static str,powerwatch_core::energy::EnergyStats>,
    selected: powerwatch_core::energy::EnergyStats,
}

#[derive(Serialize)]
struct HubEnergyResponse {
    global: HubEnergyGroup,
    nodes: BTreeMap<String,HubEnergyGroup>,
}

fn add_energy(dst: &mut powerwatch_core::energy::EnergyStats,
    src: &powerwatch_core::energy::EnergyStats) {
    dst.energy_kwh+=src.energy_kwh;
    dst.estimated_kwh+=src.estimated_kwh;
    // Infrastructure coverage is expressed in node-seconds (sum of covered nodes).
    dst.coverage_seconds+=src.coverage_seconds;
    if let Some(first)=src.first_seen_epoch {
        dst.first_seen_epoch=Some(dst.first_seen_epoch.map_or(first,|v|v.min(first)));
    }
    dst.from_epoch=src.from_epoch;
    dst.to_epoch=src.to_epoch;
}

async fn energy_overview(
    State(state): State<AppState>,Query(params):Query<EnergyQuery>,
)->Result<Json<HubEnergyResponse>,(StatusCode,String)> {
    let now=Utc::now().timestamp();
    let to=params.to.map_or(now,|value|value.timestamp());
    let selected_from=params.from.map(|value|value.timestamp());
    if to>now+60 || selected_from.is_some_and(|value|value>=to) {
        return Err((StatusCode::BAD_REQUEST,"invalid energy period".to_owned()));
    }
    let nodes=state.config.read().await.nodes.clone();
    let storage=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,
        "storage lock poisoned".to_owned()))?;
    let periods=[("24h",Some(now-86400)),("7d",Some(now-7*86400)),
        ("30d",Some(now-30*86400)),("all",None)];
    let mut global=HubEnergyGroup::default();
    let mut result=BTreeMap::new();
    for node in nodes {
        let mut group=HubEnergyGroup::default();
        for (key,start) in periods {
            let item=powerwatch_core::energy::stats(&storage.conn,&node.id,start,now)
                .map_err(|err|(StatusCode::INTERNAL_SERVER_ERROR,err.to_string()))?;
            if node.enabled && node.include_in_total {
                add_energy(global.windows.entry(key).or_default(),&item);
            }
            group.windows.insert(key,item);
        }
        group.selected=powerwatch_core::energy::stats(&storage.conn,&node.id,selected_from,to)
            .map_err(|err|(StatusCode::INTERNAL_SERVER_ERROR,err.to_string()))?;
        if node.enabled && node.include_in_total {
            add_energy(&mut global.selected,&group.selected);
        }
        result.insert(node.id,group);
    }
    // Even an empty Hub returns all expected windows.
    for (key,_) in periods {global.windows.entry(key).or_default();}
    Ok(Json(HubEnergyResponse{global,nodes:result}))
}

async fn admin_status(State(state): State<AppState>) -> Json<HubAdminStatus> {
    Json(HubAdminStatus {
        enabled: state.admin_auth.enabled(),
    })
}

async fn require_hub_admin(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> axum::response::Response {
    if state.admin_auth.authorizes(request.headers()) {
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": "Hub administrator authentication required" })),
    )
        .into_response()
}

fn build_router(state: AppState) -> Router {
    let administration = Router::new()
        .route("/api/hub/nodes", get(list_nodes).post(add_node))
        .route("/api/hub/nodes/:id", post(update_node).delete(delete_node))
        .route("/api/hub/test", post(probe_node))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_hub_admin,
        ));

    Router::new()
        .route("/", get(index))
        .route("/api/health", get(health))
        .route("/api/hub/auth/status", get(admin_status))
        .route("/api/hub/snapshot", get(snapshot))
        .route("/api/hub/history", get(history))
        .route("/api/hub/energy",get(energy_overview))
        .merge(administration)
        .with_state(state)
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let admin_auth = match HubAdminAuth::load() {
        Ok(auth) => auth,
        Err(error) => {
            eprintln!("failed to load Hub administrator authentication: {error}");
            std::process::exit(1);
        }
    };

    let mut config = load_config(&args.config);
    for node in &mut config.nodes {
        node.url = normalize_url(&node.url);
        if let Err(error) = validate_node(node) {
            eprintln!("warning: disabling invalid node {}: {error}", node.id);
            node.enabled = false;
        }
    }

    if let Err(error) = save_config(&args.config, &config) {
        eprintln!("warning: failed to persist hub config: {error}");
    }

    let storage = match HubStorage::open(&args.database) {
        Ok(storage) => storage,
        Err(error) => {
            eprintln!(
                "failed to open hub database {}: {error}",
                args.database.display()
            );
            std::process::exit(1);
        }
    };

    let client = Client::builder()
        .user_agent("PowerWatch-Hub/0.2")
        .build()
        .expect("failed to build HTTP client");

    let state = AppState {
        config: Arc::new(RwLock::new(config)),
        runtime: Arc::new(RwLock::new(HashMap::new())),
        config_path: args.config.clone(),
        storage: Arc::new(Mutex::new(storage)),
        stale_after: Duration::from_secs(args.stale_after.max(2)),
        client,
        admin_auth,
    };

    tokio::spawn(poll_loop(
        state.clone(),
        Duration::from_secs(args.refresh_interval.max(1)),
    ));
    tokio::spawn(history_loop(
        state.clone(),
        Duration::from_secs(args.history_interval.max(5)),
    ));
    tokio::spawn(backfill_all(state.clone()));

    let listener = match tokio::net::TcpListener::bind((args.host, args.port)).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("failed to bind {}:{}: {error}", args.host, args.port);
            std::process::exit(1);
        }
    };

    if !args.host.is_loopback() {
        if state.admin_auth.enabled() {
            eprintln!("PowerWatch Hub administrator authentication is enabled.");
        } else {
            eprintln!("WARNING: PowerWatch Hub administrator authentication is disabled.");
            eprintln!("Expose it only on a trusted private LAN.");
        }
    }

    println!(
        "powerwatch-hub listening on http://{}:{} ({} configured nodes)",
        args.host,
        args.port,
        state.config.read().await.nodes.len()
    );

    if let Err(error) = axum::serve(listener, build_router(state)).await {
        eprintln!("server error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    fn node_with_token(url: &str, token: Option<&str>) -> NodeConfig {
        NodeConfig {
            id: "node".into(),
            name: "Node".into(),
            url: url.into(),
            enabled: true,
            include_in_total: true,
            api_token: token.map(str::to_string),
        }
    }

    fn router_state(admin_auth: HubAdminAuth) -> (tempfile::TempDir, AppState) {
        let directory = tempfile::tempdir().unwrap();
        let storage = HubStorage::open(&directory.path().join("hub.db")).unwrap();
        let state = AppState {
            config: Arc::new(RwLock::new(HubConfig::default())),
            runtime: Arc::new(RwLock::new(HashMap::new())),
            config_path: directory.path().join("hub.json"),
            storage: Arc::new(Mutex::new(storage)),
            stale_after: Duration::from_secs(15),
            client: Client::new(),
            admin_auth,
        };
        (directory, state)
    }

    #[test]
    fn validates_node_ids_and_urls() {
        let good = NodeConfig {
            id: "lincstation-1".into(),
            name: "LincStation".into(),
            url: "http://192.168.0.196:3064".into(),
            enabled: true,
            include_in_total: true,
            api_token: None,
        };
        assert!(validate_node(&good).is_ok());

        let mut bad = good.clone();
        bad.id = "bad id".into();
        assert!(validate_node(&bad).is_err());

        bad = good;
        bad.url = "192.168.0.196".into();
        assert!(validate_node(&bad).is_err());
    }

    #[test]
    fn hub_tracks_independent_persistent_machine_energy() {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("hub-energy.db");
        let mut db=HubStorage::open(&path).unwrap();
        let start=DateTime::<Utc>::from_timestamp(1_700_000_100,0).unwrap();
        db.write_totals(start,&[("first".to_string(),20.0),("second".to_string(),30.0)]).unwrap();
        db.write_totals(start+chrono::Duration::seconds(60),&[("first".to_string(),20.0),("second".to_string(),30.0)]).unwrap();
        let first=energy::stats(&db.conn,"first",None,start.timestamp()+120).unwrap();
        let second=energy::stats(&db.conn,"second",None,start.timestamp()+120).unwrap();
        assert!(second.energy_kwh>first.energy_kwh);
        drop(db);
        let db=HubStorage::open(&path).unwrap();
        let resumed=energy::stats(&db.conn,"first",None,start.timestamp()+120).unwrap();
        assert!((resumed.energy_kwh-first.energy_kwh).abs()<1e-10);
    }

    #[test]
    fn parses_supported_history_ranges() {
        assert_eq!(parse_since("30m").unwrap(), chrono::Duration::minutes(30));
        assert_eq!(parse_since("24h").unwrap(), chrono::Duration::hours(24));
        assert_eq!(parse_since("7d").unwrap(), chrono::Duration::days(7));
        assert!(parse_since("1y").is_err());
    }

    #[test]
    fn recognizes_serialized_total_component() {
        assert!(is_total_component(&serde_json::json!("Total")));
        assert!(!is_total_component(&serde_json::json!("Cpu")));
    }

    #[test]
    fn public_node_view_never_serializes_the_api_token() {
        let node = NodeConfig {
            id: "node".into(),
            name: "Node".into(),
            url: "http://node:3000".into(),
            enabled: true,
            include_in_total: true,
            api_token: Some("pw_super_secret".into()),
        };
        let json = serde_json::to_string(&NodeConfigView::from(&node)).unwrap();
        assert!(!json.contains("pw_super_secret"));
        assert!(json.contains("\"has_api_token\":true"));
    }

    #[test]
    fn changing_a_node_url_never_carries_the_stored_token_implicitly() {
        let existing = node_with_token("http://old-node:3000", Some("pw_existing_secret"));

        let mut same_url = node_with_token("http://old-node:3000/", None);
        preserve_or_clear_token_for_update(&existing, &mut same_url);
        assert_eq!(same_url.api_token.as_deref(), Some("pw_existing_secret"));

        let mut changed_url = node_with_token("http://new-node:3000", None);
        preserve_or_clear_token_for_update(&existing, &mut changed_url);
        assert!(changed_url.api_token.is_none());

        let mut explicit_transfer =
            node_with_token("http://new-node:3000", Some("pw_explicit_secret"));
        preserve_or_clear_token_for_update(&existing, &mut explicit_transfer);
        assert_eq!(
            explicit_transfer.api_token.as_deref(),
            Some("pw_explicit_secret")
        );
    }

    #[tokio::test]
    async fn hub_administration_is_optional_and_protects_mutations_when_enabled() {
        let (_directory, state) = router_state(HubAdminAuth::with_token(None));
        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/hub/test")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"url":"http://127.0.0.1:1"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let admin_token = "a-very-long-hub-administrator-token";
        let (_directory, state) = router_state(HubAdminAuth::with_token(Some(admin_token)));
        let app = build_router(state);
        for (method, uri) in [
            ("GET", "/api/hub/nodes"),
            ("POST", "/api/hub/nodes"),
            ("POST", "/api/hub/nodes/node"),
            ("DELETE", "/api/hub/nodes/node"),
            ("POST", "/api/hub/test"),
        ] {
            let unauthorized = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED, "{uri}");
        }

        let authorized = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/hub/test")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {admin_token}"))
                    .body(Body::from(r#"{"url":"http://127.0.0.1:1"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authorized.status(), StatusCode::OK);
        let body = axum::body::to_bytes(authorized.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&body).contains(admin_token));
    }

    #[test]
    fn stores_and_reads_federated_history_without_duplicates() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut storage = HubStorage::open(tmp.path()).unwrap();
        let now = Utc::now();

        storage
            .write_totals(
                now,
                &[("garuda".into(), 70.0), ("lincstation".into(), 20.0)],
            )
            .unwrap();
        storage
            .write_history_points("garuda", &[(now, 72.0)])
            .unwrap();

        let included = ["garuda".to_string(), "lincstation".to_string()]
            .into_iter()
            .collect::<HashSet<_>>();
        let rows = storage
            .history_since(now - chrono::Duration::minutes(1), &included)
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert!((rows[0].global_watts - 92.0).abs() < f64::EPSILON);
        assert_eq!(rows[0].nodes["garuda"], 72.0);
    }
}
