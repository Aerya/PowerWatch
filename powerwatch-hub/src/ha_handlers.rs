use super::*;
use powerwatch_core::home_assistant as ha;

pub async fn get_config(State(state):State<AppState>) -> Result<Json<ha::PublicConfig>,(StatusCode,String)> {
    let cfg=ha::load(&ha::hub_path(&state.config_path)).map_err(internal)?;
    Ok(Json(ha::public(&cfg)))
}
pub async fn put_config(State(state):State<AppState>,Json(update):Json<ha::Update>) -> Result<Json<ha::PublicConfig>,(StatusCode,String)> {
    let cfg=ha::update(&ha::hub_path(&state.config_path),update).map_err(|e|(StatusCode::BAD_REQUEST,e))?;
    Ok(Json(ha::public(&cfg)))
}
pub async fn test_config(State(state):State<AppState>) -> Result<Json<serde_json::Value>,(StatusCode,String)> {
    let cfg=ha::load(&ha::hub_path(&state.config_path)).map_err(internal)?;
    ha::test(&cfg).await.map_err(|e|(StatusCode::BAD_GATEWAY,e))?;
    Ok(Json(serde_json::json!({"ok":true})))
}
fn internal(e:String)->(StatusCode,String) {(StatusCode::INTERNAL_SERVER_ERROR,e)}

pub async fn devices(state:AppState) -> Vec<ha::Device> {
    let cfg=match ha::load(&ha::hub_path(&state.config_path)) {Ok(c)=>c,Err(_)=>return vec![]};
    let snap=snapshot(State(state.clone())).await.0;
    let mut devices=vec![];
    // Live W must never be a fabricated 0 when no included node has data.
    // The durable kWh meter, however, stays available while Hub is running,
    // even when all federated nodes are offline.
    let global_power_available=snap.nodes.iter().any(|n|n.include_in_total && matches!(&n.status,NodeStatus::Online) && n.total_watts.is_some());
    if matches!(&cfg.scope,ha::Scope::All|ha::Scope::Global) {
        let mut metrics=vec![];
        if global_power_available {
            metrics.push(ha::Metric::power("power_total","Infrastructure power",snap.total_watts));
        }
        if cfg.publish_energy {
            if let Ok(storage)=state.storage.lock() {
                if let Ok(value)=super::ha_energy::total_kwh(&storage.conn) {
                    metrics.push(ha::Metric::energy("energy_total","Cumulative infrastructure energy",value));
                }
            }
        }
        devices.push(ha::Device {
            key:"hub".into(),label:"PowerWatch Hub".into(),available:true,metrics,
        });
    }
    if matches!(&cfg.scope,ha::Scope::All|ha::Scope::Nodes) {
        let now=Utc::now().timestamp();
        let storage=state.storage.lock().ok();
        for node in snap.nodes {
            let mut metrics=vec![];
            let online=matches!(&node.status,NodeStatus::Online);
            if online {
                if let Some(watts)=node.total_watts {metrics.push(ha::Metric::power("power_total","Total power",watts));}
                if cfg.publish_components {
                    for sensor in node.sensors {
                        if let Some(reading)=sensor.reading {
                            metrics.push(ha::Metric::power(format!("power_{}",ha::component_key(&sensor.name)),
                                sensor.display_name.unwrap_or(sensor.name),reading.watts));
                        }
                    }
                }
                if cfg.publish_energy {
                    if let Some(ref db)=storage {
                        if let Ok(stat)=energy::stats(&db.conn,&node.id,None,now) {
                            if stat.first_seen_epoch.is_some() {
                                metrics.push(ha::Metric::energy("energy_total","Lifetime energy",stat.energy_kwh));
                            }
                        }
                    }
                }
            }
            // Node IDs are stable Hub identifiers; use a hash to avoid MQTT topic injection.
            devices.push(ha::Device{key:format!("node_{}",ha::component_key(&node.id)),label:node.name,
                available:online,metrics});
        }
    }
    devices
}
