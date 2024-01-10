use std::{fs, io, sync::Arc, time};
use std::ops::Deref;
use std::process::Stdio;
use anyhow::{Context, Error};
use clap::Parser;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

mod cli;
mod dump;
mod catalog;
mod init;

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
			let mut ffmpeg = spawn_ffmpeg(track.deref()).expect("Failed to spawn ffmpeg");

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
					let dumper = dump::Subscriber::new(format!("{}/{}", stream_name,track_data_track), track_subscriber, init_track, ffmpeg_stdin);
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

fn spawn_ffmpeg(track: &dyn Track) -> Result<Child, Error> {
	let args = ffmpeg_args(track);


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

fn ffmpeg_args(track: &dyn Track) -> Vec<String> {

	let mut args: Vec<String> = Vec::new();

	if track.kind() == TrackKind::Audio {
		args.push("-map".to_string());
		args.push("0:a".to_string());
	} else {
		let width = 1920;
		let height = 1080;
		let fps = 30;
		args.push("-map".to_string());
		args.push("0:v".to_string());
		args.push("-c:v".to_string());
		args.push("libx264".to_string());
		args.push("-s:v".to_string());
		args.push(format!("{}x{}", width, height));
		args.push("-r".to_string());
		args.push(format!("{}", fps));
		let gop = match fps {
			30 => "96",
			50 => "160",
			_ => panic!("invalid fps")
		};
		args.push("-g".to_string());
		args.push(gop.to_string());
	};

	let preset = "ultrafast";
	let crf = "23";

	let mut args = [
		"-r", "30",
		"-analyzeduration", "1000",
		"-i", "pipe:0",
		"-preset", preset,
		"-crf", crf,
		"-sc_threshold", "0",
		"-b:v", "6.5M",
		"-maxrate", "6.5M",
		"-bufsize", "6.5M",
		"-profile:v", "main",
		"-level", "4.1",
		"-color_primaries", "1",
		"-color_trc", "1",
		"-colorspace", "1",
		"-muxdelay", "0",
		//"-var_stream_map", "v:0,name:v0",
		"-hls_segment_type", "mpegts",
		"-hls_time", "3.2",
		"-hls_flags", "delete_segments",
	].map(|s| s.to_string()).to_vec();

	args.push("-hls_segment_filename".to_string());
	args.push(format!("{}-%03d.ts", track.kind().as_str()));
	args.push(format!("{}.m3u8", track.kind().as_str()));
	args
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


#[cfg(test)]
mod tests {
	use TrackKind::Audio;
	use crate::*;
	use crate::catalog::{AudioTrack, VideoTrack};

	#[test]
	fn audio() {
		let audio = AudioTrack {
			kind: Audio,
			bit_rate: Some(128000),
			data_track: "audio.m4s".to_string(),
			init_track: "audio.mp4".to_string(),
			codec: "Opus".to_string(),
			container: "mp4".to_string(),

			sample_size: 16,
			channel_count: 2,
			sample_rate: 48000,
		};
		let command_str = ffmpeg_args(&audio).join(" ");

		println!("audio ffmpeg args\n: {command_str:?}");
	}
	#[test]
	fn video() {
		let video = VideoTrack {
			kind: TrackKind::Video,
			bit_rate: Some(128000),
			codec: "Opus".to_string(),
			container: "mp4".to_string(),
			data_track: "video.m4s".to_string(),
			init_track: "video.mp4".to_string(),

			height: 1080,
			width: 1920,
			frame_rate: 50,
		};
		let command_str = ffmpeg_args(&video).join(" ");

		println!("video ffmpeg args\n: {command_str:?}");
	}
}
