set pagination off
set confirm off
set width 0
target remote :1234
printf "=== ALL REGISTERS (CSR search) ===\n"
info all-registers
printf "=== STACK ===\n"
x/40gx $sp
printf "=== CALL SITE ===\n"
x/6i $ra-20
detach
quit
