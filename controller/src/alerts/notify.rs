use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::db::{self, Db};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SendGridConfig {
    pub api_key: String,
    pub from_email: String,
    pub to_emails: String, // comma-separated
}

impl SendGridConfig {
    pub fn is_configured(&self) -> bool {
        !self.api_key.is_empty() && !self.from_email.is_empty() && !self.to_emails.is_empty()
    }

    fn recipients(&self) -> Vec<serde_json::Value> {
        self.to_emails
            .split(',')
            .map(|e| serde_json::json!({"email": e.trim()}))
            .collect()
    }
}

pub async fn load_config(database: &Db) -> SendGridConfig {
    let api_key = db::get_setting(database, "sendgrid_api_key").await.ok().flatten().unwrap_or_default();
    let from_email = db::get_setting(database, "sendgrid_from_email").await.ok().flatten().unwrap_or_default();
    let to_emails = db::get_setting(database, "sendgrid_to_emails").await.ok().flatten().unwrap_or_default();
    SendGridConfig { api_key, from_email, to_emails }
}

pub async fn save_config(database: &Db, config: &SendGridConfig) -> Result<()> {
    db::set_setting(database, "sendgrid_api_key", &config.api_key).await?;
    db::set_setting(database, "sendgrid_from_email", &config.from_email).await?;
    db::set_setting(database, "sendgrid_to_emails", &config.to_emails).await?;
    Ok(())
}

pub struct AlertNotification {
    pub node_id: String,
    pub rule_name: String,
    pub severity: String,
    pub firing: bool,
    pub value: String,
    pub threshold: String,
    pub field: String,
}

pub async fn send(config: &SendGridConfig, alert: &AlertNotification) -> Result<()> {
    if !config.is_configured() {
        return Ok(());
    }

    let status = if alert.firing { "FIRING" } else { "RESOLVED" };
    let subject = format!("{status} — {} ({})", alert.rule_name, alert.node_id);
    let body = format!(
        "<h3>{status} — {}</h3>\
         <p><b>Node:</b> {}</p>\
         <p><b>Severity:</b> {}</p>\
         <p><b>Field:</b> {} = {} (threshold: {})</p>",
        alert.rule_name, alert.node_id, alert.severity,
        alert.field, alert.value, alert.threshold,
    );

    let payload = serde_json::json!({
        "personalizations": [{"to": config.recipients()}],
        "from": {"email": config.from_email},
        "subject": subject,
        "content": [{"type": "text/html", "value": body}],
    });

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.sendgrid.com/v3/mail/send")
        .bearer_auth(&config.api_key)
        .json(&payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("SendGrid error: {text}");
    }
    Ok(())
}

pub async fn send_test(config: &SendGridConfig) -> Result<()> {
    if !config.is_configured() {
        anyhow::bail!("SendGrid not configured");
    }

    let payload = serde_json::json!({
        "personalizations": [{"to": config.recipients()}],
        "from": {"email": config.from_email},
        "subject": "Pillar Test Notification",
        "content": [{"type": "text/html", "value": "<p>This is a test alert from <b>Pillar</b>.</p>"}],
    });

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.sendgrid.com/v3/mail/send")
        .bearer_auth(&config.api_key)
        .json(&payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("SendGrid error: {text}");
    }
    Ok(())
}
