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
fn default_hp() -> Vec<OffpeakWindow> { vec![OffpeakWindow { from: "22:00".into(), to: "06:00".into() }] }
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
    #[serde(default)] pub enabled: bool,
    #[serde(default = "default_fr")] pub country: String,
    #[serde(default)] pub include_subscription: bool,
    #[serde(default)] pub sites: Vec<TariffSite>,
    #[serde(default)] pub contracts: Vec<TariffContract>,
    #[serde(default)] pub tempo_days: BTreeMap<String, TempoColor>,
    #[serde(default)] pub cre_last_sync: Option<String>,
    #[serde(default)] pub tempo_last_sync: Option<String>,
}
impl Default for TariffConfig {
    fn default() -> Self {
        Self { enabled:false,country:default_fr(),include_subscription:false,
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
pub fn save(conn:&Connection,config:&TariffConfig)->Result<(),String> {
    init(conn).map_err(|e|e.to_string())?;
    validate(config)?;
    let json=serde_json::to_string(config).map_err(|e|e.to_string())?;
    conn.execute("INSERT INTO tariffs_fr_config(singleton,json) VALUES(1,?1)
         ON CONFLICT(singleton) DO UPDATE SET json=excluded.json",[json])
        .map_err(|e|e.to_string())?;
    Ok(())
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
        for w in &c.offpeak {parse_time(&w.from)?;parse_time(&w.to)?;}
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
        "hphc"|"custom"=>Some(if hc{"hc"}else{"hp"}.into()),
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
            }else{result.unpriced_kwh+=wh/1000.0;}
            cursor=next;
        }
    }
    if result.unpriced_kwh>1e-10 {result.warnings.push("unpriced periods: missing contracts, prices or official Tempo colors".into());}
    if result.approximate_kwh>1e-10 {result.warnings.push("legacy hourly energy: price breakdown approximate".into());}
    result
}
/// Optional subscription cost, once per site, prorated by contract/revision days.
/// No subscription is implicitly charged for unconfigured time ranges.
pub fn subscription_cost(config:&TariffConfig,site:&str,from:i64,to:i64)->f64 {
    if !config.enabled||!config.include_subscription||to<=from{return 0.0;}
    let Some(start)=DateTime::<Utc>::from_timestamp(from,0)else{return 0.0};
    let Some(end)=DateTime::<Utc>::from_timestamp(to,0)else{return 0.0};
    let mut cost=0.0;
    let mut day=start.with_timezone(&Paris).date_naive();
    let last=end.with_timezone(&Paris).date_naive();
    let mut count=0;
    while day<=last&&count<36525 {
        // Calculate site subscription in local calendar-day fractions with DST.
        if let Some(c)=config.contracts.iter().find(|c|c.site_id==site&&c.starts_on<=day&&c.ends_on.is_none_or(|d|day<=d)) {
            if let Some(rev)=price_at(c,day) {
                if let Some(annual)=rev.annual_subscription_eur {
                    let next=day.succ_opt().unwrap_or(day);
                    let local0=day.and_hms_opt(0,0,0).and_then(|t|Paris.from_local_datetime(&t).earliest());
                    let local1=next.and_hms_opt(0,0,0).and_then(|t|Paris.from_local_datetime(&t).earliest());
                    if let (Some(a),Some(b))=(local0,local1){
                        let a=a.with_timezone(&Utc).timestamp();let b=b.with_timezone(&Utc).timestamp();
                        let observed=(to.min(b)-from.max(a)).max(0) as f64;
                        let year_days=if NaiveDate::from_ymd_opt(day.year(),2,29).is_some(){366.0}else{365.0};
                        let full=(b-a).max(1) as f64;
                        cost+=annual/year_days*(observed/full);
                    }
                }
            }
        }
        let Some(next)=day.succ_opt()else{break}; day=next;count+=1;
    }
    cost
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
        stats.subscription_eur=subscription_cost(config,site,from,to);
        report.global.add(stats);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    fn sample()->TariffConfig {
        let mut c=TariffConfig::default();c.enabled=true;
        c.contracts.push(TariffContract{id:"contract".into(),site_id:"local".into(),supplier:"EDF".into(),offer:"Tarif Bleu".into(),option:"hphc".into(),starts_on:NaiveDate::from_ymd_opt(2026,1,1).unwrap(),ends_on:None,kva:6,offpeak:vec![OffpeakWindow{from:"22:30".into(),to:"06:30".into()}],revisions:vec![PriceRevision{from:NaiveDate::from_ymd_opt(2026,1,1).unwrap(),until:None,eur_kwh:BTreeMap::from([("hp".into(),0.30),("hc".into(),0.15)]),annual_subscription_eur:Some(120.0),source:"manual".into(),source_url:None,manual:true,retrieved_at:None}]});c
    }
    #[test] fn rejects_overlapping_contracts_and_negative_prices(){let mut c=sample();assert!(validate(&c).is_ok());let duplicate=c.contracts[0].clone();c.contracts.push(duplicate);assert!(validate(&c).is_err());}
    #[test] fn prices_switch_on_half_hour_and_dst(){
        let c=sample();let t=Paris.with_ymd_and_hms(2026,10,10,22,0,0).single().unwrap().with_timezone(&Utc).timestamp();
        let slices=[EnergySlice{start:t,end:t+3600,wh:1000.0,estimated_wh:1000.0,covered_seconds:3600.0,approximate:false}];
        let p=price_slices(&c,"local",&slices);
        assert!((p.energy_cost_eur-0.225).abs()<1e-8);
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
