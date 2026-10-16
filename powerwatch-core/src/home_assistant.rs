//! Optional MQTT / Home Assistant Discovery bridge, shared by the single host and Hub.
//! No network activity is performed unless the feature is explicitly enabled.
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS, Transport};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope { All, Global, Nodes }
impl Default for Scope { fn default() -> Self { Self::Global } }
fn default_port() -> u16 { 1883 }
fn default_interval() -> u64 { 30 }
fn default_prefix() -> String { "powerwatch".into() }
fn default_discovery() -> String { "homeassistant".into() }
fn default_true() -> bool { true }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)] pub enabled: bool,
    #[serde(default)] pub host: String,
    #[serde(default = "default_port")] pub port: u16,
    #[serde(default)] pub tls: bool,
    #[serde(default)] pub username: String,
    #[serde(default)] pub password: String,
    #[serde(default = "default_prefix")] pub topic_prefix: String,
    #[serde(default = "default_discovery")] pub discovery_prefix: String,
    #[serde(default = "default_interval")] pub interval_seconds: u64,
    #[serde(default = "default_true")] pub publish_components: bool,
    #[serde(default = "default_true")] pub publish_energy: bool,
    #[serde(default = "default_true")] pub publish_costs: bool,
    #[serde(default)] pub scope: Scope,
    #[serde(default)] pub id: String,
}
impl Default for Config {
    fn default() -> Self { Self { enabled:false, host:String::new(), port:1883, tls:false,
        username:String::new(), password:String::new(), topic_prefix:default_prefix(),
        discovery_prefix:default_discovery(), interval_seconds:30, publish_components:true,
        publish_energy:true, publish_costs:true, scope:Scope::Global, id:String::new() } }
}
#[derive(Debug, Deserialize)]
pub struct Update {
    pub enabled: bool, pub host: String, pub port: u16, pub tls: bool,
    pub username: String, #[serde(default)] pub password: Option<String>,
    #[serde(default)] pub clear_password: bool,
    pub topic_prefix: String, pub discovery_prefix: String, pub interval_seconds: u64,
    pub publish_components: bool, pub publish_energy: bool,
    #[serde(default = "default_true")] pub publish_costs: bool, pub scope: Scope,
}
#[derive(Debug, Serialize)]
pub struct PublicConfig {
    #[serde(flatten)] pub config: ConfigView,
    pub has_password: bool, pub connected: bool, pub status: String,
}
#[derive(Debug, Serialize)]
pub struct ConfigView {
    pub enabled: bool, pub host: String, pub port: u16, pub tls: bool,
    pub username: String, pub topic_prefix: String, pub discovery_prefix: String,
    pub interval_seconds: u64, pub publish_components: bool, pub publish_energy: bool, pub publish_costs: bool,
    pub scope: Scope,
}
#[derive(Default, Clone)]
struct Status { connected: bool, message: String }
static STATUS: OnceLock<Mutex<Status>> = OnceLock::new();
fn set_status(connected: bool, message: impl Into<String>) {
    if let Ok(mut status) = STATUS.get_or_init(|| Mutex::new(Status::default())).lock() {
        *status = Status { connected, message: message.into() };
    }
}
pub fn public(config: &Config) -> PublicConfig {
    let status = STATUS.get_or_init(|| Mutex::new(Status::default())).lock().map(|s| s.clone()).unwrap_or_default();
    PublicConfig { config: ConfigView { enabled:config.enabled, host:config.host.clone(), port:config.port,
        tls:config.tls, username:config.username.clone(), topic_prefix:config.topic_prefix.clone(),
        discovery_prefix:config.discovery_prefix.clone(), interval_seconds:config.interval_seconds,
        publish_components:config.publish_components, publish_energy:config.publish_energy,
        publish_costs:config.publish_costs,
        scope:config.scope.clone() }, has_password:!config.password.is_empty(),
        connected: status.connected && config.enabled, status: if !config.enabled { "disabled".into() } else { status.message } }
}
pub fn load(path: &Path) -> Result<Config, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("invalid MQTT settings: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e.to_string()),
    }
}
fn topic_segment(value: &str) -> bool {
    !value.is_empty() && value.len() <= 60 && value.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}
pub fn validate(config: &Config) -> Result<(), String> {
    if config.host.len() > 253 || config.host.chars().any(|c| c.is_control() || c.is_whitespace() || "/\\@".contains(c)) || (config.enabled && config.host.is_empty()) {
        return Err("invalid broker host (hostname or IP only, no scheme)".into());
    }
    if config.enabled && !topic_segment(&config.id) { return Err("missing instance identity: save the MQTT configuration in the WebUI".into()); }
    if config.port == 0 { return Err("port must be nonzero".into()); }
    if !(10..=600).contains(&config.interval_seconds) { return Err("interval must be between 10 and 600 seconds".into()); }
    if !topic_segment(&config.topic_prefix) || !topic_segment(&config.discovery_prefix) {
        return Err("MQTT prefixes must be 1–60 letters, numbers, '-' or '_'".into());
    }
    if config.username.len() > 256 || config.password.len() > 1024 || config.username.chars().any(char::is_control) {
        return Err("invalid MQTT credentials".into());
    }
    Ok(())
}
pub fn update(path: &Path, update: Update) -> Result<Config, String> {
    let previous = load(path)?;
    let config = Config {
        enabled:update.enabled, host:update.host.trim().to_string(), port:update.port, tls:update.tls,
        username:update.username.trim().to_string(),
        password:if update.clear_password { String::new() } else {
            update.password.filter(|p| !p.is_empty()).unwrap_or(previous.password)
        },
        topic_prefix:update.topic_prefix, discovery_prefix:update.discovery_prefix,
        interval_seconds:update.interval_seconds, publish_components:update.publish_components,
        publish_energy:update.publish_energy, publish_costs:update.publish_costs, scope:update.scope,
        id:if previous.id.is_empty() { format!("{:032x}", rand::random::<u128>()) } else { previous.id },
    };
    validate(&config)?;
    save(path, &config)?;
    Ok(config)
}
fn save(path: &Path, config: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e|e.to_string())?; }
    let temporary = path.with_extension("mqtt-tmp");
    let contents=serde_json::to_vec_pretty(config).map_err(|e|e.to_string())?;
    #[cfg(unix)] {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        use std::io::Write;
        let mut file=std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temporary).map_err(|e|e.to_string())?;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600)).map_err(|e|e.to_string())?;
        file.write_all(&contents).map_err(|e|e.to_string())?;
        file.sync_all().map_err(|e|e.to_string())?;
    }
    #[cfg(not(unix))] std::fs::write(&temporary, &contents).map_err(|e|e.to_string())?;
    std::fs::rename(&temporary,path).map_err(|e|e.to_string())
}
pub fn instance_path() -> PathBuf {
    let home=std::env::var("HOME").unwrap_or_else(|_|".".into());
    PathBuf::from(home).join(".local/share/powerwatch/home-assistant-mqtt.json")
}
pub fn hub_path(hub_config: &Path) -> PathBuf { hub_config.with_extension("mqtt.json") }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricKind { Power, Energy, Monetary }
#[derive(Clone, Debug)]
pub struct Metric { pub key: String, pub label: String, pub value: f64, pub kind: MetricKind }
#[derive(Clone, Debug)]
pub struct Device { pub key: String, pub label: String, pub available: bool, pub metrics: Vec<Metric> }
impl Metric {
    pub fn power(key: impl Into<String>, label: impl Into<String>, watts:f64) -> Self {
        Self { key:key.into(), label:label.into(), value:watts, kind:MetricKind::Power }
    }
    pub fn energy(key: impl Into<String>, label: impl Into<String>, kwh:f64) -> Self {
        Self { key:key.into(), label:label.into(), value:kwh, kind:MetricKind::Energy }
    }
    pub fn monetary(key: impl Into<String>, label: impl Into<String>, eur:f64) -> Self {
        Self { key:key.into(), label:label.into(), value:eur, kind:MetricKind::Monetary }
    }
}
/// Use the same pricing engine as the WebUI. Missing prices or projected history
/// must never appear in HA as a definitive zero or an apparently complete bill.
/// This is a *recalculable estimate*, not a monotonic physical energy meter.
pub fn cost_metrics(stats:&crate::tariffs_fr::CostStats, include_subscription:bool) -> Vec<Metric> {
    if stats.unpriced_kwh > 1e-8 || stats.projected_kwh > 1e-8
        || stats.projected_energy_cost_eur.abs() > 1e-8
        || !stats.energy_cost_eur.is_finite() || !stats.subscription_eur.is_finite()
        || !stats.projected_subscription_eur.is_finite()
        || (stats.billed_energy_kwh <= 0.0 && stats.subscription_eur <= 0.0) {
        return vec![];
    }
    let mut out=vec![Metric::monetary("cost_energy","Estimated electricity usage cost",stats.energy_cost_eur)];
    // An energy cost can be fully priced while the fixed subscription cannot.
    // In that case retain only the valid variable amount, not a misleading total.
    if include_subscription && !stats.missing_subscription_rate
        && stats.projected_subscription_eur.abs() <= 1e-8 {
        out.push(Metric::monetary("cost_subscription","Estimated electricity subscription",stats.subscription_eur));
        out.push(Metric::monetary("cost_total","Estimated electricity cost (incl. subscription)",stats.total_eur()));
    }
    out
}
/// Stable component slug with hash: prevent collisions between similarly named disks/GPUs.
pub fn component_key(name: &str) -> String {
    let slug:String=name.chars().filter(|c|c.is_ascii_alphanumeric()).take(20).collect::<String>().to_lowercase();
    let mut hash=0x811c9dc5u32;
    for c in name.bytes() { hash=(hash ^ u32::from(c)).wrapping_mul(0x01000193); }
    format!("{slug}_{hash:08x}")
}
fn base(cfg: &Config) -> String { format!("{}/{}",cfg.topic_prefix,cfg.id) }
fn options(cfg: &Config, with_will:bool, client_id:String) -> MqttOptions {
    let mut opts=MqttOptions::new(client_id,cfg.host.clone(),cfg.port);
    opts.set_keep_alive(Duration::from_secs(20));
    opts.set_clean_session(true);
    if !cfg.username.is_empty() { opts.set_credentials(cfg.username.clone(),cfg.password.clone()); }
    if cfg.tls { opts.set_transport(Transport::tls_with_default_config()); }
    if with_will { opts.set_last_will(LastWill::new(format!("{}/availability",base(cfg)),"offline",QoS::AtLeastOnce,true)); }
    opts
}
/// Checks MQTT ConnAck (including authentication/TLS), not just TCP reachability.
pub async fn test(config: &Config) -> Result<(), String> {
    validate(config)?;
    if config.host.is_empty() {return Err("broker host is required".into());}
    let opts=options(config,false,format!("pw-test-{:016x}",rand::random::<u64>()));
    let (_client,mut eventloop)=AsyncClient::new(opts,10);
    tokio::time::timeout(Duration::from_secs(7),async {
        loop { match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                if ack.code == rumqttc::ConnectReturnCode::Success {return Ok(());}
                return Err(format!("MQTT refused connection: {:?}", ack.code));
            },
            Ok(_) => {},
            Err(e) => return Err(format!("MQTT: {e}")),
        }}
    }).await.map_err(|_|"MQTT connection timed out".to_string())?
}
fn discovery(cfg:&Config, device:&Device, metric:&Metric) -> (String,String) {
    let id=format!("pw_{}_{}_{}",cfg.id,device.key,metric.key);
    let topic=format!("{}/sensor/{}/config",cfg.discovery_prefix,id);
    let device_base=format!("{}/{}",base(cfg),device.key);
    let payload=json!({
        "name":metric.label,
        "unique_id":id,
        "object_id":format!("powerwatch_{}_{}",device.key,metric.key),
        "state_topic":format!("{device_base}/{}",metric.key),
        "availability":[
            {"topic":format!("{}/availability",base(cfg)),"payload_available":"online","payload_not_available":"offline"},
            {"topic":format!("{device_base}/availability"),"payload_available":"online","payload_not_available":"offline"}
        ],
        "availability_mode":"all",
        "device_class":match metric.kind {MetricKind::Energy=>"energy",MetricKind::Monetary=>"monetary",MetricKind::Power=>"power"},
        "state_class":match metric.kind {MetricKind::Energy=>"total_increasing",MetricKind::Monetary=>"total",MetricKind::Power=>"measurement"},
        "expire_after":if metric.kind==MetricKind::Power {Some(cfg.interval_seconds.saturating_mul(3).max(60))} else {None},
        "unit_of_measurement":match metric.kind {MetricKind::Energy=>"kWh",MetricKind::Monetary=>"EUR",MetricKind::Power=>"W"},
        "suggested_display_precision":match metric.kind {MetricKind::Energy=>3,MetricKind::Monetary=>2,MetricKind::Power=>1},
        "device":{"identifiers":[format!("powerwatch_{}_{}",cfg.id,device.key)],
            "name":device.label,"manufacturer":"Aerya","model":"PowerWatch"},
        "origin":{"name":"PowerWatch","sw_version":env!("CARGO_PKG_VERSION"),"support_url":"https://github.com/Aerya/PowerWatch"},
    });
    let mut payload=payload;
    if metric.kind != MetricKind::Power { payload.as_object_mut().unwrap().remove("expire_after"); }
    (topic,payload.to_string())
}
async fn publish(client:&AsyncClient, topic:String, retain:bool, payload:impl Into<Vec<u8>>) -> Result<(),String> {
    client.publish(topic,QoS::AtLeastOnce,retain,payload).await.map_err(|e|e.to_string())
}
/// Retained Home Assistant Discovery topics survive broker and application restarts.
/// Persist their inventory so disabled sensors, removed nodes and changed MQTT
/// prefixes can be cleaned on the *same* broker, even after a Hub restart.
/// This file contains only topic names and no credentials or sensor readings.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Inventory {
    host: String,
    port: u16,
    tls: bool,
    root: String,
    topics: HashSet<String>,
    devices: HashSet<String>,
}
fn inventory_path(config_path:&Path) -> PathBuf { config_path.with_extension("inventory.json") }
fn read_inventory(path:&Path) -> Result<Inventory,String> {
    match std::fs::read(path) {
        Ok(data) => serde_json::from_slice(&data).map_err(|e|e.to_string()),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound => Ok(Inventory::default()),
        Err(e) => Err(e.to_string()),
    }
}
fn write_inventory(path:&Path,cfg:&Config,known:&HashSet<String>,devices:&HashSet<String>) -> Result<(),String> {
    let inventory=Inventory { host:cfg.host.clone(),port:cfg.port,tls:cfg.tls,
        root:base(cfg),topics:known.clone(),devices:devices.clone() };
    let data=serde_json::to_vec_pretty(&inventory).map_err(|e|e.to_string())?;
    let tmp=path.with_extension("inventory-tmp");
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        use std::io::Write;
        let mut file=std::fs::OpenOptions::new().write(true).create(true).truncate(true)
            .mode(0o600).open(&tmp).map_err(|e|e.to_string())?;
        file.write_all(&data).map_err(|e|e.to_string())?;
        file.sync_all().map_err(|e|e.to_string())?;
    }
    #[cfg(not(unix))] std::fs::write(&tmp,&data).map_err(|e|e.to_string())?;
    std::fs::rename(&tmp,path).map_err(|e|e.to_string())
}
fn same_broker(inv:&Inventory,cfg:&Config)->bool {
    inv.host==cfg.host && inv.port==cfg.port && inv.tls==cfg.tls
}
/// Keep advertised sensors for offline devices so an intermittent disconnect
/// makes them unavailable rather than deleting/recreating them in Home Assistant.
fn offline_topics(known:&HashSet<String>,cfg:&Config,device_key:&str)->HashSet<String> {
    let prefix=format!("{}/sensor/pw_{}_{}_",cfg.discovery_prefix,cfg.id,device_key);
    known.iter().filter(|topic|topic.starts_with(&prefix)).cloned().collect()
}
async fn publish_devices(
    client:&AsyncClient,cfg:&Config,devices:Vec<Device>,
    known:&mut HashSet<String>,session_announced:&mut HashSet<String>,
    present_devices:&mut HashSet<String>,
) -> Result<(),String> {
    let mut desired=HashSet::new();
    let mut current_devices=HashSet::new();
    let root=base(cfg);
    for device in devices {
        if !topic_segment(&device.key) { continue; }
        let device_base=format!("{root}/{}",device.key);
        current_devices.insert(device.key.clone());
        publish(client,format!("{device_base}/availability"),true,
            if device.available {"online"} else {"offline"}).await?;
        if !device.available {
            desired.extend(offline_topics(known,cfg,&device.key));
            continue;
        }
        for metric in device.metrics.iter().filter(|m|m.value.is_finite() && m.value>=0.0 && topic_segment(&m.key)) {
            let (topic,config)=discovery(cfg,&device,metric);
            desired.insert(topic.clone());
            if session_announced.insert(topic.clone()) {
                publish(client,topic.clone(),true,config).await?;
            }
            publish(client,format!("{device_base}/{}",metric.key),metric.kind!=MetricKind::Power,
                format!("{:.6}",metric.value)).await?;
        }
    }
    // Delete retained Discovery configurations that are no longer valid: cost
    // tariff disabled, scope change, removed node, renamed component, etc.
    for stale in known.difference(&desired) {
        publish(client,stale.clone(),true,Vec::<u8>::new()).await?;
        session_announced.remove(stale);
    }
    for removed in present_devices.difference(&current_devices) {
        publish(client,format!("{root}/{removed}/availability"),true,"offline").await?;
    }
    *known=desired;
    *present_devices=current_devices;
    Ok(())
}
async fn cleanup(client:&AsyncClient,cfg:&Config,known:&HashSet<String>) -> Result<(),String> {
    for topic in known {publish(client,topic.clone(),true,Vec::<u8>::new()).await?;}
    publish(client,format!("{}/availability",base(cfg)),true,"offline").await
}
/// Long-running worker. Configuration changes are picked up without restarts.
pub async fn run<F,Fut>(path:PathBuf,provider:F)
where F:Fn()->Fut+Send+Sync+'static, Fut:Future<Output=Vec<Device>>+Send {
    let manifest_path=inventory_path(&path);
    loop {
        let cfg=match load(&path) {Ok(c)=>c,Err(e)=>{set_status(false,e);tokio::time::sleep(Duration::from_secs(5)).await;continue;}};
        if !cfg.enabled {set_status(false,"disabled");tokio::time::sleep(Duration::from_secs(5)).await;continue;}
        if let Err(e)=validate(&cfg) {set_status(false,e);tokio::time::sleep(Duration::from_secs(5)).await;continue;}
        let prior=match read_inventory(&manifest_path) {Ok(i)=>i,Err(e)=>{
            set_status(false,format!("invalid MQTT sensor inventory: {e}"));
            tokio::time::sleep(Duration::from_secs(5)).await;continue;
        }};
        // Inventory from another broker must never be erased on this broker.
        let same=same_broker(&prior,&cfg);
        let mut known=if same {prior.topics} else {HashSet::new()};
        let mut present_devices=if same && prior.root==base(&cfg) {prior.devices} else {HashSet::new()};
        let old_root=if same {prior.root} else {String::new()};
        let client_id=format!("pw-{}", &cfg.id[..cfg.id.len().min(28)]);
        let (client,mut events)=AsyncClient::new(options(&cfg,true,client_id),512);
        let (notify,mut reconnect)=tokio::sync::mpsc::channel::<()>(4);
        let event_worker=tokio::spawn(async move {
            loop {
                match events.poll().await {
                    Ok(Event::Incoming(Packet::ConnAck(ack))) if ack.code==rumqttc::ConnectReturnCode::Success => {
                        let _=notify.try_send(());
                    },
                    Ok(_) => {},
                    Err(e) => {set_status(false,format!("MQTT disconnected: {e}"));tokio::time::sleep(Duration::from_secs(3)).await;}
                }
            }
        });
        set_status(false,"connecting");
        let mut session_announced=HashSet::new();
        let mut connected=false;
        let mut ticker=tokio::time::interval(Duration::from_secs(cfg.interval_seconds));
        let mut check=tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                signal=reconnect.recv()=>{
                    if signal.is_none(){break;}
                    connected=true;
                    // A reconnection may follow a broker reset: republish all
                    // Discovery topics even when recorded in our local inventory.
                    session_announced.clear();
                    set_status(true,"connected");
                    if !old_root.is_empty() && old_root!=base(&cfg) {
                        if let Err(e)=publish(&client,format!("{old_root}/availability"),true,"offline").await {
                            set_status(false,e);
                        }
                    }
                    if let Err(e)=publish(&client,format!("{}/availability",base(&cfg)),true,"online").await {set_status(false,e);}
                    ticker.reset();
                    let devices=provider().await;
                    match tokio::time::timeout(Duration::from_secs(8),publish_devices(&client,&cfg,devices,&mut known,&mut session_announced,&mut present_devices)).await {
                        Ok(Ok(()))=>match write_inventory(&manifest_path,&cfg,&known,&present_devices) {
                            Ok(())=>set_status(true,"connected"), Err(e)=>set_status(false,e)
                        },
                        Ok(Err(e))=>set_status(false,e), Err(_)=>set_status(false,"MQTT publishing timed out"),
                    }
                },
                _=ticker.tick(), if connected => {
                    let devices=provider().await;
                    match tokio::time::timeout(Duration::from_secs(8),publish_devices(&client,&cfg,devices,&mut known,&mut session_announced,&mut present_devices)).await {
                        Ok(Ok(()))=>match write_inventory(&manifest_path,&cfg,&known,&present_devices) {
                            Ok(())=>set_status(true,"connected"), Err(e)=>set_status(false,e)
                        },
                        Ok(Err(e))=>set_status(false,e), Err(_)=>set_status(false,"MQTT publishing timed out"),
                    }
                },
                _=check.tick() => {
                    match load(&path) {
                        Ok(new) if new!=cfg => {
                            if cleanup(&client,&cfg,&known).await.is_ok() {
                                if let Err(e)=write_inventory(&manifest_path,&cfg,&HashSet::new(),&HashSet::new()) {set_status(false,e);}
                            }
                            break;
                        },
                        Err(e) => {set_status(false,e);break;},
                        _=>{}
                    }
                },
            }
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        event_worker.abort();
        set_status(false,"reconfiguring");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn credentials_are_masked_and_preserved() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("mqtt.json");
        let make=|password:Option<&str>|Update{enabled:true,host:"127.0.0.1".into(),port:1883,tls:false,username:"pw".into(),password:password.map(str::to_string),clear_password:false,topic_prefix:"powerwatch".into(),discovery_prefix:"homeassistant".into(),interval_seconds:30,publish_components:true,publish_energy:true,publish_costs:true,scope:Scope::All};
        let initial=update(&path,make(Some("private"))).unwrap();
        assert!(public(&initial).has_password);
        assert!(!serde_json::to_string(&public(&initial)).unwrap().contains("private"));
        let subsequent=update(&path,make(None)).unwrap();
        assert_eq!(initial.password,subsequent.password);
        assert_eq!(initial.id,subsequent.id);
    }
    #[test] fn discovery_has_energy_semantics_and_two_availability_sources() {
        let cfg=Config{id:"fixed".into(),..Config::default()};
        let metric=Metric::energy("energy_total","Lifetime energy",12.3);
        let (_,raw)=discovery(&cfg,&Device{key:"node".into(),label:"Mini PC".into(),available:true,metrics:vec![]},&metric);
        let json:serde_json::Value=serde_json::from_str(&raw).unwrap();
        assert_eq!(json["state_class"],"total_increasing");
        assert_eq!(json["device_class"],"energy");
        assert_eq!(json["availability"].as_array().unwrap().len(),2);
    }
    #[test] fn mqtt_cost_discovery_uses_monetary_total_not_increasing() {
        let cfg=Config{id:"fixed".into(),..Config::default()};
        let metric=Metric::monetary("cost_total","Estimated cost",12.34);
        let (_,raw)=discovery(&cfg,&Device{key:"node".into(),label:"Mini PC".into(),available:true,metrics:vec![]},&metric);
        let obj:serde_json::Value=serde_json::from_str(&raw).unwrap();
        assert_eq!(obj["device_class"],"monetary");
        assert_eq!(obj["state_class"],"total");
        assert_eq!(obj["unit_of_measurement"],"EUR");
        assert!(obj.get("expire_after").is_none());
    }
    #[test] fn no_incomplete_or_unconfigured_costs_published() {
        let mut stats=crate::tariffs_fr::CostStats::default();
        assert!(cost_metrics(&stats,true).is_empty());
        stats.billed_energy_kwh=3.0;
        stats.energy_cost_eur=0.60;
        stats.subscription_eur=0.20;
        assert_eq!(cost_metrics(&stats,true).len(),3);
        assert_eq!(cost_metrics(&stats,false).len(),1);
        stats.unpriced_kwh=0.1;
        assert!(cost_metrics(&stats,true).is_empty());
        stats.unpriced_kwh=0.0;
        stats.projected_kwh=0.1;
        assert!(cost_metrics(&stats,true).is_empty());
        stats.projected_kwh=0.0;
        stats.projected_subscription_eur=0.12;
        // Estimated fixed charges must never silently become a full bill.
        assert_eq!(cost_metrics(&stats,true).len(),1);
        stats.projected_subscription_eur=0.0;
        stats.missing_subscription_rate=true;
        assert_eq!(cost_metrics(&stats,true).len(),1);
        stats.missing_subscription_rate=false;
        // Variable-only export is still valid and deliberately omits fixed charges.
        assert_eq!(cost_metrics(&stats,false).len(),1);
    }
    #[test] fn retained_inventory_preserves_offline_devices_and_uses_broker_identity() {
        let dir=tempfile::tempdir().unwrap();let file=dir.path().join("mqtt.inventory.json");
        let cfg=Config{host:"broker.local".into(),id:"fixed".into(),..Config::default()};
        let topic="homeassistant/sensor/pw_fixed_node_energy_total/config".to_string();
        let topics=HashSet::from([topic.clone()]);
        let devices=HashSet::from(["node".to_string()]);
        write_inventory(&file,&cfg,&topics,&devices).unwrap();
        let saved=read_inventory(&file).unwrap();
        assert!(same_broker(&saved,&cfg));
        assert_eq!(saved.topics,topics);
        assert_eq!(offline_topics(&saved.topics,&cfg,"node"),topics);
        let other=Config{host:"other-broker.local".into(),..cfg};
        assert!(!same_broker(&saved,&other));
    }
    #[tokio::test]
    async fn broker_handshake_test_checks_real_mqtt_connack() {
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        let server=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port=server.local_addr().unwrap().port();
        let server_task=tokio::spawn(async move {
            let (mut stream,_)=server.accept().await.unwrap();
            let mut data=[0_u8;512];
            let n=stream.read(&mut data).await.unwrap();
            assert!(n>0);
            assert_eq!(data[0],0x10); // MQTT CONNECT (not just a TCP probe)
            stream.write_all(&[0x20,0x02,0x00,0x00]).await.unwrap(); // MQTT CONNACK success
            tokio::time::sleep(Duration::from_millis(200)).await;
        });
        let cfg=Config{enabled:true,host:"127.0.0.1".into(),port,id:"fixed".into(),..Config::default()};
        test(&cfg).await.expect("valid MQTT CONNACK must pass");
        server_task.await.unwrap();
    }
        #[test] fn topic_validation_avoids_mqtt_wildcards() {
        let config=Config{enabled:true,host:"mqtt".into(),id:"fixed".into(),topic_prefix:"#".into(),..Config::default()};
        assert!(validate(&config).is_err());
        assert_ne!(component_key("disk:sda"),component_key("disk:sdb"));
    }
}
