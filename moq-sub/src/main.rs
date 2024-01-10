use std::{fs, io, sync::Arc, time};
use std::process::Stdio;

use anyhow::{Context, Error};
use clap::Parser;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::sync::mpsc::{Sender, Receiver};

mod cli;
mod dump;
mod catalog;
mod init;
mod channel;

use cli::*;

use moq_transport::cache::broadcast;
use catalog::Catalog;

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

	let (tx, mut rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = tokio::sync::mpsc::channel(1000);

	let channel_subscriber = channel::ChannelSubscriber::new(catalog_track_subscriber, tx);

	tokio::spawn(async move {
		log::info!("Waiting for catalog");
		let message = rx.recv().await.unwrap();

		log::info!("Done waiting for catalog");
		let catalog = serde_json::from_slice::<Catalog>(&message).context("failed to parse JSON").unwrap();

		for (i, track) in catalog.tracks.iter().enumerate() {
			let (tx, mut rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = tokio::sync::mpsc::channel(1000);

			let track_subscriber = subscriber
				.get_track(track.init_track().as_str())
				.context("failed to get catalog track")
				.unwrap();
			let channel_subscriber = channel::ChannelSubscriber::new(track_subscriber, tx);

		}

	});

	tokio::select! {
		res = session.run() => res.context("session error")?,
		res = channel_subscriber.run() => res.context("catalog dumper error")?,
	}




	Ok(())
}

fn spawn_ffmpeg() -> Result<Child, Error> {
	let width = 1920;
	let height = 1080;
	let PRESET = "ultrafast";
	let CRF = "23";
	let GOP = "96";

	let args = [
		"-r", "30",
		"-analyzeduration", "1000",
		"-i", "pipe:0",
		"-map", "0:a",
		"-map", "1:v",
		"-c:v", "libx264",
		"-s:v", &format!("{}x{}", width, height),
		"-preset", PRESET,
		"-crf", CRF,
		"-sc_threshold", "0",
		"-g", GOP,
		"-b:v", "6.5M",
		"-maxrate", "6.5M",
		"-bufsize", "6.5M",
		"-profile:v", "main",
		"-level", "4.1",
		"-color_primaries", "1",
		"-color_trc", "1",
		"-colorspace", "1",
		"-muxdelay", "0",
		"-var_stream_map", "v:0,name:v0",
		"-hls_segment_type", "mpegts",
		"-hls_time", "3.2",
		"-hls_flags", "delete_segments",
		"-hls_segment_filename", "%v-%d.ts",
		"-master_pl_name", "master0.m3u8",
		"variant-0-%v.m3u8"
	];

	let command_str = format!("ffmpeg {}", args.join(" "));
	log::info!("Executing: {}", command_str);

	let ffmpeg = Command::new("ffmpeg")
		.current_dir("dump")
		.args(&args)
		.stdin(Stdio::piped())
		.stdout(Stdio::inherit())
		.stderr(Stdio::inherit())
		.spawn()
		.context("failed to spawn ffmpeg process")?;

	Ok(ffmpeg)
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
