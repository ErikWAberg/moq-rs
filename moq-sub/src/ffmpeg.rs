use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{Context, Error};
use tokio::process::{Child, Command};
use tracing_subscriber::fmt::format;

use crate::catalog::Track;


pub fn change_timescale(src: &PathBuf, dst: &PathBuf) -> Result<Child, Error> {
	//MP4Box   -add "211-a0.mp4:timescale=90000"  ../211-a0-2.mp4

	let mut  args = [
		"-add", format!("{src:?}:timescale=90000", src=src).as_str(),
		dst.to_str().unwrap(),
	].map(|s| s.to_string()).to_vec();


	let mp4box = Command::new("MP4Box")
		.args(&args)
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()
		.context("failed to spawn ffmpeg process")?;

	Ok(mp4box)
}

pub fn fragment(src: &PathBuf, dst: &PathBuf, video: bool) -> Result<Child, Error> {
	let mut  args = [
		"-y", "-hide_banner",
		"-i", src.to_str().unwrap(),
		"-c", "copy",
		"-loglevel", "error",
		//"-frag_duration", "3200000",
		//"-movflags", "+dash,+faststart,+global_sidx",
		//"-movflags", "+faststart,+global_sidx",

	].map(|s| s.to_string()).to_vec();
	if video {
		args.push("-video_track_timescale".to_string());
	 	args.push("90000".to_string());
	}

	args.push(format!("{}", dst.to_str().unwrap()));

	let ffmpeg = Command::new("ffmpeg")
		.args(&args)
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()
		.context("failed to spawn ffmpeg process")?;

	Ok(ffmpeg)
}

pub fn spawn(args: Vec<String>) -> Result<Child, Error> {

	log::info!("Executing ffmpeg:\n\n{}\n", args.join(" "));

	let ffmpeg = Command::new("ffmpeg")
		.current_dir("dump")
		.args(&args)
		.stdin(Stdio::piped())
		.stdout(Stdio::inherit())
		.stderr(Stdio::inherit())
		.kill_on_drop(true)
		.spawn()
		.context("failed to spawn ffmpeg process")?;

	Ok(ffmpeg)
}

pub fn args(track: &dyn Track) -> Vec<String> {

	let mut args = [
		"-y", "-hide_banner",
		"-loglevel", "error",
	].map(|s| s.to_string()).to_vec();

	let mut post_args = [
		"-muxdelay", "0",
		"-f", "segment",
		"-segment_time", "3.2",
		"-break_non_keyframes", "1",
	].map(|s| s.to_string()).to_vec();


	args.append(&mut track.ffmpeg_args("pipe:0"));
	args.append(&mut post_args);
	//args.push("-hls_segment_filename".to_string());
	args.push(format!("%d-{}0.mp4", track.kind().as_short_str()));
	//args.push(format!("{}.m3u8", track.kind().as_str()));
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
