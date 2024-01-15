use std::path::PathBuf;
use std::process::Stdio;
use anyhow::Context;
use log::{error, info};
use tokio::process::Command;
use moq_api::ApiError;

use moq_transport::{session::Request, setup::Role, MoqError};

use crate::Origin;

#[derive(Clone)]
pub struct Session {
	origin: Origin,
	subscriber_output: Option<PathBuf>
}

impl Session {
	pub fn new(origin: Origin, subscriber_output: Option<PathBuf>) -> Self {
		Self { origin, subscriber_output}
	}

	pub async fn run(&mut self, conn: quinn::Connecting) -> anyhow::Result<()> {
		log::debug!("received QUIC handshake: ip={:?}", conn.remote_address());

		// Wait for the QUIC connection to be established.
		let conn = conn.await.context("failed to establish QUIC connection")?;

		log::debug!(
			"established QUIC connection: ip={:?} id={}",
			conn.remote_address(),
			conn.stable_id()
		);
		let id = conn.stable_id();

		// Wait for the CONNECT request.
		let request = webtransport_quinn::accept(conn)
			.await
			.context("failed to receive WebTransport request")?;

		// Strip any leading and trailing slashes to get the broadcast name.
		let path = request.url().path().trim_matches('/').to_string();

		log::debug!("received WebTransport CONNECT: id={} path={}", id, path);

		// Accept the CONNECT request.
		let session = request
			.ok()
			.await
			.context("failed to respond to WebTransport request")?;

		// Perform the MoQ handshake.
		let request = moq_transport::session::Server::accept(session)
			.await
			.context("failed to accept handshake")?;

		log::debug!("received MoQ SETUP: id={} role={:?}", id, request.role());

		let role = request.role();

		match role {
			Role::Publisher => {
				if let Err(err) = self.serve_publisher(id, request, &path).await {
					log::warn!("error serving publisher: id={} path={} err={:#?}", id, path, err);
				}
			}
			Role::Subscriber => {
				if let Err(err) = self.serve_subscriber(id, request, &path).await {
					log::warn!("error serving subscriber: id={} path={} err={:#?}", id, path, err);
				}
			}
			Role::Both => {
				log::warn!("role both not supported: id={}", id);
				request.reject(300);
			}
		};

		log::debug!("closing connection: id={}", id);

		Ok(())
	}

	async fn serve_publisher(&mut self, id: usize, request: Request, path: &str) -> anyhow::Result<()> {
		log::info!("serving publisher: id={}, path={}", id, path);

		let mut origin = match self.origin.publish(path).await {
			Ok(origin) => origin,
			Err(err) => {
				request.reject(err.code());
				return Err(err.into());
			}
		};

		let session = request.subscriber(origin.broadcast.clone()).await?;

		if let Some(output) = self.subscriber_output.clone() {
			let path = path.to_string();
			let handle = tokio::spawn(async move {
				let url = format!("https://localhost/{path}");
				let args = [
					"--output", output.to_str().unwrap(),
					"--tls-disable-verify",
					url.as_str(),
				].map(|s| s.to_string()).to_vec();
				info!("starting subscriber: {:?}", args.join(" "));

				let child = Command::new("moq-sub")
					.env("RUST_LOG", "INFO")
					.args(&args)
					.stdout(Stdio::inherit())
					.stderr(Stdio::inherit())
					//.stdout(Stdio::piped())
					//.stderr(Stdio::piped())
					.kill_on_drop(true)
					.spawn()
					.context("failed to spawn subscriber process").unwrap();
				info!("created subscriber");

				child.wait_with_output().await
			});

			tokio::select! {
				_ = session.run() => origin.close().await?,
				_ = origin.run() => (), // TODO send error to session
				output = handle => {
					let output = output.unwrap();
					if let Ok(output) = output {
						info!("subscriber exited with: {}", output.status);
						info!("subscriber stdout: {}", String::from_utf8_lossy(&output.stdout));
						info!("subscriber stderr: {}", String::from_utf8_lossy(&output.stderr));
					} else {
						error!("failed to wait for subscriber: {}", output.unwrap_err());
					}
				}
			}
			error!("exiting publisher loop");
		}

		Ok(())
	}

	async fn serve_subscriber(&mut self, id: usize, request: Request, path: &str) -> anyhow::Result<()> {
		log::info!("serving subscriber: id={} path={}", id, path);

		let subscriber = self.origin.subscribe(path);

		let session = request.publisher(subscriber.broadcast.clone()).await?;

		//TODO  should we do vompc in separate thread maybe

		let mut vompc = self.origin.vompc();
		if let Some(vompc) = vompc.as_mut() {
			let fake_id = path.chars().take(10).collect::<String>();
			info!("creating episode: {}", fake_id);

			//TODO duration!
			let res = vompc.create("GLAS_TILL_GLAS", fake_id.as_str(), 3600).await;
			match res {
				Ok(resource) => {info!("created resource: {resource}");}
				Err(error) => {
					error!("failed to create episode: {:?}", error);
					return Ok(()) // not OK but idk how to return err
				}
			}

			let res = vompc.start_auto().await;
			if let Err(err) = res {
				error!("failed to start episode: {}", err);
				return Ok(()) // not OK but idk how to return err
			}
		}


		session.run().await?;

		// Make sure this doesn't get dropped too early
		drop(subscriber);

		if let Some(vompc) = vompc.as_mut() {
			let res = vompc.stop_auto().await;
			if let Err(err) = res {
				error!("failed to stop episode: {}", err);
				return Ok(()) // not OK but idk how to return err
			}
			let res = vompc.delete_auto().await;
			if let Err(err) = res {
				error!("failed to delete episode: {}", err);
				return Ok(()) // not OK but idk how to return err
			}
		}


		Ok(())
	}
}
