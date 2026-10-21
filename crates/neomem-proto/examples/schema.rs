fn main() -> Result<(), serde_json::Error> {
    print!("{}", neomem_proto::contract_schema()?);
    Ok(())
}
