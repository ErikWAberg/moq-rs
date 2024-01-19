use std::{fs, io, net, time};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use anyhow::Context;
use log::{error, info};
use tokio::process::Command;
use tokio::select;
use moq_transport::cache::broadcast;
use moq_transport::cache::broadcast::Subscriber;
use crate::config::Config;
use crate::subscriber;


pub struct LiveScheduler {
    config: Config,
}

impl LiveScheduler {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    async fn fetch_tracks(subscriber: Subscriber) -> anyhow::Result<String> {

        let mut catalog_track_subscriber = subscriber
            .get_track(".catalog")
            .context("failed to get catalog track")?;

        let tracks = subscriber::get_catalog(&mut catalog_track_subscriber).await.unwrap().tracks;

        info!("received tracks");
        let mut channel = "GLAS_TILL_GLAS";
        for track in &tracks {
            if track.channel_count() == 1 {
                channel = "GLAS_TILL_GLAS_TYST";
            }
        }
        Ok(channel.to_string())

    }

    pub async fn run(self) -> anyhow::Result<()> {
        let mut roots = rustls::RootCertStore::empty();

        if self.config.tls_root.is_empty() {
            // Add the platform's native root certificates.
            for cert in rustls_native_certs::load_native_certs().context("could not load platform certs")? {
                roots
                    .add(&rustls::Certificate(cert.0))
                    .context("failed to add root cert")?;
            }
        } else {
            // Add the specified root certificates.
            for root in &self.config.tls_root {
                let root = fs::File::open(root).context("failed to open root cert file")?;
                let mut root = io::BufReader::new(root);

                let root = rustls_pemfile::certs(&mut root).context("failed to read root cert")?;
                anyhow::ensure!(root.len() == 1, "expected a single root cert");
                let root = rustls::Certificate(root[0].to_owned());

                roots.add(&root).context("failed to add root cert")?;
            }
        }

        let mut tls_config = rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(roots)
            .with_no_client_auth();

        // Allow disabling TLS verification altogether.
        if self.config.tls_disable_verify {
            let noop = NoCertificateVerification {};
            tls_config.dangerous().set_certificate_verifier(Arc::new(noop));
        }

        tls_config.alpn_protocols = vec![webtransport_quinn::ALPN.to_vec()]; // this one is important

        let arc_tls_config = std::sync::Arc::new(tls_config);
        let quinn_client_config = quinn::ClientConfig::new(arc_tls_config);
        //todo
        let bind: net::SocketAddr = SocketAddr::from_str("[::]:0").expect("invalid bind addr");

        let mut endpoint = quinn::Endpoint::client(bind)?;
        endpoint.set_default_client_config(quinn_client_config);

        info!("connecting to redis: url={}", self.config.redis);

        // Create the redis client.
        let redis = redis::Client::open(self.config.redis)?;
        let mut con = redis.get_connection()?;
        let mut pubsub = con.as_pubsub();
        pubsub.subscribe("event-starts")?;

        //we dont know when events end


        loop {
            let msg = pubsub.get_message()?;
            let payload : String = msg.get_payload()?;
            info!("channel '{}': {}", msg.get_channel_name(), payload);
            // channel 'event-starts': {"url":"http://localhost:4443/e7a2ff34-13d4-4c3d-af7e-5a662a622b57"}
            let origin: moq_api::Origin = serde_json::from_str(&payload).unwrap();

            // TODO move call of vompc here
            // we could fetch the catalog & tracks here to figure out which channel we should use
            // we then vompc, keeping track of running moq-sub <-> to pevi mapping here

            let session = webtransport_quinn::connect(&endpoint, &origin.url)
                .await
                .context("failed to create WebTransport session")?;
            let (publisher, subscriber) = broadcast::new("");

            let session = moq_transport::session::Client::subscriber(session, publisher.clone())
                .await
                .context("failed to create MoQ Transport session")?;

            let channel = select! {
                _ = session.run() => {
                    log::error!("session closed");
                    None
                },
                res = Self::fetch_tracks(subscriber) => {
                    Some(res.unwrap())
                }
            };

            if let Some(channel) = channel {
                let (name, target_output) = if channel == "GLAS_TILL_GLAS" {
                    ("moq-sub-g2g", "/output/glas_till_glas/noencoder")
                } else {
                    ("moq-sub-g2g-tyst", "/output/glas_till_glas_tyst/noencoder")
                };

                let args = [
                    "run",
                    "--rm",
                    //"-v", "/var/run/docker.sock:/var/run/docker.sock",
                    "--name", name,
                    "--tmpfs", "/dump",
                    "--user", "10002:10002",
                    "moq-rs",
                    //cmd:
                    "moq-sub",
                    //args:
                    "--output", target_output,
                    "--tls-disable-verify",
                    origin.url.as_str()
                ].map(|s| s.to_string()).to_vec();

                let res = Command::new("docker")
                    .args(args)
                    .spawn()
                    .unwrap()
                    .wait_with_output().await;
                if let Ok(output) = res {
                    info!("stdout: {}", String::from_utf8(output.stdout).unwrap());
                    if !output.stderr.is_empty() {
                        error!("stderr: {}", String::from_utf8(output.stderr).unwrap());
                    }
                }
            }

        }
    }
}



pub struct NoCertificateVerification {}

impl rustls::client::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item=&[u8]>,
        _ocsp_response: &[u8],
        _now: time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}