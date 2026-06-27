// main.mc — uses helper_chip (defined in helper.mc)
use ./helper.mc

module main 
{
    // Reference helper_chip from helper.mc
    helper_chip hc1
    helper_chip hc2 
    hc1.1 - SIG_OUT
    hc2.1 - SIG_IN

    // Reference shared_component from helper.mc
    shared_component sc1
    sc1.1 - VCC
}

