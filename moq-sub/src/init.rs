use anyhow::Context;
use moq_transport::cache::{fragment, segment, track};
use std::sync::Arc;

pub struct InitTrackSubscriber {
    track: track::Subscriber,
    on_init_track: Option<Arc<dyn Fn(Vec<u8>) + Send + Sync>>,
}


impl InitTrackSubscriber {
    pub fn new(track: track::Subscriber) -> Self {
        Self { track, on_init_track: None }
    }

    pub fn register_callback(&mut self, callback: Arc<dyn Fn(Vec<u8>) + Send + Sync>) {
        self.on_init_track = Some(callback);
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let on_init_track = Arc::clone(self.on_init_track.as_ref().unwrap());
        while let Some(segment) = self.track.segment().await.context("failed to get segment")? {
            log::debug!("got segment: {:?}", segment);
            let on_init_track = Arc::clone(&on_init_track);
            tokio::spawn(async move {
                if let Err(err) = Self::recv_segment(segment, Some(on_init_track)).await {
                    log::warn!("failed to receive segment: {:?}", err);
                }
            });
        }

        Ok(())
    }

    async fn recv_segment(
        mut segment: segment::Subscriber,
        on_init_track: Option<Arc<dyn Fn(Vec<u8>) + Send + Sync>>
    ) -> anyhow::Result<()> {
        let base = Vec::new();
        let mut first_init_fragment = false;
        while let Some(fragment) = segment.fragment().await? {
            log::debug!("next fragment: {:?}", fragment);
            let value = Self::recv_fragment(fragment, base.clone()).await?;

            first_init_fragment = true;
            if first_init_fragment {
                if let Some(callback) = on_init_track {
                    callback(value);
                }
                break;
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
