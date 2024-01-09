use std::io::{stderr, stdout};
use std::process::Stdio;
use std::sync::{Arc};

use tokio::sync::{Mutex};
use anyhow::Context;
use moq_transport::cache::{fragment, segment, track};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::process::{Command, ChildStdin};

pub struct Subscriber {
    track: track::Subscriber,
    name: String,
    init_track: Vec<u8>,
    ffmpeg_stdin: Arc<Mutex<ChildStdin>>,
}

impl Subscriber {
    pub fn new(name: String, track: track::Subscriber, init_track: Vec<u8>, ffmpeg_stdin: Arc<Mutex<ChildStdin>>) -> Self {
        Self { name, track, init_track, ffmpeg_stdin }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {

        self.ffmpeg_stdin.lock().await.write_all(&self.init_track).await.context("failed to write to ffmpeg stdin")?;

        let name = self.name.clone();
        while let Some(segment) = self.track.segment().await.context("failed to get segment")? {
            log::debug!("got segment: {:?}", segment);
            let segment_name = name.clone();
            let ffmpeg_stdin = Arc::clone(&self.ffmpeg_stdin);
            tokio::spawn(async move {

                if let Err(err) = Self::recv_segment(ffmpeg_stdin, segment_name, segment).await {
                    log::warn!("failed to receive segment: {:?}", err);
                }
            });
        }

        Ok(())
    }

    async fn recv_segment(
        ffmpeg_stdin: Arc<Mutex<ChildStdin>>,
        name: String,
        mut segment: segment::Subscriber
    ) -> anyhow::Result<()> {
        let filename = format!("{}.{}", name, segment.sequence);

        let base = Vec::new();
        while let Some(fragment) = segment.fragment().await? {
            log::debug!("next fragment: {:?}", fragment);
            let value = Self::recv_fragment(fragment, base.clone()).await?;

            // Write the fragment data to the ffmpeg stdin
            let mut ffmpeg_stdin = ffmpeg_stdin.lock().await;
            if let Err(err) = ffmpeg_stdin.write_all(&value).await {
                log::error!("Failed to write to ffmpeg stdin: {:?}", err);
                std::process::exit(1);
            }
        }

        Ok(())
    }

    async fn recv_fragment(mut fragment: fragment::Subscriber, mut buf: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        while let Some(data) = fragment.chunk().await? {
            buf.extend_from_slice(&data);
        }

        Ok(buf)
    }
}
