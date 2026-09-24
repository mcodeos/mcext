// net_family.mc — U258 §4 consumption fixture: free named nets offered
// through mcc's `Net` completion layer, and dotted-family member completion
// (`FAM.` lists the FAM.* components). Module name is distinct so private-port
// probes stay isolated from the shared-server fixtures.

component FAM.FA10 {
    pins = [
        1 = IN
        2 = OUT
    ]
}

component FAM.FA20 {
    pins = [
        1 = IN
        2 = OUT
    ]
}

component RES {
    pins = [
        1 = A
        2 = B
    ]
}

module netfam_top {
    RES r1(1kΩ)
    r1.1 -> V5V
    V5V -> r1.2
}
