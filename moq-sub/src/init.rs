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

    pub async fn run(mut self) -> anyhow::Result<Vec<u8>> {
        let mut base = Vec::new();

        while let Some(segment) = self.track.segment().await.context("failed to get segment")? {
            match Self::recv_segment(segment).await {
                Ok(mut data) => {base.append(data.as_mut());}
                Err(err) => {log::warn!("failed to receive segment: {:?}", err);}
            }
        }

        Ok(base)
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
