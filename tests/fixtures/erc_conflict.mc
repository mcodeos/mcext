// erc_conflict.mc — deterministic driver-conflict fixture (U258 §2 erc wiring)
//
// Same shape as mcc's own lock fixture (tests/shard7/
// flatten_net_check_diagnostics.rs BUF driver-conflict): two Out pins merged
// onto one net must fire the flat ERC `driver-conflict` rule with severity
// "error". Module name is distinct from `main` so the shared-server tests can
// isolate the module set by file.

component BUF {
    pins = [
        in 1 = A
        out 2 = Y
    ]
}

module erc_top {
    BUF b1
    BUF b2
    b1.Y -> b2.Y
}
