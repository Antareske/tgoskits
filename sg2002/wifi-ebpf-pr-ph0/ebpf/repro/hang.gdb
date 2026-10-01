set pagination off
set confirm off
set width 0
target remote :1234
printf "\n=== REGISTERS ===\n"
info registers pc ra sp gp tp
printf "\n=== BACKTRACE ===\n"
bt
printf "\n=== CODE AT PC ===\n"
x/8i $pc
printf "\n=== STACK RANGE ===\n"
info proc mappings
detach
quit
