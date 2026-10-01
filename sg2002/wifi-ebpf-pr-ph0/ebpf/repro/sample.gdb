set pagination off
set confirm off
target remote :1234
printf "SAMPLES\n"
info registers pc ra
continue &
shell sleep 2
interrupt
info registers pc ra
continue &
shell sleep 2
interrupt
info registers pc ra
continue &
shell sleep 2
interrupt
info registers pc ra
detach
quit
