use std::any::Any;
use std::ops::Deref;
use tokio::process::{Child, Command};
use anyhow::{Context, Error};
use std::process::Stdio;
use crate::catalog::{AudioTrack, Track, TrackKind, VideoTrack};

pub fn spawn(track: &dyn Track) -> Result<Child, Error> {
	let args = args(track);

	log::info!("Executing ffmpeg {}:\n\n{}\n", track.kind().as_str(), args.join(" "));

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
	let mut args = [
		"-y", "-hide_banner",
		"-i", "pipe:0",
		format!("{}-pipe.mp4", track.kind().as_str()).as_str()
	].map(|s| s.to_string()).to_vec();
/*
	let mut args = [
	//	"-i", "pipe:0",
		"-y", "-hide_banner",
		"-analyzeduration", "100000",
	].map(|s| s.to_string()).to_vec();

	let mut post_args = [
		"-muxdelay", "0",
		"-hls_segment_type", "mpegts",
		"-hls_time", "3.2",
		"-hls_flags", "delete_segments",
	].map(|s| s.to_string()).to_vec();


	args.append(&mut track.ffmpeg_args("pipe:0"));
	args.append(&mut post_args);
	args.push("-hls_segment_filename".to_string());
	args.push(format!("{}-%03d.ts", track.kind().as_str()));
	args.push(format!("{}.m3u8", track.kind().as_str()));*/
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