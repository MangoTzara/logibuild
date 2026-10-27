library ieee;
use ieee.std_logic_1164.all;

-- Hexadecimal digit to 7-segment pattern: bit 0 = segment a ... bit 6 = g,
-- bit 7 = decimal point (the 8-bit input of a multi-digit 7-segment display).
entity seg7 is
    port (
        digit : in  std_logic_vector(3 downto 0);
        seg   : out std_logic_vector(7 downto 0)
    );
end entity;

architecture rtl of seg7 is
begin
    with digit select seg <=
        x"3F" when x"0",
        x"06" when x"1",
        x"5B" when x"2",
        x"4F" when x"3",
        x"66" when x"4",
        x"6D" when x"5",
        x"7D" when x"6",
        x"07" when x"7",
        x"7F" when x"8",
        x"6F" when x"9",
        x"77" when x"A",
        x"7C" when x"B",
        x"39" when x"C",
        x"5E" when x"D",
        x"79" when x"E",
        x"71" when others;
end architecture;
