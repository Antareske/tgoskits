# 关于 Apache smoke 在 review 环境失败的回复

感谢 review 反馈。围绕 `start apache single process` 失败与 `AH00076 / errno 92`
做了定向排查，结论如下，并希望补充一些环境信息以闭合根因。

## 本地复现情况

按默认命令多次运行（含清理 rootfs 镜像与 app 构建缓存后重建）：

```bash
timeout 1800s cargo xtask starry app qemu -t apache --arch x86_64
```

本地均未能复现该失败，smoke 全程通过。本地从 Alpine 镜像拉到的 apache2 构建在
smoke 启动 httpd 时不会触发 `setsockopt(TCP_DEFER_ACCEPT)`，因此不出现 `AH00076`。

## 已定位的部分

1. 失败点 `start apache single process` 对应 smoke 脚本中的 `start_httpd`，
   它返回失败的唯一条件是 30 秒内对 `127.0.0.1:8080` 的 readiness curl 始终没有成功。
2. 反馈日志中与该阶段同时出现的唯一异常是
   `(92)Protocol not available: AH00076: Failed to enable APR_TCP_DEFER_ACCEPT`。
   该 errno 92（`ENOPROTOOPT`）来自 Apache/APR 建立监听 socket 时
   对 listen fd 调用 `setsockopt(IPPROTO_TCP, TCP_DEFER_ACCEPT, ...)`，
   当前 StarryOS 未实现该选项，落到默认分支返回 `ENOPROTOOPT`，与日志吻合。

## 针对“errno 92 是否为根因”的定向实验

为在不依赖具体 apache2/APR 构建的前提下隔离内核行为，新增了一个静态编译的
setsockopt 探针。它按 Apache/APR 的顺序执行
`bind -> listen -> setsockopt(TCP_DEFER_ACCEPT)`，并且 **setsockopt 失败后不中止**
（复刻 Apache 记录 `AH00076` 警告后继续监听的行为），随后由客户端对 `127.0.0.1`
发起真实连接、服务端 `accept` 并读取数据，以判断监听 socket 在 setsockopt 失败后
是否仍可用。该探针通过 debug 专用 qemu 配置以 `--qemu-config` 方式运行，
不影响默认 smoke / phase workflow。

两次运行的唯一变量是内核是否实现 `TCP_DEFER_ACCEPT`，其余条件一致：

未实现该选项的内核：

```text
TCP_DEFER_ACCEPT_SET_FAIL rc=-1 errno=92 (Protocol not available) (continuing, like Apache AH00076 warning)
CLIENT_CONNECT_OK
SERVER_ACCEPT_OK got=4 bytes payload="PING"
PROBE_RESULT_WARNING_ONLY
```

实现该选项的内核：

```text
TCP_DEFER_ACCEPT_SET_OK rc=0
CLIENT_CONNECT_OK
SERVER_ACCEPT_OK got=4 bytes payload="PING"
PROBE_RESULT_FIXED_OK
```

实验表明：即使 setsockopt 返回 errno 92，监听 socket 仍可正常 `accept` 连接。
因此 `AH00076` 在该场景下是一条警告，**不足以单独解释 readiness curl 的 30 秒超时**。
readiness 超时的真正根因可能另在网络时序、地址获取或环境相关因素。

## 请求补充信息（任一或多项）

由于本地无法复现，以下信息有助于确认根因：

1. 复现该失败的运行环境说明（host OS / 架构、qemu 版本、网络模式）。
2. 失败那次实际拉到的 apache2 / apr 包版本，以及 `httpd -v` 与 `httpd -V` 完整输出。
3. 失败那次 guest 内的完整诊断：`error.log`、httpd stdout 日志、readiness 阶段
   curl 的具体报错（connection refused 还是 timeout），以及当时监听 socket 状态
   （如 `ss -ltnp` 或等价输出）。
4. 若可行，在相同环境下运行上述 debug 探针配置并回贴 `VERDICT` 行，
   以确认监听 socket 在该环境下是否真的不可用。

相关 debug 探针与配置已随提交加入，便于在可复现环境中验证。
