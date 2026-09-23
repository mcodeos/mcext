// refs_main.mc — cross-file find-references fixture (U258 §2 refs wiring)
//
// Canonical shape: `use ./helper` pulls helper.mc, the module body instantiates
// the foreign class and wires it. Cursor on `helper_chip` in the instance row
// must answer with helper_chip's ClassDef in helper.mc plus the ClassRef here.

use ./helper

component MAIN {
    pins = [
        io 1 = SIG_IN
        io 2 = SIG_OUT
    ]
}

module top {
    MAIN main_i()
    helper_chip hc()
    main_i.1 -> hc.1
    main_i.2 -> hc.2
}
