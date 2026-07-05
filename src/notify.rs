use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde_json::json;

/// Outbound alert channels, configured from environment variables. Any
/// channel whose configuration is absent is silently disabled — matches the
/// rest of the app's "degrade gracefully" approach (e.g. the Docker socket
/// being unreachable doesn't crash the process either).
pub struct Notifier {
    client: reqwest::Client,
    smtp: Option<AsyncSmtpTransport<Tokio1Executor>>,
    email_from: Option<String>,
    email_to: Vec<String>,
    webhook_url: Option<String>,
}

impl Notifier {
    pub fn new(config: &crate::config::Config) -> Self {
        let smtp = config.smtp_host.as_deref().and_then(|host| {
            let mut builder = AsyncSmtpTransport::<Tokio1Executor>::relay(host)
                .inspect_err(|err| tracing::warn!(%err, "invalid SMTP host, email disabled"))
                .ok()?
                .port(config.smtp_port);

            if let (Some(user), Some(password)) = (&config.smtp_user, &config.smtp_password) {
                builder = builder.credentials(Credentials::new(user.clone(), password.clone()));
            }

            Some(builder.build())
        });

        Self {
            client: reqwest::Client::new(),
            smtp,
            email_from: config.email_from.clone(),
            email_to: config.email_to_list(),
            webhook_url: config.webhook_url.clone(),
        }
    }

    async fn send_email(&self, subject: &str, html: &str) {
        let (Some(smtp), Some(from)) = (&self.smtp, &self.email_from) else { return };
        if self.email_to.is_empty() {
            return;
        }

        let mut builder = Message::builder()
            .from(match from.parse() {
                Ok(mbox) => mbox,
                Err(err) => {
                    tracing::warn!(%err, "invalid email_from address");
                    return;
                }
            })
            .subject(subject)
            .header(ContentType::TEXT_HTML);

        for to in &self.email_to {
            match to.parse() {
                Ok(mbox) => builder = builder.to(mbox),
                Err(err) => tracing::warn!(%err, address = %to, "invalid email_to address, skipped"),
            }
        }

        let message = match builder.body(html.to_string()) {
            Ok(m) => m,
            Err(err) => {
                tracing::warn!(%err, "failed to build email message");
                return;
            }
        };

        if let Err(err) = smtp.send(message).await {
            tracing::warn!(%err, "smtp email send failed");
        }
    }

    async fn send_webhook(&self, content: &str) {
        let Some(url) = &self.webhook_url else { return };

        let result = self.client.post(url).json(&json!({ "content": content })).send().await;

        match result {
            Ok(resp) if !resp.status().is_success() => {
                tracing::warn!(status = %resp.status(), "alert webhook send failed");
            }
            Err(err) => tracing::warn!(%err, "alert webhook send failed"),
            Ok(_) => {}
        }
    }
}

/// Fires the "incident triggered" notification on both channels concurrently.
/// Intended to be `tokio::spawn`ed so a slow/unreachable endpoint never
/// blocks the collector loop.
pub async fn notify_triggered(notifier: std::sync::Arc<Notifier>, component: String, message: String) {
    let subject = format!("[Alerte RustMon] {component}");
    let html = format!("<strong>{message}</strong><br>Connectez-vous au dashboard pour acquitter l'alerte.");
    let content = format!("⚠️ **Alerte Serveur** : {message}");

    tokio::join!(notifier.send_email(&subject, &html), notifier.send_webhook(&content));
}

/// Fires the "incident resolved" notification on both channels concurrently.
pub async fn notify_resolved(notifier: std::sync::Arc<Notifier>, component: String, message: String) {
    let subject = format!("[RustMon] Incident résolu : {component}");
    let html = format!("<strong>Incident résolu</strong><br>{message}");
    let content = format!("✅ **Incident résolu** : {message}");

    tokio::join!(notifier.send_email(&subject, &html), notifier.send_webhook(&content));
}
