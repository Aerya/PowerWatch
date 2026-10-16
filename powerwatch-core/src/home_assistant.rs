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
impl Default for Scope { fn default() -> Self { Self::All } }
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
    #[serde(default)] pub scope: Scope,
    #[serde(default)] pub id: String,
}
impl Default for Config {
    fn default() -> Self { Self { enabled:false, host:String::new(), port:1883, tls:false,
        username:String::new(), password:String::new(), topic_prefix:default_prefix(),
        discovery_prefix:default_discovery(), interval_seconds:30, publish_components:true,
        publish_energy:true, scope:Scope::All, id:String::new() } }
}
#[derive(Debug, Deserialize)]
pub struct Update {
    pub enabled: bool, pub host: String, pub port: u16, pub tls: bool,
    pub username: String, #[serde(default)] pub password: Option<String>,
    #[serde(default)] pub clear_password: bool,
    pub topic_prefix: String, pub discovery_prefix: String, pub interval_seconds: u64,
    pub publish_components: bool, pub publish_energy: bool, pub scope: Scope,
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
    pub interval_seconds: u64, pub publish_components: bool, pub publish_energy: bool,
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
        publish_energy:update.publish_energy, scope:update.scope,
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

#[derive(Clone, Debug)]
pub struct Metric { pub key: String, pub label: String, pub value: f64, pub energy: bool }
#[derive(Clone, Debug)]
pub struct Device { pub key: String, pub label: String, pub available: bool, pub metrics: Vec<Metric> }
impl Metric {
    pub fn power(key: impl Into<String>, label: impl Into<String>, watts:f64) -> Self {
        Self { key:key.into(), label:label.into(), value:watts, energy:false }
    }
    pub fn energy(key: impl Into<String>, label: impl Into<String>, kwh:f64) -> Self {
        Self { key:key.into(), label:label.into(), value:kwh, energy:true }
    }
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
        "device_class":if metric.energy {"energy"} else {"power"},
        "state_class":if metric.energy {"total_increasing"} else {"measurement"},
        "expire_after":if metric.energy {None} else {Some(cfg.interval_seconds.saturating_mul(3).max(60))},
        "unit_of_measurement":if metric.energy {"kWh"} else {"W"},
        "suggested_display_precision":if metric.energy {3} else {1},
        "device":{"identifiers":[format!("powerwatch_{}_{}",cfg.id,device.key)],
            "name":device.label,"manufacturer":"Aerya","model":"PowerWatch"},
        "origin":{"name":"PowerWatch","sw_version":env!("CARGO_PKG_VERSION"),"support_url":"https://github.com/Aerya/PowerWatch"},
    });
    let mut payload=payload;
    if metric.energy { payload.as_object_mut().unwrap().remove("expire_after"); }
    (topic,payload.to_string())
}
async fn publish(client:&AsyncClient, topic:String, retain:bool, payload:impl Into<Vec<u8>>) -> Result<(),String> {
    client.publish(topic,QoS::AtLeastOnce,retain,payload).await.map_err(|e|e.to_string())
}
async fn publish_devices(client:&AsyncClient,cfg:&Config, devices:Vec<Device>, known:&mut HashSet<String>, present_devices:&mut HashSet<String>) -> Result<(),String> {
    let mut current_devices=HashSet::new();
    let root=base(cfg);
    for device in devices {
        if !topic_segment(&device.key) { continue; }
        let device_base=format!("{root}/{}",device.key);
        current_devices.insert(device.key.clone());
        publish(client,format!("{device_base}/availability"),true,if device.available {"online"} else {"offline"}).await?;
        if !device.available {continue;}
        for metric in device.metrics.iter().filter(|m|m.value.is_finite() && m.value>=0.0 && topic_segment(&m.key)) {
            let (topic,config)=discovery(cfg,&device,metric);
            if !known.contains(&topic) {
                publish(client,topic.clone(),true,config).await?;
                known.insert(topic);
            }
            // Retained energy ensures HA can restore the persistent meter after its own restart.
            publish(client,format!("{device_base}/{}",metric.key),metric.energy,format!("{:.6}",metric.value)).await?;
        }
    }
    // Machines removed from a live Hub must stop advertising availability immediately.
    for removed in present_devices.difference(&current_devices) {
        publish(client,format!("{root}/{removed}/availability"),true,"offline").await?;
    }
    *present_devices=current_devices;
    Ok(())
}
async fn cleanup(client:&AsyncClient,cfg:&Config,known:&HashSet<String>) {
    for topic in known {let _=publish(client,topic.clone(),true,Vec::<u8>::new()).await;}
    let _=publish(client,format!("{}/availability",base(cfg)),true,"offline").await;
}
/// Long-running worker. Config changes are detected without restarting either binary.
pub async fn run<F,Fut>(path:PathBuf,provider:F)
where F:Fn()->Fut+Send+Sync+'static, Fut:Future<Output=Vec<Device>>+Send {
    loop {
        let cfg=match load(&path) {Ok(c)=>c,Err(e)=>{set_status(false,e);tokio::time::sleep(Duration::from_secs(5)).await;continue;}};
        if !cfg.enabled {set_status(false,"disabled");tokio::time::sleep(Duration::from_secs(5)).await;continue;}
        if let Err(e)=validate(&cfg) {set_status(false,e);tokio::time::sleep(Duration::from_secs(5)).await;continue;}
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
        let mut known=HashSet::new();
        let mut present_devices=HashSet::new();
        let mut connected=false;
        let mut ticker=tokio::time::interval(Duration::from_secs(cfg.interval_seconds));
        let mut check=tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                signal=reconnect.recv()=>{
                    if signal.is_none(){break;}
                    connected=true; known.clear(); // New session: always re-publish retained discovery.
                    set_status(true,"connected");
                    if let Err(e)=publish(&client,format!("{}/availability",base(&cfg)),true,"online").await {set_status(false,e);}
                    ticker.reset();
                    let devices=provider().await;
                    match tokio::time::timeout(Duration::from_secs(8),publish_devices(&client,&cfg,devices,&mut known,&mut present_devices)).await {
                        Ok(Ok(()))=>{}, Ok(Err(e))=>set_status(false,e), Err(_)=>set_status(false,"MQTT publishing timed out"),
                    }
                },
                _=ticker.tick(), if connected => {
                    let devices=provider().await;
                    match tokio::time::timeout(Duration::from_secs(8),publish_devices(&client,&cfg,devices,&mut known,&mut present_devices)).await {
                        Ok(Ok(()))=>{}, Ok(Err(e))=>set_status(false,e), Err(_)=>set_status(false,"MQTT publishing timed out"),
                    }
                },
                _=check.tick() => {
                    match load(&path) {
                        Ok(new) if new!=cfg => {cleanup(&client,&cfg,&known).await;break;},
                        Err(e) => {set_status(false,e);break;},
                        _=>{}
                    }
                },
            }
        }
        // Give the MQTT event loop a chance to flush retained discovery tombstones.
        tokio::time::sleep(Duration::from_millis(300)).await;
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
        let make=|password:Option<&str>|Update{enabled:true,host:"127.0.0.1".into(),port:1883,tls:false,username:"pw".into(),password:password.map(str::to_string),clear_password:false,topic_prefix:"powerwatch".into(),discovery_prefix:"homeassistant".into(),interval_seconds:30,publish_components:true,publish_energy:true,scope:Scope::All};
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
    #[test] fn topic_validation_avoids_mqtt_wildcards() {
        let config=Config{enabled:true,host:"mqtt".into(),id:"fixed".into(),topic_prefix:"#".into(),..Config::default()};
        assert!(validate(&config).is_err());
        assert_ne!(component_key("disk:sda"),component_key("disk:sdb"));
    }
}
