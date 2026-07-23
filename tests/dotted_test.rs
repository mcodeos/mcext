//! Test: inline comments in net expressions should be tokenized.
//!
//! TODO: this test was written against the `mcc` crate, which has been removed
//! from `mcext` dependencies ("all interactions via RPC"). The original
//! `mcc::mcc_init_no_lib / mcc_load_from_string / mcc_query` flow is no longer
//! available here.
//!
//! To re-enable:
//! - Either re-add the `mcc` crate as a dependency in `Cargo.toml`, or
//! - Rewrite this test to drive parsing through the RPC pipeline
//!   (see `tests/integration.rs` for the modern pattern using
//!   `mcodels::mccsrv::MccServer`).
//!
//! The original test body is preserved below as a reference, but is gated
//! behind a never-enabled cfg so it does not break the build.

#[cfg(any())]
mod original {
    use mcc::McURI;

    #[test]
    fn test_inline_comment_in_net() {
        let src = r#"module M {
    uC.6 -> CAP(1uF) -> RES(10k)  // inline1
    -> (
        CAP(330nF) -> DAC_OUT,     // inline2
        RES(10k) -> GND            // inline3
    )
}
"#;
        mcc::mcc_init_no_lib();
        let uri = McURI::from("/test_net_comments.mc");
        mcc::mcc_load_from_string(&uri, src);

        if let Some(result) = mcc::mcc_query(&uri) {
            let tokens = result.sem_tokens.lock().unwrap();
            eprintln!("Total tokens: {}", tokens.tokens.len());
            let mut found_comments = 0;
            for t in &tokens.tokens {
                let text = &src[t.position as usize..(t.position + t.length) as usize];
                if t.type_ == 16 || t.type_ == 101 {
                    found_comments += 1;
                } else {
                    // no-op
                }
            }
            eprintln!("Found {} comments", found_comments);
        } else {
            panic!("query returned None");
        }
    }
}

#[test]
#[ignore = "placeholder; see module-level TODO for re-enable instructions"]
fn dotted_test_placeholder() {
    // Intentionally empty — see module-level TODO.
}
