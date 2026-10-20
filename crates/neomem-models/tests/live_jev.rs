#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use neomem_models::{JevJudge, Secrets};
use neomem_testkit::judge_contract;

#[test]
#[ignore = "calls the live Jev API; run with JEV_API_KEY set"]
fn the_live_jev_api_passes_the_judge_contract_quickly() {
    let key =
        Secrets::load(std::path::Path::new("/nonexistent")).get("JEV_API_KEY").expect("JEV_API_KEY is set");
    let judge = JevJudge::new(None, key, "jev-1.13.0", Duration::from_secs(5));
    let started = Instant::now();
    judge_contract(&judge);
    assert!(started.elapsed() < Duration::from_secs(5));
}
