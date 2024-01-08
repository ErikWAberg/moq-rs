use anyhow::Context;
use moq_transport::cache::{fragment, segment, track};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

pub struct Subscriber {
    track: track::Subscriber,
    name: String
}

impl Subscriber {
    pub fn new(name: String, track: track::Subscriber) -> Self {
        Self { name, track }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let name = self.name.clone();
        while let Some(segment) = self.track.segment().await.context("failed to get segment")? {
            log::debug!("got segment: {:?}", segment);
            let segment_name = name.clone();
            tokio::spawn(async move {
                if let Err(err) = Self::recv_segment(segment_name, segment).await {
                    log::warn!("failed to receive segment: {:?}", err);
                }
            });
        }

        Ok(())
    }

    async fn recv_segment(name: String, mut segment: segment::Subscriber) -> anyhow::Result<()> {

        let filename = format!("{}.{}", name, segment.sequence);

        let mut file = File::create(filename).await.context("failed to create file")?;

        let base = Vec::new();
        while let Some(fragment) = segment.fragment().await? {
            log::debug!("next fragment: {:?}", fragment);
            let value = Self::recv_fragment(fragment, base.clone()).await?;

            file.write_all(&value).await.context("failed to write to file")?;
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
