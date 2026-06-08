//! Microsoft account authentication for Java Edition (online mode):
//! MSA device-code flow → Xbox Live → XSTS → Minecraft token + profile.
//! Port of typecraft's `auth/` module.
//!
//! Token caching keys off SHA1(username); time-based validity uses epoch-ms
//! math. ISO-date (XSTS `NotAfter`) validity is treated conservatively (always
//! re-fetched) to avoid a date-parsing dependency.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde_json::{json, Value};
use sha1::{Digest as _, Sha1};

#[derive(Debug, Clone)]
pub struct AuthResult {
    pub access_token: String,
    pub username: String,
    pub uuid: String,
}

pub struct AuthOptions {
    pub username: String,
    pub profiles_folder: Option<PathBuf>,
    /// Called with the device code the user must enter to sign in.
    pub on_device_code: Option<Box<dyn Fn(&str, &str) + Send>>,
}

const CLIENT_ID: &str = "00000000402b5328";
const SCOPES: &str = "service::user.auth.xboxlive.com::MBI_SSL";
const DEVICE_CODE_URL: &str = "https://login.live.com/oauth20_connect.srf";
const TOKEN_URL: &str = "https://login.live.com/oauth20_token.srf";
const USER_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_RELYING_PARTY: &str = "rp://api.minecraftservices.com/";
const LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

type AuthError = Box<dyn std::error::Error + Send + Sync>;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ── Cache ──

fn hash_username(username: &str) -> String {
    let mut h = Sha1::new();
    h.update(username.as_bytes());
    let digest = h.finalize();
    digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()[..6]
        .to_string()
}

fn cache_path(dir: &Path, username: &str, name: &str) -> PathBuf {
    dir.join(format!("{}_{name}-cache.json", hash_username(username)))
}

fn load_cache(dir: &Path, username: &str, name: &str) -> Value {
    std::fs::read_to_string(cache_path(dir, username, name))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}))
}

fn save_cache(dir: &Path, username: &str, name: &str, data: Value) {
    let _ = std::fs::create_dir_all(dir);
    let mut existing = load_cache(dir, username, name);
    if let (Some(obj), Some(new)) = (existing.as_object_mut(), data.as_object()) {
        for (k, v) in new {
            obj.insert(k.clone(), v.clone());
        }
    }
    let _ = std::fs::write(
        cache_path(dir, username, name),
        serde_json::to_string_pretty(&existing).unwrap_or_default(),
    );
}

fn is_token_valid(obtained_on: Option<i64>, expires_in: Option<i64>) -> bool {
    let (Some(obtained), Some(expires)) = (obtained_on, expires_in) else {
        return false;
    };
    let expires_ms = if expires < 100_000 {
        expires * 1000
    } else {
        expires
    };
    obtained + expires_ms - now_ms() > 1000
}

// ── MSA device-code flow ──

async fn request_device_code(client: &reqwest::Client) -> Result<Value, AuthError> {
    let res = client
        .post(DEVICE_CODE_URL)
        .form(&[
            ("scope", SCOPES),
            ("client_id", CLIENT_ID),
            ("response_type", "device_code"),
        ])
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(format!("Device code request failed: {}", res.status()).into());
    }
    Ok(res.json().await?)
}

async fn poll_device_code(
    client: &reqwest::Client,
    device_code: &str,
    interval: u64,
    expires_in: u64,
) -> Result<Value, AuthError> {
    let deadline = now_ms() + expires_in as i64 * 1000 - 100;
    while now_ms() < deadline {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
        let res = client
            .post(format!("{TOKEN_URL}?client_id={CLIENT_ID}"))
            .form(&[
                ("client_id", CLIENT_ID),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await?;
        let data: Value = res.json().await?;
        if let Some(err) = data.get("error").and_then(|v| v.as_str()) {
            if err == "authorization_pending" {
                continue;
            }
            return Err(format!("Auth failed: {err}").into());
        }
        return Ok(data);
    }
    Err("Device code authentication timed out".into())
}

async fn refresh_msa_token(
    client: &reqwest::Client,
    refresh_token: &str,
) -> Result<Value, AuthError> {
    let res = client
        .post(TOKEN_URL)
        .form(&[
            ("scope", SCOPES),
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(format!("Token refresh failed: {}", res.status()).into());
    }
    Ok(res.json().await?)
}

async fn get_msa_token(
    client: &reqwest::Client,
    dir: &Path,
    options: &AuthOptions,
) -> Result<String, AuthError> {
    let cache = load_cache(dir, &options.username, "live");
    let cached = cache.get("token");
    if let Some(token) = cached {
        let obtained = token.get("obtainedOn").and_then(|v| v.as_i64());
        let expires = token.get("expires_in").and_then(|v| v.as_i64());
        if is_token_valid(obtained, expires) {
            if let Some(at) = token.get("access_token").and_then(|v| v.as_str()) {
                return Ok(at.to_string());
            }
        }
        if let Some(rt) = token.get("refresh_token").and_then(|v| v.as_str()) {
            if let Ok(mut refreshed) = refresh_msa_token(client, rt).await {
                refreshed["obtainedOn"] = json!(now_ms());
                save_cache(
                    dir,
                    &options.username,
                    "live",
                    json!({ "token": refreshed.clone() }),
                );
                if let Some(at) = refreshed.get("access_token").and_then(|v| v.as_str()) {
                    return Ok(at.to_string());
                }
            }
        }
    }

    let code = request_device_code(client).await?;
    let user_code = code.get("user_code").and_then(|v| v.as_str()).unwrap_or("");
    let verification_uri = code
        .get("verification_uri")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match &options.on_device_code {
        Some(cb) => cb(user_code, verification_uri),
        None => println!("To sign in, visit {verification_uri} and enter code {user_code}"),
    }

    let device_code = code
        .get("device_code")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let interval = code.get("interval").and_then(|v| v.as_u64()).unwrap_or(5);
    let expires_in = code
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .unwrap_or(900);
    let mut tokens = poll_device_code(client, device_code, interval, expires_in).await?;
    tokens["obtainedOn"] = json!(now_ms());
    save_cache(
        dir,
        &options.username,
        "live",
        json!({ "token": tokens.clone() }),
    );
    Ok(tokens
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

// ── Xbox Live (EC P-256 request signing) ──

fn sign_request(path: &str, body: &str, key: &p256::ecdsa::SigningKey) -> String {
    use p256::ecdsa::{signature::Signer, Signature};

    // Windows epoch ticks (100ns since 1601-01-01).
    let windows_ts: u64 = ((now_ms() / 1000) as u64 + 11_644_473_600) * 10_000_000;

    let mut buf = Vec::new();
    buf.extend_from_slice(&1i32.to_be_bytes()); // policy version
    buf.push(0);
    buf.extend_from_slice(&windows_ts.to_be_bytes());
    buf.push(0);
    buf.extend_from_slice(b"POST\0");
    buf.extend_from_slice(format!("{path}\0").as_bytes());
    buf.extend_from_slice(b"\0"); // empty authorization token
    buf.extend_from_slice(format!("{body}\0").as_bytes());

    let signature: Signature = key.sign(&buf);
    let raw = signature.to_bytes(); // IEEE P1363 (r||s), 64 bytes

    let mut header = Vec::with_capacity(12 + raw.len());
    header.extend_from_slice(&1i32.to_be_bytes());
    header.extend_from_slice(&windows_ts.to_be_bytes());
    header.extend_from_slice(&raw);
    base64::engine::general_purpose::STANDARD.encode(header)
}

async fn get_user_token(
    client: &reqwest::Client,
    msa: &str,
    key: &p256::ecdsa::SigningKey,
) -> Result<Value, AuthError> {
    let payload = json!({
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT",
        "Properties": { "AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("t={msa}") }
    });
    let body = serde_json::to_string(&payload)?;
    let sig = sign_request("/user/authenticate", &body, key);
    let res = client
        .post(USER_AUTH_URL)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("signature", sig)
        .header("x-xbl-contract-version", "2")
        .body(body)
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(format!("Xbox user auth failed: {}", res.status()).into());
    }
    Ok(res.json().await?)
}

async fn get_xsts_token(
    client: &reqwest::Client,
    user_token: &str,
    key: &p256::ecdsa::SigningKey,
) -> Result<Value, AuthError> {
    let payload = json!({
        "RelyingParty": MC_RELYING_PARTY,
        "TokenType": "JWT",
        "Properties": { "UserTokens": [user_token], "SandboxId": "RETAIL" }
    });
    let body = serde_json::to_string(&payload)?;
    let sig = sign_request("/xsts/authorize", &body, key);
    let res = client
        .post(XSTS_AUTH_URL)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("signature", sig)
        .header("x-xbl-contract-version", "1")
        .body(body)
        .send()
        .await?;
    let data: Value = res.json().await?;
    if data.get("XErr").is_some() {
        return Err(format!("Xbox XSTS auth failed: {data}").into());
    }
    Ok(data)
}

async fn get_xbox_token(
    client: &reqwest::Client,
    msa: &str,
) -> Result<(String, String), AuthError> {
    use p256::ecdsa::SigningKey;
    let key = SigningKey::random(&mut rand::thread_rng());

    let user = get_user_token(client, msa, &key).await?;
    let user_token = user.get("Token").and_then(|v| v.as_str()).unwrap_or("");

    let xsts = get_xsts_token(client, user_token, &key).await?;
    let xsts_token = xsts
        .get("Token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let user_hash = xsts
        .get("DisplayClaims")
        .and_then(|c| c.get("xui"))
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("uhs"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok((user_hash, xsts_token))
}

// ── Minecraft token + profile ──

async fn login_with_xbox(
    client: &reqwest::Client,
    user_hash: &str,
    xsts: &str,
) -> Result<String, AuthError> {
    let res = client
        .post(LOGIN_URL)
        .header("Content-Type", "application/json")
        .json(&json!({ "identityToken": format!("XBL3.0 x={user_hash};{xsts}") }))
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(format!("Minecraft login failed: {}", res.status()).into());
    }
    let data: Value = res.json().await?;
    Ok(data
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

async fn fetch_profile(client: &reqwest::Client, access_token: &str) -> Result<Value, AuthError> {
    let res = client
        .get(PROFILE_URL)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(format!("Profile fetch failed: {}", res.status()).into());
    }
    Ok(res.json().await?)
}

fn dash_uuid(id: &str) -> String {
    if id.len() == 32 {
        format!(
            "{}-{}-{}-{}-{}",
            &id[0..8],
            &id[8..12],
            &id[12..16],
            &id[16..20],
            &id[20..32]
        )
    } else {
        id.to_string()
    }
}

/// Authenticate with Microsoft and get a Minecraft access token + profile.
pub async fn authenticate_microsoft(options: &AuthOptions) -> Result<AuthResult, AuthError> {
    let dir = options.profiles_folder.clone().unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home)
            .join(".minecraft")
            .join("rustcraft-cache")
    });
    let client = reqwest::Client::new();

    let msa = get_msa_token(&client, &dir, options).await?;
    let (user_hash, xsts) = get_xbox_token(&client, &msa).await?;
    let access_token = login_with_xbox(&client, &user_hash, &xsts).await?;
    let profile = fetch_profile(&client, &access_token).await?;

    let id = profile.get("id").and_then(|v| v.as_str());
    let name = profile.get("name").and_then(|v| v.as_str());
    match (id, name) {
        (Some(id), Some(name)) => Ok(AuthResult {
            access_token,
            username: name.to_string(),
            uuid: dash_uuid(id),
        }),
        _ => Err(format!(
            "Failed to obtain Minecraft profile for {}. Does this account own Minecraft?",
            options.username
        )
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_hash_is_stable_prefix() {
        assert_eq!(hash_username("Steve").len(), 6);
        assert_eq!(hash_username("Steve"), hash_username("Steve"));
    }

    #[test]
    fn token_validity_math() {
        assert!(is_token_valid(Some(now_ms()), Some(3600))); // 1h in seconds
        assert!(!is_token_valid(Some(now_ms() - 7_200_000), Some(3600)));
        assert!(!is_token_valid(None, Some(3600)));
    }

    #[test]
    fn dash_uuid_formats() {
        assert_eq!(
            dash_uuid("00000000000000000000000000000000"),
            "00000000-0000-0000-0000-000000000000"
        );
    }
}
