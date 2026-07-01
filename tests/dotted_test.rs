//! Test: inline comments in net expressions should be tokenized

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
                eprintln!(
                    "COMMENT: type={} pos={} len={} => {:?}",
                    t.type_,
                    t.position,
                    t.length,
                    text.trim()
                );
            } else {
                eprintln!(
                    "OTHER: type={} pos={} len={} => {:?}",
                    t.type_,
                    t.position,
                    t.length,
                    text.trim()
                );
            }
        }
        eprintln!("Found {} comments", found_comments);
    } else {
        panic!("query returned None");
    }
}
