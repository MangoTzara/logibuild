// 8-bit Fibonacci LFSR (taps 8, 6, 5, 4): runs through the 255 non-zero
// values in a pseudo-random order. The reset value is 1, never 0.
module lfsr (
    input  wire       clk,
    input  wire       rst,
    output reg  [7:0] value
);
    wire feedback = value[7] ^ value[5] ^ value[4] ^ value[3];

    always @(posedge clk or posedge rst)
        if (rst)
            value <= 8'h01;
        else
            value <= {value[6:0], feedback};
endmodule
