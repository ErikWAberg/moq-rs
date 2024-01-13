use anyhow::Context;
use log::error;
use moq_api::ApiError;

use moq_transport::{session::Request, setup::Role, MoqError};

use crate::Origin;

#[derive(Clone)]
pub struct Session {
	origin: Origin,
}

impl Session {
	pub fn new(origin: Origin) -> Self {
		Self { origin }
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

		tokio::select! {
			_ = session.run() => origin.close().await?,
			_ = origin.run() => (), // TODO send error to session
		};

		Ok(())
	}

	async fn serve_subscriber(&mut self, id: usize, request: Request, path: &str) -> anyhow::Result<()> {
		log::info!("serving subscriber: id={} path={}", id, path);

		let subscriber = self.origin.subscribe(path);

		let session = request.publisher(subscriber.broadcast.clone()).await?;

		// should we do vompc in separate thread maybe
		// should we straight up create an event recorder here?
		let fake_id = path.chars().take(10).collect::<String>();
		let mut vompc = self.origin.vompc();
		if let Some(vompc) = vompc.as_mut() {
			// todo error type
			let res = vompc.create("ny_dörr", fake_id.as_str(), 30).await;
			if let Err(err) = res {
				error!("failed to create episode: {}", err);
				return Ok(()) // not OK but idk how to return err
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
