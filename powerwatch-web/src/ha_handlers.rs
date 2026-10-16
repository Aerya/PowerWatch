use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use powerwatch_core::home_assistant as ha;
use crate::state::AppState;

pub async fn get_config() -> Result<Json<ha::PublicConfig>,(StatusCode,String)> {
    let config=ha::load(&ha::instance_path()).map_err(internal)?;
    Ok(Json(ha::public(&config)))
}
pub async fn put_config(Json(update):Json<ha::Update>) -> Result<Json<ha::PublicConfig>,(StatusCode,String)> {
    let config=ha::update(&ha::instance_path(),update).map_err(|e|(StatusCode::BAD_REQUEST,e))?;
    Ok(Json(ha::public(&config)))
}
pub async fn test_config() -> Result<Json<serde_json::Value>,(StatusCode,String)> {
    let config=ha::load(&ha::instance_path()).map_err(internal)?;
    ha::test(&config).await.map_err(|e|(StatusCode::BAD_GATEWAY,e))?;
    Ok(Json(serde_json::json!({"ok":true})))
}
fn internal(e:String)->(StatusCode,String) {(StatusCode::INTERNAL_SERVER_ERROR,e)}

pub async fn devices(State(state):State<AppState>) -> Vec<ha::Device> {
    let cfg=match ha::load(&ha::instance_path()) {Ok(c)=>c,Err(_)=>return vec![]};
    let name=crate::app::ha_instance_name();
    let snapshot=state.latest_snapshot.read().ok().map(|s| powerwatch_core::json_snapshot::build_json_snapshot(&s));
    let mut metrics=vec![];
    if let Some(ref snapshot)=snapshot {
        if let Some(ref total)=snapshot.total {metrics.push(ha::Metric::power("power_total","Total power",total.watts));}
        if cfg.publish_components {
            for sensor in &snapshot.sensors {
                if let Some(ref reading)=sensor.reading {
                    metrics.push(ha::Metric::power(format!("power_{}",ha::component_key(&sensor.name)),
                        sensor.display_name.clone().unwrap_or_else(||sensor.name.clone()),reading.watts));
                }
            }
        }
    }
    if cfg.publish_energy {
        if let Ok(guard)=state.storage.lock() {
            if let Some(ref db)=*guard {
                if let Ok(stats)=db.energy_stats(None,chrono::Utc::now().timestamp()) {
                    if stats.first_seen_epoch.is_some() {
                        metrics.push(ha::Metric::energy("energy_total","Lifetime energy",stats.energy_kwh));
                    }
                }
            }
        }
    }
    if cfg.publish_costs {
        if let Ok(guard)=state.storage.lock() {
            if let Some(ref db)=*guard {
                let conn=db.tariff_connection();
                if let Ok(tariffs)=powerwatch_core::tariffs_fr::load(conn) {
                    if tariffs.enabled {
                        let selection=vec![("local".to_string(),"local".to_string(),true)];
                        if let Ok(cost)=powerwatch_core::tariffs_fr::lifetime_cost(conn,&tariffs,&selection,chrono::Utc::now().timestamp()) {
                            metrics.extend(ha::cost_metrics(&cost,tariffs.include_subscription));
                        }
                    }
                }
            }
        }
    }
    vec![ha::Device{key:"host".into(),label:name,available:snapshot.and_then(|s|s.total).is_some(),metrics}]
}
