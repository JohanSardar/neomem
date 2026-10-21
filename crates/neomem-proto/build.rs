use std::{env, fs, path::Path};

use quote::ToTokens;
use syn::{Attribute, Item};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut definitions = String::from("use std::path::PathBuf;\n");
    for directory in ["../neomem-core/src", "src"] {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.extension().is_none_or(|extension| extension != "rs") || path.ends_with("schema.rs") {
                continue;
            }
            println!("cargo:rerun-if-changed={}", path.display());
            for item in syn::parse_file(&fs::read_to_string(&path)?)?.items {
                let definition = match item {
                    Item::Struct(mut item) if serialized(&item.attrs) => {
                        let default = if item
                            .attrs
                            .iter()
                            .any(|attribute| attribute.to_token_stream().to_string().contains("Default"))
                        {
                            "Default,"
                        } else {
                            ""
                        };
                        item.attrs.retain(|attribute| attribute.path().is_ident("serde"));
                        let scalar = item.attrs.iter().find_map(|attribute| {
                            let text = attribute.to_token_stream().to_string();
                            if text.contains("try_from = \"String\"") {
                                Some("String")
                            } else if text.contains("try_from = \"u8\"") {
                                Some("u8")
                            } else {
                                None
                            }
                        });
                        if let Some(scalar) = scalar {
                            item.attrs.clear();
                            item.attrs.push(syn::parse_quote!(#[schemars(with = #scalar)]));
                        }
                        format!(
                            "#[derive(Debug, {default} serde::Serialize, schemars::JsonSchema)] {}",
                            item.to_token_stream()
                        )
                    }
                    Item::Enum(mut item) if serialized(&item.attrs) => {
                        item.attrs.retain(|attribute| attribute.path().is_ident("serde"));
                        format!(
                            "#[derive(Debug, serde::Serialize, schemars::JsonSchema)] {}",
                            item.to_token_stream()
                        )
                    }
                    Item::Impl(item)
                        if item.trait_.as_ref().is_some_and(|(path, _)| path.is_ident("Default")) =>
                    {
                        item.to_token_stream().to_string()
                    }
                    Item::Fn(item) if item.sig.ident.to_string().starts_with("default_") => {
                        item.to_token_stream().to_string()
                    }
                    Item::Macro(item) if item.mac.path.is_ident("prefixed_id") => {
                        let arguments = item.mac.tokens.to_string();
                        let (name, prefix) =
                            arguments.split_once(',').ok_or("invalid prefixed ID definition")?;
                        let prefix = prefix.trim().trim_matches('"');
                        format!(
                            "#[derive(Debug, serde::Serialize, schemars::JsonSchema)] #[schemars(with = \"String\", extend(\"pattern\" = \"^{prefix}_[0-7][0-9A-HJKMNP-TV-Za-hjkmnp-tv-z]{{25}}$\"))] pub struct {name}(String);"
                        )
                    }
                    _ => continue,
                };
                definitions.push_str(&definition.replace("neomem_core ::", "").replace("crate ::", ""));
                definitions.push('\n');
            }
        }
    }
    let output = env::var("OUT_DIR")?;
    fs::write(Path::new(&output).join("wire_schema.rs"), definitions)?;
    Ok(())
}

fn serialized(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("derive")
            && attribute
                .to_token_stream()
                .to_string()
                .split(|character: char| !character.is_alphanumeric())
                .any(|word| word == "Serialize" || word == "Deserialize")
    })
}
