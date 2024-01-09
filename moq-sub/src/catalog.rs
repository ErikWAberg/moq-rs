use anyhow::Context;
use moq_transport::cache::{fragment, segment, track};
use serde_json::from_slice;
use serde::Deserialize;
use std::sync::Arc;

pub struct CatalogSubscriber {
    name: String,
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
    pub fn new(name: String, track: track::Subscriber) -> Self {
        Self { name, track, on_catalog: None }
    }

    pub fn register_callback(&mut self, callback: Arc<dyn Fn(Catalog) + Send + Sync>) {
        self.on_catalog = Some(callback);
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let name = self.name.clone();
        let on_catalog = Arc::clone(self.on_catalog.as_ref().unwrap());
        while let Some(segment) = self.track.segment().await.context("failed to get segment")? {
            log::debug!("got segment: {:?}", segment);
            let segment_name = name.clone();
            let on_catalog = Arc::clone(&on_catalog);
            tokio::spawn(async move {
                if let Err(err) = Self::recv_segment(segment_name, segment, Some(on_catalog)).await {
                    log::warn!("failed to receive segment: {:?}", err);
                }
            });
        }

        Ok(())
    }

    async fn recv_segment(
        name: String,
        mut segment: segment::Subscriber,
        on_catalog: Option<Arc<dyn Fn(Catalog) + Send + Sync>>
    ) -> anyhow::Result<()> {
        let base = Vec::new();
        let mut first_catalog_parsed = false;
        let mut catalog = Catalog { tracks: Vec::new() };
        while let Some(fragment) = segment.fragment().await? {
            log::debug!("next fragment: {:?}", fragment);
            let value = Self::recv_fragment(fragment, base.clone()).await?;

            catalog = from_slice(&value).context("failed to parse JSON")?;

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
