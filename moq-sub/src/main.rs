use std::{fs, io, sync::Arc, time};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;

use anyhow::Context;
use chrono::{DateTime, Duration, Utc};
use clap::Parser;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use log::{error, info};
use notify::{Error, Event, EventKind, RecursiveMode, Watcher};
use tokio::fs as TokioFs;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use cli::*;
use moq_transport::cache::broadcast;
use moq_transport::cache::broadcast::Subscriber;

use crate::catalog::{Track, TrackKind};

mod cli;
mod dump;
mod catalog;
mod init;
mod ffmpeg;

async fn file_renamer() -> anyhow::Result<()> {

	let start = Utc::now();
	let start = start - Duration::milliseconds(start.timestamp_millis() % 3200);

	let (tx, rx) = channel::<Result<Event, Error>>();
	let mut watcher = notify::recommended_watcher(tx).unwrap();
	watcher.watch(Path::new("dump"), RecursiveMode::Recursive)?;

	loop {
		match rx.recv() {
			Ok(Ok(event)) => match event.kind {
				EventKind::Create(_) => {
					for path in event.paths {

						if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
							let parts: Vec<&str> = file_name.split('-').collect();
							if parts.len() == 2 && !parts[1].ends_with("continuous.mp4") {
								let segment = parts[0];
								let segment_no = segment.parse::<u32>().unwrap();
								let new_name = format!("out/{}-{}", segment_timestamp(start, segment_no), parts[1]); // ensure this function exists
								fs::rename(&path, new_name).expect("rename failed");
							}
						}
					}
				}
				_ => {}
			},
			Ok(Err(e)) => println!("watch error: {:?}", e),
			Err(e) => println!("receive error: {:?}", e),
		}
	}
}
fn segment_timestamp(start: DateTime<Utc>, segment_no: u32) -> String {
	let total_addition = Duration::milliseconds((segment_no as f64 * 3.2 * 1000.0) as i64);
	let now = start + total_addition;
	let seconds = now.timestamp();
	let milliseconds = now.timestamp_subsec_millis();
	format!("{}.{:03}", seconds, milliseconds)
}


async fn track_subscriber(track: Box<dyn Track>, subscriber: Subscriber) -> anyhow::Result<()> {
	let ffmpeg = ffmpeg::spawn(track.deref())?;
	let mut ffmpeg_stdin = ffmpeg.stdin.context("failed to get ffmpeg stdin").unwrap();

	let mut init_track_subscriber = subscriber
		.get_track(track.init_track().as_str())
		.context("failed to get init track")?;

	let init_track_data = init::get_segment(&mut init_track_subscriber).await?;

	let mut continuous_file = File::create(format!("dump/{}-continuous.mp4", track.kind().as_str())).await.context("failed to create init file")?;
	ffmpeg_stdin.write_all(&init_track_data).await.context("failed to write to ffmpeg stdin")?;
	continuous_file.write_all(&init_track_data).await.context("failed to write to file")?;

	let mut data_track_subscriber = subscriber
		.get_track(track.data_track().as_str())
		.context("failed to get data track")?;

	loop {
		let data_track_data = init::get_segment(&mut data_track_subscriber).await?;
		ffmpeg_stdin.write_all(&data_track_data).await.context("failed to write to ffmpeg stdin")?;
		continuous_file.write_all(&data_track_data).await.context("failed to write to file")?;
		// ffmpeg produce 3.2s segments
	}
}

async fn run_track_subscribers(subscriber: Subscriber) -> anyhow::Result<()> {
	let mut catalog_track_subscriber = subscriber
		.get_track(".catalog")
		.context("failed to get catalog track")?;

	let tracks = init::get_catalog(&mut catalog_track_subscriber).await.unwrap().tracks;
	let mut handles = FuturesUnordered::new();

	for track in tracks {
		let subscriber  = subscriber.clone();
		let handle = tokio::spawn(async move {
			track_subscriber(track, subscriber).await.unwrap();
		});
		handles.push(handle);
	}
	tokio::select! {
		_ = handles.next(), if ! handles.is_empty() => {}
	}
	Ok(())
}

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

	info!("connecting to relay: url={}", config.url);

	let session = webtransport_quinn::connect(&endpoint, &config.url)
		.await
		.context("failed to create WebTransport session")?;

	let session = moq_transport::session::Client::subscriber(session, publisher.clone())
		.await
		.context("failed to create MoQ Transport session")?;

	let stream_name = config.url.path_segments().and_then(|c| c.last()).unwrap_or("").to_string();

	let handle = tokio::spawn(async move {
		file_renamer().await.expect("file_renamer error");
	});


	tokio::select! {
		res = session.run() => res.context("session error")?,
		res = run_track_subscribers(subscriber) => res.context("application error")?,
		res = handle => res.context("renamer error")?,
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
