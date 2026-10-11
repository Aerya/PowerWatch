// Discovery and import from PUBLIC French retail tariff grids. No secrets are transmitted.
#[derive(Deserialize)] struct MarketSupplierQuery { supplier: String }
#[derive(Deserialize)] struct MarketImportRequest {
    supplier: String, offer: String, option: String, kva: u32, contract_id: String, site_id: String,
    replace_contract_id: Option<String>,
    offpeak: Option<Vec<powerwatch_core::tariffs_fr::OffpeakWindow>>,
}
#[derive(Serialize)] struct MarketImportResponse { changed: bool, config:powerwatch_core::tariffs_fr::TariffConfig }
async fn market_public_fetch(url:&str)->Result<String,(StatusCode,String)> {
    let client=reqwest::Client::builder().timeout(std::time::Duration::from_secs(20))
      .build().map_err(|e|(StatusCode::BAD_GATEWAY,e.to_string()))?;
    let reply=client.get(url).send().await.map_err(|e|(StatusCode::BAD_GATEWAY,e.to_string()))?
      .error_for_status().map_err(|e|(StatusCode::BAD_GATEWAY,e.to_string()))?;
    if reply.content_length().is_some_and(|n|n>4_000_000){return Err((StatusCode::BAD_GATEWAY,"market response too large".into()));}
    let bytes=reply.bytes().await.map_err(|e|(StatusCode::BAD_GATEWAY,e.to_string()))?;
    if bytes.len()>4_000_000 {return Err((StatusCode::BAD_GATEWAY,"market response too large".into()));}
    String::from_utf8(bytes.to_vec()).map_err(|e|(StatusCode::BAD_GATEWAY,e.to_string()))
}
async fn market_suppliers()->Result<Json<Vec<powerwatch_core::market_fr::Supplier>>,(StatusCode,String)>{
    let html=market_public_fetch(powerwatch_core::market_fr::DIRECTORY).await?;
    let suppliers=powerwatch_core::market_fr::parse_suppliers(&html);
    if suppliers.is_empty(){return Err((StatusCode::BAD_GATEWAY,"no supplier list found; source may have changed".into()));}
    Ok(Json(suppliers))
}
async fn market_offers(Query(params):Query<MarketSupplierQuery>)->Result<Json<Vec<powerwatch_core::market_fr::MarketOffer>>,(StatusCode,String)>{
    let url=powerwatch_core::market_fr::tariff_url(&params.supplier).map_err(|e|(StatusCode::BAD_REQUEST,e))?;
    let html=market_public_fetch(&url).await?;
    let offers=powerwatch_core::market_fr::parse_offers(&params.supplier,&html).map_err(|e|(StatusCode::BAD_GATEWAY,e))?;
    Ok(Json(offers))
}
async fn market_import(State(state):State<AppState>,Json(req):Json<MarketImportRequest>)
 ->Result<Json<MarketImportResponse>,(StatusCode,String)> {
    let url=powerwatch_core::market_fr::tariff_url(&req.supplier).map_err(|e|(StatusCode::BAD_REQUEST,e))?;
    let html=market_public_fetch(&url).await?;
    let offers=powerwatch_core::market_fr::parse_offers(&req.supplier,&html).map_err(|e|(StatusCode::BAD_GATEWAY,e))?;
    let offer=offers.iter().find(|o|o.offer==req.offer && o.option==req.option && o.kva==req.kva)
       .ok_or((StatusCode::BAD_REQUEST,"offer no longer available at source".into()))?;
    let guard=state.storage.lock().map_err(|_|(StatusCode::INTERNAL_SERVER_ERROR,"storage poisoned".into()))?;
    let db=guard.as_ref().ok_or((StatusCode::SERVICE_UNAVAILABLE,"SQLite storage unavailable".into()))?;
    let mut config=powerwatch_core::tariffs_fr::load(db.tariff_connection()).map_err(|e|(StatusCode::INTERNAL_SERVER_ERROR,e))?;
    let changed=powerwatch_core::market_fr::import_offer(&mut config,&req.contract_id,&req.site_id,offer,
        powerwatch_core::market_fr::today_fr(),req.replace_contract_id.as_deref())
       .map_err(|e|(StatusCode::BAD_REQUEST,e))?;
    if let Some(windows)=req.offpeak {
        if offer.option!="hphc" {return Err((StatusCode::BAD_REQUEST,"off-peak windows only apply to HP/HC".into()));}
        if let Some(contract)=config.contracts.iter_mut().find(|c|c.id==req.contract_id){contract.offpeak=windows;}
    }
    let config=powerwatch_core::tariffs_fr::save(db.tariff_connection(),&config).map_err(|e|(StatusCode::CONFLICT,e))?;
    Ok(Json(MarketImportResponse{changed,config}))
}

pub async fn market_fr_refresh_background(state:AppState) {
    // Fetch only suppliers explicitly bound by a previous user-driven market import.
    tokio::time::sleep(std::time::Duration::from_secs(300)).await;
    loop {
        let contracts={
            let Ok(storage)=state.storage.lock()else{return};
            storage.as_ref().and_then(|db|powerwatch_core::tariffs_fr::load(db.tariff_connection()).ok())
        };
        if let Some(config)=contracts.filter(|cfg|cfg.enabled) {
            use powerwatch_core::market_fr as market;
            let mut slugs=std::collections::BTreeSet::new();
            for c in &config.contracts {
                if let Some(rev)=c.revisions.iter().max_by_key(|r|r.from) {
                    if rev.source!=market::MARKET_SOURCE || rev.manual {continue;}
                    let Some(url)=rev.source_url.as_deref()else{continue};
                    let Some(tail)=url.strip_prefix("https://www.kelwatt.fr/fournisseurs/")else{continue};
                    let slug=tail.split('/').next().unwrap_or("");
                    if market::valid_slug(slug){slugs.insert(slug.to_string());}
                }
            }
            for slug in slugs.into_iter().take(50) {
                let Ok(url)=market::tariff_url(&slug)else{continue};
                let Ok(html)=market_public_fetch(&url).await else{continue};
                let Ok(offers)=market::parse_offers(&slug,&html)else{continue};
                let Ok(storage)=state.storage.lock()else{return};
                let Some(db)=storage.as_ref() else{continue};
                let Ok(mut current)=powerwatch_core::tariffs_fr::load(db.tariff_connection())else{continue};
                let targets=current.contracts.iter().filter(|c|c.revisions.iter().max_by_key(|r|r.from)
                      .is_some_and(|r|r.source==market::MARKET_SOURCE && r.source_url.as_deref()==Some(url.as_str()) && !r.manual))
                   .map(|c|(c.id.clone(),c.supplier.clone(),c.offer.clone(),c.option.clone(),c.kva)).collect::<Vec<_>>();
                let mut changed=false;
                for (id,supplier,offer_name,option,kva) in targets {
                    if let Some(offer)=offers.iter().find(|o|o.supplier==supplier&&o.offer==offer_name&&o.option==option&&o.kva==kva) {
                        if let Ok(true)=market::apply_offer(&mut current,&id,offer,market::today_fr()){changed=true;}
                    }
                }
                if changed {let _=powerwatch_core::tariffs_fr::save(db.tariff_connection(),&current);}
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(86400)).await;
    }
}
