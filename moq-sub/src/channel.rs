use anyhow::Context;
use moq_transport::cache::{fragment, segment, track};
use tokio::sync::mpsc;

pub struct ChannelSubscriber {
    track: track::Subscriber,
    sender_tx: mpsc::Sender<Vec<u8>>,
}
impl ChannelSubscriber {
    pub fn new(track: track::Subscriber, sender_tx: mpsc::Sender<Vec<u8>>) -> Self {
        Self { track, sender_tx: sender_tx }
    }


    pub async fn run(mut self) -> anyhow::Result<()> {
        log::info!("ChannelSubscriber::run");
        while let Some(segment) = self.track.segment().await.context("failed to get segment")? {
            log::debug!("got segment: {:?}", segment);
            let catalog_tx = self.sender_tx.clone();
            tokio::spawn(async move {
                if let Err(err) = Self::recv_segment(segment, catalog_tx).await {
                    log::warn!("failed to receive segment: {:?}", err);
                }
            });
        }

        Ok(())
    }

    async fn recv_segment(
        mut segment: segment::Subscriber,
        catalog_tx: mpsc::Sender<Vec<u8>>
    ) -> anyhow::Result<()> {
        let base = Vec::new();
        while let Some(fragment) = segment.fragment().await? {
            log::debug!("next fragment: {:?}", fragment);
            let value = Self::recv_fragment(fragment, base.clone()).await?;

            log::info!("Value: {:?}", String::from_utf8(value.clone()));
            catalog_tx.send(value).await.expect("failed to send catalog");
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
