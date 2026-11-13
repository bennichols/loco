//! This module defines an [`EmailSender`] responsible for sending emails using
//! either the SMTP protocol. It includes an asynchronous method `mail` for
//! sending emails with options like sender, recipient, subject, and content.

use std::time::Duration;

use lettre::{
    message::{
        header::{self, HeaderName, HeaderValue},
        MessageBuilder, MultiPart,
    },
    transport::smtp::{authentication::Credentials, extension::ClientId},
    AsyncTransport, Message, Tokio1Executor, Transport,
};
use tracing::error;

use super::{Email, EmailHeaders, MultiEmail, Result, DEFAULT_FROM_SENDER};
use crate::{config, errors::Error};

/// Headers Loco sets itself, from [`Email`]'s fields or the MIME structure,
/// so they can't be given as [`EmailHeaders::custom`].
const RESERVED_HEADERS: &[&str] = &[
    "From",
    "To",
    "Cc",
    "Bcc",
    "Reply-To",
    "Subject",
    "References",
    "In-Reply-To",
    "Message-ID",
    "Content-Type",
    "MIME-Version",
    "Content-Transfer-Encoding",
];

/// An enumeration representing the possible transport methods for sending
/// emails.
#[derive(Clone, Debug)]
pub enum EmailTransport {
    /// SMTP (Simple Mail Transfer Protocol) transport.
    Smtp(lettre::AsyncSmtpTransport<lettre::Tokio1Executor>),
    /// Test/stub transport for testing purposes.
    Test(lettre::transport::stub::StubTransport),
}

/// A structure representing the email sender, encapsulating the chosen
/// transport method.
#[derive(Clone, Debug)]
pub struct EmailSender {
    pub transport: EmailTransport,
}

#[cfg(feature = "testing-core")]
#[derive(Default, Debug)]
pub struct Deliveries {
    pub count: usize,
    pub messages: Vec<String>,
}

impl EmailSender {
    /// Creates a new `EmailSender` using the SMTP transport method based on the
    /// provided SMTP configuration.
    ///
    /// # Errors
    ///
    /// when could not initialize SMTP transport
    pub fn smtp(config: &config::SmtpMailer) -> Result<Self> {
        let host = &config.host;
        let mut email_builder = match config.tls_mode() {
            // Opportunistic upgrade on a cleartext connection (STARTTLS, port 587).
            config::MailerTls::Starttls => {
                lettre::AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
                    .map_err(|error| {
                        error!(err.msg = %error, err.detail = ?error, "smtp_init_error");
                        error
                    })?
                    .port(config.port)
            }
            // Implicit TLS — encrypted from the first byte (SMTPS, port 465).
            config::MailerTls::Implicit => {
                lettre::AsyncSmtpTransport::<Tokio1Executor>::relay(host)
                    .map_err(|error| {
                        error!(err.msg = %error, err.detail = ?error, "smtp_init_error");
                        error
                    })?
                    .port(config.port)
            }
            // Cleartext (no TLS).
            config::MailerTls::None => {
                lettre::AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
                    .port(config.port)
            }
        };

        if let Some(auth) = config.auth.as_ref() {
            email_builder = email_builder
                .credentials(Credentials::new(auth.user.clone(), auth.password.clone()));
        }

        if let Some(hello_name) = config.hello_name.as_ref() {
            email_builder = email_builder.hello_name(ClientId::Domain(hello_name.clone()));
        }

        if let Some(timeout) = config.timeout {
            email_builder = email_builder.timeout(Some(Duration::from_millis(timeout)));
        }

        Ok(Self {
            transport: EmailTransport::Smtp(email_builder.build()),
        })
    }

    #[must_use]
    pub fn stub() -> Self {
        Self {
            transport: EmailTransport::Test(lettre::transport::stub::StubTransport::new_ok()),
        }
    }

    #[cfg(feature = "testing-core")]
    #[must_use]
    pub fn deliveries(&self) -> Deliveries {
        if let EmailTransport::Test(stub) = &self.transport {
            return Deliveries {
                count: stub.messages().len(),
                messages: stub
                    .messages()
                    .iter()
                    .map(|(_, content)| content.clone())
                    .collect(),
            };
        }

        Deliveries::default()
    }

    /// Sends an email using the configured transport method.
    ///
    /// # Errors
    ///
    /// When email doesn't send successfully or has an error to build the
    /// message
    pub async fn mail(&self, email: &Email) -> Result<()> {
        let content = MultiPart::alternative_plain_html(email.text.clone(), email.html.clone());
        let mut builder = Message::builder()
            .from(
                email
                    .from
                    .clone()
                    .unwrap_or_else(|| DEFAULT_FROM_SENDER.to_string())
                    .parse()?,
            )
            .to(email.to.parse()?);

        if let Some(bcc) = &email.bcc {
            builder = builder.bcc(bcc.parse()?);
        }

        if let Some(cc) = &email.cc {
            builder = builder.cc(cc.parse()?);
        }

        if let Some(reply_to) = &email.reply_to {
            builder = builder.reply_to(reply_to.parse()?);
        }

        if let Some(headers) = &email.headers {
            builder = with_headers(builder, headers)?;
        }

        let msg = builder
            .subject(email.subject.clone())
            .multipart(content)
            .map_err(|error| {
                error!(err.msg = %error, err.detail = ?error, "email_building_error");
                error
            })?;

        self.send(msg).await
    }

    /// Sends an email with multiple recipients using the configured transport
    /// method.
    ///
    /// # Errors
    ///
    /// When email doesn't send successfully or has an error to build the
    /// message
    pub async fn mail_multi(&self, email: &MultiEmail) -> Result<()> {
        let content = MultiPart::alternative_plain_html(email.text.clone(), email.html.clone());
        let mut builder = Message::builder().from(
            email
                .from
                .clone()
                .unwrap_or_else(|| DEFAULT_FROM_SENDER.to_string())
                .parse()?,
        );

        for to in &email.to {
            builder = builder.to(to.parse()?);
        }

        for bcc in &email.bcc {
            builder = builder.bcc(bcc.parse()?);
        }

        for cc in &email.cc {
            builder = builder.cc(cc.parse()?);
        }

        if let Some(reply_to) = &email.reply_to {
            builder = builder.reply_to(reply_to.parse()?);
        }

        if let Some(headers) = &email.headers {
            builder = with_headers(builder, headers)?;
        }

        let msg = builder
            .subject(email.subject.clone())
            .multipart(content)
            .map_err(|error| {
                error!(err.msg = %error, err.detail = ?error, "email_building_error");
                error
            })?;

        self.send(msg).await
    }

    /// Sends a pre-built message using the configured transport method.
    async fn send(&self, msg: lettre::Message) -> Result<()> {
        match &self.transport {
            EmailTransport::Smtp(xp) => {
                xp.send(msg).await?;
            }
            EmailTransport::Test(xp) => {
                xp.send(&msg)
                    .map_err(|e| Error::Message(format!("sending email error: {e}")))?;
            }
        }
        Ok(())
    }
}

/// Adds the threading headers and the custom ones to a message.
///
/// # Errors
///
/// When a custom header's name is invalid, is one Loco sets itself, or is
/// given more than once
fn with_headers(mut builder: MessageBuilder, headers: &EmailHeaders) -> Result<MessageBuilder> {
    if let Some(references) = &headers.references {
        builder = builder.header(header::References::from(references.clone()));
    }
    if let Some(in_reply_to) = &headers.in_reply_to {
        builder = builder.header(header::InReplyTo::from(in_reply_to.clone()));
    }
    if let Some(message_id) = &headers.message_id {
        builder = builder.header(header::MessageId::from(message_id.clone()));
    }

    for (index, (name, value)) in headers.custom.iter().enumerate() {
        if RESERVED_HEADERS
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(name))
        {
            return Err(Error::Message(format!(
                "email header `{name}` is set by Loco and can't be a custom header"
            )));
        }
        // lettre replaces a header of the same name, so a repeat would drop
        // the earlier value.
        if headers.custom[..index]
            .iter()
            .any(|(earlier, _)| earlier.eq_ignore_ascii_case(name))
        {
            return Err(Error::Message(format!(
                "email header `{name}` is given more than once"
            )));
        }
        // lettre checks the length and rejects spaces and colons; a field name
        // must also be printable ASCII (RFC 5322 2.2), so a line break can't
        // start another header.
        let invalid = || Error::Message(format!("invalid email header name {name:?}"));
        if !name.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(invalid());
        }
        let header_name = HeaderName::new_from_ascii(name.clone()).map_err(|_| invalid())?;
        builder = builder.raw_header(HeaderValue::new(header_name, value.clone()));
    }

    Ok(builder)
}

#[cfg(test)]
mod tests {

    use insta::{assert_debug_snapshot, with_settings};
    use lettre::transport::stub::StubTransport;

    use super::*;

    #[test]
    fn smtp_builds_for_each_tls_mode() {
        use crate::config::{MailerTls, SmtpMailer};

        let cfg = |tls: Option<MailerTls>, secure: bool| SmtpMailer {
            enable: true,
            host: "smtp.example.com".to_string(),
            port: 465,
            secure,
            tls,
            auth: None,
            hello_name: None,
            timeout: None,
        };

        // Every explicit TLS mode builds a transport (incl. implicit TLS for 465 — the case
        // that was previously impossible: `secure: true` only ever did STARTTLS).
        assert!(EmailSender::smtp(&cfg(Some(MailerTls::Starttls), false)).is_ok());
        assert!(EmailSender::smtp(&cfg(Some(MailerTls::Implicit), false)).is_ok());
        assert!(EmailSender::smtp(&cfg(Some(MailerTls::None), false)).is_ok());

        // Legacy `secure` flag keeps mapping as before when `tls` is unset.
        assert_eq!(cfg(None, true).tls_mode(), MailerTls::Starttls);
        assert_eq!(cfg(None, false).tls_mode(), MailerTls::None);
        assert!(EmailSender::smtp(&cfg(None, true)).is_ok());

        // Explicit `tls` takes precedence over `secure`.
        assert_eq!(
            cfg(Some(MailerTls::Implicit), true).tls_mode(),
            MailerTls::Implicit
        );
    }

    #[tokio::test]
    async fn can_send_email() {
        let stub = StubTransport::new_ok();

        let sender = EmailSender {
            transport: EmailTransport::Test(stub.clone()),
        };

        let html = r"
;<html>
    <body>
        Test Message
    </body>
</html>";

        let data = Email {
            from: Some("test@framework.com".to_string()),
            to: "user1@framework.com".to_string(),
            reply_to: None,
            subject: "Email Subject".to_string(),
            text: "Welcome".to_string(),
            html: html.to_string(),
            bcc: None,
            cc: None,
            headers: None,
        };
        assert!(sender.mail(&data).await.is_ok());

        with_settings!({filters => vec![
            (r"[0-9A-Za-z]+{40}", "IDENTIFIER"),
            (r"\w+, \d{1,2} \w+ \d{4} \d{2}:\d{2}:\d{2} [+-]\d{4}", "DATE")
        ]}, {
            assert_debug_snapshot!(stub.messages());
        });
    }

    #[tokio::test]
    async fn can_send_email_with_custom_headers() {
        let stub = StubTransport::new_ok();

        let sender = EmailSender {
            transport: EmailTransport::Test(stub.clone()),
        };

        let html = r"
<html>
    <body>
        Test Message with Headers
    </body>
</html>";

        let headers = crate::mailer::EmailHeaders {
            references: Some("<notification-item-123@example.com>".to_string()),
            in_reply_to: Some("<notification-item-123@example.com>".to_string()),
            message_id: Some("<notification-item-123-1234567890@example.com>".to_string()),
            custom: vec![],
        };

        let data = Email {
            from: Some("test@framework.com".to_string()),
            to: "user1@framework.com".to_string(),
            reply_to: None,
            subject: "Email Subject with Headers".to_string(),
            text: "Welcome with headers".to_string(),
            html: html.to_string(),
            bcc: None,
            cc: None,
            headers: Some(headers),
        };
        assert!(sender.mail(&data).await.is_ok());

        with_settings!({filters => vec![
            (r"[0-9A-Za-z]+{40}", "IDENTIFIER"),
            (r"\w+, \d{1,2} \w+ \d{4} \d{2}:\d{2}:\d{2} [+-]\d{4}", "DATE")
        ]}, {
            assert_debug_snapshot!(stub.messages());
        });
    }

    #[tokio::test]
    async fn can_send_multi_email_with_multiple_to_recipients() {
        let stub = StubTransport::new_ok();
        let sender = EmailSender {
            transport: EmailTransport::Test(stub.clone()),
        };
        let data = MultiEmail {
            from: Some("test@framework.com".to_string()),
            to: vec![
                "user1@framework.com".to_string(),
                "user2@framework.com".to_string(),
            ],
            subject: "Multi-To".to_string(),
            text: "Hello".to_string(),
            html: "<html><body>Hello</body></html>".to_string(),
            ..Default::default()
        };
        assert!(sender.mail_multi(&data).await.is_ok());
        with_settings!({filters => vec![
            (r"[0-9A-Za-z]+{40}", "IDENTIFIER"),
            (r"\w+, \d{1,2} \w+ \d{4} \d{2}:\d{2}:\d{2} [+-]\d{4}", "DATE")
        ]}, {
            assert_debug_snapshot!(stub.messages());
        });
    }

    #[tokio::test]
    async fn can_send_multi_email_with_multiple_bcc_recipients() {
        let stub = StubTransport::new_ok();
        let sender = EmailSender {
            transport: EmailTransport::Test(stub.clone()),
        };
        let data = MultiEmail {
            from: Some("test@framework.com".to_string()),
            to: vec!["user1@framework.com".to_string()],
            bcc: vec![
                "bcc1@framework.com".to_string(),
                "bcc2@framework.com".to_string(),
            ],
            subject: "Multi-BCC".to_string(),
            text: "Hello".to_string(),
            html: "<html><body>Hello</body></html>".to_string(),
            ..Default::default()
        };
        assert!(sender.mail_multi(&data).await.is_ok());
        with_settings!({filters => vec![
            (r"[0-9A-Za-z]+{40}", "IDENTIFIER"),
            (r"\w+, \d{1,2} \w+ \d{4} \d{2}:\d{2}:\d{2} [+-]\d{4}", "DATE")
        ]}, {
            assert_debug_snapshot!(stub.messages());
        });
    }

    #[tokio::test]
    async fn can_send_multi_email_with_multiple_cc_recipients() {
        let stub = StubTransport::new_ok();
        let sender = EmailSender {
            transport: EmailTransport::Test(stub.clone()),
        };
        let data = MultiEmail {
            from: Some("test@framework.com".to_string()),
            to: vec!["user1@framework.com".to_string()],
            cc: vec![
                "cc1@framework.com".to_string(),
                "cc2@framework.com".to_string(),
            ],
            subject: "Multi-CC".to_string(),
            text: "Hello".to_string(),
            html: "<html><body>Hello</body></html>".to_string(),
            ..Default::default()
        };
        assert!(sender.mail_multi(&data).await.is_ok());
        with_settings!({filters => vec![
            (r"[0-9A-Za-z]+{40}", "IDENTIFIER"),
            (r"\w+, \d{1,2} \w+ \d{4} \d{2}:\d{2}:\d{2} [+-]\d{4}", "DATE")
        ]}, {
            assert_debug_snapshot!(stub.messages());
        });
    }

    #[tokio::test]
    async fn can_send_email_with_arbitrary_headers() {
        let stub = StubTransport::new_ok();
        let sender = EmailSender {
            transport: EmailTransport::Test(stub.clone()),
        };
        let headers = crate::mailer::EmailHeaders {
            custom: vec![
                (
                    "List-Unsubscribe".to_string(),
                    "<https://example.com/unsubscribe/abc>, <mailto:unsubscribe@example.com>"
                        .to_string(),
                ),
                (
                    "List-Unsubscribe-Post".to_string(),
                    "List-Unsubscribe=One-Click".to_string(),
                ),
                ("X-Tenant-Name".to_string(), "Café Société".to_string()),
                (
                    "X-Note".to_string(),
                    "first\r\nBcc: attacker@example.com".to_string(),
                ),
            ],
            ..Default::default()
        };

        let data = Email {
            from: Some("test@framework.com".to_string()),
            to: "user1@framework.com".to_string(),
            subject: "Email Subject".to_string(),
            text: "Welcome".to_string(),
            html: "<html><body>Welcome</body></html>".to_string(),
            headers: Some(headers.clone()),
            ..Default::default()
        };
        assert!(sender.mail(&data).await.is_ok());

        let multi = MultiEmail {
            from: Some("test@framework.com".to_string()),
            to: vec![
                "user1@framework.com".to_string(),
                "user2@framework.com".to_string(),
            ],
            subject: "Multi-Headers".to_string(),
            text: "Hello".to_string(),
            html: "<html><body>Hello</body></html>".to_string(),
            headers: Some(headers),
            ..Default::default()
        };
        assert!(sender.mail_multi(&multi).await.is_ok());

        with_settings!({filters => vec![
            (r"[0-9A-Za-z]+{40}", "IDENTIFIER"),
            (r"\w+, \d{1,2} \w+ \d{4} \d{2}:\d{2}:\d{2} [+-]\d{4}", "DATE")
        ]}, {
            assert_debug_snapshot!(stub.messages());
        });
    }

    #[tokio::test]
    async fn refuses_invalid_reserved_or_repeated_custom_header_names() {
        let stub = StubTransport::new_ok();
        let sender = EmailSender {
            transport: EmailTransport::Test(stub.clone()),
        };

        let cases = [
            // Invalid names, quoted in the error.
            ("", r#""""#),
            ("X Bad", r#""X Bad""#),
            ("X-Bad:", r#""X-Bad:""#),
            ("X-Bad\r\nBcc", r#""X-Bad\r\nBcc""#),
            ("X-Ünicode", r#""X-Ünicode""#),
            // Names Loco sets itself, in any case.
            ("from", "`from`"),
            ("TO", "`TO`"),
            ("Cc", "`Cc`"),
            ("bcc", "`bcc`"),
            ("Reply-To", "`Reply-To`"),
            ("subject", "`subject`"),
            ("References", "`References`"),
            ("in-reply-to", "`in-reply-to`"),
            ("MESSAGE-ID", "`MESSAGE-ID`"),
            ("Content-Type", "`Content-Type`"),
            ("mime-version", "`mime-version`"),
            ("Content-Transfer-Encoding", "`Content-Transfer-Encoding`"),
            // A repeat, in any case, of the `X-Tag` each email starts with.
            ("x-tag", "`x-tag`"),
        ];

        for (name, shown) in cases {
            let headers = Some(crate::mailer::EmailHeaders {
                custom: vec![
                    ("X-Tag".to_string(), "first".to_string()),
                    (name.to_string(), "value".to_string()),
                ],
                ..Default::default()
            });
            let email = Email {
                to: "user1@framework.com".to_string(),
                headers: headers.clone(),
                ..Default::default()
            };
            let multi = MultiEmail {
                to: vec!["user1@framework.com".to_string()],
                headers,
                ..Default::default()
            };

            for err in [
                sender.mail(&email).await.expect_err(name),
                sender.mail_multi(&multi).await.expect_err(name),
            ] {
                assert!(err.to_string().contains(shown), "{name:?}: {err}");
            }
        }
        assert!(stub.messages().is_empty());
    }

    #[test]
    fn smtp_applies_the_configured_timeout() {
        let transport_debug = |yaml: &str| {
            let cfg: crate::config::SmtpMailer = serde_yaml::from_str(yaml).unwrap();
            let EmailTransport::Smtp(transport) = EmailSender::smtp(&cfg).unwrap().transport else {
                panic!("expected an SMTP transport");
            };
            // lettre has no accessor for the timeout, so read it from `Debug`.
            format!("{transport:?}")
        };
        let yaml = "enable: true\nhost: smtp.example.com\nport: 587\nsecure: true";

        assert!(transport_debug(&format!("{yaml}\ntimeout: 5000")).contains("timeout: Some(5s)"));
        // Unset (the key is optional) keeps lettre's own default.
        assert!(transport_debug(yaml).contains("timeout: Some(60s)"));
    }
}
