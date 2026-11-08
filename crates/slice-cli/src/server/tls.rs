use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;

pub(crate) fn decode_pem_blocks(raw: &[u8]) -> Vec<(String, Vec<u8>)> {
    let Ok(text) = std::str::from_utf8(raw) else {
        return Vec::new();
    };
    let mut blocks = Vec::new();
    let mut cur_label: Option<String> = None;
    let mut b64_buf = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("-----BEGIN ") {
            if let Some(label) = rest.strip_suffix("-----") {
                cur_label = Some(label.trim().to_string());
                b64_buf.clear();
            }
        } else if trimmed.starts_with("-----END ") {
            if let Some(label) = cur_label.take() {
                let der = decode_base64_simple(&b64_buf);
                if !der.is_empty() {
                    blocks.push((label, der));
                }
            }
            b64_buf.clear();
        } else if cur_label.is_some() {
            b64_buf.push_str(trimmed);
        }
    }
    blocks
}

pub(crate) fn decode_base64_simple(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for b in s.bytes() {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => continue,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    out
}

pub(crate) fn load_tls_server_config(
    cert_path: &Path,
    key_path: &Path,
) -> Result<std::sync::Arc<rustls::ServerConfig>> {
    use rustls_pki_types::{
        CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, PrivateSec1KeyDer,
    };

    let cert_raw = std::fs::read(cert_path)
        .with_context(|| format!("Failed to read TLS certificate '{}'", cert_path.display()))?;
    let key_raw = std::fs::read(key_path)
        .with_context(|| format!("Failed to read TLS private key '{}'", key_path.display()))?;

    let cert_blocks = decode_pem_blocks(&cert_raw);
    let certs: Vec<CertificateDer<'static>> = if !cert_blocks.is_empty() {
        cert_blocks
            .into_iter()
            .filter(|(label, _)| label.contains("CERTIFICATE"))
            .map(|(_, der)| CertificateDer::from(der))
            .collect()
    } else {
        vec![CertificateDer::from(cert_raw)]
    };
    if certs.is_empty() {
        anyhow::bail!("No X.509 certificates found in '{}'", cert_path.display());
    }

    let key_blocks = decode_pem_blocks(&key_raw);
    let priv_key: PrivateKeyDer<'static> = if let Some((label, der)) = key_blocks.into_iter().next()
    {
        if label.contains("RSA PRIVATE KEY") {
            PrivateKeyDer::Pkcs1(PrivatePkcs1KeyDer::from(der))
        } else if label.contains("EC PRIVATE KEY") {
            PrivateKeyDer::Sec1(PrivateSec1KeyDer::from(der))
        } else {
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(der))
        }
    } else {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_raw))
    };

    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .context("Failed to configure TLS 1.3/1.2 protocol versions")?
        .with_no_client_auth()
        .with_single_cert(certs, priv_key)
        .context("Invalid TLS certificate or private key")?;
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(std::sync::Arc::new(cfg))
}

pub(crate) enum ServerConnStream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ServerConnection, TcpStream>>),
}

impl Read for ServerConnStream {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.read(buf),
            Self::Tls(s) => s.read(buf),
        }
    }
}

impl Write for ServerConnStream {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.write(buf),
            Self::Tls(s) => s.write(buf),
        }
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(s) => s.flush(),
            Self::Tls(s) => s.flush(),
        }
    }
}
