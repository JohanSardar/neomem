#[allow(dead_code)]
mod wire {
    include!(concat!(env!("OUT_DIR"), "/wire_schema.rs"));
}

#[derive(Debug, schemars::JsonSchema)]
#[schemars(untagged)]
#[allow(dead_code)]
enum ContractSchema {
    Request(wire::Envelope),
    Response(wire::Response),
}

pub fn contract_schema() -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&schemars::schema_for!(ContractSchema)).map(|schema| format!("{schema}\n"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn checked_in_schema_matches_protocol_types() {
        assert_eq!(super::contract_schema().unwrap(), include_str!("../../../contract/neomem.schema.json"));
    }
}
