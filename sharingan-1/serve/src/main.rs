//! `sharingan-serve`: the SHARINGAN-1 inference API. See sharingan-1/serve/README.md.

use serde_json::json;
use sharingan_serve::config::Config;
use sharingan_serve::decide::Core;
use sharingan_serve::log;
use sharingan_serve::server::{serve, AppState};
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match Config::parse(&|k| std::env::var(k).ok(), &args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    log::set_enabled(!matches!(std::env::var("SHARINGAN_LOG").as_deref(), Ok("off" | "0" | "false")));
    if !cfg!(panic = "unwind") {
        log::warn("startup_warning", json!({"msg": "built with panic=abort: a panic inside a search aborts the process instead of falling back to the blueprint; build with --config 'profile.release.panic=\"unwind\"'"}));
    }
    let t = Instant::now();
    let core = match Core::new(&cfg) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            log::error("startup_failed", json!({"error": e}));
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    log::info(
        "startup",
        json!({
            "bind": cfg.bind, "model_id": cfg.model_id, "blueprint": cfg.blueprint.display().to_string(), "load_ms": t.elapsed().as_millis() as u64,
            "sizing": cfg.sizing, "set": cfg.sets, "search": cfg.search, "budget_ms": cfg.budget_ms, "search_threads": cfg.search_threads,
            "search_concurrency": cfg.search_concurrency, "max_stack_bb": cfg.max_stack_bb, "auth": cfg.auth, "keys": cfg.api_keys.len(),
        }),
    );
    let state = AppState::new(core);
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("tokio runtime");
    let res = rt.block_on(async {
        let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
        log::info("listening", json!({"addr": listener.local_addr().map(|a| a.to_string()).unwrap_or_default()}));
        serve(state, listener, async {
            let _ = tokio::signal::ctrl_c().await;
            log::info("shutdown", json!({}));
        })
        .await
    });
    if let Err(e) = res {
        log::error("server_failed", json!({"error": e.to_string()}));
        std::process::exit(1);
    }
}
