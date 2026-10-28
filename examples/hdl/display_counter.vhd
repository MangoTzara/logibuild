library ieee;
use ieee.std_logic_1164.all;

-- A counter shown on a 7-segment digit: two entities, two subcircuits.
entity display_counter is
    port (
        clk   : in  std_logic;
        rst   : in  std_logic;
        en    : in  std_logic;
        value : out std_logic_vector(3 downto 0);
        seg   : out std_logic_vector(7 downto 0)
    );
end entity;

architecture rtl of display_counter is
    signal count : std_logic_vector(3 downto 0);
begin
    cnt : entity work.counter
        port map (clk => clk, rst => rst, en => en, q => count);

    dec : entity work.seg7
        port map (digit => count, seg => seg);

    value <= count;
end architecture;
