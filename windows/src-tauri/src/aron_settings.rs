use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

const CREDENTIAL_SERVICE: &str = "aron";
const JEV_ACCOUNT: &str = "typesafe-jev";
const GEMINI_INDEX_ACCOUNT: &str = "gemini#index";
const GEMINI_LEGACY_ACCOUNT: &str = "gemini";
const CREDENTIAL_METADATA_ACCOUNT: &str = "__coucou_aron_labels_v1";
const MAX_SECRET_LENGTH: usize = 16_384;
const MAX_LABEL_LENGTH: usize = 80;
const ARON_SETTINGS_SCHEMA: &str = include_str!("../../src/settings/aron-settings-schema.json");
const JEV_ENV_VARS: &[&str] = &[
    "ARON_JEV_API_KEY",
    "TYPESAFE_API_KEY",
    "ARON_TYPESAFE_JEV_API_KEY",
];
const GEMINI_ENV_VARS: &[(&str, bool)] = &[
    ("ARON_GEMINI_API_KEYS", true),
    ("GEMINI_API_KEYS", true),
    ("ARON_GEMINI_API_KEY", false),
    ("GEMINI_API_KEY", false),
    ("GOOGLE_API_KEY", false),
];
const VOICES: &[&str] = &[
    "Zephyr",
    "Puck",
    "Charon",
    "Kore",
    "Fenrir",
    "Leda",
    "Orus",
    "Aoede",
    "Callirrhoe",
    "Autonoe",
    "Enceladus",
    "Iapetus",
    "Umbriel",
    "Algieba",
    "Despina",
    "Erinome",
    "Algenib",
    "Rasalgethi",
    "Laomedeia",
    "Achernar",
    "Alnilam",
    "Schedar",
    "Gacrux",
    "Pulcherrima",
    "Achird",
    "Zubenelgenubi",
    "Vindemiatrix",
    "Sadachbia",
    "Sadaltager",
    "Sulafat",
];
const THEMES: &[&str] = &["azul", "ciano", "verde", "violeta", "ambar", "vermelho"];
const ORB_STYLES: &[&str] = &["hud", "fluxo", "hibrido"];
const ENGINES: &[&str] = &["gemini-live"];
const DECISION_ENGINES: &[&str] = &["laya", "jev"];
const LOCAL_ENGINES: &[&str] = &["laya", "nimble"];
const STT_MODELS: &[&str] = &["tiny", "base", "small"];
const STT_DEVICES: &[&str] = &["auto", "cpu", "gpu"];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AronSettingsSnapshot {
    prefs: Map<String, Value>,
    secrets: Vec<AronSecretStatus>,
    revision: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AronSecretMetadata {
    id: String,
    label: String,
    last4: String,
    source: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AronSecretStatus {
    provider: String,
    configured: bool,
    entries: Vec<AronSecretMetadata>,
}

#[derive(Deserialize)]
#[serde(transparent)]
pub struct AronSettingsPatch(Map<String, Value>);

#[derive(Default, Deserialize, Serialize)]
struct CredentialLabels {
    labels: BTreeMap<String, String>,
}

struct StoredGeminiCredential {
    id: String,
    label: String,
    value: String,
    source: String,
}

trait CredentialBackend {
    fn get_password(&self, account: &str) -> Result<Option<String>, String>;
    fn set_password(&self, account: &str, password: &str) -> Result<(), String>;
    fn delete_credential(&self, account: &str) -> Result<(), String>;
}

#[cfg(any(windows, target_os = "linux"))]
struct KeyringCredentialBackend;

#[cfg(any(windows, target_os = "linux"))]
impl CredentialBackend for KeyringCredentialBackend {
    fn get_password(&self, account: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(CREDENTIAL_SERVICE, account)
            .map_err(|_| "could not access the ARON credential store".to_string())?;
        match entry.get_password() {
            Ok(password) if !password.is_empty() => Ok(Some(password)),
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("could not read from the ARON credential store".to_string()),
        }
    }

    fn set_password(&self, account: &str, password: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(CREDENTIAL_SERVICE, account)
            .map_err(|_| "could not access the ARON credential store".to_string())?;
        entry
            .set_password(password)
            .map_err(|_| "could not write to the ARON credential store".to_string())
    }

    fn delete_credential(&self, account: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(CREDENTIAL_SERVICE, account)
            .map_err(|_| "could not access the ARON credential store".to_string())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("could not remove from the ARON credential store".to_string()),
        }
    }
}

#[cfg(any(windows, target_os = "linux"))]
type ProductionCredentialBackend = KeyringCredentialBackend;

#[cfg(not(any(windows, target_os = "linux")))]
struct UnsupportedCredentialBackend;

#[cfg(not(any(windows, target_os = "linux")))]
impl CredentialBackend for UnsupportedCredentialBackend {
    fn get_password(&self, _account: &str) -> Result<Option<String>, String> {
        Err("the ARON credential store is unavailable on this platform".to_string())
    }

    fn set_password(&self, _account: &str, _password: &str) -> Result<(), String> {
        Err("the ARON credential store is unavailable on this platform".to_string())
    }

    fn delete_credential(&self, _account: &str) -> Result<(), String> {
        Err("the ARON credential store is unavailable on this platform".to_string())
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
type ProductionCredentialBackend = UnsupportedCredentialBackend;

#[cfg(any(windows, target_os = "linux"))]
fn production_backend() -> ProductionCredentialBackend {
    KeyringCredentialBackend
}

#[cfg(not(any(windows, target_os = "linux")))]
fn production_backend() -> ProductionCredentialBackend {
    UnsupportedCredentialBackend
}

static TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[tauri::command]
pub fn load_aron_settings() -> Result<AronSettingsSnapshot, String> {
    let path = resolve_prefs_path()?;
    load_snapshot_at_with_env(&path, &production_backend(), &process_environment_secrets)
}

#[tauri::command]
pub fn update_aron_settings(
    patch: AronSettingsPatch,
    revision: String,
) -> Result<AronSettingsSnapshot, String> {
    let path = resolve_prefs_path()?;
    update_snapshot_at_with_env(
        &path,
        patch.0,
        &revision,
        &production_backend(),
        &process_environment_secrets,
    )
}

#[tauri::command]
pub fn aron_secret_add(
    provider: String,
    value: String,
    label: String,
) -> Result<AronSecretStatus, String> {
    let backend = production_backend();
    add_secret(&provider, &value, &label, &backend)?;
    secret_status_with_env(&provider, &backend, &process_environment_secrets(&provider))
}

#[tauri::command]
pub fn aron_secret_remove(provider: String, id: String) -> Result<AronSecretStatus, String> {
    let backend = production_backend();
    remove_secret(&provider, &id, &backend)?;
    secret_status_with_env(&provider, &backend, &process_environment_secrets(&provider))
}

#[tauri::command]
pub fn aron_secret_status(provider: String) -> Result<AronSecretStatus, String> {
    secret_status_with_env(
        &provider,
        &production_backend(),
        &process_environment_secrets(&provider),
    )
}

fn resolve_prefs_path() -> Result<PathBuf, String> {
    resolve_prefs_path_from(
        env::var_os("ARON_HOME").as_deref(),
        env::var_os("APPDATA").as_deref(),
    )
}

fn resolve_prefs_path_from(
    aron_home: Option<&OsStr>,
    app_data: Option<&OsStr>,
) -> Result<PathBuf, String> {
    if let Some(home) = aron_home.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home).join("prefs").join("prefs.json"));
    }
    let app_data = app_data.filter(|value| !value.is_empty()).ok_or_else(|| {
        "ARON_HOME or APPDATA must be configured to locate ARON preferences".to_string()
    })?;
    Ok(PathBuf::from(app_data)
        .join("aron")
        .join("prefs")
        .join("prefs.json"))
}

fn default_prefs() -> Map<String, Value> {
    settings_schema()["properties"]
        .as_object()
        .expect("ARON settings schema has properties")
        .iter()
        .map(|(key, definition)| {
            (
                key.clone(),
                definition
                    .get("default")
                    .expect("ARON settings schema properties have defaults")
                    .clone(),
            )
        })
        .collect()
}

fn settings_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::from_str(ARON_SETTINGS_SCHEMA).expect("ARON settings schema is valid JSON")
    })
}

fn settings_keys() -> &'static BTreeSet<String> {
    static KEYS: OnceLock<BTreeSet<String>> = OnceLock::new();
    KEYS.get_or_init(|| {
        settings_schema()["properties"]
            .as_object()
            .expect("ARON settings schema has properties")
            .keys()
            .cloned()
            .collect()
    })
}

fn with_prefs_lock<T>(
    prefs_path: &Path,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let parent = prefs_path
        .parent()
        .ok_or_else(|| "ARON preferences path is invalid".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|_| "could not create the ARON preferences directory".to_string())?;
    let lock_path = parent.join("prefs.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(|_| "could not open the ARON preferences lock".to_string())?;
    lock.lock_exclusive()
        .map_err(|_| "could not acquire the ARON preferences lock".to_string())?;

    let result = operation();
    let unlock_result = FileExt::unlock(&lock);
    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(_)) => Err("could not release the ARON preferences lock".to_string()),
        (Err(error), _) => Err(error),
    }
}

fn read_prefs(prefs_path: &Path) -> Result<(Map<String, Value>, String), String> {
    match fs::read(prefs_path) {
        Ok(bytes) => {
            let raw = serde_json::from_slice::<Map<String, Value>>(&bytes).map_err(|_| {
                "ARON preferences are malformed and were left unchanged".to_string()
            })?;
            let prefs = merge_loaded_prefs(raw);
            let revision = sha256_hex(&bytes);
            Ok((prefs, revision))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let prefs = default_prefs();
            let bytes = serde_json::to_vec(&prefs)
                .map_err(|_| "could not create default ARON preferences".to_string())?;
            Ok((prefs, sha256_hex(&bytes)))
        }
        Err(_) => Err("could not read ARON preferences".to_string()),
    }
}

fn merge_loaded_prefs(raw: Map<String, Value>) -> Map<String, Value> {
    let mut prefs = default_prefs();
    for (key, value) in raw {
        if settings_keys().contains(key.as_str()) {
            if valid_field(&key, &value, true) {
                prefs.insert(key, value);
            }
        } else {
            prefs.insert(key, value);
        }
    }
    prefs
}

fn validate_patch(patch: &Map<String, Value>) -> Result<(), String> {
    for (key, value) in patch {
        if !settings_keys().contains(key.as_str()) {
            return Err("the settings patch contains an unsupported field".to_string());
        }
        if !valid_field(key, value, false) {
            return Err("the settings patch contains an invalid value".to_string());
        }
    }
    Ok(())
}

fn valid_field(key: &str, value: &Value, stored: bool) -> bool {
    match key {
        "voice" => is_one_of(value, VOICES),
        "engine" => is_one_of(value, ENGINES),
        "theme" => is_one_of(value, THEMES),
        "orb_style" => is_one_of(value, ORB_STYLES),
        "volume" => valid_integer(value, 0, 100),
        "auto_pause" | "wake_enabled" | "history_enabled" | "history_store_replies" => {
            value.is_boolean()
        }
        "auto_pause_seconds" => valid_integer(value, 5, 600),
        "wake_name" => value
            .as_str()
            .is_some_and(|text| text.trim().chars().count() <= 40),
        "wake_aliases" | "wake_phrases" => value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string)),
        "wake_max_words" => valid_integer(value, 1, 20),
        "wake_fuzzy" => valid_number(value, 0.0, 1.0),
        "decision_engine" => is_one_of(value, DECISION_ENGINES),
        "wake_threshold_laya_multilingual" | "wake_threshold_jev" => {
            stored && (value.is_null() || valid_number(value, 0.0, 1.0))
        }
        "local_engine" => is_one_of(value, LOCAL_ENGINES),
        "stt_model" => is_one_of(value, STT_MODELS),
        "stt_device" => is_one_of(value, STT_DEVICES),
        "camera_device" => valid_string_length(value, 256),
        "camera_label" => valid_string_length(value, 160),
        "history_retention_days" => valid_integer(value, 0, 3_650),
        "hermes_profiles" => value.as_object().is_some_and(|profiles| {
            profiles.iter().all(|(role, profile)| {
                matches!(role.as_str(), "hermes_profile" | "hermes_squad")
                    && profile.as_str().is_some_and(|text| {
                        text == text.trim()
                            && (1..=128).contains(&text.chars().count())
                            && !text.chars().any(char::is_control)
                    })
            })
        }),
        _ => false,
    }
}

fn is_one_of(value: &Value, allowed: &[&str]) -> bool {
    value.as_str().is_some_and(|text| allowed.contains(&text))
}

fn valid_string_length(value: &Value, max_chars: usize) -> bool {
    value
        .as_str()
        .is_some_and(|text| text.chars().count() <= max_chars)
}

fn valid_integer(value: &Value, min: u64, max: u64) -> bool {
    value
        .as_u64()
        .is_some_and(|number| (min..=max).contains(&number))
}

fn valid_number(value: &Value, min: f64, max: f64) -> bool {
    value
        .as_f64()
        .is_some_and(|number| number >= min && number <= max)
}

fn snapshot_prefs(prefs: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut supported = Map::new();
    for key in settings_keys() {
        if let Some(value) = prefs.get(key) {
            if !valid_field(key, value, true) {
                return Err("ARON preferences contain an invalid value".to_string());
            }
            supported.insert(key.to_string(), value.clone());
        }
    }
    supported
        .entry("hermes_profiles".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    Ok(supported)
}

#[cfg(test)]
fn load_snapshot_at<B: CredentialBackend>(
    prefs_path: &Path,
    backend: &B,
) -> Result<AronSettingsSnapshot, String> {
    load_snapshot_at_with_env(prefs_path, backend, &|_| Vec::new())
}

fn load_snapshot_at_with_env<B, F>(
    prefs_path: &Path,
    backend: &B,
    environment_secrets: &F,
) -> Result<AronSettingsSnapshot, String>
where
    B: CredentialBackend,
    F: Fn(&str) -> Vec<String>,
{
    let (prefs, revision) = with_prefs_lock(prefs_path, || read_prefs(prefs_path))?;
    let supported_prefs = snapshot_prefs(&prefs)?;
    let secrets = vec![
        secret_status_with_env("jev", backend, &environment_secrets("jev"))?,
        secret_status_with_env("gemini", backend, &environment_secrets("gemini"))?,
    ];
    Ok(AronSettingsSnapshot {
        prefs: supported_prefs,
        secrets,
        revision,
    })
}

#[cfg(test)]
fn update_snapshot_at<B: CredentialBackend>(
    prefs_path: &Path,
    patch: Map<String, Value>,
    revision: &str,
    backend: &B,
) -> Result<AronSettingsSnapshot, String> {
    update_snapshot_at_with_env(prefs_path, patch, revision, backend, &|_| Vec::new())
}

fn update_snapshot_at_with_env<B, F>(
    prefs_path: &Path,
    mut patch: Map<String, Value>,
    revision: &str,
    backend: &B,
    environment_secrets: &F,
) -> Result<AronSettingsSnapshot, String>
where
    B: CredentialBackend,
    F: Fn(&str) -> Vec<String>,
{
    validate_patch(&patch)?;
    if let Some(Value::String(wake_name)) = patch.get_mut("wake_name") {
        *wake_name = wake_name.trim().to_lowercase();
    }
    let (prefs, current_revision) = with_prefs_lock(prefs_path, || {
        let (mut current, current_revision) = read_prefs(prefs_path)?;
        if current_revision != revision {
            return Err(
                "ARON settings changed since they were loaded; reload before saving".to_string(),
            );
        }
        if !patch.is_empty() {
            for (key, value) in patch {
                current.insert(key, value);
            }
            let bytes = serde_json::to_vec_pretty(&current)
                .map_err(|_| "could not serialize ARON preferences".to_string())?;
            atomic_replace(prefs_path, &bytes)?;
            let revision = sha256_hex(&bytes);
            Ok((current, revision))
        } else {
            Ok((current, current_revision))
        }
    })?;
    let supported_prefs = snapshot_prefs(&prefs)?;
    let secrets = vec![
        secret_status_with_env("jev", backend, &environment_secrets("jev"))?,
        secret_status_with_env("gemini", backend, &environment_secrets("gemini"))?,
    ];
    Ok(AronSettingsSnapshot {
        prefs: supported_prefs,
        secrets,
        revision: current_revision,
    })
}

fn atomic_replace(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "ARON preferences path is invalid".to_string())?;
    let name = path
        .file_name()
        .ok_or_else(|| "ARON preferences path is invalid".to_string())?
        .to_string_lossy();
    let (temporary_path, mut temporary_file) = (0..128)
        .find_map(|_| {
            let id = TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!("{name}.{}.{}.tmp", std::process::id(), id));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => Some(Ok((candidate, file))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(_) => Some(Err(())),
            }
        })
        .ok_or_else(|| "could not create a temporary ARON preferences file".to_string())?
        .map_err(|_| "could not create a temporary ARON preferences file".to_string())?;

    let write_result = temporary_file
        .write_all(contents)
        .and_then(|()| temporary_file.sync_all());
    drop(temporary_file);
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary_path);
        return Err("could not flush the temporary ARON preferences file".to_string());
    }
    if fs::rename(&temporary_path, path).is_err() {
        let _ = fs::remove_file(&temporary_path);
        return Err("could not atomically replace ARON preferences".to_string());
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}").expect("writing to a string cannot fail");
    }
    hex
}

fn gemini_key_id(value: &str) -> String {
    sha256_hex(value.as_bytes())[..10].to_string()
}

fn gemini_account_for_id(id: &str) -> String {
    format!("gemini#{id}")
}

fn gemini_account_id(value: &str) -> String {
    gemini_account_for_id(&gemini_key_id(value))
}

fn valid_gemini_key_id(id: &str) -> bool {
    id.len() == 10
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn get_credential<B: CredentialBackend>(
    backend: &B,
    account: &str,
) -> Result<Option<String>, String> {
    backend
        .get_password(account)
        .map_err(|_| "could not read from the ARON credential store".to_string())
}

fn set_credential<B: CredentialBackend>(
    backend: &B,
    account: &str,
    password: &str,
) -> Result<(), String> {
    backend
        .set_password(account, password)
        .map_err(|_| "could not write to the ARON credential store".to_string())
}

fn delete_credential<B: CredentialBackend>(backend: &B, account: &str) -> Result<(), String> {
    backend
        .delete_credential(account)
        .map_err(|_| "could not remove from the ARON credential store".to_string())
}

fn load_labels<B: CredentialBackend>(backend: &B) -> Result<CredentialLabels, String> {
    match get_credential(backend, CREDENTIAL_METADATA_ACCOUNT)? {
        Some(json) => serde_json::from_str(&json)
            .map_err(|_| "ARON credential metadata is invalid".to_string()),
        None => Ok(CredentialLabels::default()),
    }
}

fn save_labels<B: CredentialBackend>(backend: &B, labels: &CredentialLabels) -> Result<(), String> {
    let json = serde_json::to_string(labels)
        .map_err(|_| "could not serialize ARON credential metadata".to_string())?;
    set_credential(backend, CREDENTIAL_METADATA_ACCOUNT, &json)
}

fn load_gemini_index<B: CredentialBackend>(backend: &B) -> Result<Map<String, Value>, String> {
    let mut index = match get_credential(backend, GEMINI_INDEX_ACCOUNT)? {
        Some(json) => serde_json::from_str::<Value>(&json)
            .map_err(|_| "the Gemini credential index is malformed".to_string())?
            .as_object()
            .cloned()
            .ok_or_else(|| "the Gemini credential index is malformed".to_string())?,
        None => Map::new(),
    };
    match index.get("keys") {
        Some(Value::Array(_)) => {}
        Some(_) => return Err("the Gemini credential index is malformed".to_string()),
        None => {
            index.insert("keys".to_string(), Value::Array(Vec::new()));
        }
    }
    Ok(index)
}

fn save_gemini_index<B: CredentialBackend>(
    backend: &B,
    index: &Map<String, Value>,
) -> Result<(), String> {
    let json = serde_json::to_string(index)
        .map_err(|_| "could not serialize the Gemini credential index".to_string())?;
    set_credential(backend, GEMINI_INDEX_ACCOUNT, &json)
}

fn index_rows(index: &Map<String, Value>) -> &[Value] {
    index["keys"].as_array().expect("validated Gemini index")
}

fn index_rows_mut(index: &mut Map<String, Value>) -> &mut Vec<Value> {
    index
        .get_mut("keys")
        .and_then(Value::as_array_mut)
        .expect("validated Gemini index")
}

fn row_key_id(row: &Value) -> Option<&str> {
    row.as_object()
        .and_then(|row| row.get("id"))
        .and_then(Value::as_str)
        .filter(|id| valid_gemini_key_id(id))
}

fn row_label(row: &Value) -> Option<&str> {
    row.as_object()
        .and_then(|row| row.get("label"))
        .and_then(Value::as_str)
}

fn stored_gemini_credentials<B: CredentialBackend>(
    backend: &B,
) -> Result<Vec<StoredGeminiCredential>, String> {
    let index = load_gemini_index(backend)?;
    let mut seen = BTreeSet::new();
    let mut credentials = Vec::new();
    for row in index_rows(&index) {
        let Some(id) = row_key_id(row) else {
            continue;
        };
        if !seen.insert(id.to_string()) {
            continue;
        }
        let account = gemini_account_for_id(id);
        if let Some(value) = get_credential(backend, &account)? {
            if gemini_key_id(&value) != id {
                continue;
            }
            credentials.push(StoredGeminiCredential {
                id: id.to_string(),
                label: row_label(row).unwrap_or_default().to_string(),
                value,
                source: "aron".to_string(),
            });
        }
    }
    if let Some(value) = get_credential(backend, GEMINI_LEGACY_ACCOUNT)? {
        let id = gemini_key_id(&value);
        if seen.insert(id.clone()) {
            credentials.push(StoredGeminiCredential {
                id,
                label: String::new(),
                value,
                source: "store".to_string(),
            });
        }
    }
    Ok(credentials)
}

fn normalized_label(label: &str, value: &str, fallback: &str) -> Result<String, String> {
    let label = label.trim();
    if label.chars().count() > MAX_LABEL_LENGTH
        || label.chars().any(char::is_control)
        || label.contains(value)
    {
        return Err("the credential label is invalid".to_string());
    }
    Ok(if label.is_empty() {
        fallback.to_string()
    } else {
        label.to_string()
    })
}

fn validate_secret(value: &str) -> Result<(), String> {
    if value.chars().count() < 5
        || value.chars().count() > MAX_SECRET_LENGTH
        || value.chars().any(char::is_control)
    {
        return Err("the credential value is invalid".to_string());
    }
    Ok(())
}

fn add_secret<B: CredentialBackend>(
    provider: &str,
    value: &str,
    label: &str,
    backend: &B,
) -> Result<AronSecretStatus, String> {
    match provider {
        "jev" => {
            validate_secret(value)?;
            let label = normalized_label(label, value, "Jev")?;
            if let Some(existing) = get_credential(backend, JEV_ACCOUNT)? {
                if existing != value {
                    set_credential(backend, JEV_ACCOUNT, value)?;
                }
            } else {
                set_credential(backend, JEV_ACCOUNT, value)?;
            }
            let mut labels = load_labels(backend)?;
            labels.labels.insert(JEV_ACCOUNT.to_string(), label);
            save_labels(backend, &labels)?;
            secret_status(provider, backend)
        }
        "gemini" => add_gemini_secret(value, label, backend),
        _ => Err("unsupported ARON credential provider".to_string()),
    }
}

fn add_gemini_secret<B: CredentialBackend>(
    value: &str,
    label: &str,
    backend: &B,
) -> Result<AronSecretStatus, String> {
    let value = value.trim();
    validate_secret(value)?;
    let id = gemini_key_id(value);
    let account = gemini_account_id(value);
    let label = label.trim();
    if label.chars().count() > MAX_LABEL_LENGTH
        || label.chars().any(char::is_control)
        || label.contains(value)
    {
        return Err("the credential label is invalid".to_string());
    }

    if let Some(legacy) = get_credential(backend, GEMINI_LEGACY_ACCOUNT)? {
        if gemini_key_id(&legacy) == id {
            if legacy == value {
                return secret_status("gemini", backend);
            }
            return Err("the credential identifier is already in use".to_string());
        }
    }

    let mut index = load_gemini_index(backend)?;
    if let Some(existing) = get_credential(backend, &account)? {
        if existing != value {
            return Err("the credential identifier is already in use".to_string());
        }
    } else {
        set_credential(backend, &account, value)?;
    }

    let mut index_changed = false;
    if let Some(row) = index_rows_mut(&mut index)
        .iter_mut()
        .find(|row| row_key_id(row) == Some(id.as_str()))
    {
        if !label.is_empty() && row_label(row) != Some(label) {
            row.as_object_mut()
                .expect("matched Gemini index row is an object")
                .insert("label".to_string(), Value::String(label.to_string()));
            index_changed = true;
        }
    } else {
        index_rows_mut(&mut index).push(serde_json::json!({
            "id": id,
            "label": label
        }));
        index_changed = true;
    }
    if index_changed {
        save_gemini_index(backend, &index)?;
    }
    secret_status("gemini", backend)
}

fn remove_secret<B: CredentialBackend>(
    provider: &str,
    id: &str,
    backend: &B,
) -> Result<AronSecretStatus, String> {
    match provider {
        "jev" if id == JEV_ACCOUNT => {
            let mut labels = load_labels(backend)?;
            delete_credential(backend, JEV_ACCOUNT)?;
            labels.labels.remove(JEV_ACCOUNT);
            save_labels(backend, &labels)?;
        }
        "gemini" if valid_gemini_key_id(id) => {
            let mut index = load_gemini_index(backend)?;
            if index_rows(&index)
                .iter()
                .any(|row| row_key_id(row) == Some(id))
            {
                delete_credential(backend, &gemini_account_for_id(id))?;
                index_rows_mut(&mut index).retain(|row| row_key_id(row) != Some(id));
                save_gemini_index(backend, &index)?;
            } else if get_credential(backend, GEMINI_LEGACY_ACCOUNT)?
                .is_some_and(|value| gemini_key_id(&value) == id)
            {
                delete_credential(backend, GEMINI_LEGACY_ACCOUNT)?;
            } else {
                return Err("the ARON credential identifier is invalid".to_string());
            }
        }
        "jev" | "gemini" => return Err("the ARON credential identifier is invalid".to_string()),
        _ => return Err("unsupported ARON credential provider".to_string()),
    }
    secret_status(provider, backend)
}

fn secret_status<B: CredentialBackend>(
    provider: &str,
    backend: &B,
) -> Result<AronSecretStatus, String> {
    secret_status_with_env(provider, backend, &[])
}

fn secret_status_with_env<B: CredentialBackend>(
    provider: &str,
    backend: &B,
    environment_values: &[String],
) -> Result<AronSecretStatus, String> {
    let gemini_credentials = stored_gemini_credentials(backend)?;
    let jev_value = get_credential(backend, JEV_ACCOUNT)?;
    let labels = if provider == "jev" {
        load_labels(backend)?
    } else {
        CredentialLabels::default()
    };
    let mut secret_values: Vec<String> = gemini_credentials
        .iter()
        .map(|credential| credential.value.clone())
        .collect();
    if let Some(value) = &jev_value {
        secret_values.push(value.clone());
    }
    secret_values.extend(environment_values.iter().cloned());
    let mut entries = Vec::new();
    match provider {
        "jev" => {
            if let Some(value) = jev_value {
                let label = safe_label(
                    labels.labels.get(JEV_ACCOUNT).map(String::as_str),
                    &secret_values,
                    "Jev",
                );
                entries.push(AronSecretMetadata {
                    id: JEV_ACCOUNT.to_string(),
                    label,
                    last4: last_four(&value),
                    source: "aron".to_string(),
                });
            }
            for (index, value) in environment_values.iter().enumerate() {
                entries.push(AronSecretMetadata {
                    id: environment_key_id(index),
                    label: safe_label(Some("Environment"), &secret_values, "Environment"),
                    last4: last_four(value),
                    source: "env".to_string(),
                });
            }
        }
        "gemini" => {
            for credential in gemini_credentials {
                let label = safe_label(
                    (!credential.label.is_empty()).then_some(credential.label.as_str()),
                    &secret_values,
                    "gemini",
                );
                entries.push(AronSecretMetadata {
                    id: credential.id,
                    label,
                    last4: last_four(&credential.value),
                    source: credential.source,
                });
            }
            for (index, value) in environment_values.iter().enumerate() {
                entries.push(AronSecretMetadata {
                    id: environment_key_id(index),
                    label: safe_label(Some("Environment"), &secret_values, "Environment"),
                    last4: last_four(value),
                    source: "env".to_string(),
                });
            }
        }
        _ => return Err("unsupported ARON credential provider".to_string()),
    }
    Ok(AronSecretStatus {
        provider: provider.to_string(),
        configured: !entries.is_empty(),
        entries,
    })
}

fn process_environment_secrets(provider: &str) -> Vec<String> {
    collect_environment_secrets(provider, |name| env::var(name).ok())
}

fn collect_environment_secrets<F>(provider: &str, mut read_variable: F) -> Vec<String>
where
    F: FnMut(&str) -> Option<String>,
{
    let mut seen = BTreeSet::new();
    let mut values = Vec::new();
    match provider {
        "jev" => {
            for name in JEV_ENV_VARS {
                if let Some(value) = read_variable(name) {
                    let value = value.trim();
                    if !value.is_empty() && seen.insert(value.to_string()) {
                        values.push(value.to_string());
                    }
                }
            }
        }
        "gemini" => {
            for (name, is_list) in GEMINI_ENV_VARS {
                if let Some(value) = read_variable(name) {
                    let candidates = if *is_list {
                        value.split(',').collect::<Vec<_>>()
                    } else {
                        vec![value.as_str()]
                    };
                    for candidate in candidates {
                        let value = candidate.trim();
                        if !value.is_empty() && seen.insert(value.to_string()) {
                            values.push(value.to_string());
                        }
                    }
                }
            }
        }
        _ => {}
    }
    values
}

fn environment_key_id(index: usize) -> String {
    format!("environment-{}", index + 1)
}

fn safe_label(label: Option<&str>, secrets: &[String], fallback: &str) -> String {
    let is_safe = |text: &str| {
        text.chars().count() <= MAX_LABEL_LENGTH
            && !text.chars().any(char::is_control)
            && !secrets
                .iter()
                .any(|secret| !secret.is_empty() && text.contains(secret))
    };
    label
        .filter(|text| is_safe(text))
        .map(str::to_string)
        .unwrap_or_else(|| {
            if is_safe(fallback) {
                fallback.to_string()
            } else {
                String::new()
            }
        })
}

fn last_four(value: &str) -> String {
    if value.chars().count() <= 4 {
        return String::new();
    }
    value
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;

    #[derive(Default)]
    struct FakeCredentialBackend {
        values: Mutex<BTreeMap<String, String>>,
        secret_writes: Mutex<Vec<String>>,
        deletes: Mutex<Vec<String>>,
    }

    impl FakeCredentialBackend {
        fn with_value(account: &str, value: &str) -> Self {
            Self {
                values: Mutex::new(BTreeMap::from([(account.to_string(), value.to_string())])),
                ..Self::default()
            }
        }

        fn with_values(entries: &[(&str, &str)]) -> Self {
            Self {
                values: Mutex::new(
                    entries
                        .iter()
                        .map(|(account, value)| (account.to_string(), value.to_string()))
                        .collect(),
                ),
                ..Self::default()
            }
        }

        fn value(&self, account: &str) -> Option<String> {
            self.values.lock().unwrap().get(account).cloned()
        }
    }

    impl CredentialBackend for FakeCredentialBackend {
        fn get_password(&self, account: &str) -> Result<Option<String>, String> {
            Ok(self.values.lock().unwrap().get(account).cloned())
        }

        fn set_password(&self, account: &str, password: &str) -> Result<(), String> {
            self.values
                .lock()
                .unwrap()
                .insert(account.to_string(), password.to_string());
            if account != CREDENTIAL_METADATA_ACCOUNT && account != GEMINI_INDEX_ACCOUNT {
                self.secret_writes.lock().unwrap().push(account.to_string());
            }
            Ok(())
        }

        fn delete_credential(&self, account: &str) -> Result<(), String> {
            self.values.lock().unwrap().remove(account);
            self.deletes.lock().unwrap().push(account.to_string());
            Ok(())
        }
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let id = TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
            let path =
                env::temp_dir().join(format!("coucou-aron-settings-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn prefs(&self) -> PathBuf {
            self.0.join("prefs").join("prefs.json")
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn object(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Map<String, Value> {
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    fn snapshot_revision(path: &Path) -> String {
        load_snapshot_at(path, &FakeCredentialBackend::default())
            .unwrap()
            .revision
    }

    #[test]
    fn aron_home_precedes_app_data() {
        let home = OsStr::new("C:\\temp\\aron-home");
        let app_data = OsStr::new("C:\\users\\test\\AppData\\Roaming");
        assert_eq!(
            resolve_prefs_path_from(Some(home), Some(app_data)).unwrap(),
            PathBuf::from(home).join("prefs").join("prefs.json")
        );
    }

    #[test]
    fn default_path_uses_app_data_aron_directory() {
        let app_data = OsStr::new("C:\\users\\test\\AppData\\Roaming");
        assert_eq!(
            resolve_prefs_path_from(None, Some(app_data)).unwrap(),
            PathBuf::from(app_data)
                .join("aron")
                .join("prefs")
                .join("prefs.json")
        );
        assert!(resolve_prefs_path_from(None, None).is_err());
    }

    #[test]
    fn missing_preferences_return_defaults_without_creating_a_prefs_file() {
        let temp = TestDir::new();
        let backend = FakeCredentialBackend::default();
        let snapshot = load_snapshot_at(&temp.prefs(), &backend).unwrap();
        assert_eq!(
            snapshot.prefs.get("hermes_profiles"),
            Some(&Value::Object(Map::new()))
        );
        assert_eq!(
            snapshot.prefs.get("theme"),
            Some(&Value::String("azul".into()))
        );
        assert_eq!(snapshot.prefs.get("volume"), Some(&Value::from(70)));
        assert_eq!(snapshot.prefs.get("wake_threshold_jev"), Some(&Value::Null));
        assert!(!temp.prefs().exists());
        assert_eq!(snapshot.revision, snapshot_revision(&temp.prefs()));
    }

    #[test]
    fn malformed_preferences_are_never_overwritten() {
        let temp = TestDir::new();
        fs::create_dir_all(temp.prefs().parent().unwrap()).unwrap();
        let original = b"{ malformed prefs";
        fs::write(temp.prefs(), original).unwrap();
        let error = update_snapshot_at(
            &temp.prefs(),
            object([("theme", Value::String("azul".to_string()))]),
            "any-revision",
            &FakeCredentialBackend::default(),
        )
        .unwrap_err();
        assert!(error.contains("malformed"));
        assert_eq!(fs::read(temp.prefs()).unwrap(), original);
    }

    #[test]
    fn patch_validation_matches_python_validators_by_field_family() {
        let valid = object([
            ("theme", Value::String("azul".to_string())),
            ("volume", Value::from(50)),
            ("auto_pause_seconds", Value::from(600)),
            ("wake_max_words", Value::from(20)),
            ("wake_aliases", serde_json::json!(["", "x".repeat(300)])),
            ("camera_label", Value::String("x".repeat(160))),
            ("hermes_profiles", Value::Object(Map::new())),
        ]);
        assert!(validate_patch(&valid).is_ok());

        for invalid in [
            object([("unknown", Value::Bool(true))]),
            object([("voice", Value::String("not-a-voice".to_string()))]),
            object([("engine", Value::String("local".to_string()))]),
            object([("theme", Value::String("dark".to_string()))]),
            object([("orb_style", Value::String("orb".to_string()))]),
            object([("decision_engine", Value::String("other".to_string()))]),
            object([("local_engine", Value::String("other".to_string()))]),
            object([("stt_model", Value::String("large".to_string()))]),
            object([("stt_device", Value::String("npu".to_string()))]),
            object([("volume", Value::from(101))]),
            object([("volume", Value::Bool(true))]),
            object([("volume", Value::from(50.0))]),
            object([("auto_pause_seconds", Value::from(4))]),
            object([("auto_pause_seconds", Value::from(601))]),
            object([("auto_pause_seconds", Value::Bool(true))]),
            object([("wake_max_words", Value::from(0))]),
            object([("wake_max_words", Value::from(21))]),
            object([("wake_max_words", Value::Bool(true))]),
            object([("history_retention_days", Value::from(-1))]),
            object([("history_retention_days", Value::from(3_651))]),
            object([("history_retention_days", Value::Bool(false))]),
            object([("wake_enabled", Value::String("yes".to_string()))]),
            object([("wake_name", Value::String("x".repeat(41)))]),
            object([("wake_aliases", serde_json::json!(["ok", 7]))]),
            object([("wake_phrases", serde_json::json!([false]))]),
            object([("wake_fuzzy", Value::Bool(true))]),
            object([("wake_fuzzy", Value::from(1.01))]),
            object([("camera_device", Value::String("x".repeat(257)))]),
            object([("camera_label", Value::String("x".repeat(161)))]),
            object([("history_enabled", Value::from(1))]),
            object([("history_store_replies", Value::String("true".into()))]),
            object([("wake_threshold_jev", Value::from(0.5))]),
            object([("wake_threshold_laya_multilingual", Value::Null)]),
            object([(
                "hermes_profiles",
                serde_json::json!({"unknown_role": "agent"}),
            )]),
            object([(
                "hermes_profiles",
                serde_json::json!({"hermes_profile": " \n"}),
            )]),
            object([(
                "hermes_profiles",
                serde_json::json!({"hermes_squad": "x".repeat(129)}),
            )]),
            object([(
                "hermes_profiles",
                serde_json::json!({"hermes_profile": "agent\u{0001}name"}),
            )]),
            object([(
                "hermes_profiles",
                serde_json::json!({"hermes_profile": "agent\u{007f}name"}),
            )]),
            object([(
                "hermes_profiles",
                serde_json::json!({"hermes_profile": "agent\u{0085}name"}),
            )]),
        ] {
            assert!(validate_patch(&invalid).is_err());
        }
        assert!(valid_field("wake_threshold_jev", &Value::Null, true));
        assert!(!valid_field("wake_threshold_jev", &Value::Null, false));
    }

    #[test]
    fn stored_null_thresholds_load_and_survive_updates_but_are_never_patchable() {
        let temp = TestDir::new();
        fs::create_dir_all(temp.prefs().parent().unwrap()).unwrap();
        fs::write(
            temp.prefs(),
            br#"{"wake_threshold_laya_multilingual":null,"wake_threshold_jev":null}"#,
        )
        .unwrap();
        let backend = FakeCredentialBackend::default();
        let loaded = load_snapshot_at(&temp.prefs(), &backend).unwrap();
        assert_eq!(
            loaded.prefs["wake_threshold_laya_multilingual"],
            Value::Null
        );
        assert_eq!(loaded.prefs["wake_threshold_jev"], Value::Null);

        let updated = update_snapshot_at(
            &temp.prefs(),
            object([("theme", Value::String("ciano".to_string()))]),
            &loaded.revision,
            &backend,
        )
        .unwrap();
        assert_eq!(
            updated.prefs["wake_threshold_laya_multilingual"],
            Value::Null
        );
        assert_eq!(updated.prefs["wake_threshold_jev"], Value::Null);
        let persisted: Value = serde_json::from_slice(&fs::read(temp.prefs()).unwrap()).unwrap();
        assert!(persisted["wake_threshold_laya_multilingual"].is_null());
        assert!(persisted["wake_threshold_jev"].is_null());
    }

    #[test]
    fn invalid_stored_fields_fall_back_to_python_defaults() {
        let temp = TestDir::new();
        fs::create_dir_all(temp.prefs().parent().unwrap()).unwrap();
        fs::write(
            temp.prefs(),
            br#"{"theme":"dark","auto_pause_seconds":601,"unrelated":{"keep":true}}"#,
        )
        .unwrap();
        let backend = FakeCredentialBackend::default();
        let snapshot = load_snapshot_at(&temp.prefs(), &backend).unwrap();
        assert_eq!(snapshot.prefs["theme"], "azul");
        assert_eq!(snapshot.prefs["auto_pause_seconds"], 20);

        let updated = update_snapshot_at(
            &temp.prefs(),
            object([("volume", Value::from(35))]),
            &snapshot.revision,
            &backend,
        )
        .unwrap();
        assert_eq!(updated.prefs["theme"], "azul");
        assert_eq!(updated.prefs["auto_pause_seconds"], 20);
        let persisted: Value = serde_json::from_slice(&fs::read(temp.prefs()).unwrap()).unwrap();
        assert_eq!(persisted["unrelated"]["keep"], true);
    }

    #[test]
    fn wake_name_is_normalized_like_python_on_update() {
        let temp = TestDir::new();
        let backend = FakeCredentialBackend::default();
        let initial = load_snapshot_at(&temp.prefs(), &backend).unwrap();
        let updated = update_snapshot_at(
            &temp.prefs(),
            object([("wake_name", Value::String("  MiXeD  ".to_string()))]),
            &initial.revision,
            &backend,
        )
        .unwrap();
        assert_eq!(updated.prefs["wake_name"], "mixed");
    }

    #[test]
    fn stale_revision_conflicts_without_writing() {
        let temp = TestDir::new();
        let path = temp.prefs();
        let backend = FakeCredentialBackend::default();
        let initial = load_snapshot_at(&path, &backend).unwrap();
        let updated = update_snapshot_at(
            &path,
            object([("theme", Value::String("verde".to_string()))]),
            &initial.revision,
            &backend,
        )
        .unwrap();
        let bytes = fs::read(&path).unwrap();
        let error = update_snapshot_at(
            &path,
            object([("theme", Value::String("ciano".to_string()))]),
            &initial.revision,
            &backend,
        )
        .unwrap_err();
        assert!(error.contains("changed"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(updated.prefs["theme"], "verde");
    }

    #[test]
    fn partial_updates_merge_latest_and_atomically_replace_the_file() {
        let temp = TestDir::new();
        let path = temp.prefs();
        let backend = FakeCredentialBackend::default();
        let first_revision = snapshot_revision(&path);
        let first = update_snapshot_at(
            &path,
            object([("volume", Value::from(40))]),
            &first_revision,
            &backend,
        )
        .unwrap();
        let second = update_snapshot_at(
            &path,
            object([("theme", Value::String("ciano".to_string()))]),
            &first.revision,
            &backend,
        )
        .unwrap();
        assert_eq!(second.prefs["volume"], 40);
        assert_eq!(second.prefs["theme"], "ciano");
        let siblings = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(siblings.iter().any(|name| name == "prefs.lock"));
        assert_eq!(
            siblings
                .iter()
                .filter(|name| name.ends_with(".tmp"))
                .count(),
            0
        );
    }

    #[test]
    fn update_waits_for_the_shared_exclusive_lock() {
        let temp = TestDir::new();
        let path = temp.prefs();
        let revision = snapshot_revision(&path);
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent).unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(parent.join("prefs.lock"))
            .unwrap();
        lock.lock_exclusive().unwrap();

        let worker_path = path.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = update_snapshot_at(
                &worker_path,
                object([("theme", Value::String("verde".to_string()))]),
                &revision,
                &FakeCredentialBackend::default(),
            );
            done_tx.send(result).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
        FileExt::unlock(&lock).unwrap();
        assert!(done_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .is_ok());
        worker.join().unwrap();
    }

    #[test]
    fn jev_uses_the_fixed_account_and_returns_only_secret_metadata() {
        let backend = FakeCredentialBackend::default();
        let key = "jev-secret-value";
        let status = add_secret("jev", key, "Primary", &backend).unwrap();
        assert_eq!(backend.value(JEV_ACCOUNT).as_deref(), Some(key));
        assert_eq!(status.entries[0].id, JEV_ACCOUNT);
        assert_eq!(status.entries[0].label, "Primary");
        assert_eq!(status.entries[0].last4, "alue");
        assert_eq!(status.entries[0].source, "aron");

        let serialized = serde_json::to_string(&status).unwrap();
        assert!(!serialized.contains(key));
        assert!(!serialized.contains("\"value\""));
    }

    #[test]
    fn environment_credentials_are_deduplicated_read_only_metadata() {
        let vars = [
            (
                "ARON_GEMINI_API_KEYS",
                " first-environment-key,second-environment-key ",
            ),
            ("GEMINI_API_KEYS", "first-environment-key"),
            ("GEMINI_API_KEY", "single-environment-key"),
        ];
        let values = collect_environment_secrets("gemini", |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        });
        assert_eq!(
            values,
            vec![
                "first-environment-key",
                "second-environment-key",
                "single-environment-key"
            ]
        );

        let backend = FakeCredentialBackend::default();
        let status = secret_status_with_env("gemini", &backend, &values).unwrap();
        assert_eq!(status.entries.len(), 3);
        assert!(status.entries.iter().all(|entry| entry.source == "env"));
        let serialized = serde_json::to_string(&status).unwrap();
        for value in &values {
            assert!(!serialized.contains(value));
        }
    }

    #[test]
    fn gemini_uses_index_and_sha256_accounts_and_duplicate_add_updates_label() {
        let indexed_key = "already-indexed-gemini-key";
        let indexed_id = gemini_key_id(indexed_key);
        let index_json = serde_json::json!({
            "version": 1,
            "keys": [{
                "id": indexed_id,
                "label": "Existing",
                "keep": "unrelated row data"
            }]
        })
        .to_string();
        let backend = FakeCredentialBackend::with_values(&[
            (GEMINI_INDEX_ACCOUNT, &index_json),
            (&gemini_account_for_id(&indexed_id), indexed_key),
        ]);
        let initial = secret_status("gemini", &backend).unwrap();
        assert_eq!(initial.entries.len(), 1);
        assert_eq!(initial.entries[0].id, indexed_id);
        assert_eq!(initial.entries[0].label, "Existing");
        assert_ne!(initial.entries[0].id, GEMINI_INDEX_ACCOUNT);

        assert_eq!(gemini_key_id("secret"), "2bb80d537b");
        assert_eq!(gemini_account_id("secret"), "gemini#2bb80d537b");
        assert!(!valid_gemini_key_id("ABCDEF1234"));
        let key = "gemini-api-key-for-tests";
        let id = gemini_key_id(key);
        let account = gemini_account_for_id(&id);
        let first = add_secret("gemini", key, "Work", &backend).unwrap();
        assert!(first.entries.iter().any(|entry| entry.id == id));
        let writes_before_duplicate = backend.secret_writes.lock().unwrap().len();
        let duplicate = add_secret("gemini", key, "Personal", &backend).unwrap();
        assert_eq!(
            backend.secret_writes.lock().unwrap().len(),
            writes_before_duplicate
        );
        assert_eq!(
            duplicate
                .entries
                .iter()
                .find(|entry| entry.id == id)
                .unwrap()
                .label,
            "Personal"
        );
        assert_eq!(
            backend.secret_writes.lock().unwrap().len(),
            writes_before_duplicate
        );
        let index: Value =
            serde_json::from_str(&backend.value(GEMINI_INDEX_ACCOUNT).unwrap()).unwrap();
        assert_eq!(index["version"], 1);
        assert_eq!(index["keys"][0]["keep"], "unrelated row data");
        assert_eq!(
            index["keys"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == id)
                .unwrap()["label"],
            "Personal"
        );
        assert_eq!(backend.value(&account).as_deref(), Some(key));
        let serialized = serde_json::to_string(&duplicate).unwrap();
        assert!(!serialized.contains(key));
        assert!(!serialized.contains(indexed_key));
    }

    #[test]
    fn removal_is_limited_to_aron_owned_account_ids() {
        let key = "owned-gemini-secret";
        let other_key = "another-owned-secret";
        let id = gemini_key_id(key);
        let other_id = gemini_key_id(other_key);
        let index = serde_json::json!({
            "version": 1,
            "keys": [
                {"id": id, "label": "Remove me", "preserve": true},
                {"id": other_id, "label": "Keep me", "extra": {"safe": true}}
            ]
        })
        .to_string();
        let account = gemini_account_for_id(&id);
        let other_account = gemini_account_for_id(&other_id);
        let backend = FakeCredentialBackend::with_values(&[
            (GEMINI_INDEX_ACCOUNT, &index),
            (&account, key),
            (&other_account, other_key),
            ("coucou-unrelated", "unrelated"),
        ]);

        let index_removal = remove_secret("gemini", GEMINI_INDEX_ACCOUNT, &backend).unwrap_err();
        assert!(index_removal.contains("invalid"));
        let unknown_removal = remove_secret("gemini", "coucou-unrelated", &backend).unwrap_err();
        assert!(unknown_removal.contains("invalid"));
        assert!(backend.deletes.lock().unwrap().is_empty());

        let status = remove_secret("gemini", &id, &backend).unwrap();
        assert!(!status.entries.iter().any(|entry| entry.id == id));
        assert!(status.entries.iter().any(|entry| entry.id == other_id));
        assert_eq!(
            backend.deletes.lock().unwrap().as_slice(),
            &[account.clone()]
        );
        let updated_index: Value =
            serde_json::from_str(&backend.value(GEMINI_INDEX_ACCOUNT).unwrap()).unwrap();
        assert_eq!(updated_index["version"], 1);
        assert_eq!(updated_index["keys"].as_array().unwrap().len(), 1);
        assert_eq!(updated_index["keys"][0]["id"], other_id);
        assert_eq!(updated_index["keys"][0]["extra"]["safe"], true);
        assert_eq!(backend.value(&account), None);
        assert_eq!(backend.value(&other_account).as_deref(), Some(other_key));
        assert_eq!(
            backend.value("coucou-unrelated").as_deref(),
            Some("unrelated")
        );
        assert_eq!(backend.value(GEMINI_INDEX_ACCOUNT).is_some(), true);
    }

    #[test]
    fn legacy_gemini_account_is_reported_by_hashed_id_and_removed_only_by_that_id() {
        let key = "old-single-gemini-key";
        let id = gemini_key_id(key);
        let backend = FakeCredentialBackend::with_value(GEMINI_LEGACY_ACCOUNT, key);
        let status = secret_status("gemini", &backend).unwrap();
        assert_eq!(status.entries[0].id, id);
        assert_eq!(status.entries[0].label, "gemini");
        let removed = remove_secret("gemini", &id, &backend).unwrap();
        assert!(!removed.configured);
        assert_eq!(
            backend.deletes.lock().unwrap().as_slice(),
            &[GEMINI_LEGACY_ACCOUNT]
        );
        assert_eq!(backend.value(GEMINI_INDEX_ACCOUNT), None);
    }

    #[test]
    fn snapshots_and_errors_do_not_disclose_secret_values() {
        let temp = TestDir::new();
        let backend = FakeCredentialBackend::default();
        let secret = "secret-that-must-not-leak";
        add_secret("gemini", secret, "API key", &backend).unwrap();
        let snapshot = load_snapshot_at(&temp.prefs(), &backend).unwrap();
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains(secret));
        assert!(serialized.contains("leak"));
        assert!(!serialized.contains("\"value\""));

        let error = add_secret("not-a-provider", secret, "Label", &backend).unwrap_err();
        assert!(!error.contains(secret));
        let error = add_secret("gemini", "tiny", "Label", &backend).unwrap_err();
        assert!(!error.contains("tiny"));
    }

    #[test]
    fn environment_secret_snapshots_return_only_safe_metadata() {
        let temp = TestDir::new();
        let backend = FakeCredentialBackend::default();
        let secret = "secret-that-must-not-leak";
        let snapshot = load_snapshot_at_with_env(&temp.prefs(), &backend, &|provider| {
            if provider == "gemini" {
                vec![secret.to_string()]
            } else {
                Vec::new()
            }
        })
        .unwrap();
        let status = &snapshot.secrets[1];
        assert_eq!(status.entries[0].source, "env");
        assert_eq!(status.entries[0].label, "Environment");
        assert_eq!(status.entries[0].last4, "leak");
        assert_eq!(status.entries[0].id, "environment-1");
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains(secret));
        assert!(serialized.contains("\"source\":\"env\""));
    }

    #[test]
    fn labels_cannot_echo_the_secret_in_status() {
        let backend = FakeCredentialBackend::default();
        let secret = "private-api-key";
        assert!(add_secret("jev", secret, secret, &backend).is_err());
        backend
            .values
            .lock()
            .unwrap()
            .insert(JEV_ACCOUNT.to_string(), secret.to_string());
        let mut labels = CredentialLabels::default();
        labels
            .labels
            .insert(JEV_ACCOUNT.to_string(), secret.to_string());
        save_labels(&backend, &labels).unwrap();
        let status = secret_status("jev", &backend).unwrap();
        let serialized = serde_json::to_string(&status).unwrap();
        assert!(!serialized.contains(secret));
        assert_eq!(status.entries[0].label, "Jev");
    }
}
