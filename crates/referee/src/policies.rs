//! Policy specs for the CLI: `builtin:<name>` (random|station|nit|tag|lag|maniac) or
//! `toy:<iters>` (in-process toy blueprint) or `blueprint:<path>` (a `blueprint::io` checkpoint, with `--sizing` and `--buckets`).

use bench::bp::{bucketer_by_name, sizing_by_name, BlueprintPolicy};
use bench::builtin::builtin;
use bench::policy::PolicyFactory;
use std::path::Path;
use std::sync::Arc;

pub fn parse(
    spec: &str,
    sizing: &str,
    buckets: Option<&str>,
    capacity: usize,
) -> Result<Arc<dyn PolicyFactory>, String> {
    if let Some(name) = spec.strip_prefix("builtin:") {
        return builtin(name).ok_or(format!("unknown builtin {name}"));
    }
    // `toy:<iters>`: tiny 6-max blueprint trained in-process (coarse buckets, tiny sizing).
    if let Some(it) = spec.strip_prefix("toy:") {
        let iters: u64 = it.parse().map_err(|_| "toy:<iters>")?;
        return Ok(Arc::new(bench::bp::train_toy(6, iters, 1)));
    }
    if let Some(path) = spec.strip_prefix("blueprint:") {
        let bucketer = match buckets {
            Some(dir) => format!("tables:{dir}"),
            None => "coarse".to_string(),
        };
        let store = blueprint::io::load(Path::new(path), capacity, 0).map_err(|e| e.to_string())?;
        let b = bucketer_by_name(&bucketer).ok_or(format!("cannot load buckets {bucketer}"))?;
        let s = sizing_by_name(sizing).ok_or(format!("unknown sizing {sizing}"))?;
        return Ok(Arc::new(BlueprintPolicy::new(
            &format!("blueprint:{path}"),
            Arc::new(store),
            b,
            s,
        )));
    }
    Err(format!(
        "bad policy spec {spec} (builtin:<name> | blueprint:<path>)"
    ))
}
