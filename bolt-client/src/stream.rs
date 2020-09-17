use std::{
    fmt::Debug,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use pin_project::pin_project;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpStream, ToSocketAddrs},
};
use tokio_rustls::{client::TlsStream, rustls::ClientConfig, webpki::DNSNameRef, TlsConnector};

use crate::error::*;

#[pin_project(project = StreamProj)]
#[derive(Debug)]
pub enum Stream {
    Tcp(#[pin] TcpStream),
    SecureTcp(#[pin] Box<TlsStream<TcpStream>>),
}

impl Stream {
    pub async fn connect(
        addr: impl ToSocketAddrs,
        domain: Option<impl AsRef<str>>,
    ) -> Result<Self> {
        match domain {
            Some(domain) => {
                let mut config = ClientConfig::new();
                config
                    .root_store
                    .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
                let dns_name_ref = DNSNameRef::try_from_ascii_str(domain.as_ref())
                    .map_err(|_| Error::InvalidDNSName(domain.as_ref().to_string()))?;
                let stream = TcpStream::connect(addr).await?;
                Ok(Stream::SecureTcp(Box::new(
                    TlsConnector::from(Arc::new(config))
                        .connect(dns_name_ref, stream)
                        .await?,
                )))
            }
            None => Ok(Stream::Tcp(TcpStream::connect(addr).await?)),
        }
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match self.project() {
            StreamProj::Tcp(tcp_stream) => AsyncRead::poll_read(tcp_stream, cx, buf),
            StreamProj::SecureTcp(tls_stream) => AsyncRead::poll_read(tls_stream, cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.project() {
            StreamProj::Tcp(tcp_stream) => AsyncWrite::poll_write(tcp_stream, cx, buf),
            StreamProj::SecureTcp(tls_stream) => AsyncWrite::poll_write(tls_stream, cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        match self.project() {
            StreamProj::Tcp(tcp_stream) => AsyncWrite::poll_flush(tcp_stream, cx),
            StreamProj::SecureTcp(tls_stream) => AsyncWrite::poll_flush(tls_stream, cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        match self.project() {
            StreamProj::Tcp(tcp_stream) => AsyncWrite::poll_shutdown(tcp_stream, cx),
            StreamProj::SecureTcp(tls_stream) => AsyncWrite::poll_shutdown(tls_stream, cx),
        }
    }
}