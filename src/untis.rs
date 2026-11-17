use crate::Result;
use reqwest::Client;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::time::Duration;
use totp_rs::{Builder, Secret, Totp};

pub const HOST: &str = "heinrich-hertz-bk.webuntis.com";

pub struct Untis {
    client: Client,
    user: String,
    otp: Totp,
}

#[derive(Deserialize)]
struct Rpc<T> {
    result: Option<T>,
    error: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    user_data: User,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct User {
    elem_id: i64,
    elem_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimetableResult {
    pub timetable: Timetable,
    pub master_data: MasterData,
}

#[derive(Deserialize)]
pub struct Timetable {
    pub periods: Vec<Period>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Period {
    pub id: i64,
    pub start_date_time: String,
    pub end_date_time: String,
    #[serde(rename = "is")]
    pub states: Vec<String>,
    pub elements: Vec<Element>,
    pub text: Text,
    pub exam: Option<Exam>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: i64,
    pub org_id: i64,
}

#[derive(Deserialize)]
pub struct Text {
    pub lesson: String,
    pub substitution: String,
    pub info: String,
}

#[derive(Deserialize)]
pub struct Exam {
    pub examtype: Option<String>,
    pub name: Option<String>,
    pub text: Option<String>,
}

#[derive(Deserialize)]
pub struct MasterData {
    pub subjects: Vec<Name>,
    pub rooms: Vec<Name>,
    pub teachers: Vec<Name>,
    pub klassen: Vec<Name>,
}

#[derive(Deserialize)]
pub struct Name {
    pub id: i64,
    pub name: String,
}

fn otp(secret: &str) -> Result<Totp> {
    let secret = Secret::try_from_base32(secret).map_err(|_| "invalid mobile key encoding")?;
    if secret.as_bytes().len() < 10 {
        return Err("mobile key must contain at least 80 bits".into());
    }
    // WebUntis issues 80-bit keys. Only its key length differs from RFC 4226;
    // the fixed SHA-1, six digits and 30-second step use the library defaults.
    Ok(Builder::new().with_secret(secret).build_noncompliant())
}

impl Untis {
    pub fn new(user: String, secret: &str) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .user_agent(concat!("hhek-untis2ics/", env!("CARGO_PKG_VERSION")))
                .build()?,
            user,
            otp: otp(secret)?,
        })
    }

    async fn rpc<T: DeserializeOwned>(&self, method: &str, mut params: Value) -> Result<T> {
        let now = jiff::Timestamp::now();
        params["auth"] = json!({
            "user": self.user,
            "otp": self.otp.generate(now.as_second() as u64).to_string().parse::<u32>()?,
            "clientTime": now.as_millisecond(),
        });
        let mut response = self.client
            .post(format!("https://{HOST}/WebUntis/jsonrpc_intern.do?school=heinrich-hertz-bk&v=i2.2&m={method}"))
            .json(&json!({"id": "hhek-untis2ics", "jsonrpc": "2.0", "method": method, "params": [params]}))
            .send().await?.error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
                return Err("WebUntis response exceeds 16 MiB".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let reply: Rpc<T> = serde_json::from_slice(&bytes)?;
        if let Some(error) = reply.error {
            return Err(format!("WebUntis {method} failed (code {})", error["code"]).into());
        }
        reply
            .result
            .ok_or_else(|| "WebUntis returned no result".into())
    }

    pub async fn fetch(&self) -> Result<TimetableResult> {
        let profile: Profile = self
            .rpc(
                "getUserData2017",
                json!({
                    "elementId": 0, "deviceOs": "AND", "deviceOsVersion": ""
                }),
            )
            .await?;
        let today = jiff::Timestamp::now().in_tz("Europe/Berlin")?.date();
        self.rpc(
            "getTimetable2017",
            json!({
                "id": profile.user_data.elem_id,
                "type": profile.user_data.elem_type,
                "startDate": today.checked_sub(jiff::Span::new().days(14))?.to_string(),
                "endDate": today.checked_add(jiff::Span::new().days(90))?.to_string(),
                "masterDataTimestamp": 0,
                "timetableTimestamp": 0,
                "timetableTimestamps": [],
            }),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_and_short_webuntis_key() {
        let totp = otp("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").unwrap();
        assert_eq!(totp.generate(59).to_string(), "287082");
        assert_eq!(totp.generate(1111111109).to_string(), "081804");
        assert!(otp("GEZDGNBVGY3TQOJQ").is_ok());
        assert!(otp("AAA").is_err());
        assert!(otp("not a key!").is_err());
    }
}
