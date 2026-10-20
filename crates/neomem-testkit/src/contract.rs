use neomem_core::{Choice, Embedder, Judge, Llm, LlmMessage, LlmRequest, Purpose, ToolSpec};
use serde_json::Value;

const ANSWER_SCHEMA: &str = r#"{"type":"object","properties":{"answer":{"type":"string"},"confidence":{"type":"number"}},"required":["answer","confidence"],"additionalProperties":false}"#;
const WEATHER_PARAMETERS: &str = r#"{"type":"object","properties":{"city":{"type":"string"}},"required":["city"],"additionalProperties":false}"#;

pub fn llm_contract(llm: &dyn Llm) {
    let plain = request("Answer with the single word OK.", "ping");
    let mut deltas = Vec::new();
    let reply = llm.complete(&plain, &mut |delta| deltas.push(delta.to_owned())).expect("the model answers");
    assert!(!reply.text.trim().is_empty(), "a plain request gets text");
    assert!(!deltas.is_empty(), "text reaches on_text");
    assert_eq!(deltas.concat(), reply.text, "the streamed deltas add up to the reply");

    let mut structured = request("Answer as JSON.", "Is Paris in France?");
    structured.response_schema = Some(ANSWER_SCHEMA.to_owned());
    let reply = llm.complete(&structured, &mut |_| {}).expect("the model answers with structure");
    let value: Value = serde_json::from_str(&reply.text).expect("structured output parses as JSON");
    assert!(value["answer"].is_string() && value["confidence"].is_number(), "it fits the schema: {value}");

    let mut asking = request("Use the lookup_weather tool to answer.", "What is the weather in Paris?");
    asking.tools.push(ToolSpec {
        name: "lookup_weather".to_owned(),
        description: "Looks up today's weather for a city".to_owned(),
        parameters_schema: WEATHER_PARAMETERS.to_owned(),
    });
    let reply = llm.complete(&asking, &mut |_| {}).expect("the model answers with a tool call");
    let call = reply.tool_calls.first().expect("the model calls the tool").clone();
    assert_eq!(call.name, "lookup_weather");
    assert!(!call.id.is_empty(), "a tool call has an ID");
    let arguments: Value = serde_json::from_str(&call.arguments).expect("tool arguments are JSON");
    assert!(arguments["city"].is_string(), "the arguments fit the tool's schema: {arguments}");
    asking.messages.push(LlmMessage::Assistant { text: reply.text, tool_calls: reply.tool_calls });
    asking
        .messages
        .push(LlmMessage::ToolResult { call_id: call.id, content: "18 degrees and sunny".to_owned() });
    let answer = llm.complete(&asking, &mut |_| {}).expect("the model answers after the tool result");
    assert!(answer.tool_calls.is_empty() && !answer.text.trim().is_empty(), "the tool result leads to text");
    assert!(!llm.model_id().as_str().is_empty(), "the model names itself");
}

fn request(system: &str, user: &str) -> LlmRequest {
    LlmRequest {
        system: system.to_owned(),
        messages: vec![LlmMessage::User(user.to_owned())],
        response_schema: None,
        tools: Vec::new(),
        max_output_tokens: 256,
    }
}

const RELATIONS: &[Choice<'static>] = &[
    Choice { label: "same", meaning: "says the same thing as the existing fact" },
    Choice { label: "updates", meaning: "replaces the existing fact" },
    Choice { label: "new", meaning: "is unrelated to the existing fact" },
];

pub fn embedder_contract(embedder: &dyn Embedder) {
    let texts = ["Edge ships on November 1", "Maya is my sister", "Edge ships on November 1"];
    let vectors = embedder.embed(&texts, Purpose::Document).expect("the embedder answers");
    assert_eq!(vectors.len(), texts.len(), "one vector per text");
    assert!(vectors.iter().all(|vector| vector.dims() == embedder.dims()), "every vector has dims()");
    for vector in &vectors {
        let length: f32 = vector.as_slice().iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((length - 1.0).abs() < 1e-3, "vectors are unit length, got {length}");
    }
    assert!(vectors[0].cosine(&vectors[2]) > 0.999, "the same text embeds the same");
    assert_eq!(
        embedder.embed(&[], Purpose::Query).expect("empty input answers"),
        Vec::new(),
        "no texts, no vectors"
    );
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
