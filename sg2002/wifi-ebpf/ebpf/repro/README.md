# 诊断用复现件（个人中间产物，不入库）

本目录是 2026-09-25 定位 netmon attach 卡点时用的复现与调试件，均不参与项目追踪。

## `kallsyms-repro`：宿主侧复现内核 kallsyms 名字查找

用途：确认 `KallsymsMapped::lookup_name` 不会挂死（卡点不在符号解析）。

```sh
# 1. 从内核 ELF 抽取 .kallsyms 段（file offset / size 取自 readelf -S）
dd if=target/riscv64gc-unknown-none-elf/release/starryos \
   of=www/ebpf/repro/kallsyms.bin bs=1 skip=$((0x68b000)) count=$((0x800000)) status=none
# 2. 运行
cd www/ebpf/repro && cargo run --offline --release
```

结论：全部 10333 个符号里，探针用的三个名字都能正常解析（`sched_irq` → `0xffffffff80168dbe`），
仅 1 个 250 字符的超长名字回查失败；因此 attach 卡点与 kallsyms 查找无关。

## `hang.gdb` / `one.gdb` / `csr.gdb`：QEMU gdb stub 取样

`qemu-riscv64.toml` 的 `args` 临时加 `-s` 后：

```sh
gdb-multiarch -q -batch -x www/ebpf/repro/csr.gdb \
    target/riscv64gc-unknown-none-elf/release/starryos
```

卡死时的取样结果（4/4 次完全一致）：

| 寄存器 | 值 |
| --- | --- |
| `pc` | `0xffffffff80001148`（`trap_vector_base`） |
| `ra` | `0xffffffff80116ab4`（`split_leaf_for_boundary+1102`） |
| `scause` | `12`（指令页错误） |
| `stval` | `0xffffffff80001148`（即陷入向量自身地址） |

判读：陷入向量所在页已被解除映射，CPU 每次取指都再次陷入同一条向量 → 无限陷入风暴。
`split_leaf_for_boundary` 由 `protect_region` 调用，后者由内核文本改权限（`patch_kernel_text`）
经由块映射拆分触发；拆分过程 `clear()` 掉整个块描述符，连带解除映射了正在执行的代码与陷入向量。
