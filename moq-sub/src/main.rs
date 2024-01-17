use std::{fs, io, sync::Arc, time};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::Context;
use chrono::{Duration, Utc};
use clap::Parser;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use inotify::{Inotify, WatchMask};
use log::{error, info};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::{join, select};
use tokio::fs as TokioFs;
use tokio::process::Command;
use tokio::sync::Mutex;

use cli::*;
use moq_transport::cache::broadcast;
use moq_transport::cache::broadcast::Subscriber;

use crate::catalog::{Track, TrackKind};

mod cli;
mod catalog;
mod subscriber;
mod ffmpeg;

async fn file_renamer(target: &PathBuf, filter_kind: &str) -> anyhow::Result<()> {
    let ntp_epoch_offset = Duration::milliseconds(2208988800000);

    let mut start_ms = 0;
    let mut start_sec = 0.0;
    let mut start = 0;
    start_ms = 0;
    let mut inotify = Inotify::init()
        .expect("Error while initializing inotify instance");
    let src_dir = Path::new("/dump");
    let local_target = Path::new("/dump/encoder");
    let mut skipped_audio_segments = 0;
    fs::create_dir_all(local_target)?;

    inotify.watches().add(src_dir, WatchMask::CLOSE_WRITE)
        .expect("Failed to add file watch");

    loop {
        let mut buffer = [0; 1024];
        let events = inotify.read_events_blocking(&mut buffer)
            .expect("Error while reading events");

        for event in events {
            if let Some(file_name) = event.name {
                info!("got inotify event for file_name: {:?}", file_name);
                let file_name = file_name.to_str().unwrap();


                let parts: Vec<&str> = file_name.split('-').collect();
                let file_suffix = parts[1];

                // 17-a0.mp4
                // 17-v0.mp4


                if parts.len() == 2 && !file_suffix.ends_with("continuous.mp4") {
                    let segment_no = parts[0].parse::<u32>().unwrap();


                    let src_segment = src_dir.join(file_name);

                    // file_suffix = a0.mp4 or v0.mp4

                    if file_suffix == "v0.mp4" {
                        if start_ms == 0 {
                            fs::create_dir_all(target)?;
                            start_ms = (Utc::now().timestamp_millis() + ntp_epoch_offset.num_milliseconds()) as u64;
                            start_sec = start_ms as f64 / 1000.0;
                            start = ((start_sec * 10.0).round() * 100.0) as u64;
                            info!("first seen segment: {}, start: {}", segment_no, start);
                        }
                        // we got notified that a video segment was written
                        info!("We got notified for a video segment: {}", segment_no);

                        let dst_video = target.join(Path::new(format!("{}-{}", segment_timestamp(start, segment_no), file_suffix).as_str()));

                        ffmpeg::change_timescale_ffmpeg(&src_segment, &dst_video).await?;
                        //fs::remove_file(&src_segment).expect("remove video failed");
                        info!("copied video: {dst_video:?}");
                    } else {
                        if start_ms == 0 {
                            skipped_audio_segments += 1;
                            info!("Skipping move of audio segment {} since first video segment has not yet been seen. skipped: {}", segment_no, skipped_audio_segments);
                            continue;
                        }
                        // we got notified that a audio segment was written
                        info!("We got notified for a audio segment: {}", segment_no);

                        let src_audio = src_dir.join(Path::new(format!("{}-{}", segment_no, "a0.mp4").as_str()));
                        let timestamp = segment_timestamp(start, segment_no - skipped_audio_segments);
                        info!("Creating a new name for audio segment: {:}", segment_no);
                        let dst_audio = target.join(Path::new(format!("{}-{}", timestamp, "a0.mp4").as_str()));
                        fs::copy(&src_audio, &dst_audio).expect("copy audio failed");
                        //fs::remove_file(&src_audio).expect("remove audio failed");
                        info!("copied audio: {dst_audio:?}");
                    }
                } else {
                    info!("ignoring event for file: {:?}", file_name);
                }
            }
        }
    }
}

fn segment_timestamp(start: u64, segment_no: u32) -> String {
    let timestamp = start + (segment_no as u64 * 3200);
    let timestamp = timestamp as f64 / 1000.0;
    format!("{:.3}", timestamp)
}

async fn track_subscriber_audio(track: Box<dyn Track>, subscriber: Subscriber) -> anyhow::Result<()> {

    // ffmpeg1: -f mp4 -i pipe:0 -f s16le -c:a pcm_s16le [-ac 2] -ar 48000 -
    // ffmpeg2:                  -f s16le -c:a pcm_s16le [-ac 2] -ar 48000 -i pipe:0 -ac 2 -ar 48000 -f segment ..

    let mut ffmpeg1_args = [
        "-y", "-hide_banner",
        "-i", "pipe:0",
        "-f", "s16le",
        "-c:a", "pcm_s16le",
        "-ac", "2",
        "-ar", "48000",
        "-"
        // "-loglevel", "error",
    ].map(|s| s.to_string()).to_vec();

    let mut ffmpeg1 = Command::new("ffmpeg")
        .args(&ffmpeg1_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn ffmpeg process 1")?;

    info!("ffmpeg1\n - args: {:?}\n", ffmpeg1_args.join(" "));
    let ffmpeg2_args= [
        "-y", "-hide_banner",
        "-f", "s16le",
        "-c:a", "pcm_s16le",
        "-ac", "2",
        "-ar", "48000",
        "-i", "pipe:0",
        "-f", "segment",
        "-c:a", "aac",
        "-ac", "2",
        "-ar", "48000",
        "-muxdelay", "0",
        "-b:a", "192k",
        //"-reset_timestamps", "1",
        "-segment_time", "3.2",
        //"-loglevel", "error",
        "dump/%d-a0.mp4"
    ].map(|s| s.to_string()).to_vec();

    let ffmpeg1_stdout: Stdio = ffmpeg1
        .stdout
        .take()
        .unwrap()
        .try_into()
        .expect("failed to convert to Stdio");

    let mut ffmpeg2 = Command::new("ffmpeg")
        .args(&ffmpeg2_args)
        .stdin(ffmpeg1_stdout)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn ffmpeg process 2")?;
    info!("ffmpeg2\n - args: {:?}\n", ffmpeg2_args.join(" "));
    let mut ffmpeg_stdin = ffmpeg1.stdin.take().context("failed to get ffmpeg1 stdin").unwrap();

    let handle = tokio::spawn(async move {
        let mut init_track_subscriber = subscriber
            .get_track(track.init_track().as_str())
            .context("failed to get init track").unwrap();

        let init_track_data = subscriber::get_segment(&mut init_track_subscriber).await.unwrap();

        let mut continuous_file = File::create(format!("/dump/{}-continuous.mp4", track.kind().as_str())).await.context("failed to create init file").unwrap();
        ffmpeg_stdin.write_all(&init_track_data).await.context("failed to write to ffmpeg stdin").unwrap();
        continuous_file.write_all(&init_track_data).await.context("failed to write to file").unwrap();

        // ffprobe_stdin.write_all(&init_track_data).await.context("failed to write to ffprobe_stdin").unwrap();

        let mut data_track_subscriber = subscriber
            .get_track(track.data_track().as_str())
            .context("failed to get data track").unwrap();

        loop {
            let data_track_data = subscriber::get_segment(&mut data_track_subscriber).await.unwrap();
            ffmpeg_stdin.write_all(&data_track_data).await.context("failed to write to ffmpeg stdin").unwrap();
            continuous_file.write_all(&data_track_data).await.context("failed to write to file").unwrap();
            // ffprobe_stdin.write_all(&data_track_data).await.context("failed to write to ffprobe_stdin").unwrap();
        }
    });

    select! {
        //_ = ffprobe.wait() => {},
        _ = ffmpeg1.wait() => {},
        _ = ffmpeg2.wait() => {},
        _ = handle => {
            error!("killing audio ffmpeg");
         //   ffprobe.kill().await?;
            ffmpeg1.kill().await?;
            ffmpeg2.kill().await?;
        },
    }
    info!("done with audio track");
    Ok(())
}


async fn video_track_subscriber(track: Box<dyn Track>, subscriber: Subscriber) -> anyhow::Result<()> {
    let ffmpeg_args = ffmpeg::args(track.deref());
    let mut ffmpeg = ffmpeg::spawn(ffmpeg_args).unwrap();
    let mut ffmpeg_stdin = ffmpeg.stdin.take().context("failed to get ffmpeg stdin").unwrap();
    //let local_raw = Path::new("/dump/raw");
    //fs::create_dir_all(local_raw)?;

    let handle = tokio::spawn(async move {
        let mut init_track_subscriber = subscriber
            .get_track(track.init_track().as_str())
            .context("failed to get init track").unwrap();

        let init_track_data = subscriber::get_segment(&mut init_track_subscriber).await.unwrap();

        let mut continuous_file = File::create(format!("/dump/{}-continuous.mp4", track.kind().as_str())).await.context("failed to create init file").unwrap();
        ffmpeg_stdin.write_all(&init_track_data).await.context("failed to write to ffmpeg stdin").unwrap();
        continuous_file.write_all(&init_track_data).await.context("failed to write to file").unwrap();

        let mut data_track_subscriber = subscriber
            .get_track(track.data_track().as_str())
            .context("failed to get data track").unwrap();

        loop {
            let data_track_data = subscriber::get_segment(&mut data_track_subscriber).await.unwrap();
            ffmpeg_stdin.write_all(&data_track_data).await.context("failed to write to ffmpeg stdin").unwrap();
            continuous_file.write_all(&data_track_data).await.context("failed to write to file").unwrap();
        }
    });

    //TODO how do we prevent ffmpeg from becoming a zombie when session is terminated
    //TODO why has ffmpeg transcode process become slower (transcode speed x)
    select! {
        _ = ffmpeg.wait() => {},
        _ = handle => {
            error!("killing video ffmpeg");
            ffmpeg.kill().await?;
        },
    }
    info!("done with video track");
    Ok(())
}

async fn run_track_subscribers(subscriber: Subscriber, target: &PathBuf) -> anyhow::Result<()> {
    let mut catalog_track_subscriber = subscriber
        .get_track(".catalog")
        .context("failed to get catalog track")?;

    let tracks = subscriber::get_catalog(&mut catalog_track_subscriber).await.unwrap().tracks;
    let mut handles = FuturesUnordered::new();

    for track in tracks {
        info!("received track: {track:?}");
        let subscriber = subscriber.clone();
        let handle = tokio::spawn(async move {
            if track.kind() == TrackKind::Audio {
                track_subscriber_audio(track, subscriber).await.unwrap()
            } else {
                video_track_subscriber(track, subscriber).await.unwrap()
            }
        });
        handles.push(handle);
    }
    tokio::select! {
		_ = handles.next(), if ! handles.is_empty() => {}
	}
    Ok(())
}

async fn remove_files(path: &str) -> anyhow::Result<()> {
    if !Path::new(path).exists() {
        println!("path does not exist {path}");
        return Ok(());
    }
    let mut count = 0;
    let mut dir = TokioFs::read_dir(path).await?;
    while let Some(entry) = dir.next_entry().await? {
        if entry.file_type().await?.is_file() {
            TokioFs::remove_file(entry.path()).await?;
            count += 1;
        }
    }
    println!(" DELETED {count} files from {path}");
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
    info!("stating subscriber for {stream_name}");

    println!("working dir: {:?}", std::env::current_dir().unwrap());
    remove_files("dump/encoder").await?;
    remove_files("dump").await?;
    let target_output = config.output.clone();

    let target_output = config.output.clone();
    let handle = tokio::spawn(async move {
        let res = file_renamer(&target_output, "v0.mp4").await;
        match res {
            Ok(_) => {}
            Err(e) => error!("file_renamer exited with error: {}", e),
        }
    });


    tokio::select! {
		res = session.run() => res.context("session error")?,
		res = run_track_subscribers(subscriber, &config.output) => res.context("application error")?,
		res = handle => res.context("renamer audio error")?,
		//res = handle2 => res.context("renamer video error")?,
	}
    error!("exiting");

    Ok(())
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
