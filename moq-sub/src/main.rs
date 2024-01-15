use std::{fs, io, sync::Arc, time};
use std::io::ErrorKind;
use anyhow::Context;
use clap::Parser;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use log::info;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use std::process::Command;
use std::os::unix::io::FromRawFd;
use nix::unistd::pipe;
use std::os::fd::RawFd;
use anyhow::Result;
use std::io::{BufRead, BufReader};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::path::Path;



mod cli;
mod dump;
mod catalog;
mod init;
mod ffmpeg;

use cli::*;

use moq_transport::cache::broadcast;
use moq_transport::cache::broadcast::Subscriber;
use crate::catalog::{Track, TrackKind};


async fn track_subscriber(track: Box<dyn Track>, subscriber: Subscriber, fd: RawFd) -> anyhow::Result<()> {
    let mut init_track_subscriber = subscriber
        .get_track(track.init_track().as_str())
        .context("failed to get init track")?;


    let mut continuous_file = unsafe { File::from_raw_fd(fd) };

    let init_track_data = init::get_segment(&mut init_track_subscriber).await?;
    continuous_file.write_all(&init_track_data).await.context("failed to write to file")?;


    let mut data_track_subscriber = subscriber
        .get_track(track.data_track().as_str())
        .context("failed to get data track")?;
    loop {
        match init::get_segment(&mut data_track_subscriber).await {
            Ok(data_track_data) => {
                match continuous_file.write_all(&data_track_data).await {
                    Ok(_) => {}
                    Err(e) => {
                        if e.kind() != ErrorKind::BrokenPipe {
                            return Err(anyhow::anyhow!(e).context("failed to write to file"));
                        }
                        break;
                    }
                }
            }
            Err(e) => {
                return Err(e.context("failed to get segment"));
            }
        }
    }

    Ok(())
}

fn watch_file(file_path: String, file_type: &str, start_time: u64) -> io::Result<()> {
    let mut last_contents = Vec::new();


    loop {
        let mut current_contents = Vec::new();
        let suffix = match file_type {
            "audio" => "a0",
            "video" => "v0",
            _ => "",
        };


        // Read the current contents of the file
        match fs::File::open(&file_path) {
            Ok(file) => {
                let reader = BufReader::new(file);
                for line in reader.lines() {
                    let line = line?;
                    current_contents.push(line);
                }
            },
            Err(_) => {
                println!("Waiting for file to be created: {}", &file_path);
                // File does not exist or cannot be opened, continue to next iteration
                thread::sleep(Duration::from_millis(100));
                continue;
            }
        }

        // Compare the current contents with the last contents
        if current_contents != last_contents {
            for line in &current_contents {
                if !last_contents.contains(line) {
                    println!("New line in {}: {}", &file_path, line);

                    if let Some(segment_str) = line.split('_').nth(1) {
                        if let Ok(segment_number) = segment_str[..3].parse::<usize>() {
                            let timestamp = segment_number as f64 * 3.2;

                            // Split timestamp into seconds and milliseconds
                            let seconds = start_time + timestamp as u64;
                            let milliseconds = (timestamp.fract() * 1000.0) as u32;

                            // Format with left-padding
                            let new_file_name = format!("{:10}.{:03}_{}.mp4", seconds, milliseconds, suffix);

                            let original_file_path = Path::new("dump/").join(line);
                            let new_file_path = Path::new("dump/").join(new_file_name);

                            if let Err(e) = fs::rename(&original_file_path, &new_file_path) {
                                eprintln!("Error renaming file {:?} to {:?}: {}", original_file_path, new_file_path, e);
                            } else {
                                println!("Renamed {:?} to {:?}", original_file_path, new_file_path);
                            }
                        }
                    }
                }
            }
            last_contents = current_contents;
        }

        thread::sleep(Duration::from_millis(100));
    }
}


async fn run_track_subscribers(subscriber: Subscriber) -> anyhow::Result<()> {
    let mut catalog_track_subscriber = subscriber
        .get_track(".catalog")
        .context("failed to get catalog track")?;

    let tracks = init::get_catalog(&mut catalog_track_subscriber).await.unwrap().tracks;
    let mut handles = FuturesUnordered::new();


    // create as many pipes as there are tracks
    let mut pipes = Vec::new();
    for _ in 0..tracks.len() {
        let (reader, writer) = pipe().context("failed to create pipe")?;
        pipes.push((reader, writer));
    }

    // create as many args as there are pipes

    let mut args = Vec::new();

    args.push("-y".to_string());
    args.push("-loglevel".to_string());
    args.push("error".to_string());
    args.push("-hide_banner".to_string());
    for (reader, _) in &pipes {
        args.push("-i".to_string());
        args.push(format!("pipe:{}", reader));
    }
    args.push("-movflags".to_string());
    args.push("faststart".to_string());

    // Video segmenting
    args.push("-map".to_string());
    args.push("1:v".to_string());
    args.push("-s".to_string());
    args.push("1920x1080".to_string());
    args.push("-c:v".to_string());
    args.push("libx264".to_string());
    args.push("-force_key_frames".to_string());
    args.push("expr:gte(t,n_forced*1.92)".to_string());
    args.push("-f".to_string());
    args.push("segment".to_string());
    args.push("-segment_time".to_string());
    args.push("3.2".to_string());
    args.push("-reset_timestamps".to_string());
    args.push("1".to_string());
    args.push("-segment_list".to_string());
    args.push("video_segments.txt".to_string());
    args.push("video_%03d.mp4".to_string());

    // Audio segmenting
    args.push("-map".to_string());
    args.push("0:a".to_string());
    args.push("-c:a".to_string());
    args.push("aac".to_string());
    args.push("-f".to_string());
    args.push("segment".to_string());
    args.push("-segment_time".to_string());
    args.push("3.2".to_string());
    args.push("-reset_timestamps".to_string());
    args.push("1".to_string());
    args.push("-segment_list".to_string());
    args.push("audio_segments.txt".to_string());
    args.push("audio_%03d.mp4".to_string());

    info!("ffmpeg args: {:?}", args);

    let _ = Command::new("ffmpeg")
        .current_dir("dump")
        .args(&args)
        .spawn()
        .context("failed to spawn FFmpeg process")?;

    let start_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let audio_thread = thread::spawn(move || {
        watch_file("dump/audio_segments.txt".to_string(), "audio", start_time).unwrap();
    });

    let video_thread = thread::spawn(move || {
        watch_file("dump/video_segments.txt".to_string(), "video", start_time).unwrap();
    });


    let mut i = 0;
    for track in tracks {
        let subscriber = subscriber.clone();
        let pipes = pipes.clone();
        let handle = tokio::spawn(async move {
            let (_, writer) = pipes[i];
            track_subscriber(track, subscriber, writer).await.unwrap();
        });
        handles.push(handle);
        i = i + 1;
    }


    tokio::select! {
		_ = handles.next(), if ! handles.is_empty() => {},
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


    tokio::select! {
		res = session.run() => res.context("session error")?,
		res = run_track_subscribers(subscriber) => res.context("application error")?
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
        _scts: &mut dyn Iterator<Item=&[u8]>,
        _ocsp_response: &[u8],
        _now: time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}
