// main.mc — uses helper_chip (defined in helper.mc)

component main {
    pins = [
        io 1 = SIG_IN
        io 2 = SIG_OUT
    ]
}

// Reference helper_chip from helper.mc
helper_chip::helper_chip.1 - SIG_IN
helper_chip::helper_chip.2 - SIG_OUT

// Reference shared_component from helper.mc
shared_component::shared_component.1 - VCC