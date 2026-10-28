//! Webhook mode (TABLE_API.md section 11): POST the `your_turn` envelope, signed with
//! `X-Sharingan-Signature: sha256=HMAC(secret, body)`, and read the `act` data from the response.

use crate::hand::ActReq;
use crate::util::hmac_sha256;
use serde_json::Value;
use std::time::Duration;

pub async fn ask(url: String, secret: String, body: String, timeout: Duration) -> Result<(ActReq, Option<String>), String> {
    let r = tokio::task::spawn_blocking(move || {
        let sig = format!("sha256={}", hex::encode(hmac_sha256(secret.as_bytes(), body.as_bytes())));
        let resp = ureq::post(&url)
            .timeout(timeout)
            .set("content-type", "application/json")
            .set("x-sharingan-signature", &sig)
            .send_string(&body)
            .map_err(|e| e.to_string())?;
        let v: Value = resp.into_json().map_err(|e| e.to_string())?;
        Ok::<Value, String>(v)
    })
    .await
    .map_err(|e| e.to_string())??;
    // Accept either the bare act data or a full `act` envelope.
    let data = if r.get("data").is_some() { r["data"].clone() } else { r };
    let kind = data["type"].as_str().ok_or("response has no `type`")?.to_string();
    let to = data["to"].as_i64();
    let decision = data["decision_id"].as_str().map(|s| s.to_string());
    Ok((ActReq { kind, to }, decision))
}
