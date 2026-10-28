use arena_server::config::ServerConfig;

fn usage() -> ! {
    eprintln!("usage: sharingan-arena [--port N] [--bind HOST:PORT] [--db PATH]   (env: see arena/server/README.md)");
    std::process::exit(2)
}

#[tokio::main]
async fn main() {
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
    tokio::select! {
        _ = srv.task => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
