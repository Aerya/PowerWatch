// French tariff beta: authenticated, server-side configuration and pricing.
#[derive(Deserialize)]
struct FranceCostParams {
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
}
#[derive(Deserialize)]
struct FranceSyncRequest { kind:String, contract_id:Option<String> }
#[derive(Serialize)]
struct FranceSyncResponse { message:String, config:powerwatch_core::tariffs_fr::TariffConfig }
async fn tariff_fr_get(
    State(state):State<AppState>,
)->Result<Json<powerwatch_core::tariffs_fr::TariffConfig>,(StatusCode,String)> {
    let locked=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"storage poisoned".into()))?;
    let db=locked.as_ref().ok_or((StatusCode::SERVICE_UNAVAILABLE,"history storage unavailable".into()))?;
    powerwatch_core::tariffs_fr::load(db.tariff_connection()).map(Json)
        .map_err(|e|(StatusCode::INTERNAL_SERVER_ERROR,e))
}
async fn tariff_fr_put(
    State(state):State<AppState>,Json(config):Json<powerwatch_core::tariffs_fr::TariffConfig>,
)->Result<Json<powerwatch_core::tariffs_fr::TariffConfig>,(StatusCode,String)> {
    let locked=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"storage poisoned".into()))?;
    let db=locked.as_ref().ok_or((StatusCode::SERVICE_UNAVAILABLE,"history storage unavailable".into()))?;
    powerwatch_core::tariffs_fr::save(db.tariff_connection(),&config)
        .map_err(|e|(StatusCode::BAD_REQUEST,e))?;
    Ok(Json(config))
}
async fn tariff_fr_sync(
    State(state):State<AppState>,Json(req):Json<FranceSyncRequest>,
)->Result<Json<FranceSyncResponse>,(StatusCode,String)> {
    use powerwatch_core::tariffs_fr as tariffs;
    let (url,contract_id)=if req.kind=="tempo" {
        (tariffs::TEMPO_HISTORY,None)
    } else if req.kind=="cre" {
        let id=req.contract_id.ok_or((StatusCode::BAD_REQUEST,"missing contract_id".into()))?;
        let locked=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"storage poisoned".into()))?;
        let db=locked.as_ref().ok_or((StatusCode::SERVICE_UNAVAILABLE,"history storage unavailable".into()))?;
        let conf=tariffs::load(db.tariff_connection()).map_err(|e|(StatusCode::INTERNAL_SERVER_ERROR,e))?;
        let contract=conf.contracts.iter().find(|c|c.id==id).ok_or((StatusCode::BAD_REQUEST,"unknown contract".into()))?;
        let url=match contract.option.as_str(){"base"=>tariffs::CRE_BASE,"hphc"=>tariffs::CRE_HPHC,"tempo"=>tariffs::CRE_TEMPO,_=>return Err((StatusCode::BAD_REQUEST,"unsupported CRE option".into()))};
        if !contract.supplier.eq_ignore_ascii_case("edf")||!contract.offer.to_ascii_lowercase().contains("bleu") {
            return Err((StatusCode::BAD_REQUEST,"CRE data only matches EDF Tarif Bleu".into()));
        }
        (url,Some(id))
    }else{return Err((StatusCode::BAD_REQUEST,"unsupported sync source".into()))};
    let client=reqwest::Client::builder().timeout(std::time::Duration::from_secs(20))
        .build().map_err(|e|(StatusCode::BAD_GATEWAY,e.to_string()))?;
    let reply=client.get(url).send().await.map_err(|e|(StatusCode::BAD_GATEWAY,e.to_string()))?
        .error_for_status().map_err(|e|(StatusCode::BAD_GATEWAY,e.to_string()))?;
    if reply.content_length().is_some_and(|n|n>1_000_000){return Err((StatusCode::BAD_GATEWAY,"source response too large".into()));}
    let content=reply.text().await.map_err(|e|(StatusCode::BAD_GATEWAY,e.to_string()))?;
    if content.len()>1_000_000{return Err((StatusCode::BAD_GATEWAY,"source response too large".into()));}
    let locked=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"storage poisoned".into()))?;
    let db=locked.as_ref().ok_or((StatusCode::SERVICE_UNAVAILABLE,"history storage unavailable".into()))?;
    let mut conf=tariffs::load(db.tariff_connection()).map_err(|e|(StatusCode::INTERNAL_SERVER_ERROR,e))?;
    let count=if let Some(id)=contract_id {tariffs::merge_cre_csv(&mut conf,&id,&content)}
      else {tariffs::merge_tempo_history(&mut conf,&content)}
      .map_err(|e|(StatusCode::BAD_GATEWAY,e))?;
    tariffs::save(db.tariff_connection(),&conf).map_err(|e|(StatusCode::BAD_REQUEST,e))?;
    Ok(Json(FranceSyncResponse{message:format!("{count} source entries updated; manual overrides preserved"),config:conf}))
}
async fn tariff_fr_cost(
    State(state):State<AppState>,Query(params):Query<FranceCostParams>,
)->Result<Json<powerwatch_core::tariffs_fr::CostReport>,(StatusCode,String)> {
    let now=chrono::Utc::now().timestamp();
    let from=params.from.map_or(0,|v|v.timestamp());
    let to=params.to.map_or(now,|v|v.timestamp());
    if to>now+60 || from<0 || from>=to {return Err((StatusCode::BAD_REQUEST,"invalid cost period".into()));}
    let locked=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"storage poisoned".into()))?;
    let db=locked.as_ref().ok_or((StatusCode::SERVICE_UNAVAILABLE,"history storage unavailable".into()))?;
    let conf=powerwatch_core::tariffs_fr::load(db.tariff_connection()).map_err(|e|(StatusCode::INTERNAL_SERVER_ERROR,e))?;
    let data=powerwatch_core::tariffs_fr::cost_report(
        db.tariff_connection(),&conf,&[("local".into(),"local".into(),true)],from,to
    ).map_err(|e|(StatusCode::INTERNAL_SERVER_ERROR,e))?;
    Ok(Json(data))
}

pub async fn tariff_fr_refresh_background(state:AppState) {
    // Initial refresh, then daily. Failed sources leave existing/manual tariffs unchanged.
    tokio::time::sleep(std::time::Duration::from_secs(120)).await;
    loop {
        let conf={
            let Ok(storage)=state.storage.lock() else {return};
            storage.as_ref().and_then(|db|powerwatch_core::tariffs_fr::load(db.tariff_connection()).ok())
        };
        if let Some(conf)=conf.filter(|c|c.enabled) {
            use powerwatch_core::tariffs_fr as tariffs;
            let client=reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build();
            if let Ok(client)=client {
                let mut syncs=Vec::new();
                for c in &conf.contracts {
                    if !c.supplier.eq_ignore_ascii_case("edf") || !c.offer.to_ascii_lowercase().contains("bleu") {continue;}
                    let url=match c.option.as_str(){"base"=>tariffs::CRE_BASE,"hphc"=>tariffs::CRE_HPHC,"tempo"=>tariffs::CRE_TEMPO,_=>continue};
                    syncs.push((Some(c.id.clone()),url));
                }
                syncs.push((None,tariffs::TEMPO_HISTORY));
                syncs.push((None,tariffs::TEMPO_TODAY));
                for (id,url) in syncs {
                    let Ok(reply)=client.get(url).send().await else {continue};
                    let Ok(reply)=reply.error_for_status() else {continue};
                    if reply.content_length().is_some_and(|n|n>1_000_000){continue;}
                    let Ok(body)=reply.text().await else {continue};
                    if body.len()>1_000_000{continue;}
                    let Ok(storage)=state.storage.lock() else {return};
                    let Some(db)=storage.as_ref() else {continue};
                    let Ok(mut current)=tariffs::load(db.tariff_connection()) else {continue};
                    let ok=match id {Some(ref value)=>tariffs::merge_cre_csv(&mut current,value,&body),
                        None if url==tariffs::TEMPO_HISTORY=>tariffs::merge_tempo_history(&mut current,&body),
                        None=>tariffs::merge_tempo_today(&mut current,&body)};
                    if ok.is_ok(){let _=tariffs::save(db.tariff_connection(),&current);}
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(86400)).await;
    }
}
