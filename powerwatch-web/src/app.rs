use crate::state::AppState;
use crate::suggestions::{ApplyRequest, ApplyResponse, Proposal};
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::middleware;
use axum::response::{Html, IntoResponse};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use powerwatch_core::json_snapshot::{build_json_snapshot, JsonSnapshot};
use powerwatch_core::model::{Component, SensorReading};
use powerwatch_core::storage::AggregatedReading;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

fn convert_rss_to_mb(output: &str) -> String {
    let mut result = String::new();
    for (i, line) in output.lines().enumerate() {
        if i == 0 {
            result.push_str("    PID %CPU %MEM MEM(MB) COMMAND\n");
        } else {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 {
                let pid = parts[0];
                let cpu = parts[1];
                let mem = parts[2];
                let rss_kb: u64 = parts[3].parse().unwrap_or(0);
                let rss_mb = rss_kb / 1024;
                let comm = parts[4];
                result.push_str(&format!(
                    "{:>6} {:>5} {:>5} {:>7} {}\n",
                    pid, cpu, mem, rss_mb, comm
                ));
            } else {
                result.push_str(line);
            }
        }
    }
    result
}

const INDEX_HTML: &str = include_str!("../static/index.html");
const I18N_JS: &str = include_str!("../static/i18n.js");
const AUTH_JS: &str = include_str!("../static/auth.js");
const SECURITY_HTML: &str = include_str!("../static/security.html");

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct InstanceSettings {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryModuleInfo {
    locator: Option<String>,
    bank_locator: Option<String>,
    size_bytes: u64,
    memory_type: Option<String>,
    form_factor: Option<String>,
    speed: Option<String>,
    manufacturer: Option<String>,
    part_number: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct PowerSupplyInfo {
    name: Option<String>,
    manufacturer: Option<String>,
    model: Option<String>,
    location: Option<String>,
    status: Option<String>,
    supply_type: Option<String>,
    max_power_watts: Option<u32>,
}

#[derive(Debug, Clone, Default)]
struct HardwareInfo {
    cpu_model: Option<String>,
    memory_total_bytes: Option<u64>,
    memory_installed_bytes: Option<u64>,
    memory_modules: Vec<MemoryModuleInfo>,
    power_supplies: Vec<PowerSupplyInfo>,
    smbios_available: bool,
}

#[derive(Debug, Clone, Serialize)]
struct InstanceInfo {
    name: String,
    cpu_model: Option<String>,
    memory_total_bytes: Option<u64>,
    memory_installed_bytes: Option<u64>,
    memory_modules: Vec<MemoryModuleInfo>,
    power_supplies: Vec<PowerSupplyInfo>,
    smbios_available: bool,
}

#[derive(Debug, Deserialize)]
struct InstanceUpdate {
    name: String,
}

#[derive(Serialize)]
struct WebSnapshot {
    #[serde(flatten)]
    snapshot: JsonSnapshot,
    system: InstanceInfo,
}

fn instance_settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/powerwatch/instance.json")
}

fn load_instance_settings() -> InstanceSettings {
    std::fs::read_to_string(instance_settings_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn normalize_instance_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.chars().count() > 80 {
        return Err("instance name must be at most 80 characters".to_string());
    }
    if name.chars().any(char::is_control) {
        return Err("instance name cannot contain control characters".to_string());
    }
    Ok(name.to_string())
}

fn save_instance_settings(settings: &InstanceSettings) -> Result<(), String> {
    let path = instance_settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    std::fs::write(&tmp, data).map_err(|error| error.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|error| error.to_string())
}

fn parse_cpu_model(cpuinfo: &str) -> Option<String> {
    for key in ["model name", "Hardware", "Model"] {
        if let Some(value) = cpuinfo.lines().find_map(|line| {
            let (candidate, value) = line.split_once(':')?;
            (candidate.trim() == key)
                .then(|| value.trim().to_string())
                .filter(|value| !value.is_empty())
        }) {
            return Some(value);
        }
    }

    cpuinfo.lines().find_map(|line| {
        let (candidate, value) = line.split_once(':')?;
        if candidate.trim() != "Processor" {
            return None;
        }
        let value = value.trim();
        (!value.is_empty() && !value.chars().all(|c| c.is_ascii_digit())).then(|| value.to_string())
    })
}

fn read_cpu_model() -> Option<String> {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    parse_cpu_model(&cpuinfo)
}

fn parse_mem_total_bytes(meminfo: &str) -> Option<u64> {
    let value_kib = meminfo.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.trim() != "MemTotal" {
            return None;
        }
        value.split_whitespace().next()?.parse::<u64>().ok()
    })?;
    value_kib.checked_mul(1024)
}

fn read_memory_total_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_mem_total_bytes(&meminfo)
}

fn clean_dmi_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let lower = value.to_ascii_lowercase();
    // Ignore firmware placeholders. These are not real PSU or DIMM identities.
    if lower.starts_with("oem define")
        || lower.starts_with("default string")
        || lower.starts_with("to be filled by")
    {
        return None;
    }
    if matches!(
        lower.as_str(),
        "unknown"
            | "not specified"
            | "not provided"
            | "none"
            | "no module installed"
            | "to be filled by o.e.m."
            | "to be filled by oem"
            | "not applicable"
            | "n/a"
            | "oem"
    ) {
        return None;
    }

    Some(value.to_string())
}

fn parse_dmi_sections(output: &str, heading: &str) -> Vec<HashMap<String, String>> {
    output
        .split("\n\n")
        .filter_map(|block| {
            if !block.lines().any(|line| line.trim() == heading) {
                return None;
            }

            let mut fields = HashMap::new();
            for line in block.lines() {
                let line = line.trim();
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                let key = key.trim();
                if key.is_empty() {
                    continue;
                }
                fields.insert(key.to_string(), value.trim().to_string());
            }
            Some(fields)
        })
        .collect()
}

fn parse_capacity_bytes(value: &str) -> Option<u64> {
    let mut parts = value.split_whitespace();
    let amount = parts.next()?.parse::<u64>().ok()?;
    let unit = parts.next().unwrap_or("B").to_ascii_uppercase();
    let multiplier = match unit.as_str() {
        "B" => 1,
        "KB" | "KIB" => 1u64 << 10,
        "MB" | "MIB" => 1u64 << 20,
        "GB" | "GIB" => 1u64 << 30,
        "TB" | "TIB" => 1u64 << 40,
        _ => return None,
    };
    amount.checked_mul(multiplier)
}

fn parse_power_watts(value: &str) -> Option<u32> {
    value.split_whitespace().next()?.parse::<u32>().ok()
}

#[derive(Debug, Clone)]
struct RawSmbiosStructure {
    kind: u8,
    formatted: Vec<u8>,
    strings: Vec<String>,
}

fn le_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn le_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn parse_raw_smbios_structures(table: &[u8]) -> Vec<RawSmbiosStructure> {
    let mut structures = Vec::new();
    let mut offset = 0usize;

    while offset + 4 <= table.len() {
        let kind = table[offset];
        let length = table[offset + 1] as usize;
        if length < 4 || offset + length > table.len() {
            break;
        }

        let strings_start = offset + length;
        let mut strings_end = strings_start;
        while strings_end + 1 < table.len()
            && !(table[strings_end] == 0 && table[strings_end + 1] == 0)
        {
            strings_end += 1;
        }
        if strings_end + 1 >= table.len() {
            break;
        }

        let strings = if strings_end == strings_start {
            Vec::new()
        } else {
            table[strings_start..strings_end]
                .split(|byte| *byte == 0)
                .filter(|value| !value.is_empty())
                .map(|value| String::from_utf8_lossy(value).trim().to_string())
                .collect()
        };

        structures.push(RawSmbiosStructure {
            kind,
            formatted: table[offset..offset + length].to_vec(),
            strings,
        });

        offset = strings_end + 2;
        if kind == 127 {
            break;
        }
    }

    structures
}

fn raw_smbios_string(record: &RawSmbiosStructure, offset: usize) -> Option<String> {
    let index = *record.formatted.get(offset)? as usize;
    if index == 0 {
        return None;
    }
    record
        .strings
        .get(index - 1)
        .and_then(|value| clean_dmi_value(value))
}

fn raw_memory_size_bytes(record: &RawSmbiosStructure) -> Option<u64> {
    let size = le_u16(&record.formatted, 0x0C)?;
    match size {
        0 | 0xFFFF => None,
        0x7FFF => le_u32(&record.formatted, 0x1C)
            .filter(|value| *value > 0)
            .map(|value| u64::from(value) << 20),
        value if value & 0x8000 != 0 => Some(u64::from(value & 0x7FFF) << 10),
        value => Some(u64::from(value) << 20),
    }
}

fn raw_memory_form_factor(code: u8) -> Option<String> {
    let value = match code {
        0x03 => "SIMM",
        0x04 => "SIP",
        0x05 => "Chip",
        0x06 => "DIP",
        0x07 => "ZIP",
        0x08 => "Proprietary Card",
        0x09 => "DIMM",
        0x0A => "TSOP",
        0x0B => "Row of chips",
        0x0C => "RIMM",
        0x0D => "SODIMM",
        0x0E => "SRIMM",
        0x0F => "FB-DIMM",
        0x10 => "Die",
        _ => return None,
    };
    Some(value.to_string())
}

fn raw_memory_type(code: u8) -> Option<String> {
    let value = match code {
        0x03 => "DRAM",
        0x07 => "RAM",
        0x0F => "SDRAM",
        0x11 => "RDRAM",
        0x12 => "DDR",
        0x13 => "DDR2",
        0x14 => "DDR2 FB-DIMM",
        0x18 => "DDR3",
        0x19 => "FBD2",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x1F => "Logical non-volatile device",
        0x20 => "HBM",
        0x21 => "HBM2",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        0x24 => "HBM3",
        0x25 => "MRDIMM",
        _ => return None,
    };
    Some(value.to_string())
}

fn raw_memory_speed(record: &RawSmbiosStructure) -> Option<String> {
    let configured = le_u16(&record.formatted, 0x20);
    let configured = match configured {
        Some(0xFFFF) => le_u32(&record.formatted, 0x58).map(u64::from),
        Some(value) if value > 0 => Some(u64::from(value)),
        _ => None,
    };

    let maximum = match le_u16(&record.formatted, 0x15) {
        Some(0xFFFF) => le_u32(&record.formatted, 0x54).map(u64::from),
        Some(value) if value > 0 => Some(u64::from(value)),
        _ => None,
    };

    configured
        .or(maximum)
        .filter(|value| *value > 0)
        .map(|value| format!("{value} MT/s"))
}

fn parse_raw_memory_modules(table: &[u8]) -> Vec<MemoryModuleInfo> {
    parse_raw_smbios_structures(table)
        .into_iter()
        .filter(|record| record.kind == 17)
        .filter_map(|record| {
            let size_bytes = raw_memory_size_bytes(&record)?;
            Some(MemoryModuleInfo {
                locator: raw_smbios_string(&record, 0x10),
                bank_locator: raw_smbios_string(&record, 0x11),
                size_bytes,
                memory_type: record
                    .formatted
                    .get(0x12)
                    .and_then(|code| raw_memory_type(*code)),
                form_factor: record
                    .formatted
                    .get(0x0E)
                    .and_then(|code| raw_memory_form_factor(*code)),
                speed: raw_memory_speed(&record),
                manufacturer: raw_smbios_string(&record, 0x17),
                part_number: raw_smbios_string(&record, 0x1A),
            })
        })
        .collect()
}

fn raw_power_supply_status(characteristics: u16) -> Option<String> {
    let status = match (characteristics >> 7) & 0x7 {
        0x01 => Some("Other"),
        0x02 => Some("Unknown"),
        0x03 => Some("OK"),
        0x04 => Some("Non-critical"),
        0x05 => Some("Critical"),
        _ => None,
    };
    let present = characteristics & (1 << 1) != 0;

    match (present, status) {
        (true, Some(status)) => Some(format!("Present, {status}")),
        (false, Some(status)) => Some(format!("Not Present, {status}")),
        (true, None) => Some("Present".to_string()),
        (false, None) => None,
    }
}

fn raw_power_supply_type(characteristics: u16) -> Option<String> {
    let value = match (characteristics >> 10) & 0xF {
        0x01 => "Other",
        0x02 => "Unknown",
        0x03 => "Linear",
        0x04 => "Switching",
        0x05 => "Battery",
        0x06 => "UPS",
        0x07 => "Converter",
        0x08 => "Regulator",
        _ => return None,
    };
    Some(value.to_string())
}

// SMBIOS Type 39 is optional and often filled with generic placeholders.
// A status or supply type by itself does not identify an actual PSU.
fn credible_power_supply(supply: &PowerSupplyInfo) -> bool {
    if supply
        .status
        .as_deref()
        .is_some_and(|status| status.starts_with("Not Present"))
    {
        return false;
    }
    let identified = supply.manufacturer.is_some() || supply.model.is_some();
    let plausible_rating = supply
        .max_power_watts
        .is_some_and(|watts| (20..=3000).contains(&watts));
    identified || plausible_rating
}

fn parse_raw_power_supplies(table: &[u8]) -> Vec<PowerSupplyInfo> {
    parse_raw_smbios_structures(table)
        .into_iter()
        .filter(|record| record.kind == 39)
        .filter_map(|record| {
            let max_power_watts = le_u16(&record.formatted, 0x0C)
                .filter(|value| *value != 0 && *value != 0x8000)
                .map(u32::from);
            let characteristics = le_u16(&record.formatted, 0x0E);
            // Firmware may describe an unpopulated PSU bay: never count it.
            if characteristics.is_some_and(|bits| bits & (1 << 1) == 0) {
                return None;
            }

            let supply = PowerSupplyInfo {
                name: raw_smbios_string(&record, 0x06),
                manufacturer: raw_smbios_string(&record, 0x07),
                model: raw_smbios_string(&record, 0x0A),
                location: raw_smbios_string(&record, 0x05),
                status: characteristics.and_then(raw_power_supply_status),
                supply_type: characteristics.and_then(raw_power_supply_type),
                max_power_watts,
            };

            credible_power_supply(&supply).then_some(supply)
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn read_raw_smbios_table() -> Option<Vec<u8>> {
    let mut candidates = Vec::new();
    if let Ok(path) = std::env::var("POWERWATCH_DMI_TABLE_PATH") {
        candidates.push(PathBuf::from(path));
    }
    candidates.push(PathBuf::from("/host-sys-firmware/dmi/tables/DMI"));
    candidates.push(PathBuf::from("/sys/firmware/dmi/tables/DMI"));

    for path in candidates {
        if let Ok(table) = std::fs::read(&path) {
            if !table.is_empty() {
                return Some(table);
            }
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn read_raw_smbios_table() -> Option<Vec<u8>> {
    None
}

fn parse_memory_modules(output: &str) -> Vec<MemoryModuleInfo> {
    parse_dmi_sections(output, "Memory Device")
        .into_iter()
        .filter_map(|fields| {
            let size_bytes = fields
                .get("Size")
                .and_then(|value| parse_capacity_bytes(value))
                .filter(|size| *size > 0)?;
            let speed = fields
                .get("Configured Memory Speed")
                .or_else(|| fields.get("Speed"))
                .or_else(|| fields.get("Configured Clock Speed"))
                .and_then(|value| clean_dmi_value(value));

            Some(MemoryModuleInfo {
                locator: fields
                    .get("Locator")
                    .and_then(|value| clean_dmi_value(value)),
                bank_locator: fields
                    .get("Bank Locator")
                    .and_then(|value| clean_dmi_value(value)),
                size_bytes,
                memory_type: fields.get("Type").and_then(|value| clean_dmi_value(value)),
                form_factor: fields
                    .get("Form Factor")
                    .and_then(|value| clean_dmi_value(value)),
                speed,
                manufacturer: fields
                    .get("Manufacturer")
                    .and_then(|value| clean_dmi_value(value)),
                part_number: fields
                    .get("Part Number")
                    .and_then(|value| clean_dmi_value(value)),
            })
        })
        .collect()
}

fn parse_power_supplies(output: &str) -> Vec<PowerSupplyInfo> {
    parse_dmi_sections(output, "System Power Supply")
        .into_iter()
        .filter_map(|fields| {
            let supply = PowerSupplyInfo {
                name: fields.get("Name").and_then(|value| clean_dmi_value(value)),
                manufacturer: fields
                    .get("Manufacturer")
                    .and_then(|value| clean_dmi_value(value)),
                model: fields
                    .get("Model Part Number")
                    .and_then(|value| clean_dmi_value(value)),
                location: fields
                    .get("Location")
                    .and_then(|value| clean_dmi_value(value)),
                status: fields
                    .get("Status")
                    .and_then(|value| clean_dmi_value(value)),
                supply_type: fields.get("Type").and_then(|value| clean_dmi_value(value)),
                max_power_watts: fields
                    .get("Max Power Capacity")
                    .and_then(|value| parse_power_watts(value)),
            };

            credible_power_supply(&supply).then_some(supply)
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn read_dmidecode_type(dmi_type: &str) -> Option<String> {
    let output = std::process::Command::new("dmidecode")
        .args(["--type", dmi_type])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(not(target_os = "linux"))]
fn read_dmidecode_type(_dmi_type: &str) -> Option<String> {
    None
}

fn read_hardware_info() -> HardwareInfo {
    let raw_table = read_raw_smbios_table();

    let (memory_modules, power_supplies, smbios_available) =
        if let Some(table) = raw_table.as_deref() {
            (
                parse_raw_memory_modules(table),
                parse_raw_power_supplies(table),
                true,
            )
        } else {
            let memory_output = read_dmidecode_type("17");
            let power_output = read_dmidecode_type("39");
            let smbios_available = memory_output.is_some() || power_output.is_some();
            (
                memory_output
                    .as_deref()
                    .map(parse_memory_modules)
                    .unwrap_or_default(),
                power_output
                    .as_deref()
                    .map(parse_power_supplies)
                    .unwrap_or_default(),
                smbios_available,
            )
        };

    let memory_installed_bytes = (!memory_modules.is_empty()).then(|| {
        memory_modules.iter().fold(0u64, |total, module| {
            total.saturating_add(module.size_bytes)
        })
    });

    HardwareInfo {
        cpu_model: read_cpu_model(),
        memory_total_bytes: read_memory_total_bytes(),
        memory_installed_bytes,
        memory_modules,
        power_supplies,
        smbios_available,
    }
}

static HARDWARE_INFO: OnceLock<HardwareInfo> = OnceLock::new();

fn hardware_info() -> &'static HardwareInfo {
    HARDWARE_INFO.get_or_init(read_hardware_info)
}

fn instance_info() -> InstanceInfo {
    let settings = load_instance_settings();
    let hardware = hardware_info();
    InstanceInfo {
        name: settings.name,
        cpu_model: hardware.cpu_model.clone(),
        memory_total_bytes: hardware.memory_total_bytes,
        memory_installed_bytes: hardware.memory_installed_bytes,
        memory_modules: hardware.memory_modules.clone(),
        power_supplies: hardware.power_supplies.clone(),
        smbios_available: hardware.smbios_available,
    }
}

pub fn build_router(state: AppState) -> Router {
    let protected = Router::new()
        .route(
            "/api/alerts",
            get(crate::alerts::overview).put(crate::alerts::update),
        )
        .route("/api/alerts/test", post(crate::alerts::test_notification))
        .route("/api/instance", get(instance).put(update_instance))
        .route("/api/snapshot", get(snapshot))
        .route("/api/history", get(history))
        .route("/api/history/range", get(history_range))
        .route("/api/energy",get(energy_overview))
        .route("/api/suggestions", get(suggestions_list))
        .route("/api/suggestions/apply", post(suggestions_apply))
        .route("/api/processes/top", get(top_processes))
        .route("/api/auth/logout", post(crate::auth::logout))
        .route("/api/auth/settings", get(crate::auth::security_settings))
        .route("/api/auth/password", post(crate::auth::change_password))
        .route(
            "/api/auth/sessions/revoke",
            post(crate::auth::revoke_sessions),
        )
        .route("/api/auth/tokens", post(crate::auth::create_api_token))
        .route(
            "/api/auth/tokens/:id",
            delete(crate::auth::revoke_api_token),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_auth,
        ));

    Router::new()
        .route("/", get(index))
        .route("/static/i18n.js", get(i18n_js))
        .route("/static/auth.js", get(auth_js))
        .route("/alerts", get(crate::alerts::page))
        .route("/security", get(security_page))
        .route("/api/health", get(health))
        .route("/api/auth/status", get(crate::auth::status))
        .route("/api/auth/setup", post(crate::auth::setup))
        .route("/api/auth/login", post(crate::auth::login))
        .merge(protected)
        .with_state(state)
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
}

async fn index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Html(INDEX_HTML),
    )
}

async fn i18n_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        I18N_JS,
    )
}

async fn auth_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        AUTH_JS,
    )
}

async fn security_page() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Html(SECURITY_HTML),
    )
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn instance() -> Json<InstanceInfo> {
    Json(instance_info())
}

async fn update_instance(
    Json(request): Json<InstanceUpdate>,
) -> Result<Json<InstanceInfo>, (StatusCode, String)> {
    let name =
        normalize_instance_name(&request.name).map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    save_instance_settings(&InstanceSettings { name })
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    Ok(Json(instance_info()))
}

async fn snapshot(State(state): State<AppState>) -> Json<WebSnapshot> {
    let snapshot = state.latest_snapshot.read().unwrap();
    Json(WebSnapshot {
        snapshot: build_json_snapshot(&snapshot),
        system: instance_info(),
    })
}

#[derive(Deserialize)]
struct HistoryParams {
    since: Option<String>,
}

async fn history(
    State(state): State<AppState>,
    Query(params): Query<HistoryParams>,
) -> Result<Json<Vec<SensorReading>>, (StatusCode, String)> {
    let Some(since_text) = params.since else {
        return Err((
            StatusCode::BAD_REQUEST,
            "missing ?since=... e.g. ?since=1h".to_string(),
        ));
    };

    let lookback = powerwatch_core::duration::parse_duration(&since_text)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let cutoff = chrono::Utc::now()
        - chrono::Duration::from_std(lookback).unwrap_or(chrono::Duration::zero());

    let storage_guard = state.storage.lock().unwrap();
    let readings = match storage_guard.as_ref() {
        Some(storage) => storage
            .readings_since(cutoff)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")))?,
        None => Vec::new(),
    };

    Ok(Json(readings))
}

#[derive(Deserialize)]
struct RangeHistoryParams {
    amount: Option<u32>,
    unit: Option<String>,
    max_points: Option<u32>,
}

#[derive(Serialize)]
struct HistorySummary {
    avg_watts: f64,
    min_watts: f64,
    max_watts: f64,
    energy_kwh: f64,
    samples: u64,
}

#[derive(Serialize)]
struct RangeHistoryResponse {
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    bucket_seconds: u64,
    summary: Option<HistorySummary>,
    points: Vec<AggregatedReading>,
}

fn range_start(
    now: chrono::DateTime<chrono::Utc>,
    amount: u32,
    unit: &str,
) -> Result<chrono::DateTime<chrono::Utc>, String> {
    if amount == 0 || amount > 10_000 {
        return Err("amount must be between 1 and 10000".to_string());
    }

    match unit.trim().to_ascii_lowercase().as_str() {
        "minute" | "minutes" | "min" => Ok(now - chrono::Duration::minutes(amount as i64)),
        "hour" | "hours" | "h" => Ok(now - chrono::Duration::hours(amount as i64)),
        "day" | "days" | "d" => Ok(now - chrono::Duration::days(amount as i64)),
        "week" | "weeks" | "w" => Ok(now - chrono::Duration::weeks(amount as i64)),
        "month" | "months" => now
            .checked_sub_months(chrono::Months::new(amount))
            .ok_or_else(|| "requested month range is out of bounds".to_string()),
        "year" | "years" | "y" => {
            let months = amount
                .checked_mul(12)
                .ok_or_else(|| "requested year range is too large".to_string())?;
            now.checked_sub_months(chrono::Months::new(months))
                .ok_or_else(|| "requested year range is out of bounds".to_string())
        }
        other => Err(format!(
            "unsupported unit '{other}' (use minutes, hours, days, weeks, months or years)"
        )),
    }
}

fn choose_bucket_seconds(range_seconds: u64, max_points: u32) -> u64 {
    let max_points = max_points.clamp(100, 5_000) as u64;
    let target = range_seconds.div_ceil(max_points).max(60);
    const BUCKETS: &[u64] = &[
        60, 120, 300, 600, 900, 1_800, 3_600, 7_200, 10_800, 21_600, 43_200, 86_400, 172_800,
        604_800, 2_592_000,
    ];

    BUCKETS
        .iter()
        .copied()
        .find(|bucket| *bucket >= target)
        .unwrap_or(target)
}

fn summarize_history(points: &[AggregatedReading], bucket_seconds: u64) -> Option<HistorySummary> {
    let totals: Vec<_> = points
        .iter()
        .filter(|point| point.component == Component::Total)
        .collect();

    if totals.is_empty() {
        return None;
    }

    let samples: u64 = totals.iter().map(|point| point.samples).sum();
    let weighted_sum: f64 = totals
        .iter()
        .map(|point| point.avg_watts * point.samples as f64)
        .sum();
    let avg_watts = if samples > 0 {
        weighted_sum / samples as f64
    } else {
        0.0
    };
    let min_watts = totals
        .iter()
        .map(|point| point.min_watts)
        .fold(f64::INFINITY, f64::min);
    let max_watts = totals
        .iter()
        .map(|point| point.max_watts)
        .fold(f64::NEG_INFINITY, f64::max);

    let mut sorted_totals = totals;
    sorted_totals.sort_by_key(|point| point.timestamp);
    let mut energy_kwh = 0.0;
    for pair in sorted_totals.windows(2) {
        let previous = pair[0];
        let next = pair[1];
        let delta_seconds = (next.timestamp - previous.timestamp).num_seconds();
        if delta_seconds <= 0 {
            continue;
        }
        let covered_seconds = (delta_seconds as u64).min(bucket_seconds);
        let average_watts = (previous.avg_watts + next.avg_watts) / 2.0;
        energy_kwh += average_watts * covered_seconds as f64 / 3_600_000.0;
    }

    Some(HistorySummary {
        avg_watts,
        min_watts,
        max_watts,
        energy_kwh,
        samples,
    })
}

async fn history_range(
    State(state): State<AppState>,
    Query(params): Query<RangeHistoryParams>,
) -> Result<Json<RangeHistoryResponse>, (StatusCode, String)> {
    let amount = params.amount.unwrap_or(24);
    let unit = params.unit.as_deref().unwrap_or("hours");
    let max_points = params.max_points.unwrap_or(1_200).clamp(100, 5_000);

    let to = chrono::Utc::now();
    let from = range_start(to, amount, unit).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let range_seconds = (to - from).num_seconds().max(1) as u64;
    let bucket_seconds = choose_bucket_seconds(range_seconds, max_points);

    let storage_guard = state.storage.lock().unwrap();
    let points = match storage_guard.as_ref() {
        Some(storage) => storage
            .aggregated_since(from, bucket_seconds)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")))?,
        None => Vec::new(),
    };

    let summary = summarize_history(&points, bucket_seconds);

    Ok(Json(RangeHistoryResponse {
        from,
        to,
        bucket_seconds,
        summary,
        points,
    }))
}


#[derive(Deserialize)]
struct EnergyParams {
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize)]
struct EnergyResponse {
    windows: HashMap<&'static str, powerwatch_core::energy::EnergyStats>,
    selected: powerwatch_core::energy::EnergyStats,
}

async fn energy_overview(
    State(state): State<AppState>, Query(params): Query<EnergyParams>,
) -> Result<Json<EnergyResponse>, (StatusCode,String)> {
    let now=chrono::Utc::now().timestamp();
    let to=params.to.map_or(now,|v|v.timestamp());
    let selected_from=params.from.map(|v|v.timestamp());
    if to>now+60 || selected_from.is_some_and(|start|start>=to) {
        return Err((StatusCode::BAD_REQUEST,"invalid energy period".into()));
    }
    let storage=state.storage.lock().unwrap();
    let Some(db)=storage.as_ref() else {
        return Err((StatusCode::SERVICE_UNAVAILABLE,"history storage unavailable".into()));
    };
    let fetch=|from,until|db.energy_stats(from,until)
        .map_err(|error|(StatusCode::INTERNAL_SERVER_ERROR,format!("{error:?}")));
    let mut windows=HashMap::new();
    windows.insert("24h",fetch(Some(now-86400),now)?);
    windows.insert("7d",fetch(Some(now-7*86400),now)?);
    windows.insert("30d",fetch(Some(now-30*86400),now)?);
    windows.insert("all",fetch(None,now)?);
    let selected=fetch(selected_from,to)?;
    Ok(Json(EnergyResponse{windows,selected}))
}

async fn suggestions_list(State(state): State<AppState>) -> Json<Vec<Proposal>> {
    let suggestions = state.suggestions;
    Json(suggestions.list_with_tokens())
}

async fn suggestions_apply(
    State(state): State<AppState>,
    Json(req): Json<ApplyRequest>,
) -> Result<Json<ApplyResponse>, (StatusCode, String)> {
    let suggestions = state.suggestions;
    let token = req.token;
    if token.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "missing token".to_string()));
    }

    match suggestions.apply(&token) {
        Ok(true) => Ok(Json(ApplyResponse {
            success: true,
            message: "action applied".to_string(),
        })),
        Ok(false) => Err((
            StatusCode::CONFLICT,
            "suggestion was already applied".to_string(),
        )),
        Err(e) => Err((StatusCode::BAD_REQUEST, e)),
    }
}

async fn top_processes() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("sh")
            .args(["-c", "ps -eo pid,pcpu,pmem,rss,comm --sort=-%cpu | head -6"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                let converted = convert_rss_to_mb(&text);
                Ok(Json(serde_json::json!({ "output": converted })))
            }
            Ok(out) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("command failed: {}", out.status),
            )),
            Err(e) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to run command: {}", e),
            )),
        }
    }

    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("sh")
            .args(&["-c", "ps -eo pid,pcpu,pmem,rss,comm -m | head -6"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                let converted = convert_rss_to_mb(&text);
                Ok(Json(serde_json::json!({ "output": converted })))
            }
            Ok(out) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("command failed: {}", out.status),
            )),
            Err(e) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to run command: {}", e),
            )),
        }
    }

    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("powershell")
            .args(&["-command", "Get-Process | Sort-Object CPU -Descending | Select-Object -First 5 Id,CPU,WorkingSet,ProcessName | Format-Table -AutoSize"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                Ok(Json(serde_json::json!({ "output": text })))
            }
            Ok(out) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("command failed: {}", out.status),
            )),
            Err(e) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to run command: {}", e),
            )),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "not supported on this platform".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use powerwatch_core::model::{Component, Confidence};
    use powerwatch_core::sampler::Snapshot;
    use powerwatch_core::storage::Storage;
    use std::sync::{Arc, Mutex, RwLock};
    use tower::ServiceExt;

    fn test_state_with(snapshot: Snapshot) -> AppState {
        AppState {
            latest_snapshot: Arc::new(RwLock::new(snapshot)),
            storage: Arc::new(Mutex::new(None)),
            suggestions: crate::suggestions::SuggestionsState::new(),
            alerts: crate::alerts::AlertService::memory(),
            auth: crate::auth::AuthService::disabled(),
        }
    }

    fn test_state_with_storage(snapshot: Snapshot, storage: Storage) -> AppState {
        AppState {
            latest_snapshot: Arc::new(RwLock::new(snapshot)),
            storage: Arc::new(Mutex::new(Some(storage))),
            suggestions: crate::suggestions::SuggestionsState::new(),
            alerts: crate::alerts::AlertService::memory(),
            auth: crate::auth::AuthService::disabled(),
        }
    }

    fn empty_snapshot() -> Snapshot {
        Snapshot {
            timestamp: chrono::Utc::now(),
            results: vec![],
        }
    }

    #[test]
    fn parses_x86_cpu_model_from_proc_cpuinfo() {
        let sample = "processor : 0\nmodel name : Intel Celeron N5105 @ 2.00GHz\n";
        assert_eq!(
            parse_cpu_model(sample).as_deref(),
            Some("Intel Celeron N5105 @ 2.00GHz")
        );
    }

    #[test]
    fn parses_linux_memtotal_as_bytes() {
        let sample = "MemTotal:       32768000 kB\nMemFree:         123456 kB\n";
        assert_eq!(parse_mem_total_bytes(sample), Some(33_554_432_000));
    }

    #[test]
    fn parses_only_populated_memory_devices() {
        let sample = r#"
Handle 0x0030, DMI type 17, 92 bytes
Memory Device
        Size: 16 GB
        Form Factor: DIMM
        Locator: DIMM_A1
        Bank Locator: BANK 0
        Type: DDR4
        Speed: 3200 MT/s
        Configured Memory Speed: 3200 MT/s
        Manufacturer: Kingston
        Part Number: KF432C16

Handle 0x0031, DMI type 17, 92 bytes
Memory Device
        Size: No Module Installed
        Locator: DIMM_A2

Handle 0x0032, DMI type 17, 92 bytes
Memory Device
        Size: 16384 MB
        Form Factor: DIMM
        Locator: DIMM_B1
        Type: DDR4
        Configured Memory Speed: 3200 MT/s
        Manufacturer: Kingston
        Part Number: KF432C16
"#;
        let modules = parse_memory_modules(sample);
        assert_eq!(modules.len(), 2);
        assert_eq!(modules[0].size_bytes, 16u64 << 30);
        assert_eq!(modules[0].locator.as_deref(), Some("DIMM_A1"));
        assert_eq!(modules[1].size_bytes, 16u64 << 30);
        assert_eq!(modules[1].locator.as_deref(), Some("DIMM_B1"));
    }

    #[test]
    fn parses_smbios_system_power_supply() {
        let sample = r#"
Handle 0x0040, DMI type 39, 22 bytes
System Power Supply
        Location: PSU Bay 1
        Name: PSU 1
        Manufacturer: ExampleCorp
        Model Part Number: PX-750
        Max Power Capacity: 750 W
        Status: Present, OK
        Type: Switching
"#;
        let supplies = parse_power_supplies(sample);
        assert_eq!(supplies.len(), 1);
        assert_eq!(supplies[0].max_power_watts, Some(750));
        assert_eq!(supplies[0].manufacturer.as_deref(), Some("ExampleCorp"));
        assert_eq!(supplies[0].model.as_deref(), Some("PX-750"));
    }

    fn smbios_record(kind: u8, length: usize, strings: &[&str]) -> Vec<u8> {
        let mut record = vec![0u8; length];
        record[0] = kind;
        record[1] = length as u8;
        for value in strings {
            record.extend_from_slice(value.as_bytes());
            record.push(0);
        }
        if strings.is_empty() {
            record.push(0);
        }
        record.push(0);
        record
    }

    #[test]
    fn parses_raw_smbios_memory_device() {
        let mut record = smbios_record(17, 0x22, &["DIMM_A1", "BANK 0", "Kingston", "KF432C16"]);
        record[0x0C..0x0E].copy_from_slice(&16384u16.to_le_bytes());
        record[0x0E] = 0x09;
        record[0x10] = 1;
        record[0x11] = 2;
        record[0x12] = 0x1A;
        record[0x15..0x17].copy_from_slice(&3200u16.to_le_bytes());
        record[0x17] = 3;
        record[0x1A] = 4;
        record[0x20..0x22].copy_from_slice(&3200u16.to_le_bytes());

        let modules = parse_raw_memory_modules(&record);
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].size_bytes, 16u64 << 30);
        assert_eq!(modules[0].locator.as_deref(), Some("DIMM_A1"));
        assert_eq!(modules[0].memory_type.as_deref(), Some("DDR4"));
        assert_eq!(modules[0].form_factor.as_deref(), Some("DIMM"));
        assert_eq!(modules[0].speed.as_deref(), Some("3200 MT/s"));
        assert_eq!(modules[0].manufacturer.as_deref(), Some("Kingston"));
    }

    #[test]
    fn ignores_empty_raw_smbios_memory_device() {
        let record = smbios_record(17, 0x22, &["DIMM_A2"]);
        assert!(parse_raw_memory_modules(&record).is_empty());
    }

    #[test]
    fn parses_raw_smbios_power_supply() {
        let mut record = smbios_record(39, 0x10, &["PSU Bay 1", "PSU 1", "ExampleCorp", "PX-750"]);
        record[0x05] = 1;
        record[0x06] = 2;
        record[0x07] = 3;
        record[0x0A] = 4;
        record[0x0C..0x0E].copy_from_slice(&750u16.to_le_bytes());
        let characteristics = (4u16 << 10) | (3u16 << 7) | (1u16 << 1);
        record[0x0E..0x10].copy_from_slice(&characteristics.to_le_bytes());

        let supplies = parse_raw_power_supplies(&record);
        assert_eq!(supplies.len(), 1);
        assert_eq!(supplies[0].max_power_watts, Some(750));
        assert_eq!(supplies[0].manufacturer.as_deref(), Some("ExampleCorp"));
        assert_eq!(supplies[0].model.as_deref(), Some("PX-750"));
        assert_eq!(supplies[0].supply_type.as_deref(), Some("Switching"));
        assert_eq!(supplies[0].status.as_deref(), Some("Present, OK"));
    }

    #[test]
    fn ignores_generic_firmware_power_supply_entries() {
        let mut record = smbios_record(
            39,
            0x10,
            &["Default string", "OEM Define 2", "Default string"],
        );
        record[0x05] = 1;
        record[0x06] = 2;
        record[0x07] = 3;
        let characteristics = (8u16 << 10) | (3u16 << 7) | (1u16 << 1);
        record[0x0E..0x10].copy_from_slice(&characteristics.to_le_bytes());
        assert!(parse_raw_power_supplies(&record).is_empty());

        let fallback = "System Power Supply\n\tName: Default string\n\tManufacturer: OEM Define 2\n\tType: Regulator\n\tStatus: Present, OK\n\n";
        assert!(parse_power_supplies(fallback).is_empty());
    }

    #[test]
    fn keeps_plausible_psu_rating_without_manufacturer() {
        let mut record = smbios_record(39, 0x10, &["OEM Define 1"]);
        record[0x05] = 1;
        record[0x0C..0x0E].copy_from_slice(&75u16.to_le_bytes());
        record[0x0E..0x10].copy_from_slice(&((3u16 << 7) | (1u16 << 1)).to_le_bytes());
        let supplies = parse_raw_power_supplies(&record);
        assert_eq!(supplies.len(), 1);
        assert_eq!(supplies[0].max_power_watts, Some(75));
        assert!(supplies[0].location.is_none());
    }

    #[test]
    fn excludes_unpopulated_psu_bays() {
        let mut record = smbios_record(39, 0x10, &["RealVendor"]);
        record[0x07] = 1;
        record[0x0C..0x0E].copy_from_slice(&750u16.to_le_bytes());
        // Present bit remains off: the bay is not populated.
        assert!(parse_raw_power_supplies(&record).is_empty());
    }

    #[test]
    fn validates_instance_name_length_and_control_characters() {
        assert_eq!(
            normalize_instance_name("  DockerLab  ").unwrap(),
            "DockerLab"
        );
        assert!(normalize_instance_name(&"x".repeat(81)).is_err());
        assert!(normalize_instance_name("bad\nname").is_err());
    }

    fn reading(watts: f64) -> SensorReading {
        SensorReading {
            component: Component::Cpu,
            watts,
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now(),
        }
    }

    async fn get_body_json(app: Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn root_route_serves_the_frontend_html() {
        let app = build_router(test_state_with(empty_snapshot()));

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .unwrap();
        assert!(content_type.to_str().unwrap().contains("text/html"));

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("PowerWatch"));
        assert!(text.contains("/api/snapshot"));
    }

    #[tokio::test]
    async fn health_check_returns_ok() {
        let app = build_router(test_state_with(empty_snapshot()));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn an_unknown_route_returns_404() {
        let app = build_router(test_state_with(empty_snapshot()));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/does-not-exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn snapshot_route_returns_the_current_state_as_json() {
        let state = test_state_with(Snapshot {
            timestamp: chrono::Utc::now(),
            results: vec![("cpu".to_string(), Ok(reading(12.5)))],
        });
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["sensors"][0]["name"], "cpu");
        assert_eq!(json["sensors"][0]["reading"]["watts"], 12.5);
    }

    #[tokio::test]
    async fn snapshot_route_reflects_the_latest_state_when_it_changes() {
        let state = test_state_with(Snapshot {
            timestamp: chrono::Utc::now(),
            results: vec![("cpu".to_string(), Ok(reading(10.0)))],
        });
        *state.latest_snapshot.write().unwrap() = Snapshot {
            timestamp: chrono::Utc::now(),
            results: vec![("cpu".to_string(), Ok(reading(99.0)))],
        };
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["sensors"][0]["reading"]["watts"], 99.0);
    }

    #[tokio::test]
    async fn history_without_since_is_a_bad_request() {
        let app = build_router(test_state_with(empty_snapshot()));

        let (status, _) = get_body_json(app, "/api/history").await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn history_with_an_invalid_since_is_a_bad_request() {
        let app = build_router(test_state_with(empty_snapshot()));

        let (status, _) = get_body_json(app, "/api/history?since=not-a-duration").await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn history_with_no_storage_returns_an_empty_list_not_an_error() {
        let app = build_router(test_state_with(empty_snapshot()));

        let (status, json) = get_body_json(app, "/api/history?since=1h").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, serde_json::json!([]));
    }

    #[tokio::test]
    async fn history_returns_readings_recorded_within_the_period() {
        let storage = Storage::open_in_memory().unwrap();
        storage.insert_reading(&reading(12.5)).unwrap();
        let state = test_state_with_storage(empty_snapshot(), storage);
        let app = build_router(state);

        let (status, json) = get_body_json(app, "/api/history?since=1h").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["watts"], 12.5);
    }

    #[tokio::test]
    async fn history_excludes_readings_older_than_the_period() {
        let storage = Storage::open_in_memory().unwrap();
        let old_reading = SensorReading {
            component: Component::Cpu,
            watts: 5.0,
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now() - chrono::Duration::hours(2),
        };
        storage.insert_reading(&old_reading).unwrap();
        let state = test_state_with_storage(empty_snapshot(), storage);
        let app = build_router(state);

        let (status, json) = get_body_json(app, "/api/history?since=1h").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, serde_json::json!([]));
    }

    #[tokio::test]
    async fn suggestions_list_returns_empty_when_no_suggestions() {
        let state = test_state_with(empty_snapshot());
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/suggestions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn suggestions_apply_without_token_is_bad_request() {
        let state = test_state_with(empty_snapshot());
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/suggestions/apply")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"token":""}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 400);
    }

    fn authenticated_test_state(path: PathBuf) -> AppState {
        let mut state = test_state_with(Snapshot {
            timestamp: chrono::Utc::now(),
            results: vec![("cpu".to_string(), Ok(reading(12.5)))],
        });
        state.auth = crate::auth::AuthService::load_with_setup_secret(
            true,
            path,
            Some("bootstrap-secret-123".to_string()),
        )
        .unwrap();
        state
    }

    async fn json_request(
        app: Router,
        method: &str,
        uri: &str,
        body: &str,
        cookie: Option<&str>,
        csrf: Option<&str>,
        bearer: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        if let Some(csrf) = csrf {
            builder = builder.header("x-csrf-token", csrf);
        }
        if let Some(token) = bearer {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        (status, headers, json)
    }

    fn cookie_from(headers: &axum::http::HeaderMap) -> String {
        headers
            .get(axum::http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn complete_authentication_and_api_token_flow() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let state = authenticated_test_state(path);

        let (status, _, body) = json_request(
            build_router(state.clone()),
            "GET",
            "/api/auth/status",
            "",
            None,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["setup_required"], true);

        let (status, _, _) = json_request(
            build_router(state.clone()),
            "GET",
            "/api/snapshot",
            "",
            None,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let wrong_setup = r#"{"username":"admin","password":"correct horse battery","setup_token":"wrong-bootstrap-token"}"#;
        let (status, _, _) = json_request(
            build_router(state.clone()),
            "POST",
            "/api/auth/setup",
            wrong_setup,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let setup = r#"{"username":"admin","password":"correct horse battery","setup_token":"bootstrap-secret-123"}"#;
        let (status, headers, body) = json_request(
            build_router(state.clone()),
            "POST",
            "/api/auth/setup",
            setup,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let cookie = cookie_from(&headers);
        let csrf = body["csrf_token"].as_str().unwrap().to_string();

        let (status, _, _) = json_request(
            build_router(state.clone()),
            "GET",
            "/api/snapshot",
            "",
            Some(&cookie),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, _, _) = json_request(
            build_router(state.clone()),
            "POST",
            "/api/auth/tokens",
            r#"{"name":"Hub"}"#,
            Some(&cookie),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _, body) = json_request(
            build_router(state.clone()),
            "POST",
            "/api/auth/tokens",
            r#"{"name":"Hub"}"#,
            Some(&cookie),
            Some(&csrf),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let api_token = body["token"].as_str().unwrap().to_string();

        let (status, _, _) = json_request(
            build_router(state.clone()),
            "GET",
            "/api/snapshot",
            "",
            None,
            None,
            Some(&api_token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = json_request(
            build_router(state.clone()),
            "GET",
            "/api/snapshot",
            "",
            None,
            None,
            Some("pw_invalid"),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _, _) = json_request(
            build_router(state.clone()),
            "POST",
            "/api/suggestions/apply",
            r#"{"token":"value"}"#,
            None,
            None,
            Some(&api_token),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _, _) = json_request(
            build_router(state.clone()),
            "POST",
            "/api/auth/logout",
            "{}",
            Some(&cookie),
            Some(&csrf),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _, _) = json_request(
            build_router(state.clone()),
            "GET",
            "/api/snapshot",
            "",
            Some(&cookie),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let wrong_login = r#"{"username":"admin","password":"wrong password value"}"#;
        let (status, _, _) = json_request(
            build_router(state.clone()),
            "POST",
            "/api/auth/login",
            wrong_login,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let login = r#"{"username":"admin","password":"correct horse battery"}"#;
        let (status, headers, body) = json_request(
            build_router(state.clone()),
            "POST",
            "/api/auth/login",
            login,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let new_cookie = cookie_from(&headers);
        let new_csrf = body["csrf_token"].as_str().unwrap();

        let (status, _, _) = json_request(
            build_router(state.clone()),
            "POST",
            "/api/auth/password",
            r#"{"current_password":"correct horse battery","new_password":"new correct horse battery"}"#,
            Some(&new_cookie),
            Some(new_csrf),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, _, _) = json_request(
            build_router(state.clone()),
            "POST",
            "/api/auth/login",
            login,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let new_login = r#"{"username":"admin","password":"new correct horse battery"}"#;
        let (status, _, _) = json_request(
            build_router(state),
            "POST",
            "/api/auth/login",
            new_login,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn persisted_session_is_valid_after_service_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let state = authenticated_test_state(path.clone());
        let setup = r#"{"username":"admin","password":"correct horse battery","setup_token":"bootstrap-secret-123"}"#;
        let (_, headers, _) = json_request(
            build_router(state),
            "POST",
            "/api/auth/setup",
            setup,
            None,
            None,
            None,
        )
        .await;
        let cookie = cookie_from(&headers);

        let mut restarted = test_state_with(empty_snapshot());
        restarted.auth =
            crate::auth::AuthService::load_with_setup_secret(true, path, None).unwrap();
        let (status, _, _) = json_request(
            build_router(restarted),
            "GET",
            "/api/snapshot",
            "",
            Some(&cookie),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
}
