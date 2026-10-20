use serde_json::Value;

pub fn violation(value: &Value, schema: &Value) -> Option<String> {
    violation_at(value, schema, "the answer")
}

pub fn is_strict(schema: &Value) -> bool {
    let object_is_closed = schema["type"] != "object" || {
        let properties = schema["properties"].as_object();
        let required: Vec<&str> =
            schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        schema["additionalProperties"] == false
            && properties
                .is_none_or(|properties| properties.keys().all(|name| required.contains(&name.as_str())))
    };
    let nested = schema["properties"].as_object().into_iter().flat_map(|properties| properties.values());
    let alternatives = schema["anyOf"].as_array().into_iter().flatten();
    object_is_closed
        && nested.chain(alternatives).all(is_strict)
        && (schema["items"].is_null() || is_strict(&schema["items"]))
}

fn violation_at(value: &Value, schema: &Value, path: &str) -> Option<String> {
    if let Some(allowed) = schema["enum"].as_array()
        && !allowed.contains(value)
    {
        return Some(format!("{path} is not one of the allowed values"));
    }
    if let Some(shapes) = schema["anyOf"].as_array()
        && shapes.iter().all(|shape| violation_at(value, shape, path).is_some())
    {
        return Some(format!("{path} matches none of the allowed shapes"));
    }
    if let Some(expected) = expected_types(schema)
        && !expected.iter().any(|kind| has_type(value, kind))
    {
        return Some(format!("{path} should be {}", expected.join(" or ")));
    }
    match value {
        Value::Object(fields) => {
            let required = schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str);
            if let Some(missing) = required.into_iter().find(|name| !fields.contains_key(*name)) {
                return Some(format!("{path} is missing the field {missing}"));
            }
            fields.iter().find_map(|(name, inner)| match schema["properties"].get(name) {
                Some(field_schema) => violation_at(inner, field_schema, &format!("{path}.{name}")),
                None if schema["additionalProperties"] == false => {
                    Some(format!("{path} has a field the schema does not allow: {name}"))
                }
                None => None,
            })
        }
        Value::Array(items) if !schema["items"].is_null() => items
            .iter()
            .enumerate()
            .find_map(|(index, item)| violation_at(item, &schema["items"], &format!("{path}[{index}]"))),
        _ => None,
    }
}

fn expected_types(schema: &Value) -> Option<Vec<&str>> {
    match &schema["type"] {
        Value::String(kind) => Some(vec![kind.as_str()]),
        Value::Array(kinds) => Some(kinds.iter().filter_map(Value::as_str).collect()),
        _ => None,
    }
}

fn has_type(value: &Value, kind: &str) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "queries": { "type": "array", "items": { "type": "string" } },
                "mood": { "enum": ["calm", "busy"] }
            },
            "required": ["queries", "mood"],
            "additionalProperties": false
        })
    }

    #[test]
    fn a_fitting_answer_has_no_violation() {
        assert_eq!(violation(&json!({ "queries": ["edge"], "mood": "calm" }), &schema()), None);
    }

    #[test]
    fn missing_fields_wrong_types_and_extra_fields_are_named() {
        assert!(violation(&json!({ "queries": ["edge"] }), &schema()).unwrap().contains("mood"));
        assert!(
            violation(&json!({ "queries": [1], "mood": "calm" }), &schema()).unwrap().contains("queries[0]")
        );
        let extra = json!({ "queries": [], "mood": "busy", "note": "x" });
        assert!(violation(&extra, &schema()).unwrap().contains("note"));
    }

    #[test]
    fn strict_schemas_close_every_object_and_require_every_field() {
        assert!(is_strict(&schema()));
        assert!(!is_strict(&json!({ "type": "object", "properties": { "a": { "type": "string" } } })));
    }
}
