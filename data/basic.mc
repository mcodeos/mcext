component basic
{
    right = [2,1,5,6]

    layout = [
        left  = [7,8,3,4] // set[]
        right = [2,1,5,6]
    ]

    /*spec = [
        // single record
        Vgs = -20V ~ +20V, "Gate-Source Voltage"      // pin Gate to Source Voltage could be +/-20V
        //Vgs = [-20V ~ +20V, "Gate-Source Voltage"]      // pin Gate to Source Voltage could be +/-20V
        // multi record
        Rdson = [
            1 = 60mΩ, Vgs:-10V, Id:-4.1A                  // Rdson[1]=60mΩ, under condition Vgs:-10V, Id:-4.1A
            2 = 80mΩ, Vgs:-10V, Id:-4.1A                  // Rdson[1]=60mΩ, under condition Vgs:-10V, Id:-4.1A
            3 = 100mΩ, Vgs:-10V, Id:-4.1A                  // Rdson[1]=60mΩ, under condition Vgs:-10V, Id:-4.1A
        ]
    ]

    pins = [
        io DAP0
        io DAP1
    ]*/
    pins = 
    [ 
        io 1 = RXD, "Receive Data", [high:+3V ~ +15V, low:-15V ~ -3V]
    ]
    pins = [
        1 = G, "Gate"
    ]
    pins = [ 
        io B = Base
        io C = Collector
        io E = Emmiter
    ]
    pins = [ 
        1 = 1
        2 = 2
    ]
    
    pins = [
        [1:4] = IN[A,B,C,D] 
    ]
    pins = [                  // Pin definition
        [1:5] = USB::USB      // USB interface 
        [6,7] = [VCC, GND]  // USB GND, USB GND pin
    ]
    pins = [
        in A = INA, "Logic input A"
    ]

    pins = [ 
        [A1, Y1] = VCC
    ]

    pins = [                                        // Pin definition
        in [1,2] = VIN[Vin, GND]::DC(2.5V~5.5V)     // Power input
    ]

    pins = [
    
    ]

}
