use std::fmt;
use anyhow::Context;
use moq_transport::cache::{fragment, segment, track};
use std::sync::Arc;
use serde::{Deserialize, Deserializer};
use serde::de::{MapAccess, Visitor};
use log::info;
use crate::init;


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
#[derive(Deserialize, Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    Audio,
    Video,

}

impl TrackKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrackKind::Audio => "audio",
            TrackKind::Video => "video",
        }
    }
    pub fn as_short_str(&self) -> &'static str {
        match self {
            TrackKind::Audio => "a",
            TrackKind::Video => "v",
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct AudioTrack {
    pub kind: TrackKind,
    pub container: String,
    pub codec: String,
    pub channel_count: u32,
    pub sample_rate: u32,
    pub sample_size: u32,
    pub bit_rate: Option<u32>,
    pub init_track: String,
    pub data_track: String,
}

#[derive(Deserialize, Debug)]
pub struct VideoTrack {
    pub kind: TrackKind,
    pub container: String,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: u32,
    pub bit_rate: Option<u32>,
    pub init_track: String,
    pub data_track: String,
}

pub trait Track: std::fmt::Debug + Send {
    fn kind(&self) -> TrackKind;
    fn container(&self) -> String;
    fn codec(&self) -> String;
    fn bit_rate(&self) -> Option<u32>;
    fn init_track(&self) -> String;
    fn data_track(&self) -> String;

    fn ffmpeg_args(&self, src: &str) -> Vec<String>;
}

impl Track for AudioTrack {
    fn kind(&self) -> TrackKind {
        self.kind
    }
    fn container(&self) -> String {
        self.container.to_string()
    }
    fn codec(&self) -> String {
        self.codec.to_string()
    }
    fn bit_rate(&self) -> Option<u32> {
        self.bit_rate
    }
    fn init_track(&self) -> String {
        self.init_track.to_string()
    }
    fn data_track(&self) -> String {
        self.data_track.to_string()
    }
    fn ffmpeg_args(&self, src: &str) -> Vec<String> {
        let mut args = Vec::new();
        args.push("-i".to_string());
        args.push(src.to_string());

        args.push("-c:a".to_string());
        args.push("aac".to_string());

        args.push("-var_stream_map".to_string());
        args.push("a:0,name:a0".to_string());
        args.push("-b:a".to_string());
        args.push("192k".to_string());
        args
    }
}

impl Track for VideoTrack {
    fn kind(&self) -> TrackKind {
        self.kind
    }
    fn container(&self) -> String {
        self.container.to_string()
    }
    fn codec(&self) -> String {
        self.codec.to_string()
    }
    fn bit_rate(&self) -> Option<u32> {
        self.bit_rate
    }
    fn init_track(&self) -> String {
        self.init_track.to_string()
    }
    fn data_track(&self) -> String {
        self.data_track.to_string()
    }
    fn ffmpeg_args(&self, src: &str) -> Vec<String> {
        let mut args = Vec::new();

        args.push("-i".to_string());
        args.push(src.to_string());
        args.push("-video_track_timescale".to_string());
        args.push("90000".to_string());
        args.push("-s:v".to_string());
        args.push(format!("{}x{}", 1280, 720));
        args.push("-r".to_string());
        args.push("50".to_string());
        args.push("-c:v".to_string());
        args.push("libx264".to_string());
        args.push("-g".to_string());
        args.push("160".to_string());
        args.push("-var_stream_map".to_string());
        args.push("v:0,name:v0".to_string());
        args.push("-b:v".to_string());
        args.push("4.5M".to_string());
        args.push("-profile:v".to_string()); args.push("high".to_string());
        args.push("-color_primaries".to_string());
        args.push("1".to_string());
        args.push("-color_trc".to_string());
        args.push("1".to_string());
        args.push("-colorspace".to_string());
        args.push("1".to_string());
        args.push("-preset".to_string());
        args.push("ultrafast".to_string());
        args.push("-crf".to_string());
        args.push("23".to_string());
        args.push("-sc_threshold".to_string());
        args.push("0".to_string());
        args.push("-maxrate".to_string());
        args.push("4.5M".to_string());
        args.push("-bufsize".to_string());
        args.push("4.5M".to_string());
        args.push("-level".to_string());
        args.push("4.1".to_string());

        args
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

impl Catalog {
    pub(crate) fn from_slice(slice: &[u8]) -> Result<Catalog, serde_json::Error> {
        serde_json::from_slice(slice)
    }
    #[allow(dead_code)]
    fn from_str(slice: &str) -> Result<Catalog, serde_json::Error> {
        let root: Catalog = serde_json::from_str(slice)?;
        Ok(root)
    }
}


#[cfg(test)]
mod tests {
    use crate::catalog::*;

    #[test]
    fn it_works() {
        let catalog = Catalog::from_str(
            r#"
{
  "tracks": [
    {
      "container": "mp4",
      "kind": "audio",
      "init_track": "audio.mp4",
      "data_track": "audio.m4s",
      "codec": "Opus",
      "sample_rate": 48000,
      "sample_size": 16,
      "channel_count": 2,
      "bit_rate": 128000
    },
    {
      "container": "mp4",
      "kind": "video",
      "init_track": "video.mp4",
      "data_track": "video.m4s",
      "codec": "avc1.64001e",
      "width": 853,
      "height": 480,
      "frame_rate": 30,
      "bit_rate": 2000000
    }
  ]
}
            "#, ).unwrap();

        println!("catalog: {catalog:?}");

        assert_eq!(catalog.tracks.len(), 2);
    }
}
