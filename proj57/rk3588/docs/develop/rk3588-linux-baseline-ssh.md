# RK3588 原生 Linux ACT 推理基线测试（SSH 复现手册）

目标：在香橙派 RK3588 的**原生 Linux**（非 StarryOS）上，通过 SSH 部署 ACT
用户态推理资产并跑基线测试。本文记录主要过程与关键坑位，供未来快速复现。

> StarryOS 板测流程见 `share-rk3588-act-starry.md`。本文专注**原生 Linux** 基线。

---

## 0. 环境约定（示例值）

| 项 | 值 |
|----|----|
| 本机网口 | `enx6c1ff7c76251`，IP `192.168.100.100/24` |
| 板卡地址 | `192.168.100.101` |
| 账号 / 密码 | `orangepi` / `orangepi` |
| 本机 sudo 密码 | `123456` |
| 板卡系统 | Armbian/Ubuntu，Linux 6.1.43-rockchip-rk3588，glibc 2.35 |
| 板卡 RKNPU 驱动 | 0.9.6（支持 `rknn_set_core_mask`） |
| 部署目录 | `/act_infer_rk3588` |
| 复测资产源 | `install/rk3588_linux_aarch64/act_infer_rk3588/`（由 `scripts/build-rk3588.sh` 产出） |

---

## 1. 建立 SSH 连接

板卡默认禁用空密码 key 登录，首次需用密码。`ssh-copy-id` 走 `ssh-askpass`
无法从 stdin 喂密码，最稳的是装 `sshpass`：

```bash
echo "123456" | sudo -S apt-get install -y sshpass

# 验证密码登录
sshpass -p orangepi ssh -o StrictHostKeyChecking=no orangepi@192.168.100.101 'hostname; uname -a'

# 安装公钥实现免密（后续所有命令不再需要 sshpass）
sshpass -p orangepi ssh-copy-id -o StrictHostKeyChecking=no -i ~/.ssh/id_ed25519.pub orangepi@192.168.100.101
```

之后直接 `ssh orangepi@192.168.100.101 '...'` 即可。

---

## 2. 部署资产到板卡

先 rsync 到 `/tmp`（普通用户可写），再用 sudo 移到目标路径并 `sync`：

```bash
cd proj57/rk3588
rsync -az --delete install/rk3588_linux_aarch64/act_infer_rk3588/ \
    orangepi@192.168.100.101:/tmp/act_infer_rk3588/

ssh orangepi@192.168.100.101 '
  set -e
  printf "%s\n" orangepi | sudo -S rm -rf /act_infer_rk3588
  printf "%s\n" orangepi | sudo -S mv /tmp/act_infer_rk3588 /act_infer_rk3588
  printf "%s\n" orangepi | sudo -S chown -R root:root /act_infer_rk3588
  printf "%s\n" orangepi | sudo -S sync
'
```

也可直接用 `scripts/deploy-to-board.sh`（`BOARD_IP=192.168.100.101 bash scripts/deploy-to-board.sh`），逻辑等价。

---

## 3. 关键坑 1：glibc 运行时冲突（原生 Linux 必改）

`build-rk3588.sh` 为 **StarryOS 的 musl rootfs** 把整套交叉工具链 glibc
（`libc.so.6` / `libstdc++.so.6` / `ld-linux-aarch64.so.1` 等）打进了
`install/.../lib/`。二进制 RUNPATH 为 `$ORIGIN/lib:$ORIGIN`。

在板卡**原生 Linux**（系统 glibc 2.35）上，这套 bundled glibc 会被 RUNPATH
优先加载，与系统 loader 版本错位，报：

```
symbol lookup error: .../lib/libc.so.6: undefined symbol: __tunable_is_initialized, version GLIBC_PRIVATE
```

修法：原生 Linux 用系统 glibc，`lib/` 里**只保留 `librknnrt.so`**，其余 glibc
库移走备份（StarryOS 复测可恢复）。二进制只需 GLIBC_2.34，系统 2.35 满足；
`librknnrt.so` 只需 GLIBC_2.17。

```bash
ssh orangepi@192.168.100.101 '
  printf "%s\n" orangepi | sudo -S sh -c "
    cd /act_infer_rk3588/lib
    mkdir -p /act_infer_rk3588/lib-starry-glibc
    for f in ld-linux-aarch64.so.1 libc.so.6 libdl.so.2 libgcc_s.so.1 \
             libm.so.6 libpthread.so.0 libstdc++.so.6 libstdc++.so.6.0.33; do
      [ -e \"\$f\" ] && cp -a \"\$f\" /act_infer_rk3588/lib-starry-glibc/ || true
    done
    rm -f ld-linux-aarch64.so.1 libc.so.6 libdl.so.2 libgcc_s.so.1 \
          libm.so.6 libpthread.so.0 libstdc++.so.6 libstdc++.so.6.0.33
    sync
  "
'
```

验证（应输出 usage，退出码 2 是缺参的预期；`ldd` 全部解析、无 not found）：

```bash
ssh orangepi@192.168.100.101 '
  /act_infer_rk3588/act-infer-golden-rknn --help
  LD_LIBRARY_PATH=/act_infer_rk3588/lib ldd /act_infer_rk3588/act-infer-review-rknn | grep -i "not found" || echo "all libs resolved"
'
```

---

## 4. 关键坑 2：板上启动脚本可能是旧版（不支持 ACT_OUTPUT_JSON）

`run-golden.sh` / `run-review.sh` 新版支持用环境变量覆盖输出 JSON 路径：

```sh
output_json="${ACT_OUTPUT_JSON:-/tmp/act_golden_result.json}"
```

如果板上是旧版，多条命令都会写同一个 `/tmp/act_*_result.json` 而**彼此覆盖**。
跑批量前先确认/更新脚本：

```bash
# 检查是否支持
ssh orangepi@192.168.100.101 'grep ACT_OUTPUT_JSON /act_infer_rk3588/run-golden.sh /act_infer_rk3588/run-review.sh'

# 不支持则推新版上去
rsync -az scripts/on-board-run-golden.sh scripts/on-board-run-review.sh orangepi@192.168.100.101:/tmp/
ssh orangepi@192.168.100.101 '
  printf "%s\n" orangepi | sudo -S cp /tmp/on-board-run-golden.sh /act_infer_rk3588/run-golden.sh
  printf "%s\n" orangepi | sudo -S cp /tmp/on-board-run-review.sh /act_infer_rk3588/run-review.sh
  printf "%s\n" orangepi | sudo -S chmod 0755 /act_infer_rk3588/run-golden.sh /act_infer_rk3588/run-review.sh
'
```

---

## 5. 跑基线测试（每条用唯一 ACT_OUTPUT_JSON）

NPU 是独占设备，命令**必须串行**，不能并发。每条指定不同输出路径避免覆盖：

```bash
ssh orangepi@192.168.100.101 'ACT_REPEAT=100 ACT_CORE_MASK=auto ACT_OUTPUT_JSON=/tmp/act_01_golden_auto.json       /act_infer_rk3588/run-golden.sh'
ssh orangepi@192.168.100.101 'ACT_REPEAT=100 ACT_CORE_MASK=all  ACT_OUTPUT_JSON=/tmp/act_02_golden_all.json        /act_infer_rk3588/run-golden.sh'
ssh orangepi@192.168.100.101 'ACT_REPEAT=100 ACT_CORE_MASK=auto ACT_OUTPUT_JSON=/tmp/act_03_review_left_auto.json   /act_infer_rk3588/run-review.sh left'
ssh orangepi@192.168.100.101 'ACT_REPEAT=100 ACT_CORE_MASK=all  ACT_OUTPUT_JSON=/tmp/act_04_review_left_all.json    /act_infer_rk3588/run-review.sh left'
ssh orangepi@192.168.100.101 'ACT_REPEAT=100 ACT_CORE_MASK=auto ACT_OUTPUT_JSON=/tmp/act_05_review_right_auto.json  /act_infer_rk3588/run-review.sh right'
ssh orangepi@192.168.100.101 'ACT_REPEAT=100 ACT_CORE_MASK=all  ACT_OUTPUT_JSON=/tmp/act_06_review_right_all.json   /act_infer_rk3588/run-review.sh right'
```

下载结果（注意：rsync 不展开**远端** glob，需整体加引号让远端 shell 展开）：

```bash
rsync -az "orangepi@192.168.100.101:/tmp/act_0*.json" proj57/rk3588/docs/log/
```

---

## 6. 关键坑 3：core-mask 在原生 Linux 上是否有效？

`infer_rknn.rs` 里 `rknn_set_core_mask` **失败也只打 warning 然后继续**
（按驱动默认核跑），所以"没报错"不等于"生效了"。判定方法：

- 跑一次看 stderr 是否出现 `rknn_set_core_mask warning: ret=...`。
  - 出现 → core mask **未生效**，`auto`/`all` 实际等价。
  - 不出现 → 生效。

本板（驱动 0.9.6）**未出现 warning，core mask 生效**：`auto` 与 `all`
的 `npu_run` 明显不同（auto ≈18ms，all ≈15ms，多核加速），两套参数都是有效基线。

> 结论：原生 Linux 上 `--core-mask auto|all` 填法与 StarryOS 一致，无需改参。
> 但换板/换驱动后应重新用上面的 warning 判据确认一次。

---

## 7. 速查清单

1. `sshpass` 装好 → `ssh-copy-id` 免密
2. rsync 到 `/tmp` → sudo mv 到 `/act_infer_rk3588` → sync
3. **剥掉 bundled glibc**，`lib/` 只留 `librknnrt.so`（原生 Linux 必做）
4. 确认启动脚本支持 `ACT_OUTPUT_JSON`，否则会互相覆盖
5. 串行跑各用例，每条唯一输出路径
6. rsync 回收用引号包住远端 glob
7. 用 `rknn_set_core_mask warning` 判据确认 core-mask 是否生效
