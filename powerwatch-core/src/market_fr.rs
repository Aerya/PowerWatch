//! Experimental French retail electricity offer discovery from public tariff grids.
//! Read-only extraction: the caller must confirm the offer before persisting a revision.
use chrono::{NaiveDate, Utc};
use scraper::{Html, Selector, ElementRef};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub fn today_fr() -> NaiveDate { Utc::now().with_timezone(&chrono_tz::Europe::Paris).date_naive() }

pub const DIRECTORY: &str = "https://www.kelwatt.fr/fournisseurs";
pub const MARKET_SOURCE: &str = "Kelwatt (publicly displayed tariff grids, unverified beta)";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Supplier { pub slug: String, pub name: String }
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MarketOffer {
    pub supplier: String,
    pub supplier_slug: String,
    pub offer: String,
    pub option: String,
    pub kva: u32,
    pub eur_kwh: BTreeMap<String, f64>,
    pub annual_subscription_eur: f64,
    pub source_url: String,
    pub fetched_at: String,
}

pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty() && slug.len() <= 60 && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
pub fn tariff_url(slug:&str) -> Result<String,String> {
    if !valid_slug(slug) {return Err("invalid supplier slug".into());}
    Ok(format!("https://www.kelwatt.fr/fournisseurs/{slug}/prix"))
}
fn text(element:ElementRef<'_>) -> String {
    element.text().collect::<Vec<_>>().join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
}
fn canon(s:&str)->String {
    s.to_lowercase().replace(&['é','è','ê','ë'][..],"e").replace(&['à','â'][..],"a")
     .replace(&['î','ï'][..],"i").replace(&['ô','ö'][..],"o").replace(&['û','ù','ü'][..],"u")
     .replace(&['ç'][..],"c").replace('’',"'").split_whitespace().collect::<Vec<_>>().join(" ")
}
fn eur(cell:&str) -> Option<f64> {
    // Do not pick a comparison discount or a monthly estimated bill as a rate.
    let before_eur=cell.split('€').next()?;
    let compact=before_eur.replace(&['\u{00a0}','\u{202f}',' '][..],"");
    let end=compact.rfind(|c:char|c.is_ascii_digit())?;
    let start=compact[..=end].rfind(|c:char| !c.is_ascii_digit()&&c!=','&&c!='.') .map_or(0,|i|i+1);
    compact[start..=end].replace(',',".").parse::<f64>().ok().filter(|n|n.is_finite()&&*n>=0.0)
}
fn column(headers:&[String], names:&[&str])->Option<usize> {
    headers.iter().position(|s|names.iter().any(|key|s.contains(key)))
}
fn supplier_name(slug:&str)->String { match slug {
    "totalenergies"=>"TotalEnergies".into(),"edf"=>"EDF".into(),
    "engie"=>"Engie".into(),"octopus-energy"=>"Octopus Energy".into(),
    "ekwateur"=>"Ekwateur".into(),"mint-energie"=>"Mint Énergie".into(),
    "alpiq"=>"Alpiq".into(),"la-bellenergie"=>"La Bellenergie".into(),
    "ohm-energie"=>"OHM Énergie".into(),"primeo-energie"=>"Primeo Énergie".into(),
    _=>slug.to_string()
}}

pub fn parse_suppliers(html:&str)->Vec<Supplier> {
    let document=Html::parse_document(html);
    let a=Selector::parse("a[href]").unwrap();
    let mut seen=HashSet::new();
    let mut result=Vec::new();
    for link in document.select(&a) {
        let Some(href)=link.value().attr("href")else{continue};
        let Some(tail)=href.split("/fournisseurs/").nth(1)else{continue};
        let slug=tail.split(['/', '?', '#']).next().unwrap_or("");
        if !valid_slug(slug)||!seen.insert(slug.to_owned()){continue;}
        result.push(Supplier {slug:slug.into(),name:supplier_name(slug)});
    }
    result.sort_by(|a,b|a.name.cmp(&b.name));
    result
}

pub fn parse_offers(slug:&str,html:&str)->Result<Vec<MarketOffer>,String> {
    let url=tariff_url(slug)?;
    if html.len()>4_000_000 {return Err("tariff page too large".into());}
    let doc=Html::parse_document(html);
    let headings_and_tables=Selector::parse("h2,h3,h4,h5,table").unwrap();
    let rows=Selector::parse("tr").unwrap();
    let cells=Selector::parse("th,td").unwrap();
    let caption=Selector::parse("caption").unwrap();
    let mut offer_name=String::new();
    let mut seen=HashSet::new();
    let mut out=Vec::new();
    let supplier=supplier_name(slug);
    let stamp=Utc::now().to_rfc3339();
    for element in doc.select(&headings_and_tables) {
        if element.value().name()!="table" {
            let title=text(element);
            let lower=canon(&title);
            if lower.starts_with("grille tarifaire de l'offre ") {
                let prefix="grille tarifaire de l'offre ";
                let rest=title.chars().skip(prefix.chars().count()).collect::<String>();
                // Page titles may contain a date; supplier name is the boundary.
                let marker=format!(" de {} en ",supplier);
                offer_name=rest.split(&marker).next().unwrap_or(&rest).trim().to_string();
            } else if element.value().name()=="h3" && !title.is_empty() &&
                !lower.starts_with("quel") && !lower.starts_with("compar") &&
                !lower.starts_with("tarif") && !lower.starts_with("foire") &&
                !lower.contains("gaz") && title.chars().count()<100 {
                offer_name=title;
            }
            continue;
        }
        let mut name=offer_name.clone();
        if let Some(caption)=element.select(&caption).next(){
            let label=text(caption);
            let canonical=canon(&label);
            if canonical.starts_with("grille tarifaire de l'offre ") {
                let rest=label.chars().skip("grille tarifaire de l'offre ".chars().count()).collect::<String>();
                let marker=format!(" de {} en ",supplier);
                name=rest.split(&marker).next().unwrap_or(&rest).trim().into();
            }
        }
        if name.is_empty() || canon(&name).contains("gaz") {continue;}
        let data=element.select(&rows).map(|row|row.select(&cells).map(text).collect::<Vec<_>>()).collect::<Vec<_>>();
        let Some(headers)=data.first().filter(|h|h.len()>=3)else{continue};
        let normalized=headers.iter().map(|h|canon(h)).collect::<Vec<_>>();
        let Some(power_col)=column(&normalized,&["puissance"])else{continue};
        let Some(annual_col)=column(&normalized,&["abo. annuel","abonnement annuel","abo annuel"])else{continue};
        // Options with 3+ prices are intentionally not misclassified as ordinary HP/HC.
        if normalized.iter().any(|h|h.contains("super creuses") || h.contains("vigilance") || h.contains("pointe mobile") || h.contains("heures creuses eco")){continue;}
        let base=column(&normalized,&["base (€/kwh)","base (eur/kwh)","base €", "base (euro"]);
        let hp=column(&normalized,&["heures pleines"]);
        let hc=column(&normalized,&["heures creuses"]);
        let (option,indices) = if let (Some(hp),Some(hc))=(hp,hc){("hphc",vec![("hp",hp),("hc",hc)])}
          else if let Some(base)=base {("base",vec![("base",base)])}
          else {continue};
        for row in data.iter().skip(1) {
            if row.len()!=headers.len(){continue;}
            let raw=&row[power_col];
            let Some(before)=raw.split("kVA").next()else{continue};
            let digits=before.trim().chars().rev().take_while(|c|c.is_ascii_digit()).collect::<String>();
            let kva=digits.chars().rev().collect::<String>().parse::<u32>().ok();
            let Some(kva)=kva.filter(|n|(1..=36).contains(n))else{continue};
            let Some(subscription)=eur(&row[annual_col]).filter(|n|*n>0.0&&*n<5000.0)else{continue};
            let mut prices=BTreeMap::new();
            for (key,index) in &indices {
                if let Some(p)=eur(&row[*index]).filter(|n|*n>0.0&&*n<=10.0){prices.insert((*key).into(),p);}
            }
            if prices.len()!=indices.len(){continue;}
            if !seen.insert((canon(&name),option.to_string(),kva)){continue;}
            out.push(MarketOffer{supplier:supplier.clone(),supplier_slug:slug.into(),offer:name.clone(),option:option.into(),kva,eur_kwh:prices,annual_subscription_eur:subscription,source_url:url.clone(),fetched_at:stamp.clone()});
        }
    }
    if out.is_empty() {return Err("no usable Base/HP-HC grid found on the live supplier page; no prices imported".into());}
    out.sort_by(|a,b|a.offer.cmp(&b.offer).then(a.option.cmp(&b.option)).then(a.kva.cmp(&b.kva)));
    Ok(out)
}

pub fn apply_offer(
    config:&mut crate::tariffs_fr::TariffConfig,
    contract_id:&str,
    offer:&MarketOffer,
    today:NaiveDate,
)->Result<bool,String>{
    let c=config.contracts.iter_mut().find(|c|c.id==contract_id).ok_or("unknown contract")?;
    if c.kva!=offer.kva || c.option!=offer.option {return Err("contract option or kVA does not match selected tariff".into());}
    if canon(&c.supplier)!=canon(&offer.supplier) || canon(&c.offer)!=canon(&offer.offer){return Err("contract supplier or offer does not match tariff".into());}
    if c.starts_on>today || c.ends_on.is_some_and(|d|d<today) {return Err("contract not active on the retrieved price date".into());}
    // Preserve manual corrections for the effective date, including subscriptions.
    if c.revisions.iter().any(|rev|rev.manual&&rev.from<=today&&rev.until.is_none_or(|d|today<=d)){
        return Err("manual price revision active: automatic import refused to preserve your edits".into());
    }
    if c.revisions.iter().any(|rev|rev.from>today){return Err("a future dated price revision exists; import refused".into());}
    if c.revisions.iter().any(|rev|rev.from<=today && rev.until.is_none_or(|d|today<=d)
        && !rev.manual && rev.eur_kwh==offer.eur_kwh
        && rev.annual_subscription_eur==Some(offer.annual_subscription_eur)) {return Ok(false);}
    if let Some(previous)=c.revisions.iter_mut().filter(|r|r.from<today&&r.until.is_none_or(|u|u>=today)).max_by_key(|r|r.from) {
        previous.until=today.pred_opt();
    }
    c.revisions.retain(|rev|rev.from!=today);
    c.revisions.push(crate::tariffs_fr::PriceRevision{
       from:today,until:None,eur_kwh:offer.eur_kwh.clone(),annual_subscription_eur:Some(offer.annual_subscription_eur),
       source:MARKET_SOURCE.into(),source_url:Some(offer.source_url.clone()),manual:false,retrieved_at:Some(offer.fetched_at.clone()),
    });
    c.revisions.sort_by_key(|r|r.from);
    Ok(true)
}

/// Prepare a confirmed import in memory. The caller persists the complete result
/// only after this function and tariff validation have both succeeded.
pub fn import_offer(
    config: &mut crate::tariffs_fr::TariffConfig,
    contract_id: &str,
    site_id: &str,
    offer: &MarketOffer,
    today: NaiveDate,
    replace_contract_id: Option<&str>,
) -> Result<bool, String> {
    use crate::tariffs_fr::TariffContract;
    if !config.sites.iter().any(|s| s.id == site_id) {
        return Err("unknown electric site".into());
    }
    if config.contracts.iter().any(|c| c.id == contract_id && c.site_id != site_id) {
        return Err("contract belongs to another site".into());
    }
    let active = config.contracts.iter().position(|c| c.site_id == site_id
        && c.starts_on <= today && c.ends_on.is_none_or(|end| end >= today));
    let target = config.contracts.iter().position(|c| c.id == contract_id);
    if let Some(index) = active.filter(|index| Some(*index) != target) {
        let old = &config.contracts[index];
        if replace_contract_id != Some(old.id.as_str()) {
            return Err(format!("replacement confirmation required for active contract {}", old.id));
        }
        if old.starts_on == today && !old.revisions.is_empty() {
            return Err("active contract starts today and has price history; choose a later effective date".into());
        }
        if old.revisions.is_empty() && old.starts_on == today {
            config.contracts.remove(index);
        } else {
            config.contracts[index].ends_on = today.pred_opt();
        }
    }
    if target.is_none() {
        if config.contracts.iter().any(|c| c.id == contract_id) {
            return Err("duplicate contract ID".into());
        }
        config.contracts.push(TariffContract {
            id: contract_id.into(), site_id: site_id.into(), supplier: offer.supplier.clone(),
            offer: offer.offer.clone(), option: offer.option.clone(), starts_on: today,
            ends_on: None, kva: offer.kva, offpeak: Vec::new(), revisions: Vec::new(),
        });
    }
    let changed = apply_offer(config, contract_id, offer, today)?;
    config.enabled = true;
    Ok(changed)
}

#[cfg(test)]mod tests {
    use super::*;
    const PAGE:&str=r#"<h3>Heures Eco Electricité</h3><h4>Grille tarifaire de l'offre Heures Eco Electricité de TotalEnergies en octobre 2026 Option heures pleines heures creuses</h4>
    <table><tr><th>Puissance</th><th>Abo. annuel</th><th>Heures Pleines (€/kWh)</th><th>Heures Creuses (€/kWh)</th></tr>
    <tr><td>6 kVA Votre profil</td><td>190,32 €/an</td><td>0,2142 €</td><td>0,1589 €</td></tr></table>
    <h4>Grille tarifaire de l'offre Heures Eco Electricité de TotalEnergies en octobre 2026 Option base</h4>
    <table><tr><th>Puissance</th><th>Abo. annuel</th><th>Base (€/kWh)</th></tr>
    <tr><td>6 kVA</td><td>190,32 €/an</td><td>0,2001 €</td></tr></table>
    <h3>Charge'Heures</h3><table><tr><th>Puissance</th><th>Abo. annuel</th><th>Heures Pleines (€/kWh)</th><th>Heures Creuses (€/kWh)</th><th>Heures Super Creuses</th></tr>
    <tr><td>6 kVA</td><td>188,28 €/an</td><td>0,2302 €</td><td>0,1576 €</td><td>0,1335 €</td></tr></table>"#;
    #[test] fn parses_realistic_catalogue_grid_and_skips_special_bands(){
        let offers=parse_offers("totalenergies",PAGE).unwrap();assert_eq!(offers.len(),2);
        assert_eq!(offers[0].eur_kwh["base"],0.2001);
        assert_eq!(offers[1].eur_kwh["hp"],0.2142);
        assert_eq!(offers[1].eur_kwh["hc"],0.1589);
        assert_eq!(offers[1].annual_subscription_eur,190.32);
    }
    #[test] fn forbids_invalid_slugs(){assert!(tariff_url("../../evil").is_err());assert!(tariff_url("totalenergies").is_ok());}
    #[test] fn supplier_list_from_links(){let v=parse_suppliers(r#"<a href="/fournisseurs/totalenergies">TotalEnergies</a><a href="/fournisseurs/edf/prix">EDF</a>"#);assert_eq!(v.len(),2);}
    fn selected(option:&str)->MarketOffer {
        MarketOffer{supplier:"TotalEnergies".into(),supplier_slug:"totalenergies".into(),offer:"Access Électricité".into(),
            option:option.into(),kva:6,eur_kwh:if option=="base" {BTreeMap::from([("base".into(),0.2)])}
                else {BTreeMap::from([("hp".into(),0.23),("hc".into(),0.17)])},
            annual_subscription_eur:190.0,source_url:tariff_url("totalenergies").unwrap(),fetched_at:"2026-10-11T10:00:00Z".into()}
    }
    #[test] fn imports_base_and_hphc_without_guessing_meter_hours(){
        let today=NaiveDate::from_ymd_opt(2026,10,11).unwrap();
        for option in ["base","hphc"] {
            let mut config=crate::tariffs_fr::TariffConfig::default();
            assert!(import_offer(&mut config,"new","local",&selected(option),today,None).unwrap());
            let contract=&config.contracts[0];
            assert_eq!(contract.kva,6);assert_eq!(contract.option,option);
            assert!(contract.offpeak.is_empty());assert_eq!(contract.revisions[0].eur_kwh,selected(option).eur_kwh);
            let db=rusqlite::Connection::open_in_memory().unwrap();
            crate::tariffs_fr::save(&db,&config).unwrap();
            assert_eq!(crate::tariffs_fr::load(&db).unwrap().contracts[0].offer,"Access Électricité");
        }
    }
    #[test] fn replacement_requires_confirmation_and_preserves_dated_history(){
        let today=NaiveDate::from_ymd_opt(2026,10,11).unwrap();
        let mut config=crate::tariffs_fr::TariffConfig::default();
        import_offer(&mut config,"old","local",&selected("base"),today.pred_opt().unwrap(),None).unwrap();
        let before=serde_json::to_string(&config).unwrap();
        assert!(import_offer(&mut config,"new","local",&selected("hphc"),today,None).is_err());
        assert_eq!(serde_json::to_string(&config).unwrap(),before);
        assert!(import_offer(&mut config,"new","local",&selected("hphc"),today,Some("old")).unwrap());
        assert_eq!(config.contracts[0].ends_on,today.pred_opt());
        assert_eq!(config.contracts[0].revisions[0].eur_kwh["base"],0.2);
        let db=rusqlite::Connection::open_in_memory().unwrap();
        crate::tariffs_fr::save(&db,&config).unwrap();
    }
    #[test] fn confirmed_empty_contract_is_replaced_without_a_ghost_active_contract(){
        let today=NaiveDate::from_ymd_opt(2026,10,11).unwrap();
        let mut config=crate::tariffs_fr::TariffConfig::default();
        import_offer(&mut config,"old","local",&selected("base"),today,None).unwrap();
        config.contracts[0].revisions.clear();
        assert!(import_offer(&mut config,"new","local",&selected("hphc"),today,Some("old")).unwrap());
        assert_eq!(config.contracts.len(),1);
        assert_eq!(config.contracts[0].id,"new");
        let db=rusqlite::Connection::open_in_memory().unwrap();
        crate::tariffs_fr::save(&db,&config).unwrap();
    }
    #[test] fn same_day_history_blocks_replacement_without_changing_config(){
        let today=NaiveDate::from_ymd_opt(2026,10,11).unwrap();
        let mut config=crate::tariffs_fr::TariffConfig::default();
        import_offer(&mut config,"old","local",&selected("base"),today,None).unwrap();
        let before=serde_json::to_string(&config).unwrap();
        assert!(import_offer(&mut config,"new","local",&selected("hphc"),today,Some("old")).is_err());
        assert_eq!(serde_json::to_string(&config).unwrap(),before);
    }
}
