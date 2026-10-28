library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- 8-bit ALU: op 00 = a + b, 01 = a - b, 10 = a and b, 11 = a xor b.
entity alu is
    port (
        a, b : in  std_logic_vector(7 downto 0);
        op   : in  std_logic_vector(1 downto 0);
        y    : out std_logic_vector(7 downto 0);
        zero : out std_logic
    );
end entity;

architecture rtl of alu is
    signal r : std_logic_vector(7 downto 0);
begin
    with op select r <=
        std_logic_vector(unsigned(a) + unsigned(b)) when "00",
        std_logic_vector(unsigned(a) - unsigned(b)) when "01",
        a and b                                     when "10",
        a xor b                                     when others;

    y    <= r;
    zero <= '1' when r = x"00" else '0';
end architecture;
