use neomem_core::STARTING_TAGS;

use crate::binaries::Binaries;
use crate::proof::ProofRun;
use crate::seed;

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = seed::start_with_vault_fixture("edge-ship-date", binaries)?;
    neomem.add_fake_rules(&neomem_testkit::FakeRules {
        judge: vec![neomem_testkit::JudgeRule {
            question_contains: Some("same meaning".to_owned()),
            context_contains: Some("Candidate tag: healthcare".to_owned()),
            candidate_contains: None,
            answer: neomem_testkit::JudgeAnswer::Choice { label: "same_1".to_owned(), probability: 0.99 },
        }],
        ..neomem_testkit::FakeRules::default()
    })?;
    let tags = neomem.neomem(&["tags", "--json"])?;
    let text = String::from_utf8_lossy(&tags.stdout);
    proof.claim(
        "tags lists the starting registry",
        tags.status.success() && STARTING_TAGS.iter().all(|tag| text.contains(tag)),
        format!("{text} {}", String::from_utf8_lossy(&tags.stderr)),
    );
    let duplicate = neomem.neomem(&["tags", "--new", "healthcare", "--json"])?;
    let text = String::from_utf8_lossy(&duplicate.stdout);
    proof.claim(
        "healthcare resolves to health without creating a duplicate",
        duplicate.status.success() && text.contains("health") && !text.contains("healthcare"),
        text,
    );
    let created = neomem.neomem(&["tags", "--new", "astronomy", "--json"])?;
    let repeated = neomem.neomem(&["tags", "--new", "astronomy", "--json"])?;
    let listed = neomem.neomem(&["tags", "--json"])?;
    let registry = String::from_utf8_lossy(&listed.stdout);
    proof.claim(
        "a new tag is created once and listed",
        created.status.success() && repeated.status.success() && registry.matches("astronomy").count() == 1,
        &registry,
    );
    let listed: neomem_proto::Response = serde_json::from_slice(&listed.stdout)?;
    proof.claim(
        "the semantic duplicate creates no registry entry",
        matches!(listed, neomem_proto::Response::Tags(ref tags) if tags.len() == STARTING_TAGS.len() + 1 && !tags.iter().any(|tag| tag.as_str() == "healthcare")),
        registry,
    );
    let decisions: Vec<_> = neomem
        .fake_model_calls()?
        .into_iter()
        .filter(|call| {
            call.route == "/v1/systemone"
                && call.body["state"].as_str().is_some_and(|state| state.starts_with("Candidate tag:"))
        })
        .collect();
    proof.claim(
        "new meanings use one numbered judge choice and exact repeats skip it",
        decisions.len() == 2
            && decisions
                .iter()
                .all(|call| call.body["questions"].as_object().is_some_and(|questions| questions.len() == 1)),
        format!("{} tag decisions", decisions.len()),
    );
    external_references(proof, &neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn external_references(proof: &mut ProofRun, neomem: &crate::isolated::IsolatedNeomem) -> anyhow::Result<()> {
    let edge = "ent_01M56PGSJG83SYHRJXJG7DJ13N";
    let old = "ent_01M56PGSJG83SYHRJXJG7DJ14P";
    std::fs::write(
        neomem.home().root().join("entities/project/edge-ios.md"),
        format!(
            "---\nid: {old}\nkind: entity\nentity_kind: project\nname: Edge iOS\naliases: []\nrecorded_at: 1792300051000\nupdated_at: 1792300052000\nmerged_from: []\nmerged_into: {edge}\nrelations: []\nlinks: []\n---\n"
        ),
    )?;
    crate::scenarios::support::wait_for_index(neomem, proof, 2, 3)?;
    let linked =
        neomem.neomem(&["link", "ext:episodes/epi_123", "--to", old, "--type", "mentions", "--json"])?;
    proof.claim(
        "linking a merged ID lands on its survivor",
        linked.status.success() && String::from_utf8_lossy(&linked.stdout).contains(edge),
        String::from_utf8_lossy(&linked.stderr),
    );
    for rebuilt in [false, true] {
        if rebuilt {
            neomem.call(proof, neomem_proto::Request::Rebuild)?;
        }
        let view = neomem.neomem(&["entity", "Edge"])?;
        let text = String::from_utf8_lossy(&view.stdout);
        proof.claim(
            if rebuilt {
                "the external reference survives rebuild"
            } else {
                "entity lists mentioned in 1 item in episodes with its reference"
            },
            view.status.success()
                && text.contains("mentioned in 1 item in episodes")
                && text.contains("ext:episodes/epi_123"),
            text,
        );
    }
    for database in ["state.db", "index.db"] {
        let connection = rusqlite::Connection::open_with_flags(
            neomem.home().root().join(database),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let (table, reference, target, kind) = if database == "state.db" {
            ("external_links", "external", "target", "kind")
        } else {
            ("links", "from_id", "to_id", "type")
        };
        let sql = format!(
            "SELECT {reference}, {target}, {kind} FROM {table} WHERE {reference} = 'ext:episodes/epi_123'"
        );
        let stored: (String, String, String) =
            connection.query_row(&sql, [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        proof.claim(
            "external storage contains the reference, survivor and kind",
            stored == ("ext:episodes/epi_123".to_owned(), edge.to_owned(), "mentions".to_owned()),
            format!("{database}: {stored:?}"),
        );
        if database == "state.db" {
            let mut statement = connection.prepare("PRAGMA table_info(external_links)")?;
            let columns: Vec<String> =
                statement.query_map([], |row| row.get(1))?.collect::<Result<_, _>>()?;
            proof.claim(
                "external registry has no external content column",
                columns == ["external", "target", "kind", "recorded_at"],
                format!("{columns:?}"),
            );
        }
    }
    for (reference, target, kind) in [
        ("episodes/epi_123", edge, "mentions"),
        ("ext:episodes/epi_123", edge, "unknown"),
        ("ext:episodes/epi_123", "ent_01M56PGSJG83SYHRJXJG7DJ15Q", "mentions"),
    ] {
        let output = neomem.neomem(&["link", reference, "--to", target, "--type", kind, "--json"])?;
        proof.claim(
            "invalid external references, kinds and targets are clearly rejected",
            !output.status.success()
                && (!output.stderr.is_empty() || String::from_utf8_lossy(&output.stdout).contains("message")),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}
