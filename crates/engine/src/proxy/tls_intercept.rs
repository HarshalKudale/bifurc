//! TLS interception of a CONNECT: answer it, present a certificate for the requested host signed by the
//! engine's CA, and read the decrypted request.

use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio_rustls::{TlsAcceptor, server::TlsStream};

use super::wire::{Prefixed, RequestHead, read_body, read_head};
use crate::certificates::host_server_config;

/// Terminate TLS on `client` (a connection that just sent `CONNECT hostname:port`) and read the one request
/// that follows. `None` when no certificate could be made (the client gets a 502), the handshake failed, or
/// the client sent no request.
pub async fn intercept_tls<S: AsyncRead + AsyncWrite + Unpin>(
    mut client: S,
    hostname: &str,
) -> Option<(Tunnelled<S>, RequestHead, Vec<u8>)> {
    match host_server_config(hostname) {
        Ok(config) => intercept_tls_with(client, config).await,
        Err(_) => {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n").await;
            let _ = client.shutdown().await;
            None
        }
    }
}

/// [`intercept_tls`] with the certificate configuration already chosen. A client that speaks plain HTTP in the
/// tunnel (a browser sends `ws://` sockets and `http://` pages through CONNECT too) is read as it is.
pub async fn intercept_tls_with<S: AsyncRead + AsyncWrite + Unpin>(
    mut client: S,
    config: Arc<ServerConfig>,
) -> Option<(Tunnelled<S>, RequestHead, Vec<u8>)> {
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.ok()?;
    client.flush().await.ok()?;
    let mut first = [0u8; 1];
    client.read_exact(&mut first).await.ok()?;
    let client = Prefixed::new(first.to_vec(), client);
    let mut tunnelled = if first[0] == TLS_HANDSHAKE {
        Tunnelled::Tls(Box::new(TlsAcceptor::from(config).accept(client).await.ok()?))
    } else {
        Tunnelled::Plain(client)
    };
    let (head, leftover) = read_head(&mut tunnelled).await?;
    let body = read_body(&mut tunnelled, leftover, &head.header_lines).await.ok()?;
    Some((tunnelled, head, body))
}

/// The first byte of a TLS handshake record.
const TLS_HANDSHAKE: u8 = 0x16;

/// What a client speaks inside an answered CONNECT: TLS, decrypted here, or plain HTTP.
pub enum Tunnelled<S> {
    Tls(Box<TlsStream<Prefixed<S>>>),
    Plain(Prefixed<S>),
}

impl<S> Tunnelled<S> {
    pub fn is_tls(&self) -> bool {
        matches!(self, Tunnelled::Tls(_))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for Tunnelled<S> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Tunnelled::Tls(tls) => Pin::new(tls.as_mut()).poll_read(cx, buf),
            Tunnelled::Plain(plain) => Pin::new(plain).poll_read(cx, buf),
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for Tunnelled<S> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Tunnelled::Tls(tls) => Pin::new(tls.as_mut()).poll_write(cx, buf),
            Tunnelled::Plain(plain) => Pin::new(plain).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Tunnelled::Tls(tls) => Pin::new(tls.as_mut()).poll_flush(cx),
            Tunnelled::Plain(plain) => Pin::new(plain).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Tunnelled::Tls(tls) => Pin::new(tls.as_mut()).poll_shutdown(cx),
            Tunnelled::Plain(plain) => Pin::new(plain).poll_shutdown(cx),
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
    use tokio::io::AsyncReadExt;
    use tokio_rustls::TlsConnector;

    use super::*;

    pub(in crate::proxy) const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection Established\r\n\r\n";

    fn provider() -> Arc<rustls::crypto::CryptoProvider> {
        Arc::new(rustls::crypto::ring::default_provider())
    }

    /// A server configuration for `localhost` and a client configuration that trusts it.
    pub(in crate::proxy) fn self_signed_pair() -> (Arc<ServerConfig>, Arc<ClientConfig>) {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert = certified.cert.der().clone();
        let key = rustls::pki_types::PrivateKeyDer::try_from(certified.signing_key.serialize_der()).unwrap();
        let server = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.clone()], key)
            .unwrap();
        let mut roots = RootCertStore::empty();
        roots.add(cert).unwrap();
        let client = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        (Arc::new(server), Arc::new(client))
    }

    #[tokio::test]
    async fn answers_the_connect_then_reads_the_decrypted_request() {
        let (server_config, client_config) = self_signed_pair();
        let (mut client_end, server_end) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(async move {
            let (mut tls, head, body) = intercept_tls_with(server_end, server_config).await.unwrap();
            tls.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok").await.unwrap();
            tls.shutdown().await.unwrap();
            (head, body)
        });

        let mut established = vec![0u8; ESTABLISHED.len()];
        client_end.read_exact(&mut established).await.unwrap();
        assert_eq!(established, ESTABLISHED);
        let connector = TlsConnector::from(client_config);
        let mut tls = connector.connect(ServerName::try_from("localhost").unwrap(), client_end).await.unwrap();
        tls.write_all(b"POST /api HTTP/1.1\r\nHost: localhost\r\nContent-Length: 3\r\n\r\nabc").await.unwrap();
        let mut answer = String::new();
        let _ = tls.read_to_string(&mut answer).await;
        assert!(answer.ends_with("\r\n\r\nok"), "{answer}");

        let (head, body) = server.await.unwrap();
        assert_eq!((head.method.as_str(), head.target.as_str()), ("POST", "/api"));
        assert_eq!(body, b"abc");
    }

    #[tokio::test]
    async fn without_a_ca_the_client_gets_a_502() {
        let (mut client_end, server_end) = tokio::io::duplex(1024);
        assert!(intercept_tls(server_end, "example.test").await.is_none());
        let mut answer = String::new();
        client_end.read_to_string(&mut answer).await.unwrap();
        assert_eq!(answer, "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
    }
}
