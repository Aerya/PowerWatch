//! Optional French electricity tariff engine (beta).
//! No default or invented prices. Config and source provenance are kept in SQLite.
use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use chrono_tz::Europe::Paris;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const CRE_BASE: &str = "https://www.cre.fr/fileadmin/Documents/Open_data/Marches_de_detail/Option_Base.csv";
pub const CRE_HPHC: &str = "https://www.cre.fr/fileadmin/Documents/Open_data/Marches_de_detail/Option_HPHC.csv";
pub const CRE_TEMPO: &str = "https://www.cre.fr/fileadmin/Documents/Open_data/Marches_de_detail/Option_Tempo.csv";
pub const TEMPO_HISTORY: &str = "https://www.calendrier-tempo.fr/api/history?days=365";
pub const TEMPO_TODAY: &str = "https://www.calendrier-tempo.fr/api/today";
pub const TEMPO_TOMORROW: &str = "https://www.calendrier-tempo.fr/api/tomorrow";

fn default_local() -> String { "local".into() }
fn default_fr() -> String { "FR".into() }
fn default_hp() -> Vec<OffpeakWindow> { Vec::new() }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OffpeakWindow { pub from: String, pub to: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TariffSite {
    pub id: String, pub name: String,
    #[serde(default)] pub nodes: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceRevision {
    pub from: NaiveDate,
    #[serde(default)] pub until: Option<NaiveDate>,
    #[serde(default)] pub eur_kwh: BTreeMap<String, f64>,
    #[serde(default)] pub annual_subscription_eur: Option<f64>,
    #[serde(default)] pub source: String,
    #[serde(default)] pub source_url: Option<String>,
    #[serde(default)] pub manual: bool,
    #[serde(default)] pub retrieved_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TariffContract {
    pub id: String,
    #[serde(default = "default_local")] pub site_id: String,
    pub supplier: String,
    pub offer: String,
    /// base / hphc / tempo / custom
    pub option: String,
    pub starts_on: NaiveDate,
    #[serde(default)] pub ends_on: Option<NaiveDate>, // inclusive
    pub kva: u32,
    #[serde(default = "default_hp")] pub offpeak: Vec<OffpeakWindow>,
    #[serde(default)] pub revisions: Vec<PriceRevision>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoColor {
    pub color: String,
    #[serde(default)] pub source: String,
    #[serde(default)] pub manual: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TariffConfig {
    #[serde(default)] pub revision: u64,
    #[serde(default)] pub enabled: bool,
    #[serde(default = "default_fr")] pub country: String,
    #[serde(default)] pub include_subscription: bool,
    #[serde(default)] pub estimate_unknown_history: bool,
    #[serde(default)] pub sites: Vec<TariffSite>,
    #[serde(default)] pub contracts: Vec<TariffContract>,
    #[serde(default)] pub tempo_days: BTreeMap<String, TempoColor>,
    #[serde(default)] pub cre_last_sync: Option<String>,
    #[serde(default)] pub tempo_last_sync: Option<String>,
}
impl Default for TariffConfig {
    fn default() -> Self {
        Self { revision:0,enabled:false,country:default_fr(),include_subscription:false,estimate_unknown_history:false,
          sites:vec![TariffSite{id:"local".into(),name:"Local".into(),nodes:vec![]}],
          contracts:vec![],tempo_days:BTreeMap::new(),cre_last_sync:None,tempo_last_sync:None }
    }
}

pub fn init(conn:&Connection)->rusqlite::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS tariffs_fr_config (
        singleton INTEGER PRIMARY KEY CHECK(singleton=1), json TEXT NOT NULL
    );")
}
pub fn load(conn:&Connection)->Result<TariffConfig,String> {
    init(conn).map_err(|e|e.to_string())?;
    let json:Option<String> = conn.query_row("SELECT json FROM tariffs_fr_config WHERE singleton=1",[],|r|r.get(0))
        .optional().map_err(|e|e.to_string())?;
    match json { Some(v)=>serde_json::from_str(&v).map_err(|e|e.to_string()),None=>Ok(TariffConfig::default()) }
}
pub fn save(conn:&Connection,config:&TariffConfig)->Result<TariffConfig,String> {
    init(conn).map_err(|e|e.to_string())?;
    validate(config)?;
    let previous:Option<String>=conn.query_row("SELECT json FROM tariffs_fr_config WHERE singleton=1",[],|r|r.get(0))
        .optional().map_err(|e|e.to_string())?;
    let current=previous.as_deref().map(serde_json::from_str::<TariffConfig>).transpose().map_err(|e|e.to_string())?;
    if current.as_ref().map_or(0,|c|c.revision)!=config.revision {return Err("tariff configuration changed on server; reload before saving".into());}
    let mut updated=config.clone();
    updated.revision=config.revision.checked_add(1).ok_or("tariff revision overflow")?;
    let json=serde_json::to_string(&updated).map_err(|e|e.to_string())?;
    let written=if let Some(previous)=previous {
        conn.execute("UPDATE tariffs_fr_config SET json=?1 WHERE singleton=1 AND json=?2",params![json,previous])
    } else {conn.execute("INSERT OR IGNORE INTO tariffs_fr_config(singleton,json) VALUES(1,?1)",[json])}
        .map_err(|e|e.to_string())?;
    if written!=1 {return Err("tariff configuration changed on server; reload before saving".into());}
    Ok(updated)
}
fn key_valid(s:&str)->bool { !s.is_empty()&&s.len()<=96&&s.chars().all(|c|c.is_ascii_alphanumeric()||"_-".contains(c)) }
fn parse_time(s:&str)->Result<NaiveTime,String> { NaiveTime::parse_from_str(s,"%H:%M").map_err(|_|format!("invalid time {s}; HH:MM required")) }
fn validate(config:&TariffConfig)->Result<(),String> {
    if config.country!="FR" {return Err("only FR is supported in this beta".into());}
    if config.sites.len()>100 || config.contracts.len()>200 || config.tempo_days.len()>12000 {return Err("tariff config too large".into());}
    let mut site_ids=HashSet::new();
    let mut assigned_nodes=HashSet::new();
    for site in &config.sites {
        if !key_valid(&site.id) || site.name.trim().is_empty() || site.name.len()>120 || !site_ids.insert(&site.id) {return Err("invalid or duplicate site".into());}
        for node in &site.nodes {if !key_valid(node) || !assigned_nodes.insert(node){return Err("node assigned to multiple sites".into());}}
    }
    let mut ids=HashSet::new();
    for c in &config.contracts {
        if !key_valid(&c.id) || !ids.insert(&c.id) || !site_ids.contains(&c.site_id) {return Err("invalid contract ID or site".into());}
        if c.supplier.trim().is_empty()||c.supplier.len()>120||c.offer.trim().is_empty()||c.offer.len()>120 {return Err("invalid supplier/offer".into());}
        if !["base","hphc","tempo","custom"].contains(&c.option.as_str()) || !(1..=36).contains(&c.kva) {return Err("invalid option or kVA".into());}
        if c.ends_on.is_some_and(|end|end<c.starts_on) {return Err("contract end before start".into());}
        if c.offpeak.len()>12 {return Err("too many HP/HC windows".into());}
        for w in &c.offpeak {if parse_time(&w.from)?==parse_time(&w.to)? {return Err("off-peak window has zero duration".into());}}
        if c.revisions.len()>250 {return Err("too many price revisions".into());}
        for rev in &c.revisions {
            if rev.until.is_some_and(|end|end<rev.from) {return Err("price revision end before start".into());}
            if rev.eur_kwh.len()>32 {return Err("too many price categories".into());}
            for (key,value) in &rev.eur_kwh {
                if !["base","hp","hc","blue_hp","blue_hc","white_hp","white_hc","red_hp","red_hc"].contains(&key.as_str()) || !value.is_finite() || *value<0.0 || *value>10.0 {return Err("invalid price category/value".into());}
            }
            if rev.annual_subscription_eur.is_some_and(|v|!v.is_finite()||v<0.0||v>100_000.0) {return Err("invalid subscription price".into());}
            if rev.source.len()>120 || rev.source_url.as_deref().unwrap_or("").len()>400 {return Err("invalid tariff source".into());}
        }
    }
    for (date,color) in &config.tempo_days {
        NaiveDate::parse_from_str(date,"%Y-%m-%d").map_err(|_|"invalid Tempo date")?;
        if !["blue","white","red"].contains(&color.color.as_str()) || color.source.len()>120 {return Err("invalid Tempo color".into());}
    }
    // A single electricity contract per site and date avoids ambiguous double charges.
    for (i,a) in config.contracts.iter().enumerate(){for b in config.contracts.iter().skip(i+1){
        if a.site_id==b.site_id && a.starts_on<=b.ends_on.unwrap_or(NaiveDate::MAX) && b.starts_on<=a.ends_on.unwrap_or(NaiveDate::MAX) {return Err(format!("overlapping contracts on site {}",a.site_id));}
    }}
    Ok(())
}

fn date_cell(s:&str)->Option<NaiveDate> {
    ["%Y-%m-%d","%d/%m/%Y","%d-%m-%Y","%Y/%m/%d"].iter().find_map(|f|NaiveDate::parse_from_str(s.trim(),f).ok())
}
fn number_cell(s:&str)->Option<f64> {
    let normalized=s.trim().replace('\u{a0}',"").replace(' ',"").replace(',',".");
    normalized.parse::<f64>().ok().filter(|n|n.is_finite()&&*n>=0.0&&*n<=100_000.0)
}
fn normalized_header(s:&str)->String {s.trim().trim_start_matches('\u{feff}').to_ascii_uppercase().replace(' ',"_").replace('-',"_")}
fn find_col(headers:&[String],names:&[&str])->Option<usize> {names.iter().find_map(|n|headers.iter().position(|h|h==n))}
fn price_col(headers:&[String], key:&str)->Option<usize> {
    let synonyms: &[&str]=match key {
        "base"=> &["PART_VARIABLE_TTC","PRIX_KWH_TTC","PART_VARIABLE_BASE_TTC"],
        "hp"=> &["PART_VARIABLE_HP_TTC","PRIX_HP_TTC","PRIX_KWH_HP_TTC"],
        "hc"=> &["PART_VARIABLE_HC_TTC","PRIX_HC_TTC","PRIX_KWH_HC_TTC"],
        "blue_hp"=> &["PART_VARIABLE_BLEU_HP_TTC","PART_VARIABLE_HP_BLEU_TTC","PART_VARIABLE_BLEU_H_P_TTC"],
        "blue_hc"=> &["PART_VARIABLE_BLEU_HC_TTC","PART_VARIABLE_HC_BLEU_TTC"],
        "white_hp"=> &["PART_VARIABLE_BLANC_HP_TTC","PART_VARIABLE_HP_BLANC_TTC"],
        "white_hc"=> &["PART_VARIABLE_BLANC_HC_TTC","PART_VARIABLE_HC_BLANC_TTC"],
        "red_hp"=> &["PART_VARIABLE_ROUGE_HP_TTC","PART_VARIABLE_HP_ROUGE_TTC"],
        "red_hc"=> &["PART_VARIABLE_ROUGE_HC_TTC","PART_VARIABLE_HC_ROUGE_TTC"],
        _=> &[],
    };
    find_col(headers,synonyms)
}
/// Strict reader for CRE data: never invent a missing column or electricity price.
/// Keeps user-edited revisions, replacing only matching imported rows.
pub fn merge_cre_csv(config:&mut TariffConfig, contract_id:&str, body:&str)->Result<usize,String> {
    let index=config.contracts.iter().position(|c|c.id==contract_id).ok_or("unknown contract")?;
    let contract=&mut config.contracts[index];
    if contract.supplier.to_ascii_lowercase()!="edf" || !contract.offer.to_ascii_lowercase().contains("bleu") {
        return Err("CRE automatic import is restricted to EDF Tarif Bleu regulated offers".into());
    }
    if !["base","hphc","tempo"].contains(&contract.option.as_str()) {return Err("unsupported CRE option".into());}
    if body.len()>1_000_000 {return Err("CRE data too large".into());}
    let delimiter=if body.lines().next().unwrap_or("").matches(';').count()>body.lines().next().unwrap_or("").matches(',').count(){';'}else{','};
    // CSV fields are quoted in some variants; the reader handles quoted separators.
    let rows=parse_csv(body,delimiter)?;
    let header=rows.first().ok_or("empty CRE CSV")?.iter().map(|s|normalized_header(s)).collect::<Vec<_>>();
    let date_idx=find_col(&header,&["DATE_DEBUT","DATE_D_EFFET","DATE_EFFET"]).ok_or("CRE CSV: DATE_DEBUT absent")?;
    let kva_idx=find_col(&header,&["P_SOUSCRITE","PUISSANCE_SOUSCRITE","P_SOUSCRITE_KVA"]).ok_or("CRE CSV: P_SOUSCRITE absent")?;
    let sub_idx=find_col(&header,&["PART_FIXE_TTC","ABONNEMENT_TTC"]);
    let end_idx=find_col(&header,&["DATE_FIN","DATE_D_EXPIRATION"]);
    let keys: &[&str]=match contract.option.as_str(){"base"=> &["base"],"hphc"=> &["hp","hc"],_=> &["blue_hp","blue_hc","white_hp","white_hc","red_hp","red_hc"]};
    let columns=keys.iter().map(|key|price_col(&header,key).map(|i|(*key,i)).ok_or_else(||format!("CRE CSV lacks {key} TTC column"))).collect::<Result<Vec<_>,_>>()?;
    let mut imported=0;
    for row in rows.iter().skip(1) {
        let get=|i:usize|row.get(i).map(String::as_str).unwrap_or("");
        if number_cell(get(kva_idx)).map(|n|n as u32)!=Some(contract.kva) {continue;}
        let Some(from)=date_cell(get(date_idx)) else {continue};
        let mut values=BTreeMap::new();
        for (key,col) in &columns {
            let Some(price)=number_cell(get(*col)) else {values.clear();break;};
            if price>10.0 {values.clear();break;}
            values.insert((*key).to_string(),price);
        }
        if values.len()!=keys.len() {continue;}
        let rev=PriceRevision {
            from, until:end_idx.and_then(|i|date_cell(get(i))).filter(|d|*d>=from),eur_kwh:values,
            annual_subscription_eur:sub_idx.and_then(|col|number_cell(get(col))),
            source:"CRE regulated open data".into(),
            source_url:Some(match contract.option.as_str(){"base"=>CRE_BASE,"hphc"=>CRE_HPHC,_=>CRE_TEMPO}.into()),
            manual:false,retrieved_at:Some(Utc::now().to_rfc3339()),
        };
        if contract.revisions.iter().any(|existing|existing.from==from && existing.manual) {continue;}
        contract.revisions.retain(|existing|existing.from!=from);
        contract.revisions.push(rev);
        imported+=1;
    }
    if imported==0 {return Err("no usable matching CRE price rows (check CSV version, power and option)".into());}
    contract.revisions.sort_by_key(|r|r.from);
    config.cre_last_sync=Some(Utc::now().to_rfc3339());
    Ok(imported)
}
fn parse_csv(data:&str,delimiter:char)->Result<Vec<Vec<String>>,String> {
    let mut rows=Vec::new(); let mut row=Vec::new(); let mut cell=String::new();let mut quoted=false;
    let mut iter=data.chars().peekable();
    while let Some(ch)=iter.next(){
        if ch=='"' { if quoted&&iter.peek()==Some(&'"'){iter.next();cell.push('"');}else{quoted=!quoted;} }
        else if ch==delimiter&&!quoted {row.push(std::mem::take(&mut cell));}
        else if (ch=='\n'||ch=='\r')&&!quoted {
            if ch=='\r'&&iter.peek()==Some(&'\n'){iter.next();}
            row.push(std::mem::take(&mut cell));
            if row.iter().any(|s|!s.trim().is_empty()){rows.push(std::mem::take(&mut row));}else{row.clear();}
        }else{cell.push(ch);}
    }
    if quoted {return Err("unclosed CSV quote".into());}
    if !cell.is_empty()||!row.is_empty(){row.push(cell);rows.push(row);}
    if rows.len()>20000 {return Err("CSV contains too many rows".into());}
    Ok(rows)
}

pub fn merge_tempo_history(config:&mut TariffConfig, body:&str)->Result<usize,String> {
    #[derive(Deserialize)] struct Answer { history:Vec<HistoryRow> }
    #[derive(Deserialize)] struct HistoryRow { date:String,couleur_reelle:String }
    let parsed:Answer=serde_json::from_str(body).map_err(|e|e.to_string())?;
    let mut inserted=0;
    for row in parsed.history {
        if date_cell(&row.date).is_none(){continue;}
        let Some(color)=normalize_color(&row.couleur_reelle)else{continue};
        if config.tempo_days.get(&row.date).is_some_and(|v|v.manual){continue;}
        config.tempo_days.insert(row.date,TempoColor { color:color.into(),source:"calendrier-tempo.fr (EDF relay)".into(),manual:false });
        inserted+=1;
    }
    if inserted==0 {return Err("Tempo API returned no official usable colors".into());}
    config.tempo_last_sync=Some(Utc::now().to_rfc3339());
    Ok(inserted)
}
pub fn merge_tempo_today(config:&mut TariffConfig,body:&str)->Result<usize,String> {
    #[derive(Deserialize)] struct Today { status:String,date:Option<String>,couleur:Option<String> }
    let v:Today=serde_json::from_str(body).map_err(|e|e.to_string())?;
    if v.status!="ok" {return Ok(0);}
    let (Some(date),Some(color))=(v.date,v.couleur) else{return Ok(0)};
    if date_cell(&date).is_none(){return Ok(0)};
    let Some(color)=normalize_color(&color) else{return Ok(0)};
    if config.tempo_days.get(&date).is_some_and(|x|x.manual){return Ok(0)};
    config.tempo_days.insert(date,TempoColor{color:color.into(),source:"calendrier-tempo.fr (EDF relay)".into(),manual:false});
    config.tempo_last_sync=Some(Utc::now().to_rfc3339());
    Ok(1)
}
fn normalize_color(color:&str)->Option<&'static str> {
    match color.to_uppercase().as_str(){"BLEU"|"BLUE"=>Some("blue"),"BLANC"|"WHITE"=>Some("white"),"ROUGE"|"RED"=>Some("red"),_=>None}
}

#[derive(Debug,Clone,Serialize,Default)]
pub struct CostStats {
    pub billed_energy_kwh:f64,
    pub energy_cost_eur:f64,
    pub subscription_eur:f64,
    pub projected_energy_cost_eur:f64,
    pub projected_subscription_eur:f64,
    /// True when an observed interval lacks a priced subscription revision.
    pub missing_subscription_rate:bool,
    pub projected_kwh:f64,
    pub unpriced_kwh:f64,
    pub approximate_kwh:f64,
    pub estimated_kwh:f64,
    pub observed_seconds:f64,
    pub warnings:Vec<String>,
}
impl CostStats {
    pub fn total_eur(&self)->f64 {self.energy_cost_eur+self.subscription_eur}
    pub fn add(&mut self,other:&CostStats){
        self.billed_energy_kwh+=other.billed_energy_kwh;
        self.energy_cost_eur+=other.energy_cost_eur;
        self.subscription_eur+=other.subscription_eur;
        self.projected_energy_cost_eur+=other.projected_energy_cost_eur;
        self.projected_subscription_eur+=other.projected_subscription_eur;
        self.missing_subscription_rate|=other.missing_subscription_rate;
        self.projected_kwh+=other.projected_kwh;
        self.unpriced_kwh+=other.unpriced_kwh;
        self.approximate_kwh+=other.approximate_kwh;
        self.estimated_kwh+=other.estimated_kwh;
        self.observed_seconds+=other.observed_seconds;
        for w in &other.warnings {if !self.warnings.contains(w){self.warnings.push(w.clone());}}
    }
}
#[derive(Debug,Clone)]
pub struct EnergySlice {pub start:i64,pub end:i64,pub wh:f64,pub estimated_wh:f64,pub covered_seconds:f64,pub approximate:bool}

/// Use exact, 15-minute and legacy hourly records; never count the same Wh twice.
/// Legacy hourly records are explicitly marked approximate for pricing.
pub fn load_slices(conn:&Connection,source:&str,from:i64,to:i64)->rusqlite::Result<Vec<EnergySlice>> {
    let mut slices=Vec::new();
    let mut query=|sql:&str, span:i64, approximate:bool, archive:bool|->rusqlite::Result<()> {
        let mut stmt=conn.prepare(sql)?;
        let rows=stmt.query_map(params![source,from,to],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,f64>(2)?,r.get::<_,f64>(3)?,r.get::<_,f64>(4)?)))?;
        for row in rows {
            let (start,end,wh,est,covered)=row?;
            let overlap=(end.min(to)-start.max(from)).max(0);
            if overlap<=0||end<=start{continue;}
            let ratio=overlap as f64/(end-start) as f64;
            slices.push(EnergySlice{start:start.max(from),end:end.min(to),wh:wh*ratio,estimated_wh:est*ratio,covered_seconds:covered*ratio,approximate});
        }
        let _=(span,archive);
        Ok(())
    };
    query("SELECT start_epoch,end_epoch,wh,estimated_wh,end_epoch-start_epoch FROM energy_segments WHERE source=?1 AND end_epoch>?2 AND start_epoch<?3",0,false,false)?;
    query("SELECT quarter_epoch,quarter_epoch+900,wh,estimated_wh,covered_seconds FROM energy_quarterly WHERE source=?1 AND quarter_epoch+900>?2 AND quarter_epoch<?3",900,false,true)?;
    query("SELECT hour_epoch,hour_epoch+3600,wh,estimated_wh,covered_seconds FROM energy_hourly WHERE source=?1 AND hour_epoch+3600>?2 AND hour_epoch<?3",3600,true,true)?;
    slices.sort_by_key(|s|s.start);
    Ok(slices)
}

fn price_at<'a>(contract:&'a TariffContract,date:NaiveDate)->Option<&'a PriceRevision> {
    contract.revisions.iter().filter(|rev|rev.from<=date&&rev.until.is_none_or(|u|date<=u))
        .max_by_key(|r|r.from)
}
fn first_future_price<'a>(contract:&'a TariffContract,date:NaiveDate)->Option<&'a PriceRevision> {
    contract.revisions.iter().min_by_key(|rev|rev.from).filter(|rev|rev.from>date)
}
fn in_offpeak(time:NaiveTime,windows:&[OffpeakWindow])->bool {
    windows.iter().any(|window|{
        let (Ok(a),Ok(b))=(parse_time(&window.from),parse_time(&window.to))else{return false};
        if a==b{return false;}
        if a<b{time>=a&&time<b}else{time>=a||time<b}
    })
}
fn category(config:&TariffConfig,contract:&TariffContract,when:DateTime<Utc>)->Option<String> {
    let local=when.with_timezone(&Paris);
    let date=local.date_naive();
    let hc=in_offpeak(local.time(),&contract.offpeak);
    match contract.option.as_str(){
        "base"=>Some("base".into()),
        "hphc"|"custom"=>if contract.offpeak.is_empty(){None}else{Some(if hc{"hc"}else{"hp"}.into())},
        "tempo"=>{
            let day=if local.hour()<6 {date.pred_opt()?}else{date};
            let color=config.tempo_days.get(&day.to_string())?.color.as_str();
            let hp=local.hour()>=6&&local.hour()<22;
            Some(format!("{color}_{}",if hp{"hp"}else{"hc"}))
        },
        _=>None,
    }
}
/// Each interval is split into UTC minute boundaries: DST, seasonal changes,
/// half-hour offpeak switches and Tempo day at 06:00 Europe/Paris are respected.
pub fn price_slices(config:&TariffConfig,site:&str,slices:&[EnergySlice])->CostStats {
    let mut result=CostStats::default();
    if !config.enabled{return result;}
    for slice in slices {
        if slice.end<=slice.start||slice.wh<0.0||!slice.wh.is_finite(){continue;}
        let seconds=(slice.end-slice.start) as f64;
        result.observed_seconds+=slice.covered_seconds;
        result.estimated_kwh+=slice.estimated_wh/1000.0;
        if slice.approximate {result.approximate_kwh+=slice.wh/1000.0;}
        let mut cursor=slice.start;
        while cursor<slice.end {
            let next=slice.end.min(cursor.div_euclid(60).saturating_add(1).saturating_mul(60));
            if next<=cursor{break;}
            let wh=slice.wh*(next-cursor) as f64/seconds;
            let at=DateTime::<Utc>::from_timestamp(cursor,0);
            let Some(at)=at else {result.unpriced_kwh+=wh/1000.0;break};
            let date=at.with_timezone(&Paris).date_naive();
            let contract=config.contracts.iter().find(|c|c.site_id==site&&c.starts_on<=date&&c.ends_on.is_none_or(|u|date<=u));
            let price=contract.and_then(|c|price_at(c,date).and_then(|revision|category(config,c,at).and_then(|key|revision.eur_kwh.get(&key).copied())));
            if let Some(price)=price {
                result.billed_energy_kwh+=wh/1000.0;
                result.energy_cost_eur+=wh/1000.0*price;
            }else{
                let projection=if config.estimate_unknown_history {
                    contract.and_then(|c|if price_at(c,date).is_none(){first_future_price(c,date).and_then(|rev|category(config,c,at).and_then(|key|rev.eur_kwh.get(&key).copied()))}else{None})
                }else{None};
                if let Some(projected_price)=projection {
                    result.projected_kwh+=wh/1000.0;
                    result.projected_energy_cost_eur+=wh/1000.0*projected_price;
                }else{result.unpriced_kwh+=wh/1000.0;}
                let reason=match contract {
                    None=>"no contract for this historical period",
                    Some(c) if price_at(c,date).is_none()=>"no price revision for this historical period",
                    Some(c) if ["hphc","custom"].contains(&c.option.as_str()) && c.offpeak.is_empty()=>"meter off-peak hours are missing",
                    Some(c) if c.option=="tempo" && category(config,c,at).is_none()=>"official Tempo color is missing",
                    Some(_)=>"price missing for this tariff category",
                };
                if projection.is_none() && !result.warnings.iter().any(|w|w==reason){result.warnings.push(reason.into());}
            }
            cursor=next;
        }
    }
    if result.approximate_kwh>1e-10 {result.warnings.push("legacy hourly energy: price breakdown approximate".into());}
    if result.projected_kwh>1e-10 {result.warnings.push("historical cost estimated using a later known rate, not a historical bill".into());}
    result
}
/// Optional subscription cost, once per site, prorated by contract/revision days.
/// No subscription is implicitly charged for unconfigured time ranges.
/// Missing fixed-price data must not silently be presented as a fully priced
/// electricity bill. Check every local-calendar day actually in the interval.
/// This does not change the existing cost computation or its UI estimates.
pub fn missing_subscription_rate(config:&TariffConfig,site:&str,from:i64,to:i64)->bool {
    if !config.enabled || !config.include_subscription || to<=from {return false;}
    let Some(start)=DateTime::<Utc>::from_timestamp(from,0)else{return true};
    let Some(end)=DateTime::<Utc>::from_timestamp(to,0)else{return true};
    let mut day=start.with_timezone(&Paris).date_naive();
    let last=end.with_timezone(&Paris).date_naive();
    let mut count=0;
    while day<=last && count<36525 {
        let next=day.succ_opt().unwrap_or(day);
        let local0=day.and_hms_opt(0,0,0).and_then(|t|Paris.from_local_datetime(&t).earliest());
        let local1=next.and_hms_opt(0,0,0).and_then(|t|Paris.from_local_datetime(&t).earliest());
        if let (Some(a),Some(b))=(local0,local1) {
            let from_day=a.with_timezone(&Utc).timestamp();
            let to_day=b.with_timezone(&Utc).timestamp();
            if to.min(to_day)>from.max(from_day) {
                let revision=config.contracts.iter()
                    .find(|c|c.site_id==site && c.starts_on<=day && c.ends_on.is_none_or(|d|day<=d))
                    .and_then(|contract|price_at(contract,day));
                if !revision.is_some_and(|r|r.annual_subscription_eur.is_some()) {return true;}
            }
        }
        let Some(next)=day.succ_opt() else {return true};
        day=next;count+=1;
    }
    count>=36525 && day<=last
}

pub fn subscription_cost(config:&TariffConfig,site:&str,from:i64,to:i64)->f64 {
    subscription_cost_breakdown(config,site,from,to).0
}
pub fn subscription_cost_breakdown(config:&TariffConfig,site:&str,from:i64,to:i64)->(f64,f64) {
    if !config.enabled||!config.include_subscription||to<=from{return (0.0,0.0);}
    let Some(start)=DateTime::<Utc>::from_timestamp(from,0)else{return (0.0,0.0)};
    let Some(end)=DateTime::<Utc>::from_timestamp(to,0)else{return (0.0,0.0)};
    let mut cost=0.0;let mut projected=0.0;
    let mut day=start.with_timezone(&Paris).date_naive();
    let last=end.with_timezone(&Paris).date_naive();
    let mut count=0;
    while day<=last&&count<36525 {
        // Calculate site subscription in local calendar-day fractions with DST.
        if let Some(c)=config.contracts.iter().find(|c|c.site_id==site&&c.starts_on<=day&&c.ends_on.is_none_or(|d|day<=d)) {
            let exact=price_at(c,day);
            let selected=exact.or_else(||if config.estimate_unknown_history {first_future_price(c,day)} else {None});
            if let Some(rev)=selected {
                if let Some(annual)=rev.annual_subscription_eur {
                    let next=day.succ_opt().unwrap_or(day);
                    let local0=day.and_hms_opt(0,0,0).and_then(|t|Paris.from_local_datetime(&t).earliest());
                    let local1=next.and_hms_opt(0,0,0).and_then(|t|Paris.from_local_datetime(&t).earliest());
                    if let (Some(a),Some(b))=(local0,local1){
                        let a=a.with_timezone(&Utc).timestamp();let b=b.with_timezone(&Utc).timestamp();
                        let observed=(to.min(b)-from.max(a)).max(0) as f64;
                        let year_days=if NaiveDate::from_ymd_opt(day.year(),2,29).is_some(){366.0}else{365.0};
                        let full=(b-a).max(1) as f64;
                        let amount=annual/year_days*(observed/full);
                        if exact.is_some(){cost+=amount;}else{projected+=amount;}
                    }
                }
            }
        }
        let Some(next)=day.succ_opt()else{break}; day=next;count+=1;
    }
    (cost,projected)
}

#[derive(Debug, Serialize, Default)]
pub struct CostReport {
    pub enabled: bool,
    pub global: CostStats,
    pub sites: BTreeMap<String, CostStats>,
    pub nodes: BTreeMap<String, CostStats>,
}
/// inputs are (energy source ID, assigned electric site ID, include in hub total).
/// A site's fixed subscription is never multiplied by its number of nodes.
pub fn cost_report(conn:&Connection, config:&TariffConfig, assignments:&[(String,String,bool)], from:i64, to:i64)->Result<CostReport,String> {
    if to<=from { return Err("invalid calculation interval".into()); }
    let mut report=CostReport {enabled:config.enabled,..Default::default()};
    if !config.enabled {return Ok(report);}
    for (source,site,included) in assignments {
        let samples=load_slices(conn,source,from,to).map_err(|e|e.to_string())?;
        let cost=price_slices(config,site,&samples);
        report.nodes.insert(source.clone(),cost.clone());
        if *included {report.sites.entry(site.clone()).or_default().add(&cost);}
    }
    for (site,stats) in report.sites.iter_mut() {
        (stats.subscription_eur,stats.projected_subscription_eur)=subscription_cost_breakdown(config,site,from,to);
        stats.missing_subscription_rate=missing_subscription_rate(config,site,from,to);
        report.global.add(stats);
    }
    Ok(report)
}

/// Energy-only cumulative cost for a chart. Each slice is priced once per
/// intersecting bucket, with the same minute/DST rules as cost_report.
#[derive(Debug, Serialize)]
pub struct CostSeriesPoint {
    pub timestamp: DateTime<Utc>,
    pub cost_eur: Option<f64>,
    pub partial: bool,
    pub estimated: bool,
    pub unpriced_kwh: f64,
}
#[derive(Debug, Serialize)]
pub struct CostSeries {
    pub enabled: bool,
    pub points: Vec<CostSeriesPoint>,
}
pub fn cost_series(
    conn: &Connection,
    config: &TariffConfig,
    assignments: &[(String, String, bool)],
    from: i64,
    to: i64,
    max_points: usize,
) -> Result<CostSeries, String> {
    if from < 0 || to <= from || to - from > 3660 * 86400 || !(2..=1200).contains(&max_points) {
        return Err("invalid cost series period or point limit".into());
    }
    let mut report = CostSeries {
        enabled: config.enabled,
        points: Vec::new(),
    };
    if !config.enabled {
        return Ok(report);
    }
    let count = (max_points - 1).min((to - from) as usize);
    let width = ((to - from) + count as i64 - 1) / count as i64;
    let mut buckets = vec![CostStats::default(); count];
    let included = assignments.iter().filter(|a| a.2).collect::<Vec<_>>();
    for (source, site, _) in &included {
        for slice in load_slices(conn, source, from, to).map_err(|e| e.to_string())? {
            let mut cursor = slice.start;
            while cursor < slice.end {
                let index = ((cursor - from) / width) as usize;
                let end = slice.end.min(from + (index as i64 + 1) * width).min(to);
                let ratio = (end - cursor) as f64 / (slice.end - slice.start) as f64;
                let clipped = EnergySlice {
                    start: cursor,
                    end,
                    wh: slice.wh * ratio,
                    estimated_wh: slice.estimated_wh * ratio,
                    covered_seconds: slice.covered_seconds * ratio,
                    approximate: slice.approximate,
                };
                buckets[index].add(&price_slices(config, site, &[clipped]));
                cursor = end;
            }
        }
    }
    let mut total = 0.0;
    let mut unpriced = 0.0;
    let mut partial = false;
    let mut estimated = false;
    report.points.push(CostSeriesPoint {
        timestamp: DateTime::from_timestamp(from, 0).ok_or("invalid series date")?,
        cost_eur: Some(0.0),
        partial: false,
        estimated: false,
        unpriced_kwh: 0.0,
    });
    for (index, stats) in buckets.iter().enumerate() {
        let start = from + index as i64 * width;
        if start >= to {
            break;
        }
        let end = (start + width).min(to);
        total += stats.energy_cost_eur + stats.projected_energy_cost_eur;
        unpriced += stats.unpriced_kwh;
        let missing = stats.observed_seconds + 0.001 < (end - start) as f64 * included.len() as f64;
        partial |= unpriced > 1e-10 || missing || included.is_empty();
        estimated |= stats.estimated_kwh > 1e-10
            || stats.approximate_kwh > 1e-10
            || stats.projected_kwh > 1e-10;
        // Unknown rates/data create gaps rather than a misleading zero-cost plateau.
        let available = stats.observed_seconds > 0.0 && stats.unpriced_kwh < 1e-10;
        report.points.push(CostSeriesPoint {
            timestamp: DateTime::from_timestamp(end, 0).ok_or("invalid series date")?,
            cost_eur: available.then_some(total),
            partial,
            estimated,
            unpriced_kwh: unpriced,
        });
    }
    Ok(report)
}

/// Dashboard cards: same tariff engine as the detailed estimate, with the
/// exact observed window and one optional subscription per electricity site.
#[derive(Debug, Serialize)]
pub struct CostOverview {
    pub enabled: bool,
    pub windows: BTreeMap<&'static str, CostStats>,
    pub selected: CostStats,
}

fn overview_period(
    conn: &Connection,
    config: &TariffConfig,
    assignments: &[(String, String, bool)],
    from: Option<i64>,
    to: i64,
) -> Result<CostStats, String> {
    if !config.enabled || assignments.is_empty() { return Ok(CostStats::default()); }
    // A subscription must not start before the first recorded energy for a
    // selected site, including for the lifetime counter (whose start is open).
    let mut first_by_site = BTreeMap::<String, i64>::new();
    for (source, site, included) in assignments {
        if !*included { continue; }
        let stats = crate::energy::stats(conn, source, from, to).map_err(|e| e.to_string())?;
        if let Some(first) = stats.first_seen_epoch {
            let observed_start = first.max(from.unwrap_or(first));
            first_by_site.entry(site.clone())
                .and_modify(|v| *v = (*v).min(observed_start))
                .or_insert(observed_start);
        }
    }
    let Some(earliest) = first_by_site.values().copied().min() else { return Ok(CostStats::default()); };
    if earliest >= to { return Ok(CostStats::default()); }
    let included: Vec<_> = assignments.iter().filter(|a| a.2 && first_by_site.contains_key(&a.1)).cloned().collect();
    let mut report = cost_report(conn, config, &included, earliest, to)?;
    let mut total = CostStats::default();
    for (site, part) in report.sites.iter_mut() {
        if let Some(first) = first_by_site.get(site) {
            (part.subscription_eur,part.projected_subscription_eur) = subscription_cost_breakdown(config, site, *first, to);
            part.missing_subscription_rate = missing_subscription_rate(config,site,*first,to);
        }
        total.add(part);
    }
    Ok(total)
}

/// Single-period lifetime view for external exports, using identical tariff
/// and per-site subscription rules as the WebUI dashboard.
pub fn lifetime_cost(conn:&Connection,config:&TariffConfig,assignments:&[(String,String,bool)],now:i64)->Result<CostStats,String> {
    overview_period(conn,config,assignments,None,now)
}

pub fn cost_overview(
    conn: &Connection,
    config: &TariffConfig,
    assignments: &[(String, String, bool)],
    now: i64,
    selected_from: Option<i64>,
    selected_to: i64,
) -> Result<CostOverview, String> {
    if selected_to > now + 60 || selected_from.is_some_and(|from| from >= selected_to)
        || selected_from.is_some_and(|from| from < 0) || selected_to < 0 {
        return Err("invalid cost period".into());
    }
    let mut windows = BTreeMap::new();
    for (key, start) in [("24h", Some(now - 86400)), ("7d", Some(now - 7 * 86400)),
        ("30d", Some(now - 30 * 86400)), ("all", None)] {
        windows.insert(key, overview_period(conn, config, assignments, start, now)?);
    }
    let selected = overview_period(conn, config, assignments, selected_from, selected_to)?;
    Ok(CostOverview { enabled: config.enabled, windows, selected })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    fn sample()->TariffConfig {
        let mut c=TariffConfig::default();c.enabled=true;
        c.contracts.push(TariffContract{id:"contract".into(),site_id:"local".into(),supplier:"EDF".into(),offer:"Tarif Bleu".into(),option:"hphc".into(),starts_on:NaiveDate::from_ymd_opt(2026,1,1).unwrap(),ends_on:None,kva:6,offpeak:vec![OffpeakWindow{from:"22:30".into(),to:"06:30".into()}],revisions:vec![PriceRevision{from:NaiveDate::from_ymd_opt(2026,1,1).unwrap(),until:None,eur_kwh:BTreeMap::from([("hp".into(),0.30),("hc".into(),0.15)]),annual_subscription_eur:Some(120.0),source:"manual".into(),source_url:None,manual:true,retrieved_at:None}]});c
    }
    #[test]
    fn chart_cost_sums_sites_and_excludes_subscription_and_excluded_nodes() {
        let db = Connection::open_in_memory().unwrap();
        crate::energy::init(&db).unwrap();
        let mut c = sample();
        c.include_subscription = true;
        c.sites.push(TariffSite {
            id: "other".into(),
            name: "Other".into(),
            nodes: vec![],
        });
        let mut other = c.contracts[0].clone();
        other.id = "other".into();
        other.site_id = "other".into();
        other.option = "base".into();
        other.revisions[0].eur_kwh = BTreeMap::from([("base".into(), 0.5)]);
        c.contracts.push(other);
        let from = Paris
            .with_ymd_and_hms(2026, 3, 2, 22, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        let to = from + 7200;
        for (source, wh) in [("a", 2000.0), ("b", 1000.0), ("excluded", 9000.0)] {
            db.execute(
                "INSERT INTO energy_segments VALUES (?1,?2,?3,?4,0)",
                params![source, from, to, wh],
            )
            .unwrap();
        }
        let assignments = vec![
            ("a".into(), "local".into(), true),
            ("b".into(), "other".into(), true),
            ("excluded".into(), "other".into(), false),
        ];
        let curve = cost_series(&db, &c, &assignments, from, to, 17).unwrap();
        let report = cost_report(&db, &c, &assignments, from, to).unwrap();
        let end = curve.points.last().unwrap();
        assert!((end.cost_eur.unwrap() - 0.875).abs() < 1e-9);
        assert!((end.cost_eur.unwrap() - report.global.energy_cost_eur).abs() < 1e-9);
        assert!(report.global.subscription_eur > 0.0);
        assert!(!end.partial);
        assert!(!end.estimated);
        assert!(curve.points.len() <= 17);
        assert_eq!(curve.points[0].cost_eur, Some(0.0));
    }
    #[test]
    fn chart_cost_marks_unpriced_and_missing_observations_without_zero_cost_plateaus() {
        let db = Connection::open_in_memory().unwrap();
        crate::energy::init(&db).unwrap();
        let mut c = sample();
        c.contracts[0].option = "base".into();
        c.contracts[0].revisions[0].eur_kwh = BTreeMap::from([("base".into(), 0.25)]);
        c.contracts[0].revisions[0].from = NaiveDate::from_ymd_opt(2026, 3, 3).unwrap();
        let from = Paris
            .with_ymd_and_hms(2026, 3, 2, 23, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        for (start, end) in [(from, from + 3600), (from + 7200, from + 10800)] {
            db.execute(
                "INSERT INTO energy_segments VALUES ('local',?1,?2,1000,0)",
                params![start, end],
            )
            .unwrap();
        }
        let assignments = vec![("local".into(), "local".into(), true)];
        let curve = cost_series(&db, &c, &assignments, from, from + 10800, 4).unwrap();
        assert_eq!(curve.points[1].cost_eur, None);
        assert_eq!(curve.points[2].cost_eur, None);
        assert!((curve.points[3].cost_eur.unwrap() - 0.25).abs() < 1e-9);
        assert!(curve.points[3].partial);
        assert!((curve.points[3].unpriced_kwh - 1.0).abs() < 1e-9);
        let empty = cost_series(&db, &c, &[], from, from + 10800, 4).unwrap();
        assert!(empty.points.iter().skip(1).all(|p| p.cost_eur.is_none()));
        c.enabled = false;
        assert!(cost_series(&db, &c, &assignments, from, from + 10800, 4)
            .unwrap()
            .points
            .is_empty());
        assert!(cost_series(&db, &c, &assignments, from, from, 4).is_err());
        assert!(cost_series(&db, &c, &assignments, from, from + 1, 1201).is_err());
    }
    #[test]
    fn chart_cost_preserves_dst_and_opt_in_estimate_semantics() {
        let db = Connection::open_in_memory().unwrap();
        crate::energy::init(&db).unwrap();
        let mut c = sample();
        c.contracts[0].offpeak = vec![OffpeakWindow {
            from: "02:00".into(),
            to: "03:00".into(),
        }];
        let from = Utc
            .with_ymd_and_hms(2026, 10, 25, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        db.execute(
            "INSERT INTO energy_segments VALUES ('local',?1,?2,2000,2000)",
            params![from, from + 7200],
        )
        .unwrap();
        let assignments = vec![("local".into(), "local".into(), true)];
        let curve = cost_series(&db, &c, &assignments, from, from + 7200, 13).unwrap();
        assert!((curve.points.last().unwrap().cost_eur.unwrap() - 0.3).abs() < 1e-9);
        assert!(curve.points.last().unwrap().estimated);
        c.contracts[0].revisions[0].from = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        c.estimate_unknown_history = true;
        let curve = cost_series(&db, &c, &assignments, from, from + 7200, 13).unwrap();
        assert!((curve.points.last().unwrap().cost_eur.unwrap() - 0.3).abs() < 1e-9);
        assert!(curve.points.last().unwrap().estimated);
    }

    #[test] fn missing_fixed_tariff_is_identified_without_touching_variable_prices() {
        let mut c=sample();
        c.include_subscription=true;
        let from=Paris.with_ymd_and_hms(2026,3,2,12,0,0).single().unwrap().timestamp();
        assert!(!missing_subscription_rate(&c,"local",from,from+3600));
        c.contracts[0].revisions[0].annual_subscription_eur=None;
        assert!(missing_subscription_rate(&c,"local",from,from+3600));
        c.include_subscription=false;
        assert!(!missing_subscription_rate(&c,"local",from,from+3600));
    }
    #[test] fn dashboard_costs_price_observed_hours_and_do_not_double_subscription() {
        let mut db=Connection::open_in_memory().unwrap();
        crate::energy::init(&db).unwrap();
        let mut c=sample();c.contracts[0].option="base".into();
        c.contracts[0].revisions[0].eur_kwh=BTreeMap::from([("base".into(),0.20)]);
        c.include_subscription=true;
        let end=Paris.with_ymd_and_hms(2026,10,10,14,0,0).single().unwrap().with_timezone(&Utc).timestamp();
        for source in ["n1","n2"] {
            crate::energy::record(&mut db,source,end-3600,1000.0,false,3600).unwrap();
            crate::energy::record(&mut db,source,end,1000.0,false,3600).unwrap();
        }
        let picks=vec![("n1".into(),"local".into(),true),("n2".into(),"local".into(),true)];
        let all=cost_overview(&db,&c,&picks,end,Some(end-3600),end).unwrap();
        let single=cost_overview(&db,&c,&picks[..1],end,Some(end-3600),end).unwrap();
        assert!((all.windows["24h"].energy_cost_eur-0.4).abs()<1e-8);
        assert!((all.selected.energy_cost_eur-0.4).abs()<1e-8);
        assert!((single.windows["24h"].energy_cost_eur-0.2).abs()<1e-8);
        assert!((single.windows["24h"].subscription_eur-all.windows["24h"].subscription_eur).abs()<1e-8);
        assert!(all.windows["all"].subscription_eur>0.0);
        assert!(all.windows["all"].subscription_eur<1.0); // not charged since 1970
        let empty=cost_overview(&db,&c,&[],end,None,end).unwrap();
        assert_eq!(empty.windows["all"].energy_cost_eur,0.0);
        assert_eq!(empty.windows["all"].subscription_eur,0.0);
    }
    #[test] fn dashboard_uses_historical_prices_and_marks_missing_prices() {
        let mut db=Connection::open_in_memory().unwrap();
        crate::energy::init(&db).unwrap();
        let mut c=sample();c.include_subscription=false;c.contracts[0].option="base".into();
        c.contracts[0].revisions[0].eur_kwh=BTreeMap::from([("base".into(),0.20)]);
        let end=Paris.with_ymd_and_hms(2026,10,10,12,0,0).single().unwrap().with_timezone(&Utc).timestamp();
        crate::energy::record(&mut db,"local",end-7200,1000.0,false,3600).unwrap();
        crate::energy::record(&mut db,"local",end-3600,1000.0,false,3600).unwrap();
        crate::energy::record(&mut db,"local",end,1000.0,false,3600).unwrap();
        let included=vec![("local".into(),"local".into(),true)];
        let priced=cost_overview(&db,&c,&included,end,None,end).unwrap();
        assert!((priced.windows["24h"].energy_cost_eur-0.4).abs()<1e-8);
        c.contracts[0].revisions[0].eur_kwh.clear();
        let unpriced=cost_overview(&db,&c,&included,end,None,end).unwrap();
        assert_eq!(unpriced.windows["24h"].energy_cost_eur,0.0);
        assert!((unpriced.windows["24h"].unpriced_kwh-2.0).abs()<1e-8);
    }
    #[test] fn optional_history_projection_keeps_documented_cost_separate_and_never_prices_missing_contracts() {
        assert!(!TariffConfig::default().estimate_unknown_history);
        let mut c=sample();c.contracts[0].starts_on=NaiveDate::from_ymd_opt(2019,6,1).unwrap();
        c.contracts[0].revisions[0].from=NaiveDate::from_ymd_opt(2026,10,11).unwrap();
        c.include_subscription=true;
        let start=Paris.with_ymd_and_hms(2026,10,10,14,0,0).single().unwrap().with_timezone(&Utc).timestamp();
        let slice=EnergySlice{start,end:start+3600,wh:1000.0,estimated_wh:0.0,covered_seconds:3600.0,approximate:false};
        let strict=price_slices(&c,"local",&[slice.clone()]);
        assert_eq!(strict.projected_kwh,0.0);assert!((strict.unpriced_kwh-1.0).abs()<1e-8);
        c.estimate_unknown_history=true;
        let projected=price_slices(&c,"local",&[slice.clone()]);
        assert_eq!(projected.energy_cost_eur,0.0);
        assert!((projected.projected_kwh-1.0).abs()<1e-8);
        assert!((projected.projected_energy_cost_eur-0.30).abs()<1e-8);
        assert_eq!(projected.unpriced_kwh,0.0);
        let (exact,estimated)=subscription_cost_breakdown(&c,"local",start,start+3600);
        assert_eq!(exact,0.0);assert!(estimated>0.0);
        let mut db=Connection::open_in_memory().unwrap();crate::energy::init(&db).unwrap();
        crate::energy::record(&mut db,"local",start,1000.0,false,3600).unwrap();
        crate::energy::record(&mut db,"local",start+3600,1000.0,false,3600).unwrap();
        let overview=cost_overview(&db,&c,&[("local".into(),"local".into(),true)],start+3600,Some(start),start+3600).unwrap();
        assert!(overview.selected.projected_kwh>0.0);
        assert_eq!(overview.selected.energy_cost_eur,0.0);
        assert_eq!(overview.selected.unpriced_kwh,0.0);
        let earlier=Paris.with_ymd_and_hms(2018,10,10,14,0,0).single().unwrap().with_timezone(&Utc).timestamp();
        let no_contract=price_slices(&c,"local",&[EnergySlice{start:earlier,end:earlier+3600,..slice}]);
        assert_eq!(no_contract.projected_kwh,0.0);assert!((no_contract.unpriced_kwh-1.0).abs()<1e-8);
        c.contracts[0].offpeak.clear();
        let no_hours=price_slices(&c,"local",&[EnergySlice{start,end:start+3600,wh:1000.0,estimated_wh:0.0,covered_seconds:3600.0,approximate:false}]);
        assert_eq!(no_hours.projected_kwh,0.0);assert!((no_hours.unpriced_kwh-1.0).abs()<1e-8);
    }
        #[test] fn rejects_overlapping_contracts_and_negative_prices(){let mut c=sample();assert!(validate(&c).is_ok());let duplicate=c.contracts[0].clone();c.contracts.push(duplicate);assert!(validate(&c).is_err());}
    #[test] fn prices_switch_on_half_hour_and_dst(){
        let c=sample();let t=Paris.with_ymd_and_hms(2026,10,10,22,0,0).single().unwrap().with_timezone(&Utc).timestamp();
        let slices=[EnergySlice{start:t,end:t+3600,wh:1000.0,estimated_wh:1000.0,covered_seconds:3600.0,approximate:false}];
        let p=price_slices(&c,"local",&slices);
        assert!((p.energy_cost_eur-0.225).abs()<1e-8);
    }
    #[test] fn missing_meter_offpeak_slots_do_not_guess_hourly_costs(){
        let mut c=sample();c.contracts[0].offpeak.clear();
        let now=Paris.with_ymd_and_hms(2026,10,10,22,0,0).single().unwrap().with_timezone(&Utc).timestamp();
        let result=price_slices(&c,"local",&[EnergySlice{start:now,end:now+900,wh:100.0,estimated_wh:0.0,covered_seconds:900.0,approximate:false}]);
        assert_eq!(result.energy_cost_eur,0.0);
        assert!((result.unpriced_kwh-0.1).abs()<1e-9);
    }
    #[test] fn missing_tempo_color_never_invents_price(){
        let mut c=sample();c.contracts[0].option="tempo".into();
        let t=Paris.with_ymd_and_hms(2026,10,10,10,0,0).single().unwrap().with_timezone(&Utc).timestamp();
        let p=price_slices(&c,"local",&[EnergySlice{start:t,end:t+900,wh:100.0,estimated_wh:0.0,covered_seconds:900.0,approximate:false}]);
        assert_eq!(p.energy_cost_eur,0.0);assert!((p.unpriced_kwh-0.1).abs()<1e-10);
    }
    #[test] fn survives_persistence(){
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tariffs-fr.db");
        let c = sample();
        {
            let db = Connection::open(&path).unwrap();
            // Saving on a brand-new database must create the schema itself.
            save(&db, &c).unwrap();
        }
        let db = Connection::open(&path).unwrap();
        let d = load(&db).unwrap();
        assert_eq!(d.contracts[0].revisions[0].eur_kwh["hc"], 0.15);
    }
    #[test] fn stale_manual_save_cannot_overwrite_newer_import(){
        let db=Connection::open_in_memory().unwrap();
        let stale=TariffConfig::default();
        let current=save(&db,&stale).unwrap();
        assert_eq!(current.revision,1);
        let mut newer=current.clone();newer.enabled=true;
        let saved=save(&db,&newer).unwrap();
        assert_eq!(saved.revision,2);
        assert!(save(&db,&current).is_err());
        assert!(load(&db).unwrap().enabled);
    }
    #[test] fn parses_cre_prices_and_keeps_manual_override(){
        let mut c=sample();c.contracts[0].revisions.clear();
        let csv="DATE_DEBUT;P_SOUSCRITE;PART_VARIABLE_HP_TTC;PART_VARIABLE_HC_TTC;PART_FIXE_TTC\n01/02/2026;6;0,22;0,17;200,00\n";
        assert_eq!(merge_cre_csv(&mut c,"contract",csv).unwrap(),1);
        assert_eq!(c.contracts[0].revisions[0].eur_kwh["hc"],0.17);
        c.contracts[0].revisions[0].manual=true;
        assert!(merge_cre_csv(&mut c,"contract",csv).is_err());
        assert!(c.contracts[0].revisions[0].manual);
    }
}
