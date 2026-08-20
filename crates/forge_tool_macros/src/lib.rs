use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::{DeriveInput, Expr, ExprLit, Lit, parse_macro_input};

/// Custom attribute for specifying tool description file path
extern crate proc_macro;

#[proc_macro_attribute]
pub fn tool_description_file(_attr: TokenStream, _item: TokenStream) -> TokenStream {
    // This is just a marker attribute, the actual processing happens in
    // ToolDescription
    _item
}

#[proc_macro_derive(ToolDescription, attributes(tool_description_file))]
pub fn derive_description(input: TokenStream) -> TokenStream {
    // Parse the input struct or enum
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let generics = &input.generics;

    // Check for tool_description_file attribute first
    let mut description_file = None;
    for attr in &input.attrs {
        if attr.path().is_ident("tool_description_file")
            && let syn::Meta::NameValue(name_value) = &attr.meta
            && let Expr::Lit(ExprLit { lit: Lit::Str(lit_str), .. }) = &name_value.value
        {
            description_file = Some(lit_str.value());
        }
    }

    // If we have a description file, read it at compile time
    let doc_string = if let Some(file_path) = description_file {
        std::fs::read_to_string(&file_path)
            .unwrap_or_else(|e| {
                panic!(
                    "Failed to read tool description file '{}': {}",
                    file_path, e
                )
            })
            .trim()
            // git stores description files with LF endings; on Windows
            // checkouts (autocrlf) the on-disk bytes carry CRLF, which would
            // leak into generated tool descriptions and break snapshot
            // determinism. Normalize to the authored LF bytes.
            .replace("\r\n", "\n")
            .to_string()
    } else {
        // Collect doc lines from doc comments
        let mut doc_lines = Vec::new();
        for attr in &input.attrs {
            if attr.path().is_ident("doc") {
                // Get the doc content as a string
                let doc_string = attr.meta.to_token_stream().to_string();
                // Remove the quotes and = sign
                let clean = doc_string.trim_start_matches("=").trim_matches('"').trim();
                if !clean.is_empty() {
                    doc_lines.push(clean.to_string());
                }
            }
        }

        if doc_lines.is_empty() {
            panic!("No doc comment found for {name}");
        }
        doc_lines.join("\n")
    };

    // Generate the implementation
    let expanded = if generics.params.is_empty() {
        quote! {
            impl ToolDescription for #name {
                fn description(&self) -> String {
                    #doc_string.into()
                }
            }
        }
    } else {
        quote! {
            impl #generics ToolDescription for #name #generics {
                fn description(&self) -> String {
                    #doc_string.into()
                }
            }
        }
    };

    expanded.into()
}
