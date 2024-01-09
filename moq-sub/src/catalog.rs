use anyhow::Context;
use moq_transport::cache::{fragment, segment, track};
use serde_json::from_slice;
use std::sync::Arc;
use serde::{Deserialize, Deserializer};
use serde::de::{MapAccess, Visitor};
use std::fmt;

pub struct CatalogSubscriber {
    track: track::Subscriber,
    on_catalog: Option<Arc<dyn Fn(Catalog) + Send + Sync>>,
}

/**
{
  tracks: [
    Track
    {
      codec: "Opus",
      container: "mp4",
      data_track: "audio.m4s",
      init_track: "audio.mp4",
      kind: "audio"
    },
    Track
    {
      codec: "avc1.64001e",
      container: "mp4",
      data_track: "video.m4s",
      init_track: "video.mp4",
      kind: "video"
    }
  ]
}
**/

pub trait Track: std::fmt::Debug {
    fn kind(&self) -> String;
    fn container(&self) -> String;
    fn codec(&self) -> String;
    fn init_track(&self) -> String;
    fn data_track(&self) -> String;
}

#[derive(Deserialize, Debug)]
pub struct AudioTrack {
    kind: String,
    container: String,
    codec: String,
    channel_count: u32,
    sample_rate: u32,
    sample_size: u32,
    bit_rate: Option<u32>,
    init_track: String,
    data_track: String,
}

impl Track for AudioTrack {

        fn kind(&self) -> String {
            self.kind.to_string()
        }

        fn container(&self) -> String {
            self.container.to_string()
        }

        fn codec(&self) -> String {
            self.codec.to_string()
        }

        fn init_track(&self) -> String {
            self.init_track.to_string()
        }

        fn data_track(&self) -> String {
            self.data_track.to_string()
        }



}

#[derive(Deserialize, Debug)]
pub struct VideoTrack {
    kind: String,
    container: String,
    codec: String,
    width: u32,
    height: u32,
    frame_rate: u32,
    bit_rate: Option<u32>,
    init_track: String,
    data_track: String,
}

impl Track for VideoTrack {

    fn kind(&self) -> String {
        self.kind.to_string()
    }

    fn container(&self) -> String {
        self.container.to_string()
    }

    fn codec(&self) -> String {
        self.codec.to_string()
    }

    fn init_track(&self) -> String {
        self.init_track.to_string()
    }

    fn data_track(&self) -> String {
        self.data_track.to_string()
    }



}


struct TrackVisitor;

impl<'de> Visitor<'de> for TrackVisitor {
    type Value = Box<dyn Track>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a Track object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut kind: Option<String> = None;
        let mut value_map = serde_json::Map::new();
        while let Some(key) = map.next_key()? {
            let val: serde_json::Value = map.next_value()?;
            if key == "kind" {
                if let Some(kind_val) = val.as_str() {
                    kind = Some(kind_val.to_owned());
                }
            }
            value_map.insert(key, val);
        }
        match kind {
            Some(kind) if kind == "audio" => {
                let track: AudioTrack = serde_json::from_value(serde_json::Value::Object(value_map)).unwrap();
                Ok(Box::new(track))
            }
            Some(kind) if kind == "video" => {
                let track: VideoTrack = serde_json::from_value(serde_json::Value::Object(value_map)).unwrap();
                Ok(Box::new(track))
            }
            _ => Err(serde::de::Error::custom("kind field missing or invalid")),
        }
    }
}

impl<'de> Deserialize<'de> for Box<dyn Track> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(TrackVisitor)
    }
}
#[derive(Deserialize, Debug)]
pub struct Catalog {
    pub(crate) tracks: Vec<Box<dyn Track>>,
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

            log::info!("Value: {:?}", String::from_utf8(value.clone()));

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
