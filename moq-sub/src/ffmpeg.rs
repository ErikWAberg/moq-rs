use std::any::Any;
use std::ops::Deref;
use tokio::process::{Child, Command};
use anyhow::{Context, Error};
use std::process::Stdio;
use crate::catalog::{AudioTrack, Track, TrackKind, VideoTrack};

pub fn spawn(track: &dyn Track) -> Result<Child, Error> {
	let args = args(track);


	let command_str = format!("ffmpeg {}", args.join(" "));
	log::info!("Executing: {}", command_str);

	let ffmpeg = Command::new("ffmpeg")
		.current_dir("dump")
		.args(&args)
		.stdin(Stdio::piped())
		.stdout(Stdio::inherit())
		.stderr(Stdio::inherit())
		.spawn()
		.context("failed to spawn ffmpeg process")?;

	Ok(ffmpeg)
}

fn args(track: &dyn Track) -> Vec<String> {

	let preset = "ultrafast";
	let crf = "23";
	let mut generic = [
		//"-analyzeduration", "1000",
		"-i", "pipe:0",
		"-preset", preset,
		"-crf", crf,
		"-sc_threshold", "0",
		"-maxrate", "6.5M",
		"-bufsize", "6.5M",
		"-level", "4.1",
		"-muxdelay", "0",

		"-hls_segment_type", "mpegts",
		"-hls_time", "3.2",
		"-hls_flags", "delete_segments",
	].map(|s| s.to_string()).to_vec();

	let mut args = track.ffmpeg_args();
	args.append(&mut generic);
	args.push("-hls_segment_filename".to_string());
	args.push(format!("{}-%03d.ts", track.kind().as_str()));
	args.push(format!("{}.m3u8", track.kind().as_str()));
	args
}


#[cfg(test)]
mod tests {
	use TrackKind::Audio;
	use TrackKind::Video;
	use crate::*;
	use crate::catalog::{AudioTrack, VideoTrack};
    use crate::ffmpeg::args;

    #[test]
	fn audio() {
		let audio = AudioTrack {
			kind: Audio,
			bit_rate: Some(128000),
			data_track: "audio.m4s".to_string(),
			init_track: "audio.mp4".to_string(),
			codec: "Opus".to_string(),
			container: "mp4".to_string(),

			sample_size: 16,
			sample_rate: 48000,
			channel_count: 2,
		};
		let command_str = args(&audio).join(" ");

		println!("audio ffmpeg args\n: {command_str:?}");
	}
	#[test]
	fn video() {
		let video = VideoTrack {
			kind: Video,
			bit_rate: Some(128000),
			codec: "Opus".to_string(),
			container: "mp4".to_string(),
			data_track: "video.m4s".to_string(),
			init_track: "video.mp4".to_string(),

			height: 1080,
			width: 1920,
			frame_rate: 50,
		};
		let command_str = args(&video).join(" ");

		println!("video ffmpeg args\n: {command_str:?}");
	}
}
