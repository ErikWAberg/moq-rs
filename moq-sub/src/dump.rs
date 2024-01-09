use std::io::{stderr, stdout};
use std::process::Stdio;
use anyhow::Context;
use moq_transport::cache::{fragment, segment, track};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::process::{Command, ChildStdin};

pub struct Subscriber {
    track: track::Subscriber,
    name: String,
    init_track: Vec<u8>,
}

impl Subscriber {
    pub fn new(name: String, track: track::Subscriber, init_track: Vec<u8>) -> Self {
        Self { name, track, init_track }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let width = 1920;
        let height = 1080;
        let PRESET = "ultrafast";
        let CRF = "23";
        let GOP = "96";

        let mut ffmpeg = Command::new("ffmpeg")
            .current_dir("dump")
            .arg("-r")
            .arg("30")
            .arg("-analyzeduration")
            .arg("1000")
            .arg("-i")
            .arg("pipe:0")
            .arg("-map")
            .arg("v:0")
            .arg("-c:v")
            .arg("libx264")
            .arg("-s:v")
            .arg(format!("{}x{}", width, height))
            .arg("-preset")
            .arg(PRESET)
            .arg("-crf")
            .arg(CRF)
            .arg("-sc_threshold")
            .arg("0")
            .arg("-g")
            .arg(GOP)
            .arg("-b:v")
            .arg("6.5M")
            .arg("-maxrate")
            .arg("6.5M")
            .arg("-bufsize")
            .arg("6.5M")
            .arg("-profile:v")
            .arg("main")
            .arg("-level")
            .arg("4.1")
            .arg("-color_primaries")
            .arg("1")
            .arg("-color_trc")
            .arg("1")
            .arg("-colorspace")
            .arg("1")
            .arg("-muxdelay")
            .arg("0")
            .arg("-muxdelay")
            .arg("0")
            .arg("-var_stream_map")
            .arg("v:0,name:v0")
            .arg("-hls_segment_type")
            .arg("mpegts")
            .arg("-hls_time")
            .arg("3.2")
            .arg("-hls_flags")
            .arg("delete_segments")
            .arg("-hls_segment_filename")
            .arg("%v-%d.ts")
            .arg("-master_pl_name")
            .arg("master0.m3u8")
            .arg("variant-0-%v.m3u8")
            .stdin(Stdio::piped())
            .stdout(stdout())
            .stderr(stderr())
            .spawn()
            .context("failed to spawn ffmpeg process")?;

        log::info!("running ffmpeg: {:?}", ffmpeg);

        let name = self.name.clone();
        while let Some(segment) = self.track.segment().await.context("failed to get segment")? {
            log::debug!("got segment: {:?}", segment);
            let segment_name = name.clone();
            let ffmpeg_stdin = ffmpeg.stdin.take().context("failed to open ffmpeg stdin")?;

            tokio::spawn(async move {
                if let Err(err) = Self::recv_segment(ffmpeg_stdin, segment_name, segment).await {
                    log::warn!("failed to receive segment: {:?}", err);
                }
            });
        }

        Ok(())
    }

    async fn recv_segment(mut ffmpeg_stdin: ChildStdin, name: String, mut segment: segment::Subscriber) -> anyhow::Result<()> {
        let filename = format!("{}.{}", name, segment.sequence);

        let base = Vec::new();
        while let Some(fragment) = segment.fragment().await? {
            log::debug!("next fragment: {:?}", fragment);
            let value = Self::recv_fragment(fragment, base.clone()).await?;

            ffmpeg_stdin.write_all(&value).await.context("failed to write to ffmpeg stdin the second time")?;


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
