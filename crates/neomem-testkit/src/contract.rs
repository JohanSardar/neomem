use neomem_core::{Choice, Embedder, Judge};

const RELATIONS: &[Choice<'static>] = &[
    Choice { label: "same", meaning: "says the same thing as the existing fact" },
    Choice { label: "updates", meaning: "replaces the existing fact" },
    Choice { label: "new", meaning: "is unrelated to the existing fact" },
];

pub fn embedder_contract(embedder: &dyn Embedder) {
    let texts = ["Edge ships on November 1", "Maya is my sister", "Edge ships on November 1"];
    let vectors = embedder.embed(&texts).expect("the embedder answers");
    assert_eq!(vectors.len(), texts.len(), "one vector per text");
    assert!(vectors.iter().all(|vector| vector.dims() == embedder.dims()), "every vector has dims()");
    for vector in &vectors {
        let length: f32 = vector.as_slice().iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((length - 1.0).abs() < 1e-3, "vectors are unit length, got {length}");
    }
    assert!(vectors[0].cosine(&vectors[2]) > 0.999, "the same text embeds the same");
    assert_eq!(embedder.embed(&[]).expect("empty input answers"), Vec::new(), "no texts, no vectors");
    assert!(!embedder.model_id().as_str().is_empty(), "the model names itself");
}

pub fn judge_contract(judge: &dyn Judge) {
    let decision = judge
        .decide(
            "New fact: Edge ships Nov 1. Existing fact 1: Edge ships Oct 15.",
            "How does the new fact relate?",
            RELATIONS,
        )
        .expect("the judge decides");
    assert_eq!(decision.probabilities.len(), RELATIONS.len(), "one probability per option");
    assert!(decision.probabilities.iter().all(|probability| (0.0..=1.0).contains(probability)));
    let scores = judge
        .score(
            "The signing error is back",
            "Would this memory help right now?",
            &["Signing needs the Edge cert", "Maya likes tea"],
        )
        .expect("the judge scores");
    assert_eq!(scores.len(), 2, "one score per candidate");
    assert!(scores.iter().all(|probability| (0.0..=1.0).contains(probability)));
    assert_eq!(judge.score("context", "question", &[]).expect("empty input answers"), Vec::<f32>::new());
}
