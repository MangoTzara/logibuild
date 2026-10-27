// Traffic light: green for 6 clock cycles, yellow for 2, red for 6.
// The reset (asynchronous) starts from red.
module traffic_light (
    input  wire clk,
    input  wire rst,
    output wire red,
    output wire yellow,
    output wire green
);
    localparam RED = 2'd0, GREEN = 2'd1, YELLOW = 2'd2;

    reg [1:0] state;
    reg [2:0] timer;

    always @(posedge clk or posedge rst) begin
        if (rst) begin
            state <= RED;
            timer <= 3'd5;
        end else if (timer != 3'd0) begin
            timer <= timer - 3'd1;
        end else begin
            case (state)
                RED:     begin state <= GREEN;  timer <= 3'd5; end
                GREEN:   begin state <= YELLOW; timer <= 3'd1; end
                default: begin state <= RED;    timer <= 3'd5; end
            endcase
        end
    end

    assign red    = state == RED;
    assign yellow = state == YELLOW;
    assign green  = state == GREEN;
endmodule
