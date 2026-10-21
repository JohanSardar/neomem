use rmcp::model::Tool;
use serde_json::{Value, json};

pub(super) struct ToolEntry {
    pub(super) name: &'static str,
    description: &'static str,
    pub(super) properties: Value,
    required: &'static [&'static str],
}

pub(super) fn table() -> Vec<ToolEntry> {
    let string = json!({"type":"string"});
    let integer = json!({"type":"integer"});
    let strings = json!({"type":"array","items":{"type":"string"}});
    vec![
        ToolEntry {
            name: "tags",
            description: "List tags or register a new tag",
            properties: json!({"new":string}),
            required: &[],
        },
        ToolEntry {
            name: "link",
            description: "Attach an external reference to a memory",
            properties: json!({"external":string,"to":string,"kind":{"type":"string","enum":["mentions"]}}),
            required: &["external", "to", "kind"],
        },
        ToolEntry {
            name: "search",
            description: "Search semantic memory",
            properties: json!({"text":string,"k":integer,"tags":strings,"entities":strings,"from":integer,"to":integer,"as_of":integer,"include_invalid":{"type":"boolean"},"include_hidden":{"type":"boolean"}}),
            required: &["text"],
        },
        ToolEntry {
            name: "add",
            description: "Append content to intake",
            properties: json!({"content":string,"source":string,"tags":strings,"start":integer,"end":integer,"entity_hints":strings,"source_ref":string,"session":string}),
            required: &["content", "source"],
        },
        ToolEntry {
            name: "raw",
            description: "Search the intake log",
            properties: json!({"text":string,"from":integer,"to":integer,"source":string,"limit":integer}),
            required: &[],
        },
        ToolEntry {
            name: "entity",
            description: "Read an entity and its facts",
            properties: json!({"name":string}),
            required: &["name"],
        },
        ToolEntry {
            name: "resolve",
            description: "Resolve an entity name",
            properties: json!({"name":string}),
            required: &["name"],
        },
        ToolEntry {
            name: "show",
            description: "Read a memory and its sources",
            properties: json!({"id":string}),
            required: &["id"],
        },
        ToolEntry {
            name: "history",
            description: "Read a fact's version history",
            properties: json!({"id":string}),
            required: &["id"],
        },
        ToolEntry {
            name: "useful",
            description: "Record that a fact was useful",
            properties: json!({"id":string}),
            required: &["id"],
        },
    ]
}

impl ToolEntry {
    pub(super) fn tool(self) -> Tool {
        let schema = json!({"type":"object","properties":self.properties,"required":self.required,"additionalProperties":false});
        Tool::new(self.name, self.description, schema.as_object().cloned().unwrap_or_default())
    }
}
