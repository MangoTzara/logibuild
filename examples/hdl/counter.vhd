library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- 4-bit counter with enable and asynchronous reset.
entity counter is
    port (
        clk : in  std_logic;
        rst : in  std_logic;
        en  : in  std_logic;
        q   : out std_logic_vector(3 downto 0)
    );
end entity;

architecture rtl of counter is
    signal count : unsigned(3 downto 0);
begin
    process (clk, rst)
    begin
        if rst = '1' then
            count <= (others => '0');
        elsif rising_edge(clk) then
            if en = '1' then
                count <= count + 1;
            end if;
        end if;
    end process;

    q <= std_logic_vector(count);
end architecture;
