use neomem_core::ToolSpec;
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    SearchFacts,
    RecentFacts,
    SearchIntake,
    FollowLinks,
    Link,
    Invalidate,
    MergeFacts,
    SearchEntities,
    EntityFacts,
    CreateEntity,
    MergeEntities,
    AddAlias,
    LinkEntity,
}

pub(crate) const RECONCILER_TOOLS: &[Tool] = &[
    Tool::SearchFacts,
    Tool::RecentFacts,
    Tool::SearchIntake,
    Tool::FollowLinks,
    Tool::Link,
    Tool::Invalidate,
    Tool::MergeFacts,
];

pub(crate) const ENTITY_KEEPER_TOOLS: &[Tool] = &[
    Tool::SearchFacts,
    Tool::SearchEntities,
    Tool::EntityFacts,
    Tool::CreateEntity,
    Tool::MergeEntities,
    Tool::AddAlias,
    Tool::LinkEntity,
];

impl Tool {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::SearchFacts => "search_facts",
            Self::RecentFacts => "recent_facts",
            Self::SearchIntake => "search_intake",
            Self::FollowLinks => "follow_links",
            Self::Link => "link",
            Self::Invalidate => "invalidate",
            Self::MergeFacts => "merge_facts",
            Self::SearchEntities => "search_entities",
            Self::EntityFacts => "entity_facts",
            Self::CreateEntity => "create_entity",
            Self::MergeEntities => "merge_entities",
            Self::AddAlias => "add_alias",
            Self::LinkEntity => "link_entity",
        }
    }

    pub(crate) fn writes(self) -> bool {
        matches!(
            self,
            Self::Link
                | Self::Invalidate
                | Self::MergeFacts
                | Self::CreateEntity
                | Self::MergeEntities
                | Self::AddAlias
                | Self::LinkEntity
        )
    }

    pub(crate) fn named(name: &str, allowed: &[Self]) -> Option<Self> {
        allowed.iter().copied().find(|tool| tool.name() == name)
    }

    pub(crate) fn spec(self) -> ToolSpec {
        let (description, parameters) = match self {
            Self::SearchFacts => {
                ("Search current facts by meaning.", object(&[("query", json!({ "type": "string" }))]))
            }
            Self::RecentFacts => {
                ("List the most recently written facts.", object(&[("limit", json!({ "type": "integer" }))]))
            }
            Self::SearchIntake => (
                "Search the raw intake records by keywords.",
                object(&[("query", json!({ "type": "string" }))]),
            ),
            Self::FollowLinks => {
                ("List the facts linked to a fact.", object(&[("fact", json!({ "type": "integer" }))]))
            }
            Self::Link => (
                "Link fact `from` to fact `to`. `updates` means `from` replaces `to`.",
                object(&[
                    ("from", json!({ "type": "integer" })),
                    ("kind", json!({ "enum": ["updates", "extends", "contradicts", "part_of"] })),
                    ("to", json!({ "type": "integer" })),
                ]),
            ),
            Self::Invalidate => (
                "Mark a fact as no longer valid because another fact replaced it.",
                object(&[
                    ("fact", json!({ "type": "integer" })),
                    ("replaced_by", json!({ "type": "integer" })),
                ]),
            ),
            Self::MergeFacts => (
                "Merge a duplicate fact into the fact to keep.",
                object(&[
                    ("keep", json!({ "type": "integer" })),
                    ("duplicate", json!({ "type": "integer" })),
                ]),
            ),
            Self::SearchEntities => {
                ("Find entities by name or meaning.", object(&[("query", json!({ "type": "string" }))]))
            }
            Self::EntityFacts => {
                ("List an entity's current facts.", object(&[("entity", json!({ "type": "integer" }))]))
            }
            Self::CreateEntity => (
                "Create an entity that the given facts are about.",
                object(&[
                    ("name", json!({ "type": "string" })),
                    ("kind", json!({ "type": "string" })),
                    ("aliases", json!({ "type": "array", "items": { "type": "string" } })),
                    ("facts", json!({ "type": "array", "items": { "type": "integer" } })),
                ]),
            ),
            Self::MergeEntities => (
                "Merge a duplicate entity into the entity to keep.",
                object(&[
                    ("keep", json!({ "type": "integer" })),
                    ("duplicate", json!({ "type": "integer" })),
                ]),
            ),
            Self::AddAlias => (
                "Add another name to an entity.",
                object(&[("entity", json!({ "type": "integer" })), ("alias", json!({ "type": "string" }))]),
            ),
            Self::LinkEntity => (
                "Link two entities with a short label such as part_of, possibly_same or works_at.",
                object(&[
                    ("from", json!({ "type": "integer" })),
                    ("label", json!({ "type": "string" })),
                    ("to", json!({ "type": "integer" })),
                ]),
            ),
        };
        ToolSpec {
            name: self.name().to_owned(),
            description: description.to_owned(),
            parameters_schema: parameters.to_string(),
        }
    }
}

fn object(fields: &[(&str, serde_json::Value)]) -> serde_json::Value {
    let properties: serde_json::Map<String, serde_json::Value> =
        fields.iter().map(|(name, schema)| ((*name).to_owned(), schema.clone())).collect();
    let required: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
    json!({ "type": "object", "additionalProperties": false, "required": required, "properties": properties })
}
