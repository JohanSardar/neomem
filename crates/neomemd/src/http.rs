use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::thread;

use anyhow::Context;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use neomem_proto::{Home, IncomingEnvelope, Response, publish_file};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};

use crate::config::HttpConfig;
use crate::dispatch::{Dispatcher, finish_if_shutting_down};

pub fn start(dispatcher: Arc<Dispatcher>, config: &HttpConfig, home: &Home) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("neomemd-http")
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    let listener = if config.enabled { Some(runtime.block_on(bind(config.port))?) } else { None };
    if let Some(listener) = &listener {
        let address = listener.local_addr()?;
        publish_file(&home.http_address_path(), address.to_string()).context("recording the HTTP address")?;
        tracing::info!("HTTP listening on {address}");
    }
    thread::Builder::new().name("neomemd-async".to_owned()).spawn(move || {
        runtime.block_on(run(dispatcher, listener));
    })?;
    Ok(())
}

async fn bind(port: u16) -> anyhow::Result<TcpListener> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpListener::bind(address).await.with_context(|| format!("binding HTTP on {address}"))
}

async fn run(dispatcher: Arc<Dispatcher>, listener: Option<TcpListener>) {
    tokio::spawn(stop_on_terminate(Arc::clone(&dispatcher)));
    if let Some(listener) = listener {
        let routes = Router::new().route("/v1/request", post(handle_request)).with_state(dispatcher);
        if let Err(error) = axum::serve(listener, routes).await {
            tracing::error!("HTTP server stopped: {error}");
        }
    } else {
        std::future::pending::<()>().await;
    }
}

async fn handle_request(
    State(dispatcher): State<Arc<Dispatcher>>,
    Json(incoming): Json<IncomingEnvelope>,
) -> Json<Response> {
    let worker = Arc::clone(&dispatcher);
    let response = tokio::task::spawn_blocking(move || worker.dispatch(incoming)).await;
    let response = response.unwrap_or_else(|error| {
        Response::Failed(neomem_proto::Failure::new(neomem_proto::FailureKind::Internal, error.to_string()))
    });
    if matches!(response, Response::ShuttingDown) {
        tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            finish_if_shutting_down(&Response::ShuttingDown);
        });
    }
    Json(response)
}

async fn stop_on_terminate(dispatcher: Arc<Dispatcher>) {
    let Ok(mut terminate) = signal(SignalKind::terminate()) else {
        tracing::warn!("cannot listen for SIGTERM");
        return;
    };
    let Ok(mut interrupt) = signal(SignalKind::interrupt()) else {
        tracing::warn!("cannot listen for SIGINT");
        return;
    };
    tokio::select! {
        _ = terminate.recv() => {}
        _ = interrupt.recv() => {}
    }
    dispatcher.engine().flush();
    tracing::info!("neomemd stopped by signal");
    std::process::exit(0);
}
