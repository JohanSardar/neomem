use std::fs;
use std::path::Path;

use neomem_core::{EntityId, EpochMs, FactId, IntakeId};
use neomem_proto::{Request, Response, SearchHit, SearchQuery};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;

const DAY: i64 = 86_400_000;

struct Aged {
    old: FactId,
    fresh: FactId,
    sister: FactId,
    ancient: FactId,
}

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = IsolatedNeomem::start(binaries)?;
    let aged = write_aged_facts(neomem.home().root())?;
    support::wait_for_index(&neomem, proof, 5, 1)?;
    let question = "where does the Edge analytics dashboard live";
    let first = search(&neomem, proof, question, false)?;
    let old_before = decay_of(&first, aged.old);
    proof.claim(
        "a fresh fact outranks an equally relevant 400-day-old one, and the old one is still shown",
        rank_of(&first, aged.fresh) < rank_of(&first, aged.old) && rank_of(&first, aged.old) < usize::MAX,
        format!("decay fresh {:.2}, old {:.2}", decay_of(&first, aged.fresh), old_before),
    );
    proof.claim(
        "decay stays inside [0.3, 1.5]",
        first.iter().all(|hit| (0.3..=1.5).contains(&hit.decay)),
        "",
    );
    for _ in 0..10 {
        search(&neomem, proof, question, false)?;
    }
    let after_reads = decay_of(&search(&neomem, proof, question, false)?, aged.old);
    proof.claim(
        "retrievals are not evidence: ten searches leave the old fact's decay unchanged",
        (after_reads - old_before).abs() < 0.01,
        format!("{old_before:.3} -> {after_reads:.3}"),
    );
    let mut marked = true;
    for _ in 0..4 {
        let output = neomem.neomem(&["useful", &aged.old.to_string()])?;
        marked &= output.status.success();
    }
    let after_useful = decay_of(&search(&neomem, proof, question, false)?, aged.old);
    proof.claim(
        "neomem useful is evidence: four marks raise the old fact's decay factor",
        marked && after_useful > old_before + 0.2,
        format!("{old_before:.3} -> {after_useful:.3}"),
    );
    let sister = search(&neomem, proof, "who is Maya", false)?;
    proof.claim(
        "a stable fact eight years old does not decay at all",
        (decay_of(&sister, aged.sister) - 1.0).abs() < 1e-6,
        format!("{:.3}", decay_of(&sister, aged.sister)),
    );
    let ancient = search(&neomem, proof, "The Edge prototype was called Sketchpad.", false)?;
    proof.claim(
        "a strong match is never buried: an eight-year-old exact match stays in the top 3, with decay at the 0.3 floor or above",
        rank_of(&ancient, aged.ancient) < 3 && decay_of(&ancient, aged.ancient) >= 0.3,
        format!("{:.3}", decay_of(&ancient, aged.ancient)),
    );
    let history_style = search(&neomem, proof, question, true)?;
    proof.claim(
        "history-style queries (--include-invalid) switch decay off",
        history_style.iter().all(|hit| (hit.decay - 1.0).abs() < 1e-6),
        "",
    );
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn search(
    neomem: &IsolatedNeomem,
    proof: &mut ProofRun,
    text: &str,
    include_invalid: bool,
) -> anyhow::Result<Vec<SearchHit>> {
    Ok(
        match neomem.call(
            proof,
            Request::Search(SearchQuery { text: text.to_owned(), include_invalid, ..SearchQuery::default() }),
        )? {
            Response::SearchResults(results) => results.hits,
            _ => Vec::new(),
        },
    )
}

fn rank_of(hits: &[SearchHit], id: FactId) -> usize {
    hits.iter().position(|hit| hit.fact.id() == id).unwrap_or(usize::MAX)
}

fn decay_of(hits: &[SearchHit], id: FactId) -> f32 {
    hits.iter().find(|hit| hit.fact.id() == id).map_or(-1.0, |hit| hit.decay)
}

fn now_ms() -> i64 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    i64::try_from(since.as_millis()).unwrap_or(0)
}

fn write_aged_facts(home: &Path) -> anyhow::Result<Aged> {
    let now = now_ms();
    let edge = EntityId::from_parts(EpochMs::from_millis(1_700_000_000_000), 3);
    fs::create_dir_all(home.join("entities/project"))?;
    fs::write(
        home.join("entities/project/edge.md"),
        format!(
            "---\nid: {edge}\nkind: entity\nentity_kind: project\nname: Edge\naliases: []\nrecorded_at: 1700000000000\nupdated_at: 1700000000000\nmerged_from: []\nmerged_into: null\nrelations: []\nlinks: []\n---\n"
        ),
    )?;
    let aged = Aged {
        old: FactId::from_parts(EpochMs::from_millis(now - 400 * DAY), 61),
        fresh: FactId::from_parts(EpochMs::from_millis(now - DAY / 24), 62),
        sister: FactId::from_parts(EpochMs::from_millis(now - 3000 * DAY), 63),
        ancient: FactId::from_parts(EpochMs::from_millis(now - 3000 * DAY), 64),
    };
    let facts = [
        (aged.old, now - 400 * DAY, "The Edge analytics dashboard lives at metrics.edge.app.", "ordinary"),
        (aged.fresh, now - DAY / 24, "The Edge analytics dashboard lives in the Grafana board.", "ordinary"),
        (aged.sister, now - 3000 * DAY, "Maya is Johan's sister.", "stable"),
        (aged.ancient, now - 3000 * DAY, "The Edge prototype was called Sketchpad.", "ordinary"),
        (
            FactId::from_parts(EpochMs::from_millis(now - DAY), 65),
            now - DAY,
            "Edge has a dark mode toggle in settings.",
            "ordinary",
        ),
    ];
    for (id, recorded, content, durability) in facts {
        let directory = home.join("facts/2026/01");
        fs::create_dir_all(&directory)?;
        let source = IntakeId::from_parts(EpochMs::from_millis(recorded), 1);
        fs::write(
            directory.join(format!("{id}_aged.md")),
            format!(
                "---\nid: {id}\nkind: fact\ntags: [code]\nabout: [{edge}]\nrecorded_at: {recorded}\nupdated_at: {recorded}\nvalid_at: {recorded}\ninvalid_at: null\nimportance: 1\ndurability: {durability}\nsources: [{source}]\nlinks: []\nevidence: []\n---\n{content}\n"
            ),
        )?;
    }
    Ok(aged)
}
