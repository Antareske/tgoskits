setenv fitaddr 0x0a000000
setenv bootargs root=PARTLABEL=starry-rootfs earlycon=uart8250,mmio32,0xfeb50000 rootwait rootfstype=ext4
for dev in 1 0; do
    if fatload mmc ${dev}:1 ${fitaddr} starry-image.fit; then
        bootm ${fitaddr}
    fi
done
