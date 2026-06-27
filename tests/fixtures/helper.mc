// helper.mc — defines helper_chip component

component helper_chip {
    pins = [
        io 1 = IN_A
        io 2 = IN_B
        io 3 = OUT
    ]
}

component shared_component {
    pins = [
        io 1 = VCC
        io 2 = GND
    ]
}