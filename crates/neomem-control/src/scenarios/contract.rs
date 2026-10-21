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
    extraction_uses_registry(proof, &neomem)?;
    let schema = std::fs::read_to_string(crate::repo::root()?.join("contract/neomem.schema.json"))?;
    proof.claim(
        "the JSON Schema matches the real protocol types",
        schema == neomem_proto::contract_schema()?,
        "generated from Request and Response",
    );
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
    fact_links(proof, neomem)?;
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
    merged_references(proof, neomem, edge)?;
    Ok(())
}

fn extraction_uses_registry(
    proof: &mut ProofRun,
    neomem: &crate::isolated::IsolatedNeomem,
) -> anyhow::Result<()> {
    use neomem_core::{EpochMs, IntakeRecord, Source, Span, Tag};
    use neomem_proto::{Request, Response, SearchQuery};
    use neomem_testkit::{FakeRules, LlmRule};

    neomem.add_fake_rules(&FakeRules {
        llm: vec![LlmRule {
            user_contains: Some("contract basil".to_owned()),
            reply: r#"{"facts":[{"content":"Johan grows basil on the balcony.","sources":[1],"about":["E1"],"valid_from":null,"importance":3,"durability":"ordinary","tags":["healthcare","horticulture"]}],"entities":[{"ref":"E1","name":"Johan","kind":"person","aliases":[],"existing":null}]}"#.to_owned(),
            ..LlmRule::default()
        }],
        ..FakeRules::default()
    })?;
    let elapsed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
    let now = EpochMs::from_millis(i64::try_from(elapsed.as_millis())?);
    neomem.call(
        proof,
        Request::Add(IntakeRecord {
            source: Source::new("note")?,
            occurred: Span::instant(now),
            content: "contract basil: Johan grows basil on the balcony".to_owned(),
            tags: vec![Tag::new("healthcare")?],
            entity_hints: vec!["Johan".to_owned()],
            source_ref: None,
            session: None,
        }),
    )?;
    neomem.set_clock(now.plus_millis(31 * 60_000))?;
    crate::scenarios::support::wait_for_index(neomem, proof, 3, 3)?;
    let result = neomem.call(
        proof,
        Request::Search(SearchQuery {
            text: "basil".to_owned(),
            tags: vec![Tag::new("horticulture")?],
            ..SearchQuery::default()
        }),
    )?;
    proof.claim("extractor proposals use the same semantic registry and remain searchable by tag",
        matches!(&result, Response::SearchResults(results) if results.hits.iter().any(|hit| hit.fact.content() == "Johan grows basil on the balcony." && hit.fact.tags().iter().any(|tag| tag.as_str() == "health") && !hit.fact.tags().iter().any(|tag| tag.as_str() == "healthcare"))),
        format!("{result:?}"));
    let registry = neomem.call(proof, Request::Tags { new: None })?;
    proof.claim("extractor tags enter the registry without semantic duplicates",
        matches!(&registry, Response::Tags(tags) if tags.iter().any(|tag| tag.as_str() == "horticulture") && !tags.iter().any(|tag| tag.as_str() == "healthcare")),
        format!("{registry:?}"));
    Ok(())
}

fn fact_links(proof: &mut ProofRun, neomem: &crate::isolated::IsolatedNeomem) -> anyhow::Result<()> {
    let fact = "fct_01M59NY2N09AC5V7E9Z0C1HT0H";
    for _ in 0..2 {
        let output =
            neomem.neomem(&["link", "ext:notes/note_1", "--to", fact, "--type", "mentions", "--json"])?;
        anyhow::ensure!(
            output.status.success(),
            "fact link failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let connection = rusqlite::Connection::open_with_flags(
        neomem.home().state_db_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM external_links WHERE external = 'ext:notes/note_1' AND target = ?1",
        [fact],
        |row| row.get(0),
    )?;
    proof.claim("external references can target facts idempotently", count == 1, count);

    Ok(())
}

fn merged_references(
    proof: &mut ProofRun,
    neomem: &crate::isolated::IsolatedNeomem,
    edge: &str,
) -> anyhow::Result<()> {
    use neomem_core::{ExternalLinkKind, ExternalRef, MemoryId};
    use neomem_proto::{Request, Response};

    let duplicate = "ent_01M56PGSJG83SYHRJXJG7DJ16R";
    let path = neomem.home().root().join("entities/project/edge-web.md");
    let unmerged = format!(
        "---\nid: {duplicate}\nkind: entity\nentity_kind: project\nname: Edge web\naliases: []\nrecorded_at: 1792300051000\nupdated_at: 1792300052000\nmerged_from: []\nmerged_into: null\nrelations: []\nlinks: []\n---\n"
    );
    std::fs::write(&path, &unmerged)?;
    neomem.call(proof, Request::Rebuild)?;
    let external: ExternalRef = "ext:books/item_1".parse()?;
    for target in [edge, duplicate] {
        let result = neomem.call(
            proof,
            Request::Link {
                external: external.clone(),
                to: target.parse()?,
                kind: ExternalLinkKind::Mentions,
            },
        )?;
        anyhow::ensure!(matches!(result, Response::Linked(_)), "link setup failed: {result:?}");
    }
    std::fs::write(&path, unmerged.replace("merged_into: null", &format!("merged_into: {edge}")))?;
    neomem.call(proof, Request::Rebuild)?;
    let Response::Entity(view) = neomem.call(proof, Request::Entity { name: edge.to_owned() })? else {
        anyhow::bail!("merged entity view is unavailable");
    };
    let mentions: Vec<_> = view.external.into_iter().filter(|link| link.external == external).collect();
    let survivor: MemoryId = edge.parse()?;
    proof.claim(
        "an item linked to both entities is counted once after their merge",
        matches!(mentions.as_slice(), [link] if link.to == survivor),
        format!("{mentions:?}"),
    );
    Ok(())
}
