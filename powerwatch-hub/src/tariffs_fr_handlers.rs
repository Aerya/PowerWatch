// French electricity tariff beta, persisted in the Hub SQLite database.
#[derive(Deserialize)]
struct HubTariffCostParams {from:Option<DateTime<Utc>>,to:Option<DateTime<Utc>>}
#[derive(Deserialize)]
struct HubTariffSyncRequest {kind:String,contract_id:Option<String>}
#[derive(Serialize)]
struct HubTariffSyncResponse {message:String,config:powerwatch_core::tariffs_fr::TariffConfig}
async fn hub_tariff_get(State(state):State<AppState>)
 ->Result<Json<powerwatch_core::tariffs_fr::TariffConfig>,(StatusCode,String)> {
    let db=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"storage poisoned".into()))?;
    powerwatch_core::tariffs_fr::load(&db.conn).map(Json).map_err(|e|(StatusCode::INTERNAL_SERVER_ERROR,e))
}
async fn hub_tariff_put(State(state):State<AppState>,Json(config):Json<powerwatch_core::tariffs_fr::TariffConfig>)
 ->Result<Json<powerwatch_core::tariffs_fr::TariffConfig>,(StatusCode,String)> {
    let db=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"storage poisoned".into()))?;
    powerwatch_core::tariffs_fr::save(&db.conn,&config).map_err(|e|(StatusCode::BAD_REQUEST,e))?;
    Ok(Json(config))
}
async fn hub_tariff_sync(State(state):State<AppState>,Json(req):Json<HubTariffSyncRequest>)
 ->Result<Json<HubTariffSyncResponse>,(StatusCode,String)> {
    use powerwatch_core::tariffs_fr as tariffs;
    let (url,contract_id)=if req.kind=="tempo" {(tariffs::TEMPO_HISTORY,None)}
    else if req.kind=="cre" {
        let id=req.contract_id.ok_or((StatusCode::BAD_REQUEST,"missing contract_id".into()))?;
        let db=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"storage poisoned".into()))?;
        let conf=tariffs::load(&db.conn).map_err(|e|(StatusCode::INTERNAL_SERVER_ERROR,e))?;
        let contract=conf.contracts.iter().find(|c|c.id==id).ok_or((StatusCode::BAD_REQUEST,"unknown contract".into()))?;
        if !contract.supplier.eq_ignore_ascii_case("edf")||!contract.offer.to_ascii_lowercase().contains("bleu") {return Err((StatusCode::BAD_REQUEST,"CRE data only matches EDF Tarif Bleu".into()));}
        let url=match contract.option.as_str(){"base"=>tariffs::CRE_BASE,"hphc"=>tariffs::CRE_HPHC,"tempo"=>tariffs::CRE_TEMPO,_=>return Err((StatusCode::BAD_REQUEST,"unsupported CRE option".into()))};
        (url,Some(id))
    }else{return Err((StatusCode::BAD_REQUEST,"unsupported source".into()))};
    let client=Client::builder().timeout(Duration::from_secs(20)).build().map_err(|e|(StatusCode::BAD_GATEWAY,e.to_string()))?;
    let reply=client.get(url).send().await.map_err(|e|(StatusCode::BAD_GATEWAY,e.to_string()))?.error_for_status().map_err(|e|(StatusCode::BAD_GATEWAY,e.to_string()))?;
    if reply.content_length().is_some_and(|n|n>1_000_000){return Err((StatusCode::BAD_GATEWAY,"response too large".into()));}
    let body=reply.text().await.map_err(|e|(StatusCode::BAD_GATEWAY,e.to_string()))?;
    if body.len()>1_000_000{return Err((StatusCode::BAD_GATEWAY,"response too large".into()));}
    let db=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"storage poisoned".into()))?;
    let mut config=tariffs::load(&db.conn).map_err(|e|(StatusCode::INTERNAL_SERVER_ERROR,e))?;
    let count=match contract_id {Some(id)=>tariffs::merge_cre_csv(&mut config,&id,&body),None=>tariffs::merge_tempo_history(&mut config,&body)}
        .map_err(|e|(StatusCode::BAD_GATEWAY,e))?;
    tariffs::save(&db.conn,&config).map_err(|e|(StatusCode::BAD_REQUEST,e))?;
    Ok(Json(HubTariffSyncResponse{message:format!("{count} entries updated; manual edits retained"),config}))
}
async fn hub_tariff_cost(State(state):State<AppState>,Query(params):Query<HubTariffCostParams>)
 ->Result<Json<powerwatch_core::tariffs_fr::CostReport>,(StatusCode,String)> {
    let now=Utc::now().timestamp();
    let from=params.from.map_or(0,|v|v.timestamp());
    let to=params.to.map_or(now,|v|v.timestamp());
    if from<0||from>=to||to>now+60 {return Err((StatusCode::BAD_REQUEST,"invalid cost period".into()));}
    let nodes=state.config.read().await.nodes.clone();
    let db=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"storage poisoned".into()))?;
    let config=powerwatch_core::tariffs_fr::load(&db.conn).map_err(|e|(StatusCode::INTERNAL_SERVER_ERROR,e))?;
    let mut assignments=Vec::new();
    for node in nodes {
        let site=config.sites.iter().find(|site|site.nodes.iter().any(|n|n==&node.id))
            .map(|site|site.id.clone()).unwrap_or_else(||"__unassigned__".to_string());
        assignments.push((node.id,site,node.enabled&&node.include_in_total));
    }
    let answer=powerwatch_core::tariffs_fr::cost_report(&db.conn,&config,&assignments,from,to)
       .map_err(|e|(StatusCode::INTERNAL_SERVER_ERROR,e))?;
    Ok(Json(answer))
}

async fn hub_tariff_refresh_background(state:AppState) {
    tokio::time::sleep(Duration::from_secs(120)).await;
    loop {
        let conf={let Ok(db)=state.storage.lock()else{return};powerwatch_core::tariffs_fr::load(&db.conn).ok()};
        if let Some(conf)=conf.filter(|c|c.enabled) {
            use powerwatch_core::tariffs_fr as tariffs;
            if let Ok(client)=Client::builder().timeout(Duration::from_secs(20)).build(){
                let mut syncs=Vec::new();
                for c in &conf.contracts {
                    if !c.supplier.eq_ignore_ascii_case("edf")||!c.offer.to_ascii_lowercase().contains("bleu"){continue;}
                    let url=match c.option.as_str(){"base"=>tariffs::CRE_BASE,"hphc"=>tariffs::CRE_HPHC,"tempo"=>tariffs::CRE_TEMPO,_=>continue};
                    syncs.push((Some(c.id.clone()),url));
                }
                syncs.push((None,tariffs::TEMPO_HISTORY));syncs.push((None,tariffs::TEMPO_TODAY));
                for (id,url) in syncs {
                    let Ok(reply)=client.get(url).send().await else{continue};
                    let Ok(reply)=reply.error_for_status()else{continue};
                    if reply.content_length().is_some_and(|n|n>1_000_000){continue;}
                    let Ok(body)=reply.text().await else{continue};if body.len()>1_000_000{continue;}
                    let Ok(db)=state.storage.lock()else{return};
                    let Ok(mut current)=tariffs::load(&db.conn)else{continue};
                    let updated=match id{Some(ref v)=>tariffs::merge_cre_csv(&mut current,v,&body),
                        None if url==tariffs::TEMPO_HISTORY=>tariffs::merge_tempo_history(&mut current,&body),
                        None=>tariffs::merge_tempo_today(&mut current,&body)};
                    if updated.is_ok(){let _=tariffs::save(&db.conn,&current);}
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(86400)).await;
    }
}
