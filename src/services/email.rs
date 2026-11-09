//! Email delivery.
//!
//! The Go baseline dials SMTP per message with `use_ssl` selecting implicit TLS
//! (port 465) or, for plain ports, opportunistic STARTTLS (go-mail's default
//! `StartTLSPolicy`). The transport is built once and shared through
//! `AppState`; tests inject a recording `Mailer`.

use std::time::Duration;

use anyhow::{Context, bail};
use async_trait::async_trait;
use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::EmailConfig;

#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(&self, to: &str, subject: &str, html: &str) -> anyhow::Result<()>;
}

pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: String,
}

#[async_trait]
impl Mailer for SmtpMailer {
    async fn send(&self, to: &str, subject: &str, html: &str) -> anyhow::Result<()> {
        let from: Mailbox = self.from.parse().context("invalid SMTP from address")?;
        let to: Mailbox = to.parse().context("invalid recipient address")?;

        let email = Message::builder()
            .from(from)
            .to(to)
            .subject(subject)
            .header(ContentType::TEXT_HTML)
            .body(html.to_string())
            .context("failed to build email")?;

        self.transport
            .send(email)
            .await
            .context("failed to send email")?;
        Ok(())
    }
}

/// Used when `email.host` is empty. The Go server starts successfully with an
/// incomplete email config and fails only when a request tries to send mail.
pub struct UnconfiguredMailer;

#[async_trait]
impl Mailer for UnconfiguredMailer {
    async fn send(&self, _to: &str, _subject: &str, _html: &str) -> anyhow::Result<()> {
        bail!("email is not configured")
    }
}

pub fn build_mailer(cfg: &EmailConfig) -> anyhow::Result<std::sync::Arc<dyn Mailer>> {
    // lettre is built with `rustls-no-provider`; install ring once before any
    // TLS parameters are created. A second install (tests) returns `Err`.
    let _ = rustls::crypto::ring::default_provider().install_default();

    if cfg.host.trim().is_empty() {
        return Ok(std::sync::Arc::new(UnconfiguredMailer));
    }

    let tls_parameters = TlsParameters::new(cfg.host.clone())
        .with_context(|| format!("invalid SMTP host {}", cfg.host))?;
    let tls = if cfg.use_ssl {
        // Implicit TLS, used by the production 465 endpoint.
        Tls::Wrapper(tls_parameters)
    } else {
        // go-mail's default StartTLSPolicy is opportunistic: upgrade when the
        // server advertises STARTTLS, otherwise continue in plaintext.
        Tls::Opportunistic(tls_parameters)
    };

    let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(cfg.host.clone())
        .port(cfg.port)
        .tls(tls)
        .timeout(Some(Duration::from_secs(10)));

    if !cfg.username.is_empty() {
        builder = builder.credentials(Credentials::new(cfg.username.clone(), cfg.password.clone()));
    }

    Ok(std::sync::Arc::new(SmtpMailer {
        transport: builder.build(),
        from: cfg.username.clone(),
    }))
}

/// Subject and HTML body of the verification email. The code is generated from
/// digits only, so no escaping is required.
pub fn verification_email(code: &str) -> (String, String) {
    (
        format!("Smileyan 验证码：{code}"),
        format!(
            "<html><body><h2>您的验证码是：</h2><h1>{code}</h1><p>有效期10分钟</p></body></html>"
        ),
    )
}

/// HTML-escape user-controlled values embedded in mail bodies. Go's baseline
/// interpolates them raw; Q10 requires escaping as part of the reply-link fix.
pub fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn base_url(site_url: &str) -> &str {
    site_url.trim_end_matches('/')
}

/// `POST /api/subscribe` confirmation. The copy and structure match
/// `controllers/subscription.go`; only `site_url` is configurable.
pub fn subscribe_confirmation_email(site_url: &str, token: &str, email: &str) -> (String, String) {
    let subject = "欢迎订阅 Smileyan 博客".to_string();
    let body = format!(
        "<html><body><h2>感谢您的订阅！</h2><p>您将收到新文章发布通知。</p>\
         <p><a href='{}/unsubscribe?token={}&email={}'>退订</a></p></body></html>",
        base_url(site_url),
        escape_html(token),
        escape_html(email)
    );
    (subject, body)
}

/// Reply notification. Q10 fixes the relative link by prefixing `site_url`;
/// the nickname and comment body are HTML-escaped.
pub fn comment_reply_email(
    site_url: &str,
    nickname: &str,
    content: &str,
    post_slug: &str,
    comment_id: u64,
) -> (String, String) {
    let subject = "您在 Smileyan 博客的评论有新回复".to_string();
    let url = format!(
        "{}/post/{}#comment-{}",
        base_url(site_url),
        escape_html(post_slug),
        comment_id
    );
    let body = format!(
        "<html><body><p>您好，{}</p><p>您有一条新的回复：</p>\
         <blockquote>{}</blockquote><p><a href='{}'>查看回复</a></p></body></html>",
        escape_html(nickname),
        escape_html(content),
        url
    );
    (subject, body)
}

/// New-post notification for active subscribers. The subject stays plain text;
/// the title and excerpt are escaped and the unsubscribe link stays absolute.
pub fn new_post_notification_email(
    site_url: &str,
    title: &str,
    excerpt: &str,
    post_slug: &str,
    token: &str,
    email: &str,
) -> (String, String) {
    let subject = format!("【新文章】{title}");
    let body = format!(
        "<html><body><h2>Smileyan 博客有新文章发布</h2><h3>{}</h3><p>{}</p>\
         <p><a href='{}/post/{}'>阅读全文</a></p><hr>\
         <p><a href='{}/unsubscribe?token={}&email={}'>退订</a></p></body></html>",
        escape_html(title),
        escape_html(excerpt),
        base_url(site_url),
        escape_html(post_slug),
        base_url(site_url),
        escape_html(token),
        escape_html(email)
    );
    (subject, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_email_matches_go_fixture() {
        let (subject, body) = verification_email("123456");
        assert_eq!(subject, "Smileyan 验证码：123456");
        assert_eq!(
            body,
            "<html><body><h2>您的验证码是：</h2><h1>123456</h1><p>有效期10分钟</p></body></html>"
        );
    }

    #[test]
    fn subscribe_confirmation_matches_go_fixture() {
        let (subject, body) = subscribe_confirmation_email(
            "https://smileyan.cn",
            "0123456789abcdef0123456789abcdef",
            "newsub@example.com",
        );
        assert_eq!(subject, "欢迎订阅 Smileyan 博客");
        assert_eq!(
            body,
            "<html><body><h2>感谢您的订阅！</h2><p>您将收到新文章发布通知。</p>\
             <p><a href='https://smileyan.cn/unsubscribe?token=0123456789abcdef0123456789abcdef&email=newsub@example.com'>退订</a></p></body></html>"
        );
    }

    #[test]
    fn reply_email_uses_absolute_url_and_escapes_user_input() {
        let (subject, body) = comment_reply_email(
            "https://smileyan.cn/",
            "<Admin>",
            "hi & <b>bye</b>",
            "published-post",
            101,
        );
        assert_eq!(subject, "您在 Smileyan 博客的评论有新回复");
        assert_eq!(
            body,
            "<html><body><p>您好，&lt;Admin&gt;</p><p>您有一条新的回复：</p>\
             <blockquote>hi &amp; &lt;b&gt;bye&lt;/b&gt;</blockquote>\
             <p><a href='https://smileyan.cn/post/published-post#comment-101'>查看回复</a></p></body></html>"
        );
    }

    #[test]
    fn notify_email_matches_go_fixture() {
        let (subject, body) = new_post_notification_email(
            "https://smileyan.cn",
            "Published Post 已发布",
            "",
            "published-post",
            "0123456789abcdef0123456789abcdef",
            "newsub@example.com",
        );
        assert_eq!(subject, "【新文章】Published Post 已发布");
        assert_eq!(
            body,
            "<html><body><h2>Smileyan 博客有新文章发布</h2>\
             <h3>Published Post 已发布</h3><p></p>\
             <p><a href='https://smileyan.cn/post/published-post'>阅读全文</a></p><hr>\
             <p><a href='https://smileyan.cn/unsubscribe?token=0123456789abcdef0123456789abcdef&email=newsub@example.com'>退订</a></p></body></html>"
        );
    }

    #[test]
    fn escape_html_covers_the_five_characters() {
        assert_eq!(escape_html(r#"<&>"'"#), "&lt;&amp;&gt;&quot;&#39;");
        assert_eq!(escape_html("普通文本"), "普通文本");
    }

    #[tokio::test]
    async fn empty_host_builds_unconfigured_mailer() {
        let mailer = build_mailer(&EmailConfig::default()).expect("builds");
        // The trait object is opaque; send must fail without touching the network.
        let result = mailer.send("user@example.com", "s", "<p>x</p>").await;
        assert!(result.is_err());
    }
}
