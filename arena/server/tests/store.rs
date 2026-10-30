//! `Store` contract tests, run against SQLite always and against Postgres when
//! `ARENA_TEST_DATABASE_URL=postgres://...` is set (each test uses its own fresh schema).

use arena_server::model::*;
use arena_server::pgstore::PgStore;
use arena_server::store::{SqliteStore, Store};

fn backends(tag: &str) -> Vec<(&'static str, Box<dyn Store>)> {
    let mut v: Vec<(&'static str, Box<dyn Store>)> = vec![("sqlite", Box::new(SqliteStore::open(":memory:").unwrap()))];
    if let Ok(url) = std::env::var("ARENA_TEST_DATABASE_URL") {
        if !url.is_empty() {
            v.push(("postgres", Box::new(PgStore::open(&url, Some(&schema(tag)), 2).unwrap())));
        }
    }
    v
}

fn schema(tag: &str) -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    format!("s_{tag}_{}_{}", std::process::id(), t % 1_000_000_007)
}

fn player(id: &str, house: bool) -> PlayerRecord {
    PlayerRecord {
        player_id: id.into(),
        name: format!("name-{id}"),
        kind: "bot".into(),
        owner: Some("o".into()),
        created_at: "2026-01-01T00:00:00Z".into(),
        house,
        token_hash: format!("hash-{id}"),
        webhook_url: Some("https://x/act".into()),
        webhook_secret: Some("s".into()),
    }
}

fn run(id: &str, track: &str, pid: &str) -> RunRecord {
    RunRecord {
        run_id: id.into(),
        track: track.into(),
        panel_version: "1".into(),
        panel: vec!["house:nit".into()],
        player_id: pid.into(),
        name: "n".into(),
        match_id: format!("m-{id}"),
        seed: "s".into(),
        deals: None,
        hands_planned: 600,
        hands_done: 0,
        status: "running".into(),
        verified: false,
        hosted: false,
        mbb_per_game: None,
        ci95_mbb: None,
        method: None,
        blueprint_fingerprint: None,
        created_at: "2026-01-01T00:00:00Z".into(),
        finished_at: None,
    }
}

#[test]
fn players_roundtrip_upsert_and_order() {
    for (name, s) in backends("players") {
        s.ping().unwrap();
        s.put_player(&player("p1", false)).unwrap();
        s.put_player(&player("p2", true)).unwrap();
        let mut p1 = player("p1", false);
        p1.name = "renamed".into();
        p1.webhook_url = None;
        s.put_player(&p1).unwrap();
        let g = s.get_player("p1").unwrap().unwrap();
        assert_eq!((g.name.as_str(), g.webhook_url.clone(), g.house), ("renamed", None, false), "{name}");
        assert!(s.get_player("p2").unwrap().unwrap().house, "{name}");
        assert!(s.get_player("nope").unwrap().is_none(), "{name}");
        assert_eq!(s.player_by_token_hash("hash-p2").unwrap().unwrap().player_id, "p2", "{name}");
        assert!(s.player_by_token_hash("").unwrap().is_none(), "{name}");
        let ids: Vec<String> = s.list_players(10).unwrap().into_iter().map(|p| p.player_id).collect();
        assert_eq!(ids, ["p1", "p2"], "{name}: insertion order survives an upsert");
        assert_eq!(s.list_players(1).unwrap().len(), 1, "{name}");
    }
}

#[test]
fn runs_filter_order_and_update_in_place() {
    for (name, s) in backends("runs") {
        s.put_run(&run("r1", "ladder-v1", "a")).unwrap();
        s.put_run(&run("r2", "ladder-v1", "b")).unwrap();
        s.put_run(&run("r3", "sharingan-1", "a")).unwrap();
        let mut r1 = run("r1", "ladder-v1", "a");
        r1.status = "finished".into();
        r1.verified = true;
        r1.hosted = true;
        r1.mbb_per_game = Some(12.5);
        s.put_run(&r1).unwrap();
        let ids = |t: Option<&str>, p: Option<&str>| -> Vec<String> { s.list_runs(t, p, 10).unwrap().into_iter().map(|r| r.run_id).collect() };
        assert_eq!(ids(None, None), ["r3", "r2", "r1"], "{name}: newest first, update keeps position");
        assert_eq!(ids(Some("ladder-v1"), None), ["r2", "r1"], "{name}");
        assert_eq!(ids(None, Some("a")), ["r3", "r1"], "{name}");
        assert_eq!(ids(Some("ladder-v1"), Some("a")), ["r1"], "{name}");
        assert_eq!(s.list_runs(None, None, 1).unwrap().len(), 1, "{name}");
        let g = s.get_run("r1").unwrap().unwrap();
        assert_eq!((g.status.as_str(), g.verified, g.hosted, g.mbb_per_game), ("finished", true, true, Some(12.5)), "{name}");
        assert!(s.get_run("zz").unwrap().is_none(), "{name}");
    }
}

#[test]
fn results_replace_per_match() {
    for (name, s) in backends("results") {
        let row = |m: &str, p: &str, net: i64| StoredResult {
            match_id: m.into(),
            player_id: p.into(),
            hands: 600,
            deals: 100,
            net_chips: net,
            score_mbb: 1.25,
            se_mbb: 0.5,
            method: "aivat".into(),
        };
        s.put_results("m1", &[row("m1", "a", 10), row("m1", "b", -10)]).unwrap();
        s.put_results("m2", &[row("m2", "a", 5)]).unwrap();
        s.put_results("m1", &[row("m1", "a", 99)]).unwrap(); // replaces m1 wholesale
        let all = s.all_results().unwrap();
        assert_eq!(all.len(), 2, "{name}");
        let a = s.results_of_player("a").unwrap();
        assert_eq!(a.len(), 2, "{name}");
        assert!(a.iter().any(|r| r.match_id == "m1" && r.net_chips == 99 && r.hands == 600 && r.score_mbb == 1.25), "{name}");
        assert!(s.results_of_player("b").unwrap().is_empty(), "{name}");
    }
}

#[test]
fn postgres_migrations_are_idempotent_and_schemas_are_isolated() {
    let Ok(url) = std::env::var("ARENA_TEST_DATABASE_URL") else { return };
    let sch = schema("mig");
    let a = PgStore::open(&url, Some(&sch), 2).unwrap();
    a.put_player(&player("p1", false)).unwrap();
    // a second process starting against the same schema re-runs migrations harmlessly and sees the data
    let b = PgStore::open(&url, Some(&sch), 2).unwrap();
    assert!(b.get_player("p1").unwrap().is_some());
    // another schema is empty
    let c = PgStore::open(&url, Some(&schema("mig2")), 1).unwrap();
    assert!(c.get_player("p1").unwrap().is_none());
    // a bad schema name is refused (it is interpolated into SQL)
    assert!(PgStore::open(&url, Some("x\"; drop table players; --"), 1).is_err());
    // an unreachable database is a clean error, not a hang or a panic
    assert!(PgStore::open("postgres://arena@127.0.0.1:1/none?connect_timeout=2", None, 1).is_err());
}
