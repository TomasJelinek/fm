//! Linux transport: ureq over rustls with the ring provider, trusting the
//! distribution's certificate store (rustls-native-certs). No OpenSSL is linked.
//! Redirects are turned off; the client follows them after its checks.
use crate::transport::{
    CONNECT_TIMEOUT, Head, RECEIVE_TIMEOUT, Request, Sink, Transport, TransportError, USER_AGENT,
    UrlPolicy, parse_content_range,
};
use std::{io::Read, sync::Arc};
use ureq::{
    Agent,
    tls::{Certificate, RootCerts, TlsConfig, TlsProvider},
};

/// The system transport. Certificates are loaded once per transport.
pub struct SystemTransport {
    policy: UrlPolicy,
    agent: Result<Agent, String>,
}

impl Default for SystemTransport {
    fn default() -> Self {
        Self::new(UrlPolicy::GitHub)
    }
}

impl SystemTransport {
    /// `policy` decides which addresses the transport connects to; the client checks
    /// the same policy, and this second check keeps a misused transport from plain HTTP.
    pub fn new(policy: UrlPolicy) -> Self {
        Self {
            agent: agent(&policy),
            policy,
        }
    }
}

fn agent(policy: &UrlPolicy) -> Result<Agent, String> {
    let loaded = rustls_native_certs::load_native_certs();
    let roots: Vec<Certificate<'static>> = loaded
        .certs
        .iter()
        .map(|der| Certificate::from_der(der.as_ref()).to_owned())
        .collect();
    if roots.is_empty() && *policy == UrlPolicy::GitHub {
        return Err("no trusted certificates were found on this system".into());
    }
    let tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(RootCerts::new_with_certs(&roots))
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    let config = Agent::config_builder()
        .https_only(*policy == UrlPolicy::GitHub)
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .user_agent(USER_AGENT)
        .accept_encoding("identity")
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(RECEIVE_TIMEOUT))
        .timeout_per_read(Some(RECEIVE_TIMEOUT))
        .tls_config(tls)
        .build();
    Ok(config.new_agent())
}

fn header_number(response: &ureq::http::Response<ureq::Body>, name: &str) -> Option<u64> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
}

fn transport_error(error: ureq::Error) -> TransportError {
    match error {
        ureq::Error::Timeout(_) => TransportError::TimedOut,
        ureq::Error::HostNotFound => TransportError::Unreachable("name not resolved".into()),
        ureq::Error::ConnectionFailed => TransportError::Unreachable("connection failed".into()),
        ureq::Error::Io(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
            TransportError::Unreachable("connection refused".into())
        }
        ureq::Error::Tls(detail) => TransportError::Tls(detail.into()),
        ureq::Error::Rustls(error) => TransportError::Tls(error.to_string()),
        other => TransportError::Protocol(other.to_string()),
    }
}

impl Transport for SystemTransport {
    fn get(&self, request: &Request, sink: &mut dyn Sink) -> Result<Head, TransportError> {
        self.policy
            .check(&request.url)
            .map_err(TransportError::Protocol)?;
        let agent = self
            .agent
            .as_ref()
            .map_err(|detail| TransportError::Tls(detail.clone()))?;
        let mut builder = agent.get(&request.url).header("Accept", request.accept);
        if request.accept.contains("github") {
            builder = builder.header("X-GitHub-Api-Version", "2022-11-28");
        }
        if request.range_from > 0 {
            builder = builder.header("Range", format!("bytes={}-", request.range_from));
        }
        let mut response = builder.call().map_err(transport_error)?;
        let head = Head {
            status: response.status().as_u16(),
            location: response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
            content_length: header_number(&response, "content-length"),
            range_start: response
                .headers()
                .get("content-range")
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range),
            retry_after: header_number(&response, "retry-after"),
            rate_remaining: header_number(&response, "x-ratelimit-remaining"),
            rate_reset: header_number(&response, "x-ratelimit-reset"),
        };
        if !head.is_success() {
            return Ok(head);
        }
        sink.begin(&head).map_err(TransportError::Sink)?;
        let mut reader = response.body_mut().with_config().limit(u64::MAX).reader();
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| TransportError::Protocol(error.to_string()))?;
            if read == 0 {
                break;
            }
            sink.write(&buffer[..read]).map_err(TransportError::Sink)?;
        }
        Ok(head)
    }
}
