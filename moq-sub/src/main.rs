use std::{fs, io, sync::Arc, time};
use std::ops::Deref;
use anyhow::Context;
use clap::Parser;
use tokio::sync::Mutex;

mod cli;
mod dump;
mod catalog;
mod init;
mod ffmpeg;

use cli::*;

use moq_transport::cache::broadcast;
use catalog::Catalog;
use crate::catalog::{Track, TrackKind};

// TODO: clap complete

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	env_logger::init();

	// Disable tracing so we don't get a bunch of Quinn spam.
	let tracer = tracing_subscriber::FmtSubscriber::builder()
		.with_max_level(tracing::Level::WARN)
		.finish();
	tracing::subscriber::set_global_default(tracer).unwrap();

	let config = Config::parse();

	let (publisher, subscriber) = broadcast::new("");

	// Create a list of acceptable root certificates.
	let mut roots = rustls::RootCertStore::empty();

	if config.tls_root.is_empty() {
		// Add the platform's native root certificates.
		for cert in rustls_native_certs::load_native_certs().context("could not load platform certs")? {
			roots
				.add(&rustls::Certificate(cert.0))
				.context("failed to add root cert")?;
		}
	} else {
		// Add the specified root certificates.
		for root in &config.tls_root {
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
	if config.tls_disable_verify {
		let noop = NoCertificateVerification {};
		tls_config.dangerous().set_certificate_verifier(Arc::new(noop));
	}

	tls_config.alpn_protocols = vec![webtransport_quinn::ALPN.to_vec()]; // this one is important

	let arc_tls_config = std::sync::Arc::new(tls_config);
	let quinn_client_config = quinn::ClientConfig::new(arc_tls_config);

	let mut endpoint = quinn::Endpoint::client(config.bind)?;
	endpoint.set_default_client_config(quinn_client_config);

	log::info!("connecting to relay: url={}", config.url);

	let session = webtransport_quinn::connect(&endpoint, &config.url)
		.await
		.context("failed to create WebTransport session")?;

	let session = moq_transport::session::Client::subscriber(session, publisher.clone())
		.await
		.context("failed to create MoQ Transport session")?;

	let stream_name = config.url.path_segments().and_then(|c| c.last()).unwrap_or("").to_string();

	let catalog_track_subscriber = subscriber
		.get_track(".catalog")
		.context("failed to get catalog track")?;

	let mut catalog_subscriber = catalog::CatalogSubscriber::new(catalog_track_subscriber);

	catalog_subscriber.register_callback(Arc::new(move |catalog: Catalog| {
		log::info!("Parsed catalog: {:?}", catalog);

		for track in catalog.tracks {
			let mut ffmpeg = ffmpeg::spawn(track.deref()).expect("Failed to spawn ffmpeg");

			log::info!("running ffmpeg: {:?}", ffmpeg);

			let ffmpeg_stdin = ffmpeg.stdin.take().expect("Failed to open stdin");
			let ffmpeg_stdin = Arc::new(Mutex::new(ffmpeg_stdin));

			// Dump the init_track
			let init_track = track.init_track().clone();
			let init_track_subscriber = match subscriber.get_track(init_track.as_str()) {
				Ok(subscriber) => subscriber,
				Err(err) => {
					log::error!("Failed to get init track {}: {:?}", init_track, err);
					continue;
				}
			};
			let mut init_dumper = init::InitTrackSubscriber::new(init_track_subscriber);
			let subscriber = subscriber.clone();
			let stream_name = stream_name.clone();
			let ffmpeg_stdin = ffmpeg_stdin.clone();
			let data_track = track.data_track().clone();

			init_dumper.register_callback(Arc::new(move |init_track: Vec<u8>| {
				log::info!("Got init track");

				let track_subscriber = match subscriber.get_track(data_track.as_str()) {
					Ok(subscriber) => Some(subscriber),
					Err(err) => {
						log::error!("Failed to get track {}: {:?}", data_track, err);
						None
					}
				};
				if let Some(track_subscriber) = track_subscriber {
					let track_data_track = data_track.clone();
					let ffmpeg_stdin = ffmpeg_stdin.clone();
					let dumper = dump::Subscriber::new(track_data_track.to_string(), format!("{}/{}", stream_name, track_data_track), track_subscriber, init_track, ffmpeg_stdin);
					tokio::spawn(async move {
						if let Err(err) = dumper.run().await {
							log::warn!("Failed to run dumper for track {}: {:?}", track_data_track, err);
						}
					});
				}

			}));
			tokio::spawn(async move {
				if let Err(err) = init_dumper.run().await {
					log::warn!("Failed to run dumper for init track {}: {:?}", init_track, err);
				}
			});

		}
	}));

	tokio::select! {
		res = session.run() => res.context("session error")?,
		res = catalog_subscriber.run() => res.context("catalog dumper error")?,
	}
	tokio::select! {
		_ =  tokio::task::yield_now() => {},
	}

	Ok(())
}

pub struct NoCertificateVerification {}

impl rustls::client::ServerCertVerifier for NoCertificateVerification {
	fn verify_server_cert(
		&self,
		_end_entity: &rustls::Certificate,
		_intermediates: &[rustls::Certificate],
		_server_name: &rustls::ServerName,
		_scts: &mut dyn Iterator<Item = &[u8]>,
		_ocsp_response: &[u8],
		_now: time::SystemTime,
	) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
		Ok(rustls::client::ServerCertVerified::assertion())
	}
}
