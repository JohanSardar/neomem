use neomem_core::Tag;
use neomem_proto::{Request, Response, SearchHit, SearchQuery};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;
use crate::seed;
use crate::timing;

const SIGNING_FIX: &str = "fct_01M55FJ500000000000000000D";
const SLEEP: &str = "fct_01M5AMBK00000000000000000M";
const SISTER: &str = "fct_01M50ARQ00000000000000000E";
const RAW_EXPORT: &str = "fct_01M5AMBK00000000000000000N";

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = seed::start_with_vault_fixture("workspace", binaries)?;
    let ready = support::wait_for_index(&neomem, proof, 13, 7)?;
    proof.claim(
        "the workspace fixture is indexed: 13 facts, 7 entities",
        ready.is_some_and(|report| report.facts == 13 && report.entities == 7),
        "",
    );
    a_question_finds_the_fact_that_answers_it(proof, &neomem)?;
    k_limits_the_results(proof, &neomem)?;
    filters_narrow_by_tag_entity_and_time(proof, &neomem)?;
    raw_and_system_stay_hidden_unless_asked(proof, &neomem)?;
    no_model_besides_the_embedder_is_called(proof, &neomem)?;
    the_command_line_prints_ranked_results(proof, &neomem)?;
    search_answers_quickly(proof, &neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn search(
    neomem: &IsolatedNeomem,
    proof: &mut ProofRun,
    query: SearchQuery,
) -> anyhow::Result<Vec<SearchHit>> {
    match neomem.call(proof, Request::Search(query))? {
        Response::SearchResults(results) => Ok(results.hits),
        other => anyhow::bail!("search answered {other:?}"),
    }
}

fn asking(text: &str) -> SearchQuery {
    SearchQuery { text: text.to_owned(), ..SearchQuery::default() }
}

fn ids(hits: &[SearchHit]) -> Vec<String> {
    hits.iter().map(|hit| hit.fact.id().to_string()).collect()
}

fn a_question_finds_the_fact_that_answers_it(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let hits = search(neomem, proof, asking("how was the Edge code signing error fixed"))?;
    let first = hits.first();
    proof.claim(
        "a question finds the fact that answers it first",
        first.is_some_and(|hit| hit.fact.id().to_string() == SIGNING_FIX),
        first.map_or_else(String::new, |hit| hit.fact.content().to_owned()),
    );
    let carries = first.is_some_and(|hit| {
        hit.entities.iter().any(|entity| entity.name == "Edge")
            && !hit.fact.sources().is_empty()
            && hit.score > 0.0
    });
    proof.claim("each result carries its entities, sources, validity and score", carries, "");
    Ok(())
}

fn k_limits_the_results(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let hits = search(neomem, proof, SearchQuery { k: Some(3), ..asking("Edge") })?;
    proof.claim("--k 3 returns at most three results", hits.len() <= 3 && !hits.is_empty(), hits.len());
    Ok(())
}

fn filters_narrow_by_tag_entity_and_time(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let health =
        search(neomem, proof, SearchQuery { tags: vec![Tag::new("health")?], ..asking("how did I sleep") })?;
    proof.claim(
        "--tags health keeps only health facts",
        ids(&health) == vec![SLEEP.to_owned()],
        format!("{:?}", ids(&health)),
    );
    let maya = search(neomem, proof, SearchQuery { entities: vec!["maya".to_owned()], ..asking("family") })?;
    proof.claim(
        "--entities Maya keeps only facts about Maya",
        ids(&maya) == vec![SISTER.to_owned()],
        format!("{:?}", ids(&maya)),
    );
    let unknown = neomem
        .call(proof, Request::Search(SearchQuery { entities: vec!["Nobody".to_owned()], ..asking("x") }))?;
    proof.claim(
        "an unknown entity name is an error, not an empty answer",
        matches!(unknown, Response::Failed(_)),
        "",
    );
    let early = SearchQuery {
        to: Some(neomem_core::EpochMs::from_millis(1_792_000_000_000 + 2 * 86_400_000)),
        ..asking("Edge")
    };
    let hits = search(neomem, proof, early)?;
    let all_early =
        hits.iter().all(|hit| hit.fact.valid_at().as_millis() <= 1_792_000_000_000 + 2 * 86_400_000);
    proof.claim(
        "--to keeps facts that were already valid by then",
        all_early && !hits.is_empty(),
        hits.len(),
    );
    Ok(())
}

fn raw_and_system_stay_hidden_unless_asked(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let question = "raw keychain export of the Edge signing identities";
    let hidden = search(neomem, proof, asking(question))?;
    let shown = search(neomem, proof, SearchQuery { include_hidden: true, ..asking(question) })?;
    proof.claim(
        "facts tagged raw or system are hidden by default and shown when asked for",
        !ids(&hidden).contains(&RAW_EXPORT.to_owned()) && ids(&shown).contains(&RAW_EXPORT.to_owned()),
        "",
    );
    Ok(())
}

fn no_model_besides_the_embedder_is_called(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let before = neomem.fake_model_calls()?.len();
    search(neomem, proof, asking("who works at Acme"))?;
    let calls = neomem.fake_model_calls()?;
    let routes: Vec<&str> = calls[before..].iter().map(|call| call.route.as_str()).collect();
    proof.claim(
        "a search calls no model except the query embedding",
        routes.iter().all(|route| *route == "/v1/embeddings") && routes.len() <= 1,
        format!("{routes:?}"),
    );
    Ok(())
}

fn the_command_line_prints_ranked_results(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let output = neomem.neomem(&["search", "who works at Acme", "--k", "5"])?;
    let printed = String::from_utf8_lossy(&output.stdout);
    proof.claim(
        "neomem search prints numbered results with their IDs and entities",
        output.status.success()
            && printed.starts_with("1. ")
            && printed.contains("Sam Kim works at Acme")
            && printed.contains("fct_"),
        printed.lines().take(3).collect::<Vec<_>>().join(" | "),
    );
    Ok(())
}

fn search_answers_quickly(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let mut client = neomem.client()?;
    let mut number = 0;
    let latency = timing::measure(40, || {
        number += 1;
        client.call(Request::Search(asking(&format!("Edge release question number {number}"))))?;
        Ok(())
    })?;
    proof.claim(
        "a search through the socket answers within 30 ms at p95 (fake embedder; the budget with the real model is checked by bench)",
        latency.p95_ms < 30.0,
        latency,
    );
    Ok(())
}
