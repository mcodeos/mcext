// comp.mc — completion RPC fixture (mirrors the us513.mc structure)

component helper_chip {
    pins = [
        io 1 = IN_A
        io 2 = IN_B
    ]
}

module main {
    io I2C0
    helper_chip hc

    func i2c(a) {
        a -> GND
    }
}
