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

#[derive(Deserialize)]
struct TariffPropagationRequest {
    source_id: String,
    target_ids: Vec<String>,
    include_hub: bool,
}

#[derive(Serialize)]
struct TariffPropagationResult {
    updated: Vec<String>,
    failed: Vec<String>,
    hub_updated: bool,
}

async fn remote_tariffs(client:&Client,node:&NodeConfig,token:Option<&str>)
 ->Result<powerwatch_core::tariffs_fr::TariffConfig,String> {
    let mut request=client.get(format!("{}/api/tariffs-fr",node.url));
    if let Some(token)=token.filter(|value|!value.trim().is_empty()) {request=request.bearer_auth(token);}
    let mut response=request.timeout(Duration::from_secs(8)).send().await.map_err(|_|"connexion impossible".to_string())?;
    if !response.status().is_success(){return Err(format!("HTTP {}",response.status()));}
    if response.content_length().is_some_and(|length|length>1_000_000){return Err("configuration trop grande".into());}
    let mut body=Vec::new();
    while let Some(chunk)=response.chunk().await.map_err(|_|"lecture impossible".to_string())? {
        if body.len()+chunk.len()>1_000_000{return Err("configuration trop grande".into());}
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_|"configuration invalide".to_string())
}

async fn hub_tariff_propagate(State(state):State<AppState>,Json(request):Json<TariffPropagationRequest>)
 ->Result<Json<TariffPropagationResult>,(StatusCode,String)> {
    use powerwatch_core::tariffs_fr::{TariffConfig,TariffSite};
    let nodes=state.config.read().await.nodes.clone();
    let source=nodes.iter().find(|node|node.id==request.source_id)
        .ok_or((StatusCode::BAD_REQUEST,"source inconnue".into()))?;
    if request.target_ids.len()>nodes.len() || !request.include_hub && request.target_ids.is_empty() {
        return Err((StatusCode::BAD_REQUEST,"aucune destination".into()));
    }
    let mut ids=HashSet::new();
    for id in &request.target_ids {
        if !ids.insert(id.as_str()) || id==&source.id || !nodes.iter().any(|node|&node.id==id) {
            return Err((StatusCode::BAD_REQUEST,"destination invalide".into()));
        }
    }
    let source_config=remote_tariffs(&state.client,source,source.api_token.as_deref())
        .await.map_err(|error|(StatusCode::BAD_GATEWAY,format!("source : {error}")))?;
    if !source_config.enabled || source_config.contracts.is_empty() {
        return Err((StatusCode::BAD_REQUEST,"la source n'a aucun tarif actif".into()));
    }
    let site_ids:HashSet<_>=source_config.contracts.iter().map(|contract|contract.site_id.as_str()).collect();
    if site_ids.len()!=1 {
        return Err((StatusCode::BAD_REQUEST,"la source doit avoir un seul site tarifaire".into()));
    }
    let mut updated=Vec::new();
    let mut failed=Vec::new();
    for id in &request.target_ids {
        let node=nodes.iter().find(|node|&node.id==id).expect("validated node");
        let token=node.tariff_token.as_deref();
        let result=async {
            let current=remote_tariffs(&state.client,node,token).await?;
            let mut copy:TariffConfig=source_config.clone();
            copy.revision=current.revision;
            copy.sites=vec![TariffSite{id:"local".into(),name:"Local".into(),nodes:vec![]}];
            for contract in &mut copy.contracts {contract.site_id="local".into();}
            let mut put=state.client.put(format!("{}/api/tariffs-fr",node.url)).json(&copy);
            if let Some(token)=token.filter(|value|!value.trim().is_empty()){put=put.bearer_auth(token);}
            let response=put.timeout(Duration::from_secs(8)).send().await.map_err(|_|"connexion impossible".to_string())?;
            if !response.status().is_success(){return Err(format!("HTTP {}",response.status()));}
            Ok::<(),String>(())
        }.await;
        match result {Ok(())=>updated.push(id.clone()),Err(error)=>failed.push(format!("{id} : {error}"))}
    }
    let mut hub_updated=false;
    if request.include_hub {
        let db=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"stockage indisponible".into()))?;
        let mut current=powerwatch_core::tariffs_fr::load(&db.conn)
            .map_err(|error|(StatusCode::INTERNAL_SERVER_ERROR,error))?;
        let mut shared_nodes=updated.clone();
        shared_nodes.push(source.id.clone());
        for site in &mut current.sites {site.nodes.retain(|id|!shared_nodes.contains(id));}
        let empty_shared:HashSet<String>=current.sites.iter()
            .filter(|site|site.id.starts_with("hub_shared_")&&site.nodes.is_empty())
            .map(|site|site.id.clone()).collect();
        current.sites.retain(|site|!empty_shared.contains(&site.id));
        current.contracts.retain(|contract|!empty_shared.contains(&contract.site_id));
        let mut site_index=0;
        let shared=loop {
            let id=format!("hub_shared_{site_index}");
            if !current.sites.iter().any(|site|site.id==id){break id;}
            site_index+=1;
        };
        current.sites.push(TariffSite{id:shared.clone(),name:"Tarifs partagés".into(),nodes:shared_nodes});
        let mut used:HashSet<String>=current.contracts.iter().map(|contract|contract.id.clone()).collect();
        for (index,mut contract) in source_config.contracts.clone().into_iter().enumerate() {
            let mut id=format!("{shared}_{index}");
            while used.contains(&id){id.push('x');}
            used.insert(id.clone());
            contract.id=id;
            contract.site_id=shared.clone();
            current.contracts.push(contract);
        }
        current.enabled=true;
        current.country=source_config.country.clone();
        current.include_subscription=source_config.include_subscription;
        current.estimate_unknown_history=source_config.estimate_unknown_history;
        current.tempo_days=source_config.tempo_days.clone();
        match powerwatch_core::tariffs_fr::save(&db.conn,&current) {
            Ok(_)=>hub_updated=true,
            Err(error)=>failed.push(format!("Hub : {error}")),
        }
    }
    Ok(Json(TariffPropagationResult{updated,failed,hub_updated}))
}
async fn hub_tariff_put(State(state):State<AppState>,Json(config):Json<powerwatch_core::tariffs_fr::TariffConfig>)
 ->Result<Json<powerwatch_core::tariffs_fr::TariffConfig>,(StatusCode,String)> {
    let db=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"storage poisoned".into()))?;
    let config=powerwatch_core::tariffs_fr::save(&db.conn,&config).map_err(|e|(StatusCode::CONFLICT,e))?;
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
    let config=tariffs::save(&db.conn,&config).map_err(|e|(StatusCode::CONFLICT,e))?;
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

#[derive(Deserialize)]
struct HubCostOverviewParams {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    // Empty selection is meaningful. Omitted = normal Hub total.
    nodes: Option<String>,
}
async fn hub_tariff_overview(
    State(state): State<AppState>, Query(params): Query<HubCostOverviewParams>,
) -> Result<Json<powerwatch_core::tariffs_fr::CostOverview>, (StatusCode,String)> {
    let now=Utc::now().timestamp();
    let to=params.to.map_or(now,|v|v.timestamp());
    let from=params.from.map(|v|v.timestamp());
    let nodes=state.config.read().await.nodes.clone();
    // Explicit picks include disabled/excluded nodes, matching the energy picker.
    let chosen: Option<HashSet<&str>> = params.nodes.as_deref().map(|v|v.split(',').filter(|id|!id.is_empty()).collect());
    if params.nodes.as_ref().is_some_and(|v|v.len()>8192) {
        return Err((StatusCode::BAD_REQUEST,"too many selected nodes".into()));
    }
    let db=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"storage poisoned".into()))?;
    let config=powerwatch_core::tariffs_fr::load(&db.conn)
        .map_err(|e|(StatusCode::INTERNAL_SERVER_ERROR,e))?;
    let assignments: Vec<_> = nodes.into_iter().filter(|node|chosen.as_ref().map_or(
        node.enabled&&node.include_in_total,|selected|selected.contains(node.id.as_str())
    )).map(|node| {
        let site=config.sites.iter().find(|s|s.nodes.contains(&node.id))
            .map(|s|s.id.clone()).unwrap_or_else(||"__unassigned__".into());
        (node.id,site,true)
    }).collect();
    powerwatch_core::tariffs_fr::cost_overview(&db.conn,&config,&assignments,now,from,to)
        .map(Json).map_err(|e|(StatusCode::BAD_REQUEST,e))
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
