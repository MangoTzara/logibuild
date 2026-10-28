// 16 x 8 single-port RAM: written on the rising edge when we = 1, read at
// any time. Becomes one RAM component.
module ram16x8 (
    input  wire       clk,
    input  wire       we,
    input  wire [3:0] addr,
    input  wire [7:0] din,
    output wire [7:0] dout
);
    reg [7:0] mem [0:15];

    always @(posedge clk)
        if (we)
            mem[addr] <= din;

    assign dout = mem[addr];
endmodule

module ram16x85 (
    input  wire       clk,
    input  wire       we,
    input  wire [3:0] addr,
    input  wire [7:0] din,
    output wire [7:0] dout
);
    reg [7:0] mem [0:15];

    always @(posedge clk)
        if (we)
            mem[addr] <= din;

    assign dout = mem[addr];
endmodule