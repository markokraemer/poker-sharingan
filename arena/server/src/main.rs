use arena_server::config::ServerConfig;

fn usage() -> ! {
    eprintln!("usage: sharingan-arena [--port N] [--bind HOST:PORT] [--db PATH] [--database-url postgres://...]   (env: see arena/server/README.md)\n       sharingan-arena run --bot \"<cmd>\" --track ladder-v1 ...   (headless match worker, see `run --help`)");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("run") => std::process::exit(arena_server::worker::cli_run(args[1..].to_vec())),
        Some("pipe-house") => std::process::exit(arena_server::pipe_bot::cli_pipe_house(args[1..].to_vec())),
        _ => {}
    }
    tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime").block_on(serve());
}

async fn serve() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let mut cfg = ServerConfig::from_env();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--port" => {
                let p: u16 = val().parse().unwrap_or_else(|_| usage());
                let host = cfg.bind.rsplit_once(':').map(|x| x.0.to_string()).unwrap_or("127.0.0.1".into());
                cfg.bind = format!("{host}:{p}");
            }
            "--bind" => cfg.bind = val(),
            "--db" => cfg.db_path = val(),
            "--database-url" => cfg.database_url = Some(val()),
            _ => usage(),
        }
    }
    let generated = cfg.admin_token.is_none();
    let srv = match arena_server::start(cfg).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!("sharingan-arena listening on http://{}", srv.addr);
    if generated {
        tracing::info!("admin token (generated, set ARENA_ADMIN_TOKEN to fix it): {}", srv.app.admin_token);
    }
    wait_for_signal().await;
    tracing::info!("shutdown requested: draining (a second signal exits immediately)");
    tokio::select! {
        _ = srv.shutdown() => {}
        _ = wait_for_signal() => { tracing::warn!("second signal: exiting now"); }
    }
}

async fn wait_for_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
