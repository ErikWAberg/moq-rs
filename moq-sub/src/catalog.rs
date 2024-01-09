use anyhow::Context;
use moq_transport::cache::{fragment, segment, track};
use serde_json::from_slice;
use serde::Deserialize;
use std::sync::Arc;

pub struct CatalogSubscriber {
    track: track::Subscriber,
    on_catalog: Option<Arc<dyn Fn(Catalog) + Send + Sync>>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct Track {
    pub codec: String,
    pub container: String,
    pub data_track: String,
    pub height: i64,
    pub init_track: String,
    pub kind: String,
    pub width: i64,
}

#[derive(Deserialize, Debug)]
pub(crate) struct Catalog {
    pub tracks: Vec<Track>,
}

impl CatalogSubscriber {
    pub fn new(track: track::Subscriber) -> Self {
        Self { track, on_catalog: None }
    }

    pub fn register_callback(&mut self, callback: Arc<dyn Fn(Catalog) + Send + Sync>) {
        self.on_catalog = Some(callback);
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let on_catalog = Arc::clone(self.on_catalog.as_ref().unwrap());
        while let Some(segment) = self.track.segment().await.context("failed to get segment")? {
            log::debug!("got segment: {:?}", segment);
            let on_catalog = Arc::clone(&on_catalog);
            tokio::spawn(async move {
                if let Err(err) = Self::recv_segment(segment, Some(on_catalog)).await {
                    log::warn!("failed to receive segment: {:?}", err);
                }
            });
        }

        Ok(())
    }

    async fn recv_segment(
        mut segment: segment::Subscriber,
        on_catalog: Option<Arc<dyn Fn(Catalog) + Send + Sync>>
    ) -> anyhow::Result<()> {
        let base = Vec::new();
        let mut first_catalog_parsed = false;
        while let Some(fragment) = segment.fragment().await? {
            log::debug!("next fragment: {:?}", fragment);
            let value = Self::recv_fragment(fragment, base.clone()).await?;

            let catalog = from_slice(&value).context("failed to parse JSON")?;

            first_catalog_parsed = true;
            if first_catalog_parsed {
                if let Some(callback) = on_catalog {
                    callback(catalog);
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
