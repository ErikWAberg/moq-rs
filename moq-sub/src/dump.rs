use std::sync::Arc;

use anyhow::Context;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::process::ChildStdin;
use tokio::sync::Mutex;

use moq_transport::cache::{segment, track};

pub struct Subscriber {
    track: track::Subscriber,
    format: String,
    name: String,
    init_track: Vec<u8>,
    ffmpeg_stdin: Arc<Mutex<ChildStdin>>,
}

impl Subscriber {
    pub fn new(format: String, name: String, track: track::Subscriber, init_track: Vec<u8>, ffmpeg_stdin: Arc<Mutex<ChildStdin>>) -> Self {
        Self { format, name, track, init_track, ffmpeg_stdin }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {

        self.ffmpeg_stdin.lock().await.write_all(&self.init_track).await.context("failed to write to ffmpeg stdin")?;

        let filename = format!("dump/{}-continuous.mp4", self.format);
        let mut continuous_file = File::create(filename).await.context("failed to create init file")?;
        continuous_file.write_all(&self.init_track).await.context("failed to write to file")?;

        File::create(format!("dump/{}-init.mp4", self.format)).await.context("failed to create init file")?
                .write_all(&self.init_track).await.context("failed to write to file")?;

        while let Some(segment) = self.track.segment().await.context("failed to get segment")? {
            log::debug!("got segment: {:?}", segment);

            let segment_sequence = segment.sequence;

            match Self::recv_segment(segment).await {
                Ok(data) => {
                    File::create(format!("dump/{}-{}.mp4", self.format, segment_sequence)).await?
                        .write_all(data.as_slice()).await?;

                    continuous_file.write_all(data.as_slice()).await?;

                    let mut ffmpeg_stdin = self.ffmpeg_stdin.lock().await;
                    if let Err(err) = ffmpeg_stdin.write_all(data.as_slice()).await {
                        log::error!("Failed to write to ffmpeg stdin: {:?}", err);
                        std::process::exit(1);
                    }
                }
                Err(err) => {log::warn!("failed to receive segment: {:?}", err);}
            }
        }

        Ok(())
    }

    async fn recv_segment(mut segment: segment::Subscriber) -> anyhow::Result<Vec<u8>> {
        let mut base = Vec::new();
        while let Some(mut fragment) = segment.fragment().await? {
            log::debug!("next fragment: {:?}", fragment);
            while let Some(data) = fragment.chunk().await? {
                base.extend_from_slice(&data);
            }
        }
        Ok(base)
    }
}
